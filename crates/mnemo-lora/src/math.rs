//! Pure-Rust matrix operations for TinyLoRA.
//!
//! All matrices are stored as flat `Vec<f32>` in row-major order.
//!
//! **Shapes used:**
//! - A: r×d  — `a[i][j] = a_flat[i * d + j]`
//! - B: d×r  — `b[i][j] = b_flat[i * r + j]`
//!
//! Adaptation formula (in matrix notation):
//! `v_adapted = v + scale * B · (A · v)`
//!
//! Steps:
//! 1. `h = A · v`  — shape r   (down-project: d→r)
//! 2. `u = B · h`  — shape d   (up-project:   r→d)
//! 3. `v_adapted = v + scale * u`

/// Compute `h = A · v` where A is shape r×d (stored row-major in `a_flat`).
/// Returns a Vec of length `r`.
pub fn mat_vec_r_times_d(a_flat: &[f32], v: &[f32], r: usize, d: usize) -> Vec<f32> {
    debug_assert_eq!(a_flat.len(), r * d);
    debug_assert_eq!(v.len(), d);
    let mut h = vec![0.0f32; r];
    for i in 0..r {
        let row_start = i * d;
        let mut acc = 0.0f32;
        for j in 0..d {
            acc += a_flat[row_start + j] * v[j];
        }
        h[i] = acc;
    }
    h
}

/// Compute `u = B · h` where B is shape d×r (stored row-major in `b_flat`).
/// Returns a Vec of length `d`.
pub fn mat_vec_d_times_r(b_flat: &[f32], h: &[f32], d: usize, r: usize) -> Vec<f32> {
    debug_assert_eq!(b_flat.len(), d * r);
    debug_assert_eq!(h.len(), r);
    let mut u = vec![0.0f32; d];
    for i in 0..d {
        let row_start = i * r;
        let mut acc = 0.0f32;
        for j in 0..r {
            acc += b_flat[row_start + j] * h[j];
        }
        u[i] = acc;
    }
    u
}

/// Apply the LoRA residual:
/// `v_adapted[i] = v[i] + scale * u[i]`
pub fn add_scaled(base: &[f32], residual: &[f32], scale: f32) -> Vec<f32> {
    debug_assert_eq!(base.len(), residual.len());
    base.iter()
        .zip(residual.iter())
        .map(|(b, r)| b + scale * r)
        .collect()
}

/// Full adaptation: `v_adapted = v + scale * B · (A · v)`
pub fn adapt(
    a_flat: &[f32],
    b_flat: &[f32],
    v: &[f32],
    scale: f32,
    d: usize,
    r: usize,
) -> Vec<f32> {
    let h = mat_vec_r_times_d(a_flat, v, r, d);
    let u = mat_vec_d_times_r(b_flat, &h, d, r);
    add_scaled(v, &u, scale)
}

/// L2 norm of a vector.
#[inline]
pub fn l2_norm(v: &[f32]) -> f32 {
    v.iter().map(|x| x * x).sum::<f32>().sqrt()
}

/// Normalize a vector in-place to unit length.  No-op if norm is zero.
pub fn normalize_in_place(v: &mut [f32]) {
    let norm = l2_norm(v);
    if norm > 1e-10 {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
}

/// Frobenius norm of a flat matrix.
pub fn frobenius_norm(flat: &[f32]) -> f32 {
    flat.iter().map(|x| x * x).sum::<f32>().sqrt()
}

/// Outer product update:  `M += lr * outer(u, v)`
/// M is shape m×n stored row-major. `u` has length m, `v` has length n.
pub fn outer_add(m_flat: &mut [f32], u: &[f32], v: &[f32], lr: f32, rows: usize, cols: usize) {
    debug_assert_eq!(m_flat.len(), rows * cols);
    debug_assert_eq!(u.len(), rows);
    debug_assert_eq!(v.len(), cols);
    for i in 0..rows {
        let row_start = i * cols;
        for j in 0..cols {
            m_flat[row_start + j] += lr * u[i] * v[j];
        }
    }
}

/// Clamp B's Frobenius norm to `max_norm` to prevent unbounded growth.
/// If `||B||_F > max_norm`, scale B down uniformly.
pub fn clamp_frobenius(b_flat: &mut [f32], max_norm: f32) {
    let norm = frobenius_norm(b_flat);
    if norm > max_norm {
        let scale = max_norm / norm;
        for x in b_flat.iter_mut() {
            *x *= scale;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identity_when_b_zero() {
        let d = 4;
        let r = 2;
        let a_flat = vec![1.0, 0.0, 0.0, 1.0, 0.5, 0.5, 0.5, 0.5]; // r×d
        let b_flat = vec![0.0f32; d * r]; // zero B
        let v = vec![1.0, 2.0, 3.0, 4.0];
        let adapted = adapt(&a_flat, &b_flat, &v, 0.125, d, r);
        // B=0 → residual=0 → v_adapted == v
        assert_eq!(adapted, v, "Zero B must yield identity adaptation");
    }

    #[test]
    fn test_nonzero_b_changes_output() {
        let d = 4;
        let r = 2;
        let a_flat = vec![1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0]; // r×d
        let b_flat = vec![1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0]; // d×r
        let v = vec![1.0, 2.0, 3.0, 4.0];
        let adapted = adapt(&a_flat, &b_flat, &v, 0.125, d, r);
        // A·v = [1.0, 2.0]
        // B·h = [1.0, 2.0, 0.0, 0.0]
        // adapted = v + 0.125 * [1,2,0,0] = [1.125, 2.25, 3.0, 4.0]
        assert!((adapted[0] - 1.125).abs() < 1e-5);
        assert!((adapted[1] - 2.25).abs() < 1e-5);
        assert!((adapted[2] - 3.0).abs() < 1e-5);
        assert!((adapted[3] - 4.0).abs() < 1e-5);
    }

    #[test]
    fn test_l2_norm() {
        let v = vec![3.0, 4.0];
        assert!((l2_norm(&v) - 5.0).abs() < 1e-5);
    }

    #[test]
    fn test_normalize() {
        let mut v = vec![0.0, 3.0, 4.0];
        normalize_in_place(&mut v);
        assert!((l2_norm(&v) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn test_normalize_zero_vector_noop() {
        let mut v = vec![0.0, 0.0, 0.0];
        normalize_in_place(&mut v);
        assert_eq!(v, vec![0.0, 0.0, 0.0]);
    }

    #[test]
    fn test_clamp_frobenius() {
        let mut b = vec![3.0, 4.0]; // norm = 5
        clamp_frobenius(&mut b, 1.0);
        assert!((frobenius_norm(&b) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn test_clamp_frobenius_no_op_when_under_limit() {
        let mut b = vec![0.1, 0.1];
        let orig = b.clone();
        clamp_frobenius(&mut b, 10.0);
        assert_eq!(b, orig);
    }

    #[test]
    fn test_outer_add() {
        let mut m = vec![0.0f32; 4]; // 2×2
        let u = vec![1.0, 2.0];
        let v = vec![3.0, 4.0];
        outer_add(&mut m, &u, &v, 1.0, 2, 2);
        // outer = [[3, 4], [6, 8]]
        assert!((m[0] - 3.0).abs() < 1e-5);
        assert!((m[1] - 4.0).abs() < 1e-5);
        assert!((m[2] - 6.0).abs() < 1e-5);
        assert!((m[3] - 8.0).abs() < 1e-5);
    }
}
