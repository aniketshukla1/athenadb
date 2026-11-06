//! A 3D view of high-dimensional vectors, for the dashboard's explorer.
//!
//! Principal component analysis over unit-length vectors, since cosine is what the
//! store compares. It's a picture and it loses information: `explained` says how
//! much survives, and real neighbours always come from a search in the full space.

use rayon::prelude::*;

const MAX_ITERATIONS: usize = 80;
const CONVERGED: f32 = 1e-5;

pub struct Projection {
    /// One point per input vector, scaled to fit in `[-1, 1]` on every axis.
    pub coords: Vec<[f32; 3]>,
    /// The share of total variance each axis keeps, from 0 to 1.
    pub explained: [f32; 3],
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn normalize(v: &mut [f32]) -> f32 {
    let norm = dot(v, v).sqrt();
    if norm > 0.0 {
        v.iter_mut().for_each(|x| *x /= norm);
    }
    norm
}

/// Project `vectors`, which must all have the same length. Deterministic: the same
/// input gives the same picture.
pub fn project(vectors: &[&[f32]]) -> Projection {
    let n = vectors.len();
    let d = vectors.first().map_or(0, |v| v.len());
    if n == 0 || d == 0 {
        return Projection { coords: vec![[0.0; 3]; n], explained: [0.0; 3] };
    }

    // Unit length, then centred on the mean.
    let mut matrix = vec![0.0f32; n * d];
    matrix.par_chunks_mut(d).zip(vectors.par_iter()).for_each(|(row, v)| {
        row.copy_from_slice(v);
        normalize(row);
    });
    let mut mean = vec![0.0f32; d];
    for row in matrix.chunks(d) {
        mean.iter_mut().zip(row).for_each(|(m, x)| *m += x / n as f32);
    }
    matrix.par_chunks_mut(d).for_each(|row| row.iter_mut().zip(&mean).for_each(|(x, m)| *x -= m));
    let total: f32 = matrix.par_iter().map(|x| x * x).sum::<f32>() / n as f32;

    // Covariance times v, without ever forming the d x d matrix: X^T (X v) / n.
    let covariance_times = |v: &[f32]| -> Vec<f32> {
        let out = matrix
            .par_chunks(d)
            .fold(
                || vec![0.0f32; d],
                |mut acc, row| {
                    let along = dot(row, v);
                    acc.iter_mut().zip(row).for_each(|(a, x)| *a += x * along);
                    acc
                },
            )
            .reduce(
                || vec![0.0f32; d],
                |mut a, b| {
                    a.iter_mut().zip(&b).for_each(|(x, y)| *x += y);
                    a
                },
            );
        out.into_iter().map(|x| x / n as f32).collect()
    };

    // Power iteration for each component, keeping it orthogonal to the earlier ones.
    let mut components: Vec<Vec<f32>> = Vec::new();
    let mut explained = [0.0f32; 3];
    let mut state = 0x9e37_79b9_7f4a_7c15u64;
    for (c, share) in explained.iter_mut().enumerate().take(d.min(3)) {
        let mut v: Vec<f32> = (0..d)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                (state >> 40) as f32 / (1u64 << 24) as f32 - 0.5
            })
            .collect();
        let orthogonalize = |v: &mut Vec<f32>, components: &[Vec<f32>]| {
            for p in components {
                let along = dot(v, p);
                v.iter_mut().zip(p).for_each(|(x, y)| *x -= along * y);
            }
        };
        orthogonalize(&mut v, &components);
        normalize(&mut v);
        let mut eigenvalue = 0.0;
        for _ in 0..MAX_ITERATIONS {
            let mut next = covariance_times(&v);
            orthogonalize(&mut next, &components);
            eigenvalue = normalize(&mut next);
            let moved: f32 = next.iter().zip(&v).map(|(a, b)| (a - b).abs()).sum();
            v = next;
            if eigenvalue == 0.0 || moved < CONVERGED {
                break;
            }
        }
        if total > 0.0 {
            *share = eigenvalue / total;
        }
        debug_assert!(c == components.len());
        components.push(v);
    }

    let mut coords: Vec<[f32; 3]> = matrix
        .par_chunks(d)
        .map(|row| {
            let mut p = [0.0f32; 3];
            for (axis, component) in components.iter().enumerate() {
                p[axis] = dot(row, component);
            }
            p
        })
        .collect();
    let scale = coords.iter().flatten().fold(0.0f32, |m, x| m.max(x.abs()));
    if scale > 0.0 {
        coords.iter_mut().flatten().for_each(|x| *x /= scale);
    }
    Projection { coords, explained }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_is_fine() {
        let p = project(&[]);
        assert!(p.coords.is_empty());
        assert_eq!(p.explained, [0.0; 3]);
    }

    #[test]
    fn separated_clusters_stay_separated() {
        // Two tight clusters at right angles: the first axis has to split them.
        let mut owned = Vec::new();
        for i in 0..20 {
            let e = i as f32 * 0.001;
            owned.push(vec![1.0, e, 0.0, 0.0]);
            owned.push(vec![e, 1.0, 0.0, 0.0]);
        }
        let refs: Vec<&[f32]> = owned.iter().map(|v| v.as_slice()).collect();
        let p = project(&refs);
        assert!(p.explained[0] > 0.9, "first axis should dominate: {:?}", p.explained);
        for pair in p.coords.chunks(2) {
            assert!((pair[0][0] - pair[1][0]).abs() > 1.5, "clusters overlap: {pair:?}");
        }
    }

    #[test]
    fn output_fits_in_the_unit_cube_and_is_deterministic() {
        let owned: Vec<Vec<f32>> = (0..50)
            .map(|i| (0..8).map(|j| ((i * 7 + j * 13) % 11) as f32 - 5.0).collect())
            .collect();
        let refs: Vec<&[f32]> = owned.iter().map(|v| v.as_slice()).collect();
        let a = project(&refs);
        let b = project(&refs);
        assert_eq!(a.coords, b.coords);
        assert!(a.coords.iter().flatten().all(|x| x.abs() <= 1.0));
        assert!(a.explained.iter().sum::<f32>() <= 1.0 + 1e-3);
    }

    #[test]
    fn fewer_dimensions_than_axes_leave_the_rest_at_zero() {
        let owned = [vec![1.0, 0.0], vec![0.0, 1.0], vec![1.0, 1.0]];
        let refs: Vec<&[f32]> = owned.iter().map(|v| v.as_slice()).collect();
        let p = project(&refs);
        assert!(p.coords.iter().all(|c| c[2] == 0.0));
    }
}
