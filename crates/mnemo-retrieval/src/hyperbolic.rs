//! Hyperbolic geometry for entity hierarchy embedding.
//!
//! Implements Poincare ball model operations for representing hierarchical
//! knowledge graph structures. Entities that form natural trees/taxonomies
//! (person -> works_at -> company -> in -> industry) get better nearest-neighbor
//! results in hyperbolic space, which provides exponentially more room at the
//! periphery — matching how tree breadth grows exponentially with depth.
//!
//! ## Architecture
//!
//! Qdrant doesn't natively support hyperbolic distance, so we use a two-phase
//! approach:
//! 1. Store Poincare-projected embeddings in Qdrant with Cosine distance
//!    (approximate retrieval)
//! 2. Re-rank the top-K candidates using true Poincare ball distance
//!    (exact hyperbolic ordering)
//!
//! The projection from Euclidean to Poincare ball uses the exponential map
//! at the origin, which maps tangent vectors to the ball interior while
//! preserving local geometry.

use serde::Serialize;

// ─── Constants ────────────────────────────────────────────────────

/// Default curvature for the Poincare ball. c = 1.0 gives the standard
/// unit ball. Larger values increase curvature (more compression of
/// hierarchy levels).
pub const DEFAULT_CURVATURE: f32 = 1.0;

/// Numerical epsilon to prevent division by zero and keep points
/// strictly inside the ball (||x|| < 1).
const EPS: f32 = 1e-7;

/// Maximum norm allowed inside the Poincare ball. Points are clamped
/// to this to maintain numerical stability (||x|| < 1 required).
const MAX_NORM: f32 = 1.0 - 1e-5;

// ─── Poincare ball operations ─────────────────────────────────────

/// Compute the Poincare ball distance between two points.
///
/// Formula: d(u, v) = (1/√c) · arcosh(1 + 2c · ||u - v||² / ((1 - c||u||²)(1 - c||v||²)))
///
/// For the standard ball (c = 1.0):
/// d(u, v) = arcosh(1 + 2||u - v||² / ((1 - ||u||²)(1 - ||v||²)))
pub fn poincare_distance(u: &[f32], v: &[f32], curvature: f32) -> f32 {
    debug_assert_eq!(u.len(), v.len(), "vectors must have same dimension");
    if u.is_empty() {
        return 0.0;
    }

    let diff_sq: f32 = u.iter().zip(v.iter()).map(|(a, b)| (a - b).powi(2)).sum();
    let u_sq: f32 = u.iter().map(|x| x * x).sum();
    let v_sq: f32 = v.iter().map(|x| x * x).sum();

    let denom = (1.0 - curvature * u_sq).max(EPS) * (1.0 - curvature * v_sq).max(EPS);
    let arg = 1.0 + 2.0 * curvature * diff_sq / denom;

    // arcosh(x) = ln(x + sqrt(x² - 1)), but x must be >= 1.0
    let arg = arg.max(1.0);
    let dist = (1.0 / curvature.sqrt()) * acosh(arg);

    dist.max(0.0)
}

/// Hyperbolic arcosh: arcosh(x) = ln(x + sqrt(x² - 1))
fn acosh(x: f32) -> f32 {
    let x = x.max(1.0); // clamp for numerical safety
    (x + (x * x - 1.0).max(0.0).sqrt()).ln()
}

/// Project a Euclidean vector onto the Poincare ball using the
/// exponential map at the origin.
///
/// exp_0(v) = tanh(√c · ||v|| / 2) · v / (√c · ||v||)
///
/// This maps any Euclidean vector to a point inside the unit ball.
/// Vectors with small norm land near the origin (general/root concepts);
/// vectors with large norm land near the boundary (specific/leaf concepts).
pub fn exp_map_origin(v: &[f32], curvature: f32) -> Vec<f32> {
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm < EPS {
        return vec![0.0; v.len()];
    }

    let sqrt_c = curvature.sqrt();
    let scale = (sqrt_c * norm / 2.0).tanh() / (sqrt_c * norm);

    let mut result: Vec<f32> = v.iter().map(|x| x * scale).collect();
    clamp_to_ball(&mut result);
    result
}

/// Inverse: logarithmic map at origin. Maps a Poincare ball point back to
/// tangent space (Euclidean).
///
/// log_0(y) = (2 / √c) · arctanh(√c · ||y||) · y / ||y||
pub fn log_map_origin(y: &[f32], curvature: f32) -> Vec<f32> {
    let norm: f32 = y.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm < EPS {
        return vec![0.0; y.len()];
    }

    let sqrt_c = curvature.sqrt();
    let scale = (2.0 / sqrt_c) * atanh((sqrt_c * norm).min(MAX_NORM)) / norm;

    y.iter().map(|x| x * scale).collect()
}

/// Hyperbolic arctanh: arctanh(x) = 0.5 · ln((1+x)/(1-x))
fn atanh(x: f32) -> f32 {
    let x = x.clamp(-MAX_NORM, MAX_NORM);
    0.5 * ((1.0 + x) / (1.0 - x)).ln()
}

/// Mobius addition in the Poincare ball: x ⊕ y
///
/// x ⊕ y = ((1 + 2c<x,y> + c||y||²)x + (1 - c||x||²)y) / (1 + 2c<x,y> + c²||x||²||y||²)
pub fn mobius_add(x: &[f32], y: &[f32], curvature: f32) -> Vec<f32> {
    debug_assert_eq!(x.len(), y.len());

    let dot: f32 = x.iter().zip(y.iter()).map(|(a, b)| a * b).sum();
    let x_sq: f32 = x.iter().map(|v| v * v).sum();
    let y_sq: f32 = y.iter().map(|v| v * v).sum();
    let c = curvature;

    let num_x = 1.0 + 2.0 * c * dot + c * y_sq;
    let num_y = 1.0 - c * x_sq;
    let denom = (1.0 + 2.0 * c * dot + c * c * x_sq * y_sq).max(EPS);

    let mut result: Vec<f32> = x
        .iter()
        .zip(y.iter())
        .map(|(xi, yi)| (num_x * xi + num_y * yi) / denom)
        .collect();

    clamp_to_ball(&mut result);
    result
}

/// Clamp a vector to lie strictly inside the Poincare ball (||x|| < 1).
fn clamp_to_ball(v: &mut [f32]) {
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm >= MAX_NORM {
        let scale = MAX_NORM / norm;
        for x in v.iter_mut() {
            *x *= scale;
        }
    }
}

// ─── Re-ranking ───────────────────────────────────────────────────

/// A candidate entity with its Cosine score from Qdrant and its
/// Poincare-ball embedding for hyperbolic re-ranking.
#[derive(Debug, Clone)]
pub struct HyperbolicCandidate {
    pub entity_id: uuid::Uuid,
    /// Cosine similarity score from Qdrant (original ranking signal).
    pub cosine_score: f32,
    /// Poincare ball embedding of this entity.
    pub poincare_embedding: Vec<f32>,
}

/// Result of hyperbolic re-ranking.
#[derive(Debug, Clone, Serialize)]
pub struct HyperbolicRankedResult {
    pub entity_id: uuid::Uuid,
    /// Final blended score (higher is better).
    pub final_score: f32,
    /// Original Cosine score from Qdrant.
    pub cosine_score: f32,
    /// Poincare distance to query (lower means closer in hierarchy).
    pub poincare_distance: f32,
    /// Hierarchy depth estimate: how far from the ball origin (0 = root, 1 = leaf).
    pub hierarchy_depth: f32,
}

/// Re-rank entity candidates using Poincare ball distance.
///
/// Blends the original Cosine similarity with hyperbolic proximity.
/// `alpha` controls the blend: 0.0 = pure Cosine, 1.0 = pure hyperbolic.
///
/// The hyperbolic score is computed as `1.0 / (1.0 + poincare_distance)`,
/// which converts distance to a similarity in [0, 1].
pub fn hyperbolic_rerank(
    query_embedding: &[f32],
    candidates: &[HyperbolicCandidate],
    curvature: f32,
    alpha: f32,
) -> Vec<HyperbolicRankedResult> {
    let alpha = alpha.clamp(0.0, 1.0);

    // Project query to Poincare ball
    let query_poincare = exp_map_origin(query_embedding, curvature);

    let mut results: Vec<HyperbolicRankedResult> = candidates
        .iter()
        .map(|c| {
            let pdist = poincare_distance(&query_poincare, &c.poincare_embedding, curvature);
            let hyper_score = 1.0 / (1.0 + pdist);
            let final_score = (1.0 - alpha) * c.cosine_score + alpha * hyper_score;
            let norm: f32 = c
                .poincare_embedding
                .iter()
                .map(|x| x * x)
                .sum::<f32>()
                .sqrt();

            HyperbolicRankedResult {
                entity_id: c.entity_id,
                final_score,
                cosine_score: c.cosine_score,
                poincare_distance: pdist,
                hierarchy_depth: norm, // distance from origin = depth in hierarchy
            }
        })
        .collect();

    // Sort by final_score descending
    results.sort_by(|a, b| {
        b.final_score
            .partial_cmp(&a.final_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    results
}

/// Estimate the hierarchy depth of an entity from its Poincare embedding.
/// Returns a value in [0, 1]: 0 = root/general concept, 1 = leaf/specific.
pub fn hierarchy_depth(embedding: &[f32]) -> f32 {
    let norm: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
    norm.min(1.0)
}

// ─── Batch projection ─────────────────────────────────────────────

/// Project a batch of Euclidean embeddings to Poincare ball.
pub fn batch_project_to_poincare(embeddings: &[Vec<f32>], curvature: f32) -> Vec<Vec<f32>> {
    embeddings
        .iter()
        .map(|e| exp_map_origin(e, curvature))
        .collect()
}

/// Configuration for hyperbolic operations.
#[derive(Debug, Clone, Serialize)]
pub struct HyperbolicConfig {
    /// Whether hyperbolic re-ranking is enabled.
    pub enabled: bool,
    /// Curvature of the Poincare ball (default: 1.0).
    pub curvature: f32,
    /// Blend factor: 0.0 = pure Cosine, 1.0 = pure hyperbolic.
    pub alpha: f32,
}

impl Default for HyperbolicConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            curvature: DEFAULT_CURVATURE,
            alpha: 0.3, // 70% Cosine, 30% hyperbolic by default
        }
    }
}

/// Status information about hyperbolic operations.
#[derive(Debug, Clone, Serialize)]
pub struct HyperbolicStatus {
    pub enabled: bool,
    pub curvature: f32,
    pub alpha: f32,
    pub description: &'static str,
}

impl HyperbolicConfig {
    pub fn status(&self) -> HyperbolicStatus {
        HyperbolicStatus {
            enabled: self.enabled,
            curvature: self.curvature,
            alpha: self.alpha,
            description: if self.enabled {
                "Poincare ball re-ranking active: entity search results are re-ranked using hyperbolic distance to better capture hierarchical relationships"
            } else {
                "Hyperbolic re-ranking disabled: entity search uses standard Cosine similarity only"
            },
        }
    }
}

// ─── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to create deterministic test UUIDs.
    fn test_uuid(n: u128) -> uuid::Uuid {
        uuid::Uuid::from_u128(n)
    }

    // ─── Poincare distance tests ──────────────────────────────────

    #[test]
    fn test_poincare_distance_same_point_is_zero() {
        let p = vec![0.3, 0.2, 0.1];
        let d = poincare_distance(&p, &p, 1.0);
        assert!(d.abs() < 1e-5, "Distance to self should be ~0, got {}", d);
    }

    #[test]
    fn test_poincare_distance_origin_to_point() {
        let origin = vec![0.0, 0.0, 0.0];
        let point = vec![0.5, 0.0, 0.0];
        let d = poincare_distance(&origin, &point, 1.0);
        assert!(d > 0.0, "Distance should be positive");
        // For origin to (0.5,0,0): d = arcosh(1 + 2*0.25/((1)(1-0.25))) = arcosh(1 + 0.5/0.75) = arcosh(1.667)
        let expected = acosh(1.0 + 2.0 * 0.25 / 0.75);
        assert!(
            (d - expected).abs() < 1e-4,
            "Expected {}, got {}",
            expected,
            d
        );
    }

    #[test]
    fn test_poincare_distance_symmetric() {
        let u = vec![0.3, 0.2];
        let v = vec![-0.1, 0.4];
        let d1 = poincare_distance(&u, &v, 1.0);
        let d2 = poincare_distance(&v, &u, 1.0);
        assert!(
            (d1 - d2).abs() < 1e-5,
            "Distance should be symmetric: {} vs {}",
            d1,
            d2
        );
    }

    #[test]
    fn test_poincare_distance_increases_near_boundary() {
        // Points near the boundary are exponentially far apart
        let near = vec![0.1, 0.0];
        let far = vec![0.9, 0.0];
        let d_near = poincare_distance(&vec![0.0, 0.0], &near, 1.0);
        let d_far = poincare_distance(&vec![0.0, 0.0], &far, 1.0);
        assert!(
            d_far > d_near * 3.0,
            "Boundary distance should grow super-linearly: near={}, far={}",
            d_near,
            d_far
        );
    }

    #[test]
    fn test_poincare_distance_empty_vectors() {
        let d = poincare_distance(&[], &[], 1.0);
        assert_eq!(d, 0.0);
    }

    #[test]
    fn test_poincare_distance_triangle_inequality() {
        let a = vec![0.1, 0.2];
        let b = vec![0.3, -0.1];
        let c = vec![-0.2, 0.3];
        let dab = poincare_distance(&a, &b, 1.0);
        let dbc = poincare_distance(&b, &c, 1.0);
        let dac = poincare_distance(&a, &c, 1.0);
        assert!(
            dac <= dab + dbc + 1e-5,
            "Triangle inequality violated: d(a,c)={} > d(a,b)+d(b,c)={}",
            dac,
            dab + dbc
        );
    }

    #[test]
    fn test_poincare_distance_with_curvature() {
        let u = vec![0.3, 0.2];
        let v = vec![-0.1, 0.4];
        let d1 = poincare_distance(&u, &v, 0.5);
        let d2 = poincare_distance(&u, &v, 2.0);
        // Higher curvature compresses distances differently
        assert!(
            d1 != d2,
            "Different curvatures should give different distances"
        );
    }

    // ─── Exponential/logarithmic map tests ────────────────────────

    #[test]
    fn test_exp_map_origin_zero_vector() {
        let v = vec![0.0, 0.0, 0.0];
        let result = exp_map_origin(&v, 1.0);
        assert_eq!(result, vec![0.0, 0.0, 0.0]);
    }

    #[test]
    fn test_exp_map_origin_stays_inside_ball() {
        let v = vec![100.0, 200.0, 300.0]; // huge vector
        let result = exp_map_origin(&v, 1.0);
        let norm: f32 = result.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(
            norm < 1.0,
            "Projected point must be inside ball, norm={}",
            norm
        );
    }

    #[test]
    fn test_exp_map_preserves_direction() {
        let v = vec![3.0, 4.0];
        let result = exp_map_origin(&v, 1.0);
        // Direction should be preserved (same angle)
        let orig_angle = v[1].atan2(v[0]);
        let proj_angle = result[1].atan2(result[0]);
        assert!(
            (orig_angle - proj_angle).abs() < 1e-5,
            "Direction should be preserved"
        );
    }

    #[test]
    fn test_exp_log_roundtrip() {
        let v = vec![0.5, -0.3, 0.8];
        let projected = exp_map_origin(&v, 1.0);
        let recovered = log_map_origin(&projected, 1.0);
        for (a, b) in v.iter().zip(recovered.iter()) {
            assert!(
                (a - b).abs() < 1e-3,
                "Roundtrip failed: original={:?}, recovered={:?}",
                v,
                recovered
            );
        }
    }

    #[test]
    fn test_log_map_origin_zero() {
        let result = log_map_origin(&vec![0.0, 0.0], 1.0);
        assert_eq!(result, vec![0.0, 0.0]);
    }

    // ─── Mobius addition tests ────────────────────────────────────

    #[test]
    fn test_mobius_add_origin_identity() {
        let origin = vec![0.0, 0.0];
        let y = vec![0.3, 0.4];
        let result = mobius_add(&origin, &y, 1.0);
        for (a, b) in result.iter().zip(y.iter()) {
            assert!(
                (a - b).abs() < 1e-5,
                "Origin ⊕ y should equal y: {:?} vs {:?}",
                result,
                y
            );
        }
    }

    #[test]
    fn test_mobius_add_stays_inside_ball() {
        let x = vec![0.8, 0.0];
        let y = vec![0.0, 0.8];
        let result = mobius_add(&x, &y, 1.0);
        let norm: f32 = result.iter().map(|v| v * v).sum::<f32>().sqrt();
        assert!(
            norm < 1.0,
            "Mobius addition must stay inside ball, norm={}",
            norm
        );
    }

    // ─── Re-ranking tests ─────────────────────────────────────────

    #[test]
    fn test_hyperbolic_rerank_empty_candidates() {
        let results = hyperbolic_rerank(&[0.1, 0.2], &[], 1.0, 0.3);
        assert!(results.is_empty());
    }

    #[test]
    fn test_hyperbolic_rerank_preserves_all_candidates() {
        let candidates = vec![
            HyperbolicCandidate {
                entity_id: test_uuid(1),
                cosine_score: 0.9,
                poincare_embedding: vec![0.1, 0.0],
            },
            HyperbolicCandidate {
                entity_id: test_uuid(2),
                cosine_score: 0.7,
                poincare_embedding: vec![0.5, 0.3],
            },
        ];
        let results = hyperbolic_rerank(&[0.2, 0.1], &candidates, 1.0, 0.3);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_hyperbolic_rerank_alpha_zero_is_pure_cosine() {
        let id1 = test_uuid(10);
        let id2 = test_uuid(20);
        let candidates = vec![
            HyperbolicCandidate {
                entity_id: id1,
                cosine_score: 0.9,
                poincare_embedding: vec![0.8, 0.0], // far from query
            },
            HyperbolicCandidate {
                entity_id: id2,
                cosine_score: 0.5,
                poincare_embedding: vec![0.01, 0.0], // close to query
            },
        ];
        let results = hyperbolic_rerank(&[0.0, 0.0], &candidates, 1.0, 0.0);
        // With alpha=0, pure Cosine: higher Cosine score wins
        assert_eq!(results[0].entity_id, id1);
        assert!((results[0].final_score - 0.9).abs() < 1e-5);
    }

    #[test]
    fn test_hyperbolic_rerank_alpha_one_is_pure_hyperbolic() {
        let id1 = test_uuid(30);
        let id2 = test_uuid(40);
        let candidates = vec![
            HyperbolicCandidate {
                entity_id: id1,
                cosine_score: 0.9,
                poincare_embedding: vec![0.8, 0.0], // far from query in hyperbolic space
            },
            HyperbolicCandidate {
                entity_id: id2,
                cosine_score: 0.2,
                poincare_embedding: vec![0.01, 0.0], // close to query
            },
        ];
        let results = hyperbolic_rerank(&[0.0, 0.0], &candidates, 1.0, 1.0);
        // With alpha=1, pure hyperbolic: closer in Poincare wins
        assert_eq!(
            results[0].entity_id, id2,
            "Closer Poincare point should rank first with alpha=1.0"
        );
    }

    #[test]
    fn test_hyperbolic_rerank_sorted_descending() {
        let candidates = vec![
            HyperbolicCandidate {
                entity_id: test_uuid(50),
                cosine_score: 0.3,
                poincare_embedding: vec![0.1, 0.0],
            },
            HyperbolicCandidate {
                entity_id: test_uuid(51),
                cosine_score: 0.9,
                poincare_embedding: vec![0.1, 0.0],
            },
            HyperbolicCandidate {
                entity_id: test_uuid(52),
                cosine_score: 0.6,
                poincare_embedding: vec![0.1, 0.0],
            },
        ];
        let results = hyperbolic_rerank(&[0.0, 0.0], &candidates, 1.0, 0.3);
        for i in 1..results.len() {
            assert!(
                results[i - 1].final_score >= results[i].final_score,
                "Results should be sorted descending"
            );
        }
    }

    #[test]
    fn test_hyperbolic_rerank_scores_bounded() {
        let candidates = vec![
            HyperbolicCandidate {
                entity_id: test_uuid(60),
                cosine_score: 1.0,
                poincare_embedding: vec![0.0, 0.0],
            },
            HyperbolicCandidate {
                entity_id: test_uuid(61),
                cosine_score: 0.0,
                poincare_embedding: vec![0.99, 0.0],
            },
        ];
        let results = hyperbolic_rerank(&[0.0, 0.0], &candidates, 1.0, 0.5);
        for r in &results {
            assert!(
                r.final_score >= 0.0 && r.final_score <= 1.0,
                "Score should be in [0,1], got {}",
                r.final_score
            );
            assert!(
                r.poincare_distance >= 0.0,
                "Distance should be non-negative"
            );
            assert!(
                r.hierarchy_depth >= 0.0 && r.hierarchy_depth <= 1.0,
                "Depth should be in [0,1]"
            );
        }
    }

    // ─── Hierarchy depth tests ────────────────────────────────────

    #[test]
    fn test_hierarchy_depth_origin_is_root() {
        let depth = hierarchy_depth(&[0.0, 0.0, 0.0]);
        assert_eq!(depth, 0.0);
    }

    #[test]
    fn test_hierarchy_depth_boundary_is_leaf() {
        let depth = hierarchy_depth(&[0.99, 0.0]);
        assert!(depth > 0.9, "Near-boundary should be leaf, got {}", depth);
    }

    #[test]
    fn test_hierarchy_depth_monotonic_with_norm() {
        let d1 = hierarchy_depth(&[0.2, 0.0]);
        let d2 = hierarchy_depth(&[0.5, 0.0]);
        let d3 = hierarchy_depth(&[0.8, 0.0]);
        assert!(d1 < d2 && d2 < d3, "Depth should increase with norm");
    }

    // ─── Batch projection tests ───────────────────────────────────

    #[test]
    fn test_batch_project_empty() {
        let result = batch_project_to_poincare(&[], 1.0);
        assert!(result.is_empty());
    }

    #[test]
    fn test_batch_project_all_inside_ball() {
        let embeddings = vec![
            vec![1.0, 2.0, 3.0],
            vec![-1.0, 0.5, -0.5],
            vec![10.0, 10.0, 10.0],
        ];
        let projected = batch_project_to_poincare(&embeddings, 1.0);
        assert_eq!(projected.len(), 3);
        for p in &projected {
            let norm: f32 = p.iter().map(|x| x * x).sum::<f32>().sqrt();
            assert!(norm < 1.0, "All projected points must be inside ball");
        }
    }

    // ─── Config tests ─────────────────────────────────────────────

    #[test]
    fn test_default_config() {
        let cfg = HyperbolicConfig::default();
        assert!(!cfg.enabled);
        assert_eq!(cfg.curvature, 1.0);
        assert!((cfg.alpha - 0.3).abs() < 1e-5);
    }

    #[test]
    fn test_status_disabled() {
        let cfg = HyperbolicConfig::default();
        let status = cfg.status();
        assert!(!status.enabled);
        assert!(status.description.contains("disabled"));
    }

    #[test]
    fn test_status_enabled() {
        let cfg = HyperbolicConfig {
            enabled: true,
            curvature: 1.5,
            alpha: 0.5,
        };
        let status = cfg.status();
        assert!(status.enabled);
        assert!(status.description.contains("active"));
        assert_eq!(status.curvature, 1.5);
    }

    #[test]
    fn test_status_serializes() {
        let cfg = HyperbolicConfig::default();
        let json = serde_json::to_value(cfg.status()).unwrap();
        assert!(json.get("enabled").is_some());
        assert!(json.get("curvature").is_some());
        assert!(json.get("alpha").is_some());
        assert!(json.get("description").is_some());
    }

    // ─── Falsification round: adversarial tests ──────────────────

    #[test]
    fn test_falsify_zero_curvature_no_panic() {
        // Zero curvature: sqrt(0) = 0, division by 0 in distance formula.
        // Should not panic. Distance may be 0 or meaningless, but no crash.
        let u = vec![0.3, 0.2];
        let v = vec![-0.1, 0.4];
        let d = poincare_distance(&u, &v, 0.0);
        assert!(
            d.is_finite(),
            "Distance with zero curvature should be finite, got {}",
            d
        );
    }

    #[test]
    fn test_falsify_negative_curvature_no_panic() {
        // Negative curvature is nonsensical for Poincare ball but should not crash.
        let u = vec![0.3, 0.2];
        let v = vec![-0.1, 0.4];
        let d = poincare_distance(&u, &v, -1.0);
        // May be NaN due to sqrt of negative, but should not panic
        let _ = d; // just don't panic
    }

    #[test]
    fn test_falsify_boundary_point_distance() {
        // Points exactly at norm = MAX_NORM (near boundary)
        let u = vec![MAX_NORM, 0.0];
        let v = vec![0.0, MAX_NORM];
        let d = poincare_distance(&u, &v, 1.0);
        assert!(
            d.is_finite(),
            "Boundary distance should be finite, got {}",
            d
        );
        assert!(d > 0.0, "Boundary points should be far apart");
    }

    #[test]
    fn test_falsify_high_dimensional_exp_log_roundtrip() {
        // 384-dimensional roundtrip (production embedding size)
        let v: Vec<f32> = (0..384).map(|i| (i as f32 * 0.01).sin() * 0.5).collect();
        let projected = exp_map_origin(&v, 1.0);
        let norm: f32 = projected.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(
            norm < 1.0,
            "384-dim projection must be inside ball, norm={}",
            norm
        );

        let recovered = log_map_origin(&projected, 1.0);
        let max_err: f32 = v
            .iter()
            .zip(recovered.iter())
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(
            max_err < 0.01,
            "384-dim roundtrip max error should be < 0.01, got {}",
            max_err
        );
    }

    #[test]
    fn test_falsify_exp_map_very_large_vector() {
        // Vector with norm > 1000 — should still map inside ball
        let v = vec![1000.0; 100];
        let p = exp_map_origin(&v, 1.0);
        let norm: f32 = p.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(
            norm < 1.0,
            "Huge vector must still map inside ball, norm={}",
            norm
        );
        // Should be very close to boundary (tanh → 1)
        assert!(
            norm > 0.99,
            "Huge vector should map near boundary, norm={}",
            norm
        );
    }

    #[test]
    fn test_falsify_alpha_clamping() {
        let candidates = vec![HyperbolicCandidate {
            entity_id: test_uuid(100),
            cosine_score: 0.8,
            poincare_embedding: vec![0.1, 0.0],
        }];
        // alpha > 1.0 should be clamped to 1.0
        let r1 = hyperbolic_rerank(&[0.0, 0.0], &candidates, 1.0, 5.0);
        let r2 = hyperbolic_rerank(&[0.0, 0.0], &candidates, 1.0, 1.0);
        assert!(
            (r1[0].final_score - r2[0].final_score).abs() < 1e-5,
            "alpha>1 should clamp to 1.0"
        );

        // alpha < 0 should be clamped to 0.0
        let r3 = hyperbolic_rerank(&[0.0, 0.0], &candidates, 1.0, -5.0);
        let r4 = hyperbolic_rerank(&[0.0, 0.0], &candidates, 1.0, 0.0);
        assert!(
            (r3[0].final_score - r4[0].final_score).abs() < 1e-5,
            "alpha<0 should clamp to 0.0"
        );
    }

    #[test]
    fn test_falsify_identical_poincare_embeddings_all_same_score() {
        // All candidates have identical embeddings → same hyperbolic score
        let emb = vec![0.3, 0.2];
        let candidates: Vec<HyperbolicCandidate> = (0..5)
            .map(|i| HyperbolicCandidate {
                entity_id: test_uuid(200 + i),
                cosine_score: 0.5,
                poincare_embedding: emb.clone(),
            })
            .collect();
        let results = hyperbolic_rerank(&[0.1, 0.1], &candidates, 1.0, 0.5);
        // All should have identical final scores
        let first_score = results[0].final_score;
        for r in &results {
            assert!(
                (r.final_score - first_score).abs() < 1e-5,
                "Identical embeddings should produce identical scores"
            );
        }
    }

    #[test]
    fn test_falsify_single_candidate_reranking() {
        let candidates = vec![HyperbolicCandidate {
            entity_id: test_uuid(300),
            cosine_score: 0.7,
            poincare_embedding: vec![0.4, 0.3],
        }];
        let results = hyperbolic_rerank(&[0.0, 0.0], &candidates, 1.0, 0.5);
        assert_eq!(results.len(), 1);
        assert!(results[0].final_score > 0.0);
        assert!(results[0].poincare_distance > 0.0);
    }

    #[test]
    fn test_falsify_mobius_add_inverse() {
        // x ⊕ (-x) should be near origin (Mobius negation: -x in Poincare is just -x)
        let x = vec![0.3, 0.2];
        let neg_x: Vec<f32> = x.iter().map(|v| -v).collect();
        let result = mobius_add(&x, &neg_x, 1.0);
        let norm: f32 = result.iter().map(|v| v * v).sum::<f32>().sqrt();
        assert!(norm < 0.1, "x ⊕ (-x) should be near origin, norm={}", norm);
    }

    #[test]
    fn test_falsify_poincare_distance_non_negative() {
        // Exhaustive check: random-ish points should all have d >= 0
        let points: Vec<Vec<f32>> = vec![
            vec![0.0, 0.0],
            vec![0.5, 0.0],
            vec![0.0, 0.5],
            vec![-0.3, 0.4],
            vec![0.9, 0.0],
            vec![0.0, -0.9],
        ];
        for i in 0..points.len() {
            for j in 0..points.len() {
                let d = poincare_distance(&points[i], &points[j], 1.0);
                assert!(
                    d >= 0.0,
                    "Distance should be non-negative: d({:?}, {:?}) = {}",
                    points[i],
                    points[j],
                    d
                );
            }
        }
    }

    #[test]
    fn test_falsify_hierarchy_preserves_parent_child_ordering() {
        // Simulate hierarchy: root at origin, children further out
        let root = exp_map_origin(&[0.1, 0.0, 0.0], 1.0);
        let child = exp_map_origin(&[1.0, 0.0, 0.0], 1.0);
        let grandchild = exp_map_origin(&[5.0, 0.0, 0.0], 1.0);

        let root_depth = hierarchy_depth(&root);
        let child_depth = hierarchy_depth(&child);
        let grandchild_depth = hierarchy_depth(&grandchild);

        assert!(
            root_depth < child_depth && child_depth < grandchild_depth,
            "Hierarchy depth should increase: root={}, child={}, grandchild={}",
            root_depth,
            child_depth,
            grandchild_depth
        );
    }
}
