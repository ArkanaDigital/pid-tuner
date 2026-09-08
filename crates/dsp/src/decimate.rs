//! Min/max decimation for plotting long series without aliasing visual peaks.

/// Decimate `(x, y)` into at most `2 * px_width` points: for each pixel bucket
/// emit the min and max sample (in time order). Returns `(xs, ys)`.
pub fn minmax(x: &[f32], y: &[f32], px_width: usize) -> (Vec<f32>, Vec<f32>) {
    assert_eq!(x.len(), y.len());
    let n = x.len();
    if n == 0 || px_width == 0 {
        return (vec![], vec![]);
    }
    if n <= 2 * px_width {
        return (x.to_vec(), y.to_vec());
    }
    let bucket = n as f32 / px_width as f32;
    let mut xs = Vec::with_capacity(2 * px_width);
    let mut ys = Vec::with_capacity(2 * px_width);
    for b in 0..px_width {
        let start = (b as f32 * bucket) as usize;
        let end = (((b + 1) as f32 * bucket) as usize).min(n);
        if start >= end {
            continue;
        }
        let mut imin = start;
        let mut imax = start;
        for i in start..end {
            if y[i] < y[imin] {
                imin = i;
            }
            if y[i] > y[imax] {
                imax = i;
            }
        }
        let (a, c) = if imin <= imax {
            (imin, imax)
        } else {
            (imax, imin)
        };
        xs.push(x[a]);
        ys.push(y[a]);
        if c != a {
            xs.push(x[c]);
            ys.push(y[c]);
        }
    }
    (xs, ys)
}

/// Slice indices `[i0, i1)` of `x` (ascending) covering `[t0, t1]`.
pub fn range_indices(x: &[f32], t0: f32, t1: f32) -> (usize, usize) {
    let i0 = x.partition_point(|v| *v < t0);
    let i1 = x.partition_point(|v| *v <= t1);
    (i0, i1.max(i0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_extremes() {
        let x: Vec<f32> = (0..1000).map(|i| i as f32).collect();
        let mut y = vec![0f32; 1000];
        y[500] = 100.0;
        y[501] = -100.0;
        let (_, ys) = minmax(&x, &y, 10);
        assert!(ys.contains(&100.0));
        assert!(ys.contains(&-100.0));
        assert!(ys.len() <= 20);
    }
}
