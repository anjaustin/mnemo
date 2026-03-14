//! # mnemo-gnn
//!
//! Lightweight Graph Attention Network (GAT) re-ranking layer for Mnemo
//! retrieval. Operates on the local subgraph of candidate nodes returned by
//! fusion (RRF/MMR) and uses multi-head attention to re-score candidates based
//! on graph structure.
//!
//! **Design constraints:**
//! - Pure Rust, zero ML framework dependency
//! - Operates on 10-50 nodes (not the full graph) → <1ms latency
//! - Learns from feedback: which retrieved items the agent actually used
//! - Weights persist in Redis via JSON serialization

pub mod benchmark;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Number of attention heads in the GAT layer.
const NUM_HEADS: usize = 4;

/// Hidden dimension per head. Total hidden = NUM_HEADS * HEAD_DIM.
const HEAD_DIM: usize = 8;

/// LeakyReLU negative slope for attention coefficient computation.
const LEAKY_RELU_SLOPE: f32 = 0.2;

/// Learning rate for online weight updates from feedback.
const LEARNING_RATE: f32 = 0.01;

// ─── Types ─────────────────────────────────────────────────────────

/// A candidate node in the local subgraph, ready for GNN re-ranking.
#[derive(Debug, Clone)]
pub struct CandidateNode {
    /// The entity/edge/episode ID.
    pub id: Uuid,
    /// The fusion score from RRF or MMR (input relevance).
    pub fusion_score: f64,
    /// Feature vector for this node. For entities/edges, this is the embedding
    /// from Qdrant. If unavailable, a zero vector is used.
    pub features: Vec<f32>,
}

/// An edge in the local subgraph connecting two candidate nodes.
#[derive(Debug, Clone)]
pub struct SubgraphEdge {
    pub source_idx: usize,
    pub target_idx: usize,
    /// Edge weight (e.g., confidence of the fact, or 1.0 for structural edges).
    pub weight: f32,
}

/// The local subgraph extracted for GNN re-ranking.
#[derive(Debug, Clone)]
pub struct LocalSubgraph {
    pub nodes: Vec<CandidateNode>,
    pub edges: Vec<SubgraphEdge>,
}

/// Persisted GAT model weights. Small enough to store in Redis as JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatWeights {
    /// Per-head weight matrices W_h: maps input features to HEAD_DIM.
    /// Shape per head: [input_dim, HEAD_DIM].
    /// Stored flattened as [NUM_HEADS][input_dim * HEAD_DIM].
    pub w_heads: Vec<Vec<f32>>,

    /// Per-head attention vectors a_h: used to compute attention coefficients.
    /// Shape per head: [2 * HEAD_DIM] (concatenation of source and target projections).
    pub a_heads: Vec<Vec<f32>>,

    /// Output projection: maps concatenated multi-head output to a scalar score.
    /// Shape: [NUM_HEADS * HEAD_DIM].
    pub output_proj: Vec<f32>,

    /// Bias term for the output score.
    pub output_bias: f32,

    /// Input feature dimension this model was initialized for.
    pub input_dim: usize,

    /// Number of feedback updates applied to these weights.
    pub update_count: u64,
}

/// Result of GNN re-ranking: original IDs with updated scores.
#[derive(Debug, Clone)]
pub struct RerankedCandidate {
    pub id: Uuid,
    /// Original fusion score (preserved for blending).
    pub fusion_score: f64,
    /// GNN-computed score in [0, 1].
    pub gnn_score: f64,
    /// Blended final score: `alpha * fusion_score + (1 - alpha) * gnn_score`.
    pub final_score: f64,
}

// ─── GAT Implementation ───────────────────────────────────────────

impl GatWeights {
    /// Initialize with small random-like weights (deterministic seed for reproducibility).
    pub fn initialize(input_dim: usize) -> Self {
        let mut w_heads = Vec::with_capacity(NUM_HEADS);
        let mut a_heads = Vec::with_capacity(NUM_HEADS);

        for h in 0..NUM_HEADS {
            // Xavier-style initialization: scale = sqrt(2 / (fan_in + fan_out))
            let scale = (2.0 / (input_dim + HEAD_DIM) as f32).sqrt();
            let w: Vec<f32> = (0..input_dim * HEAD_DIM)
                .map(|i| {
                    // Deterministic pseudo-random using a simple hash
                    let seed = (h * 10000 + i) as f32;
                    (((seed * 2654435761.0) % 1000.0) / 1000.0 - 0.5) * 2.0 * scale
                })
                .collect();
            w_heads.push(w);

            let a_scale = (1.0 / (2 * HEAD_DIM) as f32).sqrt();
            let a: Vec<f32> = (0..2 * HEAD_DIM)
                .map(|i| {
                    let seed = ((h + NUM_HEADS) * 10000 + i) as f32;
                    (((seed * 2654435761.0) % 1000.0) / 1000.0 - 0.5) * 2.0 * a_scale
                })
                .collect();
            a_heads.push(a);
        }

        let out_dim = NUM_HEADS * HEAD_DIM;
        let out_scale = (1.0 / out_dim as f32).sqrt();
        let output_proj: Vec<f32> = (0..out_dim)
            .map(|i| {
                let seed = (NUM_HEADS * 20000 + i) as f32;
                (((seed * 2654435761.0) % 1000.0) / 1000.0 - 0.5) * 2.0 * out_scale
            })
            .collect();

        Self {
            w_heads,
            a_heads,
            output_proj,
            output_bias: 0.0,
            input_dim,
            update_count: 0,
        }
    }

    /// Run GAT forward pass on a local subgraph. Returns re-ranked candidates.
    ///
    /// `alpha` controls the blend between fusion score and GNN score.
    /// - `alpha = 1.0`: pure fusion (GNN disabled)
    /// - `alpha = 0.0`: pure GNN
    /// - `alpha = 0.5`: equal blend (recommended default)
    pub fn forward(&self, subgraph: &LocalSubgraph, alpha: f32) -> Vec<RerankedCandidate> {
        let n = subgraph.nodes.len();
        if n == 0 {
            return Vec::new();
        }

        // Build adjacency list (with self-loops)
        let mut adj: Vec<Vec<(usize, f32)>> = vec![Vec::new(); n];
        for (i, row) in adj.iter_mut().enumerate() {
            row.push((i, 1.0)); // self-loop
        }
        for edge in &subgraph.edges {
            if edge.source_idx < n && edge.target_idx < n {
                adj[edge.target_idx].push((edge.source_idx, edge.weight));
            }
        }

        // Project node features through each head
        let mut head_outputs: Vec<Vec<Vec<f32>>> = Vec::with_capacity(NUM_HEADS);

        for h in 0..NUM_HEADS {
            // Project all nodes: H_i = X_i * W_h
            let projected: Vec<Vec<f32>> = subgraph
                .nodes
                .iter()
                .map(|node| self.project_features(&node.features, h))
                .collect();

            // Compute attention-weighted aggregation for each node
            let mut aggregated = Vec::with_capacity(n);
            for i in 0..n {
                let neighbors = &adj[i];
                if neighbors.is_empty() {
                    aggregated.push(projected[i].clone());
                    continue;
                }

                // Compute attention coefficients
                let mut attn_scores: Vec<f32> = Vec::with_capacity(neighbors.len());
                for &(j, edge_w) in neighbors {
                    let score = self.attention_score(&projected[i], &projected[j], h) * edge_w;
                    attn_scores.push(score);
                }

                // Softmax over attention scores
                let max_score = attn_scores
                    .iter()
                    .cloned()
                    .fold(f32::NEG_INFINITY, f32::max);
                let exp_scores: Vec<f32> =
                    attn_scores.iter().map(|s| (s - max_score).exp()).collect();
                let sum_exp: f32 = exp_scores.iter().sum();

                // Weighted sum of neighbor projections
                let mut agg = vec![0.0f32; HEAD_DIM];
                for (k, &(j, _)) in neighbors.iter().enumerate() {
                    let alpha_k = exp_scores[k] / (sum_exp + 1e-10);
                    for (d, agg_d) in agg.iter_mut().enumerate() {
                        *agg_d += alpha_k * projected[j][d];
                    }
                }

                // ELU activation
                for val in agg.iter_mut() {
                    if *val < 0.0 {
                        *val = val.exp() - 1.0;
                    }
                }

                aggregated.push(agg);
            }

            head_outputs.push(aggregated);
        }

        // Concatenate heads and compute output score
        let mut results = Vec::with_capacity(n);
        for i in 0..n {
            // Concatenate all heads: [h0_d0, h0_d1, ..., h3_d7]
            let mut concat = Vec::with_capacity(NUM_HEADS * HEAD_DIM);
            for head_output in head_outputs.iter().take(NUM_HEADS) {
                concat.extend_from_slice(&head_output[i]);
            }

            // Linear projection to scalar
            let raw_score: f32 = concat
                .iter()
                .zip(self.output_proj.iter())
                .map(|(x, w)| x * w)
                .sum::<f32>()
                + self.output_bias;

            // Sigmoid to [0, 1]
            let gnn_score = sigmoid(raw_score) as f64;

            let fusion_score = subgraph.nodes[i].fusion_score;
            let final_score = alpha as f64 * fusion_score + (1.0 - alpha as f64) * gnn_score;

            results.push(RerankedCandidate {
                id: subgraph.nodes[i].id,
                fusion_score,
                gnn_score,
                final_score,
            });
        }

        // Sort by final_score descending
        results.sort_by(|a, b| {
            b.final_score
                .partial_cmp(&a.final_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results
    }

    /// Apply a simple online gradient update from feedback.
    ///
    /// `positive_ids` are the IDs the agent actually used (reward signal).
    /// `all_candidates` are all IDs that were returned by retrieval.
    /// This nudges the output projection to rank positive IDs higher.
    pub fn update_from_feedback(&mut self, subgraph: &LocalSubgraph, positive_ids: &[Uuid]) {
        let n = subgraph.nodes.len();
        if n == 0 || positive_ids.is_empty() {
            return;
        }

        let positive_set: std::collections::HashSet<Uuid> = positive_ids.iter().copied().collect();

        // Run forward to get current scores
        let results = self.forward(subgraph, 0.0); // pure GNN scores for gradient

        // Simple gradient: push positive items' scores up, negative items' scores down
        // We adjust output_proj based on the concatenated head features
        // This is a simplified online update — not full backprop

        // Re-compute head outputs for gradient (simplified: just adjust output layer)
        let head_features = self.compute_head_features(subgraph);

        for (i, result) in results.iter().enumerate() {
            let target = if positive_set.contains(&result.id) {
                1.0f32
            } else {
                0.0f32
            };
            let predicted = result.gnn_score as f32;
            let error = target - predicted;

            // Gradient of sigmoid cross-entropy w.r.t. output_proj
            let grad_scale = error * predicted * (1.0 - predicted) * LEARNING_RATE;

            if let Some(features) = head_features.get(i) {
                for (w, f) in self.output_proj.iter_mut().zip(features.iter()) {
                    *w += grad_scale * f;
                }
                self.output_bias += grad_scale;
            }
        }

        self.update_count += 1;
    }

    // ── Private helpers ────────────────────────────────────────

    /// Project node features through head h's weight matrix.
    fn project_features(&self, features: &[f32], head: usize) -> Vec<f32> {
        let w = &self.w_heads[head];
        let input_dim = self.input_dim.min(features.len());
        let mut projected = vec![0.0f32; HEAD_DIM];
        for d in 0..HEAD_DIM {
            for j in 0..input_dim {
                projected[d] += features[j] * w[j * HEAD_DIM + d];
            }
        }
        projected
    }

    /// Compute attention score between source and target projections using head h.
    fn attention_score(&self, source: &[f32], target: &[f32], head: usize) -> f32 {
        let a = &self.a_heads[head];
        let mut score = 0.0f32;
        for d in 0..HEAD_DIM {
            score += a[d] * source[d];
            score += a[HEAD_DIM + d] * target[d];
        }
        leaky_relu(score)
    }

    /// Compute concatenated head features for all nodes (used during feedback update).
    fn compute_head_features(&self, subgraph: &LocalSubgraph) -> Vec<Vec<f32>> {
        subgraph
            .nodes
            .iter()
            .map(|node| {
                let mut concat = Vec::with_capacity(NUM_HEADS * HEAD_DIM);
                for h in 0..NUM_HEADS {
                    concat.extend_from_slice(&self.project_features(&node.features, h));
                }
                concat
            })
            .collect()
    }
}

// ─── Activation functions ──────────────────────────────────────────

fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

fn leaky_relu(x: f32) -> f32 {
    if x >= 0.0 {
        x
    } else {
        LEAKY_RELU_SLOPE * x
    }
}

// ─── Subgraph construction helper ──────────────────────────────────

/// Build a local subgraph from candidate IDs and a graph adjacency lookup.
///
/// `candidates`: the fused candidate list (IDs + fusion scores).
/// `graph_edges`: edges between entities known to the system.
/// `features_map`: pre-fetched embeddings for each candidate ID.
///
/// Returns a `LocalSubgraph` with nodes and edges connecting candidates
/// that share graph relationships.
pub fn build_local_subgraph(
    candidates: &[(Uuid, f64)],
    graph_edges: &[(Uuid, Uuid, f32)], // (source_entity_id, target_entity_id, confidence)
    features_map: &HashMap<Uuid, Vec<f32>>,
    default_dim: usize,
) -> LocalSubgraph {
    let id_to_idx: HashMap<Uuid, usize> = candidates
        .iter()
        .enumerate()
        .map(|(i, (id, _))| (*id, i))
        .collect();

    let nodes: Vec<CandidateNode> = candidates
        .iter()
        .map(|(id, score)| CandidateNode {
            id: *id,
            fusion_score: *score,
            features: features_map
                .get(id)
                .cloned()
                .unwrap_or_else(|| vec![0.0; default_dim]),
        })
        .collect();

    let mut edges = Vec::new();
    for (src, tgt, weight) in graph_edges {
        if let (Some(&src_idx), Some(&tgt_idx)) = (id_to_idx.get(src), id_to_idx.get(tgt)) {
            edges.push(SubgraphEdge {
                source_idx: src_idx,
                target_idx: tgt_idx,
                weight: *weight,
            });
            // Bidirectional
            edges.push(SubgraphEdge {
                source_idx: tgt_idx,
                target_idx: src_idx,
                weight: *weight,
            });
        }
    }

    LocalSubgraph { nodes, edges }
}

// ─── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_candidates(n: usize) -> Vec<CandidateNode> {
        (0..n)
            .map(|i| CandidateNode {
                id: Uuid::from_u128(i as u128 + 1),
                fusion_score: 1.0 - (i as f64 * 0.1),
                features: vec![i as f32 * 0.1; 16],
            })
            .collect()
    }

    fn dummy_subgraph(n: usize) -> LocalSubgraph {
        let nodes = dummy_candidates(n);
        // Chain: 0->1->2->...->n-1
        let edges: Vec<SubgraphEdge> = (0..n.saturating_sub(1))
            .map(|i| SubgraphEdge {
                source_idx: i,
                target_idx: i + 1,
                weight: 0.9,
            })
            .collect();
        LocalSubgraph { nodes, edges }
    }

    #[test]
    fn test_gat_weights_initialize() {
        let w = GatWeights::initialize(16);
        assert_eq!(w.w_heads.len(), NUM_HEADS);
        assert_eq!(w.a_heads.len(), NUM_HEADS);
        assert_eq!(w.w_heads[0].len(), 16 * HEAD_DIM);
        assert_eq!(w.a_heads[0].len(), 2 * HEAD_DIM);
        assert_eq!(w.output_proj.len(), NUM_HEADS * HEAD_DIM);
        assert_eq!(w.input_dim, 16);
        assert_eq!(w.update_count, 0);
    }

    #[test]
    fn test_forward_empty_subgraph() {
        let w = GatWeights::initialize(16);
        let sg = LocalSubgraph {
            nodes: Vec::new(),
            edges: Vec::new(),
        };
        let results = w.forward(&sg, 0.5);
        assert!(results.is_empty());
    }

    #[test]
    fn test_forward_single_node() {
        let w = GatWeights::initialize(16);
        let sg = LocalSubgraph {
            nodes: vec![CandidateNode {
                id: Uuid::from_u128(1),
                fusion_score: 0.8,
                features: vec![0.5; 16],
            }],
            edges: Vec::new(),
        };
        let results = w.forward(&sg, 0.5);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, Uuid::from_u128(1));
        assert!(results[0].gnn_score >= 0.0 && results[0].gnn_score <= 1.0);
        // Blended score should be between gnn and fusion
        let expected = 0.5 * 0.8 + 0.5 * results[0].gnn_score;
        assert!((results[0].final_score - expected).abs() < 1e-6);
    }

    #[test]
    fn test_forward_preserves_all_candidates() {
        let w = GatWeights::initialize(16);
        let sg = dummy_subgraph(10);
        let results = w.forward(&sg, 0.5);
        assert_eq!(results.len(), 10);
        // All original IDs should be present
        let result_ids: std::collections::HashSet<Uuid> = results.iter().map(|r| r.id).collect();
        for node in &sg.nodes {
            assert!(result_ids.contains(&node.id));
        }
    }

    #[test]
    fn test_forward_sorted_by_final_score_descending() {
        let w = GatWeights::initialize(16);
        let sg = dummy_subgraph(10);
        let results = w.forward(&sg, 0.5);
        for pair in results.windows(2) {
            assert!(pair[0].final_score >= pair[1].final_score);
        }
    }

    #[test]
    fn test_forward_pure_fusion_alpha_1() {
        let w = GatWeights::initialize(16);
        let sg = dummy_subgraph(5);
        let results = w.forward(&sg, 1.0);
        // With alpha=1.0, final_score == fusion_score
        for r in &results {
            assert!(
                (r.final_score - r.fusion_score).abs() < 1e-6,
                "alpha=1.0 should yield pure fusion: final={}, fusion={}",
                r.final_score,
                r.fusion_score
            );
        }
    }

    #[test]
    fn test_forward_pure_gnn_alpha_0() {
        let w = GatWeights::initialize(16);
        let sg = dummy_subgraph(5);
        let results = w.forward(&sg, 0.0);
        // With alpha=0.0, final_score == gnn_score
        for r in &results {
            assert!(
                (r.final_score - r.gnn_score).abs() < 1e-6,
                "alpha=0.0 should yield pure GNN: final={}, gnn={}",
                r.final_score,
                r.gnn_score
            );
        }
    }

    #[test]
    fn test_gnn_scores_bounded_0_to_1() {
        let w = GatWeights::initialize(16);
        let sg = dummy_subgraph(20);
        let results = w.forward(&sg, 0.5);
        for r in &results {
            assert!(
                r.gnn_score >= 0.0 && r.gnn_score <= 1.0,
                "GNN score must be in [0,1], got {}",
                r.gnn_score
            );
        }
    }

    #[test]
    fn test_feedback_update_changes_weights() {
        let mut w = GatWeights::initialize(16);
        let sg = dummy_subgraph(5);
        let original_proj = w.output_proj.clone();
        let _original_bias = w.output_bias;

        // Provide feedback: node 0 was useful
        w.update_from_feedback(&sg, &[Uuid::from_u128(1)]);

        assert_eq!(w.update_count, 1);
        // Weights should have changed
        assert_ne!(
            w.output_proj, original_proj,
            "weights must change after feedback"
        );
        // Bias may change too (but check update_count as primary signal)
    }

    #[test]
    fn test_feedback_update_improves_positive_ranking() {
        let mut w = GatWeights::initialize(16);
        let sg = dummy_subgraph(5);
        let positive_id = sg.nodes[3].id; // originally rank 4

        // Apply feedback 50 times saying node 3 was the best
        for _ in 0..50 {
            w.update_from_feedback(&sg, &[positive_id]);
        }

        let results = w.forward(&sg, 0.0); // pure GNN
        let pos_rank = results.iter().position(|r| r.id == positive_id).unwrap();
        // After 50 feedback rounds, node 3 should have moved up (lower rank number)
        assert!(
            pos_rank < 4,
            "After 50 feedback rounds, positive node should rank better than 4th, got {}",
            pos_rank
        );
    }

    #[test]
    fn test_serialization_roundtrip() {
        let w = GatWeights::initialize(384);
        let json = serde_json::to_string(&w).unwrap();
        let w2: GatWeights = serde_json::from_str(&json).unwrap();
        assert_eq!(w.w_heads.len(), w2.w_heads.len());
        assert_eq!(w.input_dim, w2.input_dim);
        assert_eq!(w.output_proj.len(), w2.output_proj.len());
        for i in 0..w.output_proj.len() {
            assert!((w.output_proj[i] - w2.output_proj[i]).abs() < 1e-7);
        }
    }

    #[test]
    fn test_build_local_subgraph() {
        let candidates = vec![
            (Uuid::from_u128(1), 0.9),
            (Uuid::from_u128(2), 0.8),
            (Uuid::from_u128(3), 0.7),
        ];
        let graph_edges = vec![
            (Uuid::from_u128(1), Uuid::from_u128(2), 0.95),
            (Uuid::from_u128(2), Uuid::from_u128(3), 0.85),
        ];
        let mut features_map = HashMap::new();
        features_map.insert(Uuid::from_u128(1), vec![0.1; 16]);
        features_map.insert(Uuid::from_u128(2), vec![0.2; 16]);

        let sg = build_local_subgraph(&candidates, &graph_edges, &features_map, 16);

        assert_eq!(sg.nodes.len(), 3);
        assert_eq!(sg.edges.len(), 4); // 2 edges * 2 (bidirectional)
                                       // Node 3 should have zero features (not in features_map)
        assert_eq!(sg.nodes[2].features, vec![0.0; 16]);
    }

    #[test]
    fn test_build_local_subgraph_no_edges() {
        let candidates = vec![(Uuid::from_u128(1), 0.9), (Uuid::from_u128(2), 0.8)];
        let features_map = HashMap::new();
        let sg = build_local_subgraph(&candidates, &[], &features_map, 8);
        assert_eq!(sg.nodes.len(), 2);
        assert!(sg.edges.is_empty());
    }

    #[test]
    fn test_build_local_subgraph_ignores_external_edges() {
        let candidates = vec![(Uuid::from_u128(1), 0.9)];
        // Edge to a node NOT in candidates — should be ignored
        let graph_edges = vec![(Uuid::from_u128(1), Uuid::from_u128(99), 0.5)];
        let features_map = HashMap::new();
        let sg = build_local_subgraph(&candidates, &graph_edges, &features_map, 8);
        assert_eq!(sg.nodes.len(), 1);
        assert!(sg.edges.is_empty());
    }
}
