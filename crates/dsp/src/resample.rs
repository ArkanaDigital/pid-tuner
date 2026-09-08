//! Resampling irregular / dropped-frame time series onto a uniform grid.

/// Result of gap detection: (start_s, end_s) intervals that were interpolated.
pub type Gaps = Vec<(f32, f32)>;

/// Build a uniform time grid at `fs` covering `[t0, t_end]`.
pub fn uniform_grid(t0: f64, t_end: f64, fs: f64) -> Vec<f32> {
    let n = ((t_end - t0) * fs).floor() as usize + 1;
    (0..n).map(|i| (i as f64 / fs) as f32).collect()
}

/// Linear interpolation of `y(t_src)` onto `t_dst` (both ascending, seconds,
/// `t_dst` relative to `t_src[0]`). Values outside are clamped to the edges.
pub fn interp_linear(t_src: &[f64], y_src: &[f32], t_dst: &[f32]) -> Vec<f32> {
    assert_eq!(t_src.len(), y_src.len());
    let n = t_src.len();
    let mut out = Vec::with_capacity(t_dst.len());
    if n == 0 {
        out.resize(t_dst.len(), 0.0);
        return out;
    }
    let base = t_src[0];
    let mut j = 0usize;
    for &td in t_dst {
        let t = base + td as f64;
        while j + 1 < n && t_src[j + 1] <= t {
            j += 1;
        }
        if j + 1 >= n {
            out.push(y_src[n - 1]);
            continue;
        }
        let (ta, tb) = (t_src[j], t_src[j + 1]);
        let (ya, yb) = (y_src[j], y_src[j + 1]);
        let f = if tb > ta {
            ((t - ta) / (tb - ta)) as f32
        } else {
            0.0
        };
        out.push(ya + (yb - ya) * f.clamp(0.0, 1.0));
    }
    out
}

/// Nearest-sample resampling of `y(t_src)` onto `t_dst` (for discrete-valued
/// series such as Betaflight `debug[]` axis/flag channels, where interpolation
/// would invent intermediate values).
pub fn interp_nearest(t_src: &[f64], y_src: &[f32], t_dst: &[f32]) -> Vec<f32> {
    assert_eq!(t_src.len(), y_src.len());
    let n = t_src.len();
    let mut out = Vec::with_capacity(t_dst.len());
    if n == 0 {
        out.resize(t_dst.len(), 0.0);
        return out;
    }
    let base = t_src[0];
    let mut j = 0usize;
    for &td in t_dst {
        let t = base + td as f64;
        while j + 1 < n && t_src[j + 1] <= t {
            j += 1;
        }
        let k = if j + 1 < n && (t_src[j + 1] - t) < (t - t_src[j]) {
            j + 1
        } else {
            j
        };
        out.push(y_src[k]);
    }
    out
}

/// Find intervals where consecutive source timestamps are more than
/// `max_dt_mult × nominal_dt` apart.
pub fn find_gaps(t_src: &[f64], nominal_dt: f64, max_dt_mult: f64) -> Gaps {
    let mut gaps = Vec::new();
    let base = t_src.first().copied().unwrap_or(0.0);
    for w in t_src.windows(2) {
        if w[1] - w[0] > nominal_dt * max_dt_mult {
            gaps.push(((w[0] - base) as f32, (w[1] - base) as f32));
        }
    }
    gaps
}

/// Median of consecutive differences — robust estimate of the source sample period.
pub fn median_dt(t_src: &[f64]) -> f64 {
    if t_src.len() < 2 {
        return 0.0;
    }
    let mut d: Vec<f64> = t_src.windows(2).map(|w| w[1] - w[0]).collect();
    d.sort_by(|a, b| a.partial_cmp(b).unwrap());
    d[d.len() / 2]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interp_identity_on_grid() {
        let t: Vec<f64> = (0..10).map(|i| i as f64 * 0.001).collect();
        let y: Vec<f32> = (0..10).map(|i| i as f32).collect();
        let grid = uniform_grid(0.0, 0.009, 1000.0);
        let out = interp_linear(&t, &y, &grid);
        assert_eq!(out.len(), 10);
        for (a, b) in out.iter().zip(&y) {
            assert!((a - b).abs() < 1e-4);
        }
    }

    #[test]
    fn nearest_keeps_discrete_values() {
        let t = vec![0.0, 0.001, 0.002, 0.003];
        let y = vec![-1.0, 0.0, 0.0, 2.0];
        let grid = vec![0.0, 0.0004, 0.0006, 0.0026, 0.003, 0.01];
        let out = interp_nearest(&t, &y, &grid);
        assert_eq!(out, vec![-1.0, -1.0, 0.0, 2.0, 2.0, 2.0]);
    }

    #[test]
    fn gaps_detected() {
        let t = vec![0.0, 0.001, 0.002, 0.010, 0.011];
        let g = find_gaps(&t, 0.001, 3.0);
        assert_eq!(g.len(), 1);
        assert!((g[0].0 - 0.002).abs() < 1e-6);
    }
}
