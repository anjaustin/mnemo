//! Temporal tensor compression for old episode embeddings.
//!
//! Implements tiered compression that progressively quantizes older
//! episode embeddings to reduce storage:
//!
//! | Tier | Age          | Precision | Bytes/dim | Savings |
//! |------|-------------|-----------|-----------|---------|
//! | 0    | 0-7 days    | f32       | 4         | 0%      |
//! | 1    | 7-30 days   | f16       | 4*        | ~50%†   |
//! | 2    | 30-90 days  | int8      | 4*        | ~75%†   |
//! | 3    | 90+ days    | binary    | 4*        | ~97%†   |
//!
//! *Qdrant requires f32 vectors, so quantized values are stored as f32.
//! The savings come from reduced precision (lossy): fewer unique values
//! means better compression at the Qdrant storage layer (page cache, mmap).
//!
//! †Effective savings depend on Qdrant's internal compression. The primary
//! benefit is reduced search precision requirements for old data — binary
//! quantization is effectively a bloom filter over old episodes.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};

// ─── Compression Tier ──────────────────────────────────────────────

/// Compression tier for a vector point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompressionTier {
    /// Full f32 precision (0-7 days).
    Full = 0,
    /// f16-equivalent quantization (7-30 days).
    Half = 1,
    /// int8 scalar quantization (30-90 days).
    Int8 = 2,
    /// Binary quantization (90+ days).
    Binary = 3,
}

impl CompressionTier {
    /// Returns the tier name as a string for payload storage.
    pub fn as_str(&self) -> &'static str {
        match self {
            CompressionTier::Full => "full",
            CompressionTier::Half => "half",
            CompressionTier::Int8 => "int8",
            CompressionTier::Binary => "binary",
        }
    }

    /// Parse from a string (from Qdrant payload).
    pub fn from_str_opt(s: &str) -> Option<Self> {
        match s {
            "full" => Some(CompressionTier::Full),
            "half" => Some(CompressionTier::Half),
            "int8" => Some(CompressionTier::Int8),
            "binary" => Some(CompressionTier::Binary),
            _ => None,
        }
    }

    /// Effective bytes per dimension at this tier.
    pub fn bytes_per_dim(&self) -> f64 {
        match self {
            CompressionTier::Full => 4.0,
            CompressionTier::Half => 2.0,
            CompressionTier::Int8 => 1.0,
            CompressionTier::Binary => 0.125,
        }
    }
}

// ─── Tier Thresholds ───────────────────────────────────────────────

/// Configuration for compression tier age thresholds (in days).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressionConfig {
    /// Enable temporal compression. Default: false.
    pub enabled: bool,
    /// Tier 1 threshold: days after which episodes are f16-quantized.
    pub tier1_days: u32,
    /// Tier 2 threshold: days after which episodes are int8-quantized.
    pub tier2_days: u32,
    /// Tier 3 threshold: days after which episodes are binary-quantized.
    pub tier3_days: u32,
    /// How often the compression sweep runs (seconds).
    pub sweep_interval_secs: u64,
    /// Maximum points to process per sweep (to bound CPU/latency).
    pub max_points_per_sweep: u32,
}

impl Default for CompressionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            tier1_days: 7,
            tier2_days: 30,
            tier3_days: 90,
            sweep_interval_secs: 3600, // 1 hour
            max_points_per_sweep: 1000,
        }
    }
}

impl CompressionConfig {
    /// Determine the target compression tier for a given age.
    pub fn tier_for_age_days(&self, age_days: i64) -> CompressionTier {
        if age_days >= self.tier3_days as i64 {
            CompressionTier::Binary
        } else if age_days >= self.tier2_days as i64 {
            CompressionTier::Int8
        } else if age_days >= self.tier1_days as i64 {
            CompressionTier::Half
        } else {
            CompressionTier::Full
        }
    }

    /// Determine the target compression tier for a given timestamp.
    pub fn tier_for_timestamp(&self, created_at: DateTime<Utc>) -> CompressionTier {
        let age_days = (Utc::now() - created_at).num_days();
        self.tier_for_age_days(age_days)
    }

    /// Get the cutoff timestamp for a given tier.
    pub fn cutoff_for_tier(&self, tier: CompressionTier) -> DateTime<Utc> {
        let days = match tier {
            CompressionTier::Full => 0,
            CompressionTier::Half => self.tier1_days,
            CompressionTier::Int8 => self.tier2_days,
            CompressionTier::Binary => self.tier3_days,
        };
        Utc::now() - Duration::days(days as i64)
    }
}

// ─── Quantization Functions ────────────────────────────────────────

/// Quantize an f32 vector to f16-equivalent precision.
/// Rounds each component to f16 precision and stores back as f32.
/// This reduces effective precision from ~7 decimal digits to ~3.
pub fn quantize_f16(vector: &[f32]) -> Vec<f32> {
    vector
        .iter()
        .map(|&v| {
            // f32 -> f16 -> f32 roundtrip via half-precision truncation
            // f16 has 10-bit mantissa, so we truncate to that precision
            let bits = v.to_bits();
            // Zero out the lower 13 bits of the mantissa (23 - 10 = 13)
            let truncated = bits & 0xFFFF_E000;
            f32::from_bits(truncated)
        })
        .collect()
}

/// Quantize an f32 vector to int8 scalar quantization.
/// Maps the dynamic range [min, max] to [-128, 127] and stores as f32.
pub fn quantize_int8(vector: &[f32]) -> Vec<f32> {
    if vector.is_empty() {
        return vec![];
    }
    let min = vector.iter().cloned().fold(f32::INFINITY, f32::min);
    let max = vector.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    let range = (max - min).max(f32::EPSILON);
    let scale = 255.0 / range;
    let inv_scale = range / 255.0;

    vector
        .iter()
        .map(|&v| {
            // Map to [0, 255], round, then map back
            let quantized = ((v - min) * scale).round().clamp(0.0, 255.0);
            min + quantized * inv_scale
        })
        .collect()
}

/// Quantize an f32 vector to binary (sign-based).
/// Each component becomes +1.0 or -1.0 based on sign.
/// Extreme lossy compression but preserves angular direction.
pub fn quantize_binary(vector: &[f32]) -> Vec<f32> {
    vector
        .iter()
        .map(|&v| if v >= 0.0 { 1.0 } else { -1.0 })
        .collect()
}

/// Apply the appropriate quantization for a given tier.
pub fn quantize_for_tier(vector: &[f32], tier: CompressionTier) -> Vec<f32> {
    match tier {
        CompressionTier::Full => vector.to_vec(),
        CompressionTier::Half => quantize_f16(vector),
        CompressionTier::Int8 => quantize_int8(vector),
        CompressionTier::Binary => quantize_binary(vector),
    }
}

// ─── Compression Statistics ────────────────────────────────────────

/// Atomic counters for compression stats. Thread-safe for background tasks.
#[derive(Debug, Default)]
pub struct CompressionStats {
    /// Total points at each tier.
    pub tier0_full_count: AtomicU64,
    pub tier1_half_count: AtomicU64,
    pub tier2_int8_count: AtomicU64,
    pub tier3_binary_count: AtomicU64,
    /// Total points compressed during the last sweep.
    pub last_sweep_compressed: AtomicU64,
    /// Timestamp (epoch seconds) of last completed sweep.
    pub last_sweep_epoch: AtomicU64,
    /// Total points examined during the last sweep.
    pub last_sweep_examined: AtomicU64,
    /// Total sweeps completed.
    pub total_sweeps: AtomicU64,
}

impl CompressionStats {
    /// Snapshot the stats as a JSON value.
    pub fn to_json(&self, config: &CompressionConfig, dimensions: u32) -> serde_json::Value {
        let t0 = self.tier0_full_count.load(Ordering::Relaxed);
        let t1 = self.tier1_half_count.load(Ordering::Relaxed);
        let t2 = self.tier2_int8_count.load(Ordering::Relaxed);
        let t3 = self.tier3_binary_count.load(Ordering::Relaxed);
        let total = t0 + t1 + t2 + t3;

        // Compute storage estimates
        let dims = dimensions as f64;
        let bytes_full = t0 as f64 * dims * CompressionTier::Full.bytes_per_dim();
        let bytes_half = t1 as f64 * dims * CompressionTier::Half.bytes_per_dim();
        let bytes_int8 = t2 as f64 * dims * CompressionTier::Int8.bytes_per_dim();
        let bytes_binary = t3 as f64 * dims * CompressionTier::Binary.bytes_per_dim();
        let actual_bytes = bytes_full + bytes_half + bytes_int8 + bytes_binary;
        let uncompressed_bytes = total as f64 * dims * 4.0;
        let savings_pct = if uncompressed_bytes > 0.0 {
            (1.0 - actual_bytes / uncompressed_bytes) * 100.0
        } else {
            0.0
        };

        let last_sweep = self.last_sweep_epoch.load(Ordering::Relaxed);
        let last_sweep_ts = if last_sweep > 0 {
            DateTime::from_timestamp(last_sweep as i64, 0)
                .map(|dt| dt.to_rfc3339())
                .unwrap_or_else(|| "unknown".to_string())
        } else {
            "never".to_string()
        };

        serde_json::json!({
            "enabled": config.enabled,
            "dimensions": dimensions,
            "tiers": {
                "full": {
                    "count": t0,
                    "age_range": format!("0-{} days", config.tier1_days),
                    "precision": "f32",
                    "estimated_bytes": bytes_full as u64,
                },
                "half": {
                    "count": t1,
                    "age_range": format!("{}-{} days", config.tier1_days, config.tier2_days),
                    "precision": "f16",
                    "estimated_bytes": bytes_half as u64,
                },
                "int8": {
                    "count": t2,
                    "age_range": format!("{}-{} days", config.tier2_days, config.tier3_days),
                    "precision": "int8",
                    "estimated_bytes": bytes_int8 as u64,
                },
                "binary": {
                    "count": t3,
                    "age_range": format!("{}+ days", config.tier3_days),
                    "precision": "binary",
                    "estimated_bytes": bytes_binary as u64,
                },
            },
            "total_points": total,
            "storage": {
                "estimated_bytes": actual_bytes as u64,
                "uncompressed_bytes": uncompressed_bytes as u64,
                "savings_percent": (savings_pct * 100.0).round() / 100.0,
            },
            "sweep": {
                "interval_secs": config.sweep_interval_secs,
                "max_points_per_sweep": config.max_points_per_sweep,
                "last_sweep_at": last_sweep_ts,
                "last_sweep_compressed": self.last_sweep_compressed.load(Ordering::Relaxed),
                "last_sweep_examined": self.last_sweep_examined.load(Ordering::Relaxed),
                "total_sweeps": self.total_sweeps.load(Ordering::Relaxed),
            },
        })
    }

    /// Reset tier counts before a fresh count.
    pub fn reset_tier_counts(&self) {
        self.tier0_full_count.store(0, Ordering::Relaxed);
        self.tier1_half_count.store(0, Ordering::Relaxed);
        self.tier2_int8_count.store(0, Ordering::Relaxed);
        self.tier3_binary_count.store(0, Ordering::Relaxed);
    }

    /// Increment the counter for a given tier.
    pub fn increment_tier(&self, tier: CompressionTier) {
        match tier {
            CompressionTier::Full => self.tier0_full_count.fetch_add(1, Ordering::Relaxed),
            CompressionTier::Half => self.tier1_half_count.fetch_add(1, Ordering::Relaxed),
            CompressionTier::Int8 => self.tier2_int8_count.fetch_add(1, Ordering::Relaxed),
            CompressionTier::Binary => self.tier3_binary_count.fetch_add(1, Ordering::Relaxed),
        };
    }
}

// ─── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tier_for_age_days() {
        let config = CompressionConfig::default();
        assert_eq!(config.tier_for_age_days(0), CompressionTier::Full);
        assert_eq!(config.tier_for_age_days(3), CompressionTier::Full);
        assert_eq!(config.tier_for_age_days(6), CompressionTier::Full);
        assert_eq!(config.tier_for_age_days(7), CompressionTier::Half);
        assert_eq!(config.tier_for_age_days(15), CompressionTier::Half);
        assert_eq!(config.tier_for_age_days(29), CompressionTier::Half);
        assert_eq!(config.tier_for_age_days(30), CompressionTier::Int8);
        assert_eq!(config.tier_for_age_days(60), CompressionTier::Int8);
        assert_eq!(config.tier_for_age_days(89), CompressionTier::Int8);
        assert_eq!(config.tier_for_age_days(90), CompressionTier::Binary);
        assert_eq!(config.tier_for_age_days(365), CompressionTier::Binary);
    }

    #[test]
    fn test_tier_ordering() {
        assert!(CompressionTier::Full < CompressionTier::Half);
        assert!(CompressionTier::Half < CompressionTier::Int8);
        assert!(CompressionTier::Int8 < CompressionTier::Binary);
    }

    #[test]
    fn test_tier_as_str_roundtrip() {
        for tier in [
            CompressionTier::Full,
            CompressionTier::Half,
            CompressionTier::Int8,
            CompressionTier::Binary,
        ] {
            let s = tier.as_str();
            assert_eq!(CompressionTier::from_str_opt(s), Some(tier));
        }
        assert_eq!(CompressionTier::from_str_opt("unknown"), None);
    }

    #[test]
    fn test_quantize_f16_preserves_magnitude() {
        let v = vec![0.123_456_79, -0.987_654_3, 0.0, 1.0, -1.0];
        let q = quantize_f16(&v);
        assert_eq!(q.len(), v.len());
        for (orig, quant) in v.iter().zip(q.iter()) {
            // f16 precision: ~3 decimal digits
            assert!(
                (orig - quant).abs() < 0.01,
                "f16 quantization too lossy: {} -> {}",
                orig,
                quant
            );
        }
    }

    #[test]
    fn test_quantize_f16_reduces_unique_values() {
        // Two values very close together should quantize to the same f16
        let v = vec![0.12345, 0.12346, 0.12347];
        let q = quantize_f16(&v);
        // All should be the same after f16 truncation
        assert_eq!(q[0], q[1]);
        assert_eq!(q[1], q[2]);
    }

    #[test]
    fn test_quantize_int8_preserves_range() {
        let v = vec![-1.0, -0.5, 0.0, 0.5, 1.0];
        let q = quantize_int8(&v);
        assert_eq!(q.len(), v.len());
        // Endpoints should be preserved exactly
        assert!((q[0] - (-1.0)).abs() < 0.01);
        assert!((q[4] - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_quantize_int8_reduces_to_256_levels() {
        // 1000 distinct values should map to at most 256 unique values
        let v: Vec<f32> = (0..1000).map(|i| i as f32 / 999.0).collect();
        let q = quantize_int8(&v);
        let unique: std::collections::HashSet<u32> = q.iter().map(|f| f.to_bits()).collect();
        assert!(
            unique.len() <= 256,
            "int8 should have at most 256 levels, got {}",
            unique.len()
        );
    }

    #[test]
    fn test_quantize_int8_empty() {
        assert!(quantize_int8(&[]).is_empty());
    }

    #[test]
    fn test_quantize_binary_signs() {
        let v = vec![0.5, -0.3, 0.0, -0.001, 1.0];
        let q = quantize_binary(&v);
        assert_eq!(q, vec![1.0, -1.0, 1.0, -1.0, 1.0]);
    }

    #[test]
    fn test_quantize_binary_cosine_similarity_preserves_direction() {
        // Cosine similarity between original and binary-quantized should be positive
        let v = vec![0.5, -0.3, 0.2, -0.7, 0.1];
        let q = quantize_binary(&v);
        let dot: f32 = v.iter().zip(q.iter()).map(|(a, b)| a * b).sum();
        assert!(
            dot > 0.0,
            "binary quantized vector should preserve general direction"
        );
    }

    #[test]
    fn test_quantize_for_tier_full_is_identity() {
        let v = vec![0.1, 0.2, 0.3];
        let q = quantize_for_tier(&v, CompressionTier::Full);
        assert_eq!(v, q);
    }

    #[test]
    fn test_quantize_for_tier_dispatches_correctly() {
        let v = vec![0.5, -0.3, 0.0];
        assert_eq!(
            quantize_for_tier(&v, CompressionTier::Half),
            quantize_f16(&v)
        );
        assert_eq!(
            quantize_for_tier(&v, CompressionTier::Int8),
            quantize_int8(&v)
        );
        assert_eq!(
            quantize_for_tier(&v, CompressionTier::Binary),
            quantize_binary(&v)
        );
    }

    #[test]
    fn test_compression_stats_to_json() {
        let config = CompressionConfig::default();
        let stats = CompressionStats::default();
        stats.tier0_full_count.store(100, Ordering::Relaxed);
        stats.tier1_half_count.store(50, Ordering::Relaxed);
        stats.tier2_int8_count.store(30, Ordering::Relaxed);
        stats.tier3_binary_count.store(20, Ordering::Relaxed);
        stats.total_sweeps.store(5, Ordering::Relaxed);

        let json = stats.to_json(&config, 384);
        assert_eq!(json["total_points"], 200);
        assert_eq!(json["enabled"], false);
        assert_eq!(json["dimensions"], 384);
        assert_eq!(json["tiers"]["full"]["count"], 100);
        assert_eq!(json["tiers"]["half"]["count"], 50);
        assert_eq!(json["tiers"]["int8"]["count"], 30);
        assert_eq!(json["tiers"]["binary"]["count"], 20);
        assert!(json["storage"]["savings_percent"].as_f64().unwrap() > 0.0);
        assert_eq!(json["sweep"]["total_sweeps"], 5);
    }

    #[test]
    fn test_compression_stats_increment_tier() {
        let stats = CompressionStats::default();
        stats.increment_tier(CompressionTier::Full);
        stats.increment_tier(CompressionTier::Full);
        stats.increment_tier(CompressionTier::Half);
        stats.increment_tier(CompressionTier::Int8);
        stats.increment_tier(CompressionTier::Binary);
        assert_eq!(stats.tier0_full_count.load(Ordering::Relaxed), 2);
        assert_eq!(stats.tier1_half_count.load(Ordering::Relaxed), 1);
        assert_eq!(stats.tier2_int8_count.load(Ordering::Relaxed), 1);
        assert_eq!(stats.tier3_binary_count.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_bytes_per_dim_decreasing() {
        assert!(CompressionTier::Full.bytes_per_dim() > CompressionTier::Half.bytes_per_dim());
        assert!(CompressionTier::Half.bytes_per_dim() > CompressionTier::Int8.bytes_per_dim());
        assert!(CompressionTier::Int8.bytes_per_dim() > CompressionTier::Binary.bytes_per_dim());
    }

    #[test]
    fn test_custom_config_tier_thresholds() {
        let config = CompressionConfig {
            enabled: true,
            tier1_days: 3,
            tier2_days: 14,
            tier3_days: 60,
            sweep_interval_secs: 300,
            max_points_per_sweep: 500,
        };
        assert_eq!(config.tier_for_age_days(2), CompressionTier::Full);
        assert_eq!(config.tier_for_age_days(3), CompressionTier::Half);
        assert_eq!(config.tier_for_age_days(13), CompressionTier::Half);
        assert_eq!(config.tier_for_age_days(14), CompressionTier::Int8);
        assert_eq!(config.tier_for_age_days(59), CompressionTier::Int8);
        assert_eq!(config.tier_for_age_days(60), CompressionTier::Binary);
    }
}
