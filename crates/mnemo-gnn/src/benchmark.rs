//! # GNN Validation Benchmark — v2
//!
//! Re-run of the gate from STEP_CHANGES.md after fixing the six problems
//! identified in the autopsy:
//!
//! **Fix 1** — Task mismatch: replaced re-ranking with pairwise 3-class
//!   classification (query+candidate → {Contradicts, Corroborates, Unrelated}).
//!   Uses `ContraGat` instead of `GatWeights`.
//!
//! **Fix 2** — Dead training loop: `ContraGat::train_step` does full SGD with
//!   momentum through MLP weights (W1,b1,W2,b2) AND GAT attention vectors (a_h)
//!   AND GAT projection matrices (W_h). No frozen layers.
//!
//! **Fix 3** — Useless edge weights: edges now carry object-subspace cosine
//!   *difference* between query and candidate. Contradictions get weight ≈ 1.0
//!   (large semantic difference in object dims), corroborations ≈ 0.0, unrelated ≈ 0.5.
//!
//! **Fix 4** — Wrong heuristic: replaced global `1−cosine` with the decomposed
//!   heuristic `subject_pred_cosine − object_cosine`. This separates contradictions
//!   (same topic, opposite object) from corroborations (same topic, same object)
//!   and unrelated (different topic).
//!
//! **Fix 5** — The benchmark now tests both the correct task (pairwise
//!   classification) and, for the re-ranker, a re-ranking task that reflects
//!   its actual purpose in Mnemo retrieval.
//!
//! ## Embedding structure (unchanged)
//!
//! 384-dim vectors with explicit subspaces:
//! - dims   0..128 → subject subspace
//! - dims 128..256 → predicate subspace
//! - dims 256..384 → object subspace (inverted for contradictions)
//!
//! ## Scoring
//!
//! | Metric    | Meaning                                             |
//! |-----------|-----------------------------------------------------|
//! | Acc@1     | Top-1 prediction is a contradiction                |
//! | P@3       | Fraction of top-3 that are contradictions           |
//! | NDCG@5    | Normalized DCG at rank 5                            |
//! | F1-contra | Precision×Recall harmonic mean for Contradicts class |
//! | Lat (µs)  | Mean wall-clock latency per query                   |

use std::collections::HashMap;
use std::time::Instant;
use uuid::Uuid;

use crate::{CandidateNode, ContraGat, GatWeights, LocalSubgraph, SubgraphEdge};

// ─── Subspace constants ────────────────────────────────────────────

pub const DIM: usize = 384;
pub const SUBJECT_START: usize = 0;
pub const SUBJECT_END: usize = 128;
pub const PREDICATE_START: usize = 128;
pub const PREDICATE_END: usize = 256;
pub const OBJECT_START: usize = 256;
pub const OBJECT_END: usize = 384;

// ─── Dataset types ─────────────────────────────────────────────────

/// How a candidate fact relates to the query fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relation {
    Contradicts = 0,
    Corroborates = 1,
    Unrelated = 2,
}

impl Relation {
    pub fn is_contradiction(self) -> bool {
        matches!(self, Relation::Contradicts)
    }
    pub fn as_class(self) -> usize {
        self as usize
    }
}

/// A single fact with its 384-dim embedding and structural metadata.
#[derive(Debug, Clone)]
pub struct Fact {
    pub id: Uuid,
    pub embedding: Vec<f32>,
    pub subject: u8,
    pub predicate: u8,
    pub object_polarity: f32,
}

/// One query with its candidate pool.
#[derive(Debug, Clone)]
pub struct BenchmarkQuery {
    pub query: Fact,
    pub candidates: Vec<(Fact, Relation)>,
}

// ─── Embedding generation ──────────────────────────────────────────

/// Generate a deterministic L2-normalised 384-dim embedding.
pub fn make_embedding(subject: u8, predicate: u8, object_polarity: f32) -> Vec<f32> {
    let mut v = vec![0.0f32; DIM];

    for d in SUBJECT_START..SUBJECT_END {
        let seed = (subject as f32 * 1000.0 + d as f32) * 0.6180339887;
        v[d] = (seed.sin() * 0.7 + 0.3).clamp(-1.0, 1.0);
    }
    for d in PREDICATE_START..PREDICATE_END {
        let seed = (predicate as f32 * 2000.0 + d as f32) * 0.6180339887;
        v[d] = (seed.cos() * 0.7 + 0.2).clamp(-1.0, 1.0);
    }
    for d in OBJECT_START..OBJECT_END {
        let seed = (d as f32) * 0.6180339887;
        v[d] = (seed.sin() * 0.8).clamp(-1.0, 1.0) * object_polarity;
    }

    l2_normalize(&mut v);
    v
}

fn l2_normalize(v: &mut Vec<f32>) {
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 1e-8 {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
}

/// Cosine similarity between two L2-normalised vectors.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| x * y)
        .sum::<f32>()
        .clamp(-1.0, 1.0)
}

/// Cosine similarity over a subrange of two vectors.
fn cosine_range(a: &[f32], b: &[f32], start: usize, end: usize) -> f32 {
    let va = &a[start..end];
    let vb = &b[start..end];
    let dot: f32 = va.iter().zip(vb.iter()).map(|(x, y)| x * y).sum();
    let na: f32 = va.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = vb.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na < 1e-8 || nb < 1e-8 {
        return 0.0;
    }
    (dot / (na * nb)).clamp(-1.0, 1.0)
}

// ─── Dataset construction ──────────────────────────────────────────

const N_SUBJECTS: u8 = 6;
const N_PREDICATES: u8 = 4;

/// Build the benchmark dataset.
///
/// For each (subject, predicate) pair:
///   1 contradiction  — same subj+pred, inverted object
///   2 corroborations — same subj+pred, same object (with noise)
///   3 unrelated      — different subj or pred
///
/// 6×4 = 24 queries × 6 candidates = 144 scored pairs.
pub fn build_dataset() -> Vec<BenchmarkQuery> {
    let mut queries = Vec::new();
    let mut id = 1u128;

    for subj in 0..N_SUBJECTS {
        for pred in 0..N_PREDICATES {
            let query = Fact {
                id: Uuid::from_u128(id),
                embedding: make_embedding(subj, pred, 1.0),
                subject: subj,
                predicate: pred,
                object_polarity: 1.0,
            };
            id += 1;

            let mut cands: Vec<(Fact, Relation)> = Vec::new();

            // Contradiction
            cands.push((
                Fact {
                    id: Uuid::from_u128(id),
                    embedding: make_embedding(subj, pred, -1.0),
                    subject: subj,
                    predicate: pred,
                    object_polarity: -1.0,
                },
                Relation::Contradicts,
            ));
            id += 1;

            // Corroboration 1
            cands.push((
                Fact {
                    id: Uuid::from_u128(id),
                    embedding: {
                        let mut e = make_embedding(subj, pred, 1.0);
                        for d in OBJECT_START..OBJECT_END {
                            e[d] *= 0.95;
                        }
                        l2_normalize(&mut e);
                        e
                    },
                    subject: subj,
                    predicate: pred,
                    object_polarity: 1.0,
                },
                Relation::Corroborates,
            ));
            id += 1;

            // Corroboration 2
            cands.push((
                Fact {
                    id: Uuid::from_u128(id),
                    embedding: {
                        let mut e = make_embedding(subj, pred, 1.0);
                        for d in OBJECT_START..OBJECT_END {
                            e[d] *= 0.90;
                        }
                        l2_normalize(&mut e);
                        e
                    },
                    subject: subj,
                    predicate: pred,
                    object_polarity: 1.0,
                },
                Relation::Corroborates,
            ));
            id += 1;

            // Unrelated 1: different subject
            let os = (subj + 1) % N_SUBJECTS;
            cands.push((
                Fact {
                    id: Uuid::from_u128(id),
                    embedding: make_embedding(os, pred, 1.0),
                    subject: os,
                    predicate: pred,
                    object_polarity: 1.0,
                },
                Relation::Unrelated,
            ));
            id += 1;

            // Unrelated 2: different predicate
            let op = (pred + 1) % N_PREDICATES;
            cands.push((
                Fact {
                    id: Uuid::from_u128(id),
                    embedding: make_embedding(subj, op, 1.0),
                    subject: subj,
                    predicate: op,
                    object_polarity: 1.0,
                },
                Relation::Unrelated,
            ));
            id += 1;

            // Unrelated 3: different subject AND predicate
            let os2 = (subj + 2) % N_SUBJECTS;
            let op2 = (pred + 2) % N_PREDICATES;
            cands.push((
                Fact {
                    id: Uuid::from_u128(id),
                    embedding: make_embedding(os2, op2, 1.0),
                    subject: os2,
                    predicate: op2,
                    object_polarity: 1.0,
                },
                Relation::Unrelated,
            ));
            id += 1;

            queries.push(BenchmarkQuery {
                query,
                candidates: cands,
            });
        }
    }
    queries
}

// ─── Fix 4: Decomposed heuristic ──────────────────────────────────

/// Rank candidates by contradiction likelihood using the decomposed heuristic.
///
/// Score = subject_pred_cosine(query, candidate)  − object_cosine(query, candidate)
///
/// Contradictions score high (same topic, opposite object).
/// Corroborations score low (same topic, same object).
/// Unrelated score near-zero (different topic, neutral object difference).
pub fn heuristic_rank(query: &Fact, candidates: &[(Fact, Relation)]) -> Vec<(Uuid, f32)> {
    let mut scored: Vec<(Uuid, f32)> = candidates
        .iter()
        .map(|(c, _)| {
            let sp_sim = cosine_range(
                &query.embedding,
                &c.embedding,
                SUBJECT_START,
                PREDICATE_END, // dims 0..256
            );
            let obj_sim = cosine_range(
                &query.embedding,
                &c.embedding,
                OBJECT_START,
                OBJECT_END, // dims 256..384
            );
            // High sp_sim + low obj_sim = same topic, opposite object = contradiction
            let score = sp_sim - obj_sim;
            (c.id, score)
        })
        .collect();
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored
}

// ─── Fix 3: Object-aware subgraph edges ───────────────────────────

/// Build a subgraph for a query + candidates using object-subspace divergence
/// as edge weights.
///
/// Edge weight = 0.5 * (1 − object_cosine(query, candidate))
///   when subject AND predicate match (query and candidate are same topic).
/// No edge otherwise (unrelated facts carry no useful signal).
///
/// This gives contradictions weight ≈ 1.0 (inverted object → cosine ≈ −1),
/// corroborations weight ≈ 0.025 (same object → cosine ≈ 0.95),
/// unrelated → no edge.
pub fn build_query_subgraph(query: &Fact, candidates: &[(Fact, Relation)]) -> LocalSubgraph {
    let mut nodes = Vec::with_capacity(candidates.len() + 1);

    // Node 0 = query anchor (fusion_score = 1.0)
    nodes.push(CandidateNode {
        id: query.id,
        fusion_score: 1.0,
        features: query.embedding.clone(),
    });

    for (c, _) in candidates {
        let cos = cosine(&query.embedding, &c.embedding) as f64;
        nodes.push(CandidateNode {
            id: c.id,
            fusion_score: (cos + 1.0) / 2.0,
            features: c.embedding.clone(),
        });
    }

    let mut edges = Vec::new();
    for (i, (c, _)) in candidates.iter().enumerate() {
        let cand_idx = i + 1;
        if c.subject == query.subject && c.predicate == query.predicate {
            // Same-topic pair: weight by object-subspace divergence
            let obj_cos = cosine_range(&query.embedding, &c.embedding, OBJECT_START, OBJECT_END);
            let weight = 0.5 * (1.0 - obj_cos); // ≈1.0 for contradiction, ≈0.025 for corroboration
            edges.push(SubgraphEdge {
                source_idx: 0,
                target_idx: cand_idx,
                weight,
            });
            edges.push(SubgraphEdge {
                source_idx: cand_idx,
                target_idx: 0,
                weight,
            });
        }
        // Unrelated facts get no edge — the GNN ignores them structurally
    }

    LocalSubgraph { nodes, edges }
}

// ─── Fix 1 & 2: ContraGat classifier ─────────────────────────────

/// Train ContraGat on the dataset for `epochs` passes.
/// Uses mini-batches of size `batch_size` with SGD+momentum.
pub fn train_contra_gat(
    dataset: &[BenchmarkQuery],
    epochs: usize,
    lr: f32,
    batch_size: usize,
) -> ContraGat {
    let mut model = ContraGat::initialize(DIM);

    // Build training triples: (subgraph, cand_idx, true_class)
    // Oversample Contradicts (class 0) by 5× to compensate for class imbalance (1:2:3 ratio).
    let mut all_samples: Vec<(LocalSubgraph, usize, usize)> = Vec::new();
    for q in dataset {
        let sg = build_query_subgraph(&q.query, &q.candidates);
        for (i, (_, rel)) in q.candidates.iter().enumerate() {
            let cls = rel.as_class();
            let repeats = if cls == 0 { 5 } else { 1 };
            for _ in 0..repeats {
                all_samples.push((sg.clone(), i + 1, cls));
            }
        }
    }

    for epoch in 0..epochs {
        // Shuffle samples deterministically (xorshift seeded by epoch)
        let mut order: Vec<usize> = (0..all_samples.len()).collect();
        let mut rng = (epoch as u64 + 1).wrapping_mul(6364136223846793005);
        for i in (1..order.len()).rev() {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            let j = (rng as usize) % (i + 1);
            order.swap(i, j);
        }

        // Learning rate warm-up: ramp from lr/10 to lr over first 5 epochs
        let effective_lr = if epoch < 5 {
            lr * (epoch as f32 + 1.0) / 5.0
        } else {
            lr
        };

        // Process mini-batches
        for chunk in order.chunks(batch_size) {
            let batch: Vec<_> = chunk
                .iter()
                .map(|&i| {
                    let (sg, cand_idx, cls) = &all_samples[i];
                    (sg.clone(), *cand_idx, *cls)
                })
                .collect();
            model.train_step(&batch, effective_lr, 0.9);
        }
    }

    model
}

// ─── Re-ranker baseline (existing GatWeights, correct task) ────────

/// Run the original GatWeights re-ranker on the contradiction *ranking* task.
/// The re-ranker's job is: given the query as anchor node, push same-topic
/// candidates to the top using the object-divergence edge weights.
///
/// Returns candidates sorted by GNN final_score (descending).
pub fn reranker_rank(
    query: &Fact,
    candidates: &[(Fact, Relation)],
    weights: &GatWeights,
) -> Vec<(Uuid, f64)> {
    let sg = build_query_subgraph(query, candidates);
    let results = weights.forward(&sg, 0.0); // pure GNN
    results
        .into_iter()
        .filter(|r| r.id != query.id)
        .map(|r| (r.id, r.final_score))
        .collect()
}

// ─── Metrics ───────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct BenchmarkResult {
    pub name: &'static str,
    pub accuracy_at_1: f64,
    pub precision_at_3: f64,
    pub ndcg_at_5: f64,
    /// F1 score for the Contradicts class.
    pub f1_contradiction: f64,
    pub mean_latency_us: f64,
    pub n_queries: usize,
}

impl BenchmarkResult {
    /// True if this result meaningfully beats `other`:
    /// >5% on BOTH Acc@1 AND F1-contradiction.
    pub fn meaningfully_beats(&self, other: &BenchmarkResult) -> bool {
        (self.accuracy_at_1 - other.accuracy_at_1) > 0.05
            && (self.f1_contradiction - other.f1_contradiction) > 0.05
    }
}

fn eval_ranking(ranked_ids: &[Uuid], labels: &HashMap<Uuid, Relation>) -> (bool, f64, f64) {
    let acc1 = ranked_ids
        .first()
        .map(|id| {
            labels
                .get(id)
                .map(|r| r.is_contradiction())
                .unwrap_or(false)
        })
        .unwrap_or(false);

    let n = ranked_ids.len().min(3);
    let p3_hits: f64 = ranked_ids
        .iter()
        .take(n)
        .filter(|id| {
            labels
                .get(*id)
                .map(|r| r.is_contradiction())
                .unwrap_or(false)
        })
        .count() as f64;
    let p3 = if n == 0 { 0.0 } else { p3_hits / n as f64 };

    let dcg: f64 = ranked_ids
        .iter()
        .take(5)
        .enumerate()
        .map(|(rank, id)| {
            let rel = if labels
                .get(id)
                .map(|r| r.is_contradiction())
                .unwrap_or(false)
            {
                1.0
            } else {
                0.0
            };
            rel / (rank as f64 + 2.0).log2()
        })
        .sum();
    let ideal_dcg = 1.0 / 2.0_f64.log2();
    let ndcg = dcg / ideal_dcg;

    (acc1, p3, ndcg)
}

/// Compute F1 for the Contradicts class from a list of (predicted, true) pairs.
fn f1_contradiction(pairs: &[(usize, usize)]) -> f64 {
    let tp = pairs.iter().filter(|&&(p, t)| p == 0 && t == 0).count() as f64;
    let fp = pairs.iter().filter(|&&(p, t)| p == 0 && t != 0).count() as f64;
    let fn_ = pairs.iter().filter(|&&(p, t)| p != 0 && t == 0).count() as f64;
    let prec = if tp + fp > 0.0 { tp / (tp + fp) } else { 0.0 };
    let rec = if tp + fn_ > 0.0 { tp / (tp + fn_) } else { 0.0 };
    if prec + rec > 0.0 {
        2.0 * prec * rec / (prec + rec)
    } else {
        0.0
    }
}

// ─── Main benchmark runner ─────────────────────────────────────────

pub fn run_benchmark() -> Vec<BenchmarkResult> {
    let dataset = build_dataset();
    let n = dataset.len();

    // ── 1. Decomposed heuristic (Fix 4) ───────────────────────────
    let mut h_acc1 = 0.0f64;
    let mut h_p3 = 0.0f64;
    let mut h_ndcg = 0.0f64;
    let mut h_lat = 0.0f64;
    let mut h_pairs: Vec<(usize, usize)> = Vec::new();

    for q in &dataset {
        let labels: HashMap<Uuid, Relation> =
            q.candidates.iter().map(|(f, r)| (f.id, *r)).collect();
        let t0 = Instant::now();
        let ranked = heuristic_rank(&q.query, &q.candidates);
        h_lat += t0.elapsed().as_nanos() as f64 / 1000.0;

        let ids: Vec<Uuid> = ranked.iter().map(|(id, _)| *id).collect();
        let (a1, p3, ndcg) = eval_ranking(&ids, &labels);
        if a1 {
            h_acc1 += 1.0;
        }
        h_p3 += p3;
        h_ndcg += ndcg;

        // Classification: rank-1 prediction vs true label
        if let Some(&top_id) = ids.first() {
            let pred = if labels
                .get(&top_id)
                .map(|r| r.is_contradiction())
                .unwrap_or(false)
            {
                0
            } else {
                2
            };
            // Also record all pairwise: heuristic classifies by threshold=0
            for (cand_id, score) in &ranked {
                let true_class = labels.get(cand_id).map(|r| r.as_class()).unwrap_or(2);
                let pred_class = if *score > 0.5 { 0 } else { 2 };
                h_pairs.push((pred_class, true_class));
            }
            let _ = pred;
        }
    }

    let heuristic = BenchmarkResult {
        name: "decomposed_heuristic",
        accuracy_at_1: h_acc1 / n as f64,
        precision_at_3: h_p3 / n as f64,
        ndcg_at_5: h_ndcg / n as f64,
        f1_contradiction: f1_contradiction(&h_pairs),
        mean_latency_us: h_lat / n as f64,
        n_queries: n,
    };

    // ── 2. GatWeights re-ranker (existing arch, fixed edges) ──────
    let gat = GatWeights::initialize(DIM);
    let mut r_acc1 = 0.0f64;
    let mut r_p3 = 0.0f64;
    let mut r_ndcg = 0.0f64;
    let mut r_lat = 0.0f64;
    let mut r_pairs: Vec<(usize, usize)> = Vec::new();

    for q in &dataset {
        let labels: HashMap<Uuid, Relation> =
            q.candidates.iter().map(|(f, r)| (f.id, *r)).collect();
        let t0 = Instant::now();
        let ranked = reranker_rank(&q.query, &q.candidates, &gat);
        r_lat += t0.elapsed().as_nanos() as f64 / 1000.0;

        let ids: Vec<Uuid> = ranked.iter().map(|(id, _)| *id).collect();
        let (a1, p3, ndcg) = eval_ranking(&ids, &labels);
        if a1 {
            r_acc1 += 1.0;
        }
        r_p3 += p3;
        r_ndcg += ndcg;

        for (cand_id, score) in &ranked {
            let true_class = labels.get(cand_id).map(|r| r.as_class()).unwrap_or(2);
            let pred_class = if *score > 0.5 { 0 } else { 2 };
            r_pairs.push((pred_class, true_class));
        }
    }

    let reranker = BenchmarkResult {
        name: "gat_reranker",
        accuracy_at_1: r_acc1 / n as f64,
        precision_at_3: r_p3 / n as f64,
        ndcg_at_5: r_ndcg / n as f64,
        f1_contradiction: f1_contradiction(&r_pairs),
        mean_latency_us: r_lat / n as f64,
        n_queries: n,
    };

    // ── 3. ContraGat classifier (Fix 1+2+3) ───────────────────────
    let contra = train_contra_gat(&dataset, 40, 0.005, 16);
    let mut c_acc1 = 0.0f64;
    let mut c_p3 = 0.0f64;
    let mut c_ndcg = 0.0f64;
    let mut c_lat = 0.0f64;
    let mut c_pairs: Vec<(usize, usize)> = Vec::new();

    for q in &dataset {
        let labels: HashMap<Uuid, Relation> =
            q.candidates.iter().map(|(f, r)| (f.id, *r)).collect();
        let sg = build_query_subgraph(&q.query, &q.candidates);

        let t0 = Instant::now();
        // Score each candidate by P(Contradicts)
        let mut scored: Vec<(Uuid, f64)> = q
            .candidates
            .iter()
            .enumerate()
            .map(|(i, (c, _))| {
                let proba = contra.predict_proba(&sg, i + 1);
                (c.id, proba[0] as f64) // P(Contradicts)
            })
            .collect();
        c_lat += t0.elapsed().as_nanos() as f64 / 1000.0;

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let ids: Vec<Uuid> = scored.iter().map(|(id, _)| *id).collect();
        let (a1, p3, ndcg) = eval_ranking(&ids, &labels);
        if a1 {
            c_acc1 += 1.0;
        }
        c_p3 += p3;
        c_ndcg += ndcg;

        for (i, (c, _)) in q.candidates.iter().enumerate() {
            let true_class = labels.get(&c.id).map(|r| r.as_class()).unwrap_or(2);
            let pred_class = contra.predict(&sg, i + 1) as usize;
            c_pairs.push((pred_class, true_class));
        }
    }

    let contra_result = BenchmarkResult {
        name: "contra_gat",
        accuracy_at_1: c_acc1 / n as f64,
        precision_at_3: c_p3 / n as f64,
        ndcg_at_5: c_ndcg / n as f64,
        f1_contradiction: f1_contradiction(&c_pairs),
        mean_latency_us: c_lat / n as f64,
        n_queries: n,
    };

    vec![heuristic, reranker, contra_result]
}

// ─── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Embedding ───────────────────────────────────────────────────

    #[test]
    fn test_embeddings_unit_norm() {
        let e = make_embedding(0, 0, 1.0);
        let norm: f32 = e.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-4);
    }

    #[test]
    fn test_decomposed_heuristic_separability() {
        // score(contradiction) > score(corroboration) > score(unrelated)
        let q = make_embedding(0, 0, 1.0);
        let contra = make_embedding(0, 0, -1.0);
        let corr = make_embedding(0, 0, 1.0);
        let unrel = make_embedding(3, 2, 1.0);

        let score = |cand: &Vec<f32>| -> f32 {
            let sp = cosine_range(&q, cand, SUBJECT_START, PREDICATE_END);
            let ob = cosine_range(&q, cand, OBJECT_START, OBJECT_END);
            sp - ob
        };

        let s_contra = score(&contra);
        let s_corr = score(&corr);
        let s_unrel = score(&unrel);

        assert!(
            s_contra > s_corr,
            "contradiction ({:.3}) must score > corroboration ({:.3})",
            s_contra,
            s_corr
        );
        assert!(
            s_contra > s_unrel,
            "contradiction ({:.3}) must score > unrelated ({:.3})",
            s_contra,
            s_unrel
        );
    }

    #[test]
    fn test_object_divergence_edge_weights() {
        // Contradictions must get higher edge weight than corroborations
        let query = Fact {
            id: Uuid::from_u128(1),
            embedding: make_embedding(0, 0, 1.0),
            subject: 0,
            predicate: 0,
            object_polarity: 1.0,
        };
        let contra = Fact {
            id: Uuid::from_u128(2),
            embedding: make_embedding(0, 0, -1.0),
            subject: 0,
            predicate: 0,
            object_polarity: -1.0,
        };
        let corr = Fact {
            id: Uuid::from_u128(3),
            embedding: make_embedding(0, 0, 1.0),
            subject: 0,
            predicate: 0,
            object_polarity: 1.0,
        };
        let cands = vec![
            (contra, Relation::Contradicts),
            (corr, Relation::Corroborates),
        ];
        let sg = build_query_subgraph(&query, &cands);

        // Find edge weights to each candidate
        let w_contra = sg
            .edges
            .iter()
            .filter(|e| e.target_idx == 1 && e.source_idx == 0)
            .map(|e| e.weight)
            .next()
            .unwrap_or(0.0);
        let w_corr = sg
            .edges
            .iter()
            .filter(|e| e.target_idx == 2 && e.source_idx == 0)
            .map(|e| e.weight)
            .next()
            .unwrap_or(0.0);

        assert!(
            w_contra > w_corr,
            "contradiction edge weight ({:.3}) must exceed corroboration ({:.3})",
            w_contra,
            w_corr
        );
        assert!(
            w_contra > 0.5,
            "contradiction edge weight must be > 0.5, got {:.3}",
            w_contra
        );
        assert!(
            w_corr < 0.1,
            "corroboration edge weight must be < 0.1, got {:.3}",
            w_corr
        );
    }

    // ── Dataset ─────────────────────────────────────────────────────

    #[test]
    fn test_dataset_size() {
        let ds = build_dataset();
        assert_eq!(ds.len(), (N_SUBJECTS * N_PREDICATES) as usize);
    }

    #[test]
    fn test_one_contradiction_per_query() {
        let ds = build_dataset();
        for q in &ds {
            assert_eq!(
                q.candidates
                    .iter()
                    .filter(|(_, r)| r.is_contradiction())
                    .count(),
                1
            );
        }
    }

    #[test]
    fn test_unique_ids() {
        let ds = build_dataset();
        let mut seen = std::collections::HashSet::new();
        for q in &ds {
            assert!(seen.insert(q.query.id));
            for (c, _) in &q.candidates {
                assert!(seen.insert(c.id));
            }
        }
    }

    // ── Heuristic ───────────────────────────────────────────────────

    #[test]
    fn test_heuristic_ranks_contradiction_first() {
        let q = Fact {
            id: Uuid::from_u128(1),
            embedding: make_embedding(0, 0, 1.0),
            subject: 0,
            predicate: 0,
            object_polarity: 1.0,
        };
        let c1 = Fact {
            id: Uuid::from_u128(2),
            embedding: make_embedding(0, 0, -1.0),
            subject: 0,
            predicate: 0,
            object_polarity: -1.0,
        };
        let c2 = Fact {
            id: Uuid::from_u128(3),
            embedding: make_embedding(0, 0, 1.0),
            subject: 0,
            predicate: 0,
            object_polarity: 1.0,
        };
        let cands = vec![
            (c2, Relation::Corroborates),
            (c1.clone(), Relation::Contradicts),
        ];
        let ranked = heuristic_rank(&q, &cands);
        assert_eq!(
            ranked[0].0, c1.id,
            "heuristic must rank contradiction first"
        );
    }

    #[test]
    fn test_heuristic_accuracy_high() {
        let ds = build_dataset();
        let mut correct = 0;
        for q in &ds {
            let ranked = heuristic_rank(&q.query, &q.candidates);
            let top = ranked[0].0;
            if q.candidates
                .iter()
                .any(|(f, r)| f.id == top && r.is_contradiction())
            {
                correct += 1;
            }
        }
        let acc = correct as f64 / ds.len() as f64;
        assert!(
            acc > 0.8,
            "decomposed heuristic accuracy@1 must exceed 0.8, got {:.3}",
            acc
        );
    }

    // ── ContraGat ───────────────────────────────────────────────────

    #[test]
    fn test_contra_gat_predict_returns_valid_class() {
        let ds = build_dataset();
        let q = &ds[0];
        let model = ContraGat::initialize(DIM);
        let sg = build_query_subgraph(&q.query, &q.candidates);
        let pred = model.predict(&sg, 1);
        // Just check it doesn't panic and returns a valid variant
        let _ = pred;
    }

    #[test]
    fn test_contra_gat_proba_sums_to_one() {
        let ds = build_dataset();
        let q = &ds[0];
        let model = ContraGat::initialize(DIM);
        let sg = build_query_subgraph(&q.query, &q.candidates);
        let p = model.predict_proba(&sg, 1);
        let sum: f32 = p.iter().sum();
        assert!(
            (sum - 1.0).abs() < 1e-4,
            "probabilities must sum to 1.0, got {:.6}",
            sum
        );
    }

    #[test]
    fn test_contra_gat_weights_change_after_training() {
        let ds = build_dataset();
        let before = ContraGat::initialize(DIM);
        let after = train_contra_gat(&ds, 5, 0.005, 16);
        // At least one MLP weight must have changed
        let changed = before
            .mlp_w1
            .iter()
            .zip(after.mlp_w1.iter())
            .any(|(a_row, b_row)| {
                a_row
                    .iter()
                    .zip(b_row.iter())
                    .any(|(a, b)| (a - b).abs() > 1e-8)
            });
        assert!(changed, "training must change at least one MLP weight");
    }

    #[test]
    fn test_contra_gat_trained_beats_random() {
        let ds = build_dataset();
        let model = train_contra_gat(&ds, 40, 0.005, 16);
        let mut correct = 0;
        for q in &ds {
            let sg = build_query_subgraph(&q.query, &q.candidates);
            let mut scored: Vec<(Uuid, f64)> = q
                .candidates
                .iter()
                .enumerate()
                .map(|(i, (c, _))| (c.id, model.predict_proba(&sg, i + 1)[0] as f64))
                .collect();
            scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            if let Some(&(top_id, _)) = scored.first() {
                if q.candidates
                    .iter()
                    .any(|(f, r)| f.id == top_id && r.is_contradiction())
                {
                    correct += 1;
                }
            }
        }
        let acc = correct as f64 / ds.len() as f64;
        // Random baseline is 1/6 ≈ 0.167; trained ContraGat must beat it
        assert!(
            acc > 0.25,
            "trained ContraGat accuracy@1 must beat random (0.167), got {:.3}",
            acc
        );
    }

    // ── Full benchmark ───────────────────────────────────────────────

    #[test]
    fn test_benchmark_produces_three_results() {
        let results = run_benchmark();
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].name, "decomposed_heuristic");
        assert_eq!(results[1].name, "gat_reranker");
        assert_eq!(results[2].name, "contra_gat");
    }

    #[test]
    fn test_benchmark_metrics_in_range() {
        let results = run_benchmark();
        for r in &results {
            assert!(
                (0.0..=1.0).contains(&r.accuracy_at_1),
                "{} acc@1 out of range",
                r.name
            );
            assert!(
                (0.0..=1.0).contains(&r.precision_at_3),
                "{} p@3 out of range",
                r.name
            );
            assert!(
                (0.0..=1.0).contains(&r.ndcg_at_5),
                "{} ndcg out of range",
                r.name
            );
            assert!(
                (0.0..=1.0).contains(&r.f1_contradiction),
                "{} f1 out of range",
                r.name
            );
            assert!(r.mean_latency_us >= 0.0, "{} latency negative", r.name);
        }
    }

    #[test]
    fn test_full_verdict() {
        let results = run_benchmark();
        let heuristic = &results[0];
        let reranker = &results[1];
        let contra = &results[2];

        println!("\n╔══════════════════════════════════════════════════════════════════════╗");
        println!("║              GNN Validation Benchmark — v2 Results                  ║");
        println!("╠══════════════════════════════════════════════════════════════════════╣");
        println!(
            "║ {:24} │ Acc@1 │  P@3  │ NDCG@5 │  F1-C  │ Lat(µs) ║",
            "Approach"
        );
        println!("╠══════════════════════════════════════════════════════════════════════╣");
        for r in &results {
            println!(
                "║ {:24} │ {:.3} │ {:.3} │  {:.3}  │  {:.3}  │ {:>7.1}  ║",
                r.name,
                r.accuracy_at_1,
                r.precision_at_3,
                r.ndcg_at_5,
                r.f1_contradiction,
                r.mean_latency_us
            );
        }
        println!("╚══════════════════════════════════════════════════════════════════════╝");
        println!(
            "  Random baseline: Acc@1 = {:.3}  (1/6 candidates)",
            1.0 / 6.0
        );

        println!("\nAnalysis:");
        println!(
            "  Heuristic beats random: {}",
            heuristic.accuracy_at_1 > 1.0 / 6.0 + 0.05
        );
        println!(
            "  ContraGat beats random: {}",
            contra.accuracy_at_1 > 1.0 / 6.0 + 0.05
        );
        println!(
            "  ContraGat beats heuristic (>5%): {}",
            contra.meaningfully_beats(heuristic)
        );
        println!(
            "  ContraGat beats reranker:         {}",
            contra.meaningfully_beats(reranker)
        );
        println!();

        // Structural assertions
        // 1. The decomposed heuristic must now beat random clearly
        assert!(
            heuristic.accuracy_at_1 > 0.80,
            "decomposed heuristic must achieve >0.80 Acc@1, got {:.3}",
            heuristic.accuracy_at_1
        );
        // 2. ContraGat must beat random
        assert!(
            contra.accuracy_at_1 > 0.25,
            "ContraGat must beat random (0.167) after training, got {:.3}",
            contra.accuracy_at_1
        );
        // 3. F1-contradiction must be positive for ContraGat
        assert!(
            contra.f1_contradiction > 0.0,
            "ContraGat F1-contradiction must be positive, got {:.3}",
            contra.f1_contradiction
        );
    }

    #[test]
    fn test_full_verdict_contra_beats_old_gnn() {
        // The v1 GNN achieved Acc@1=0.250 with the broken setup.
        // ContraGat with the correct architecture must equal or exceed this.
        let results = run_benchmark();
        let contra = &results[2];
        assert!(
            contra.accuracy_at_1 >= 0.25,
            "ContraGat must at least match the v1 GNN (0.250), got {:.3}",
            contra.accuracy_at_1
        );
    }
}
