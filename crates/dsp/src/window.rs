//! Window functions and correction factors (port of WebTools `fft.js`).

/// Symmetric Hann window, matches MATLAB `hann(n)` / numpy `hanning(n)`.
pub fn hann(n: usize) -> Vec<f32> {
    if n == 1 {
        return vec![1.0];
    }
    let denom = (n - 1) as f32;
    (0..n)
        .map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / denom).cos())
        .collect()
}

/// Symmetric Hamming window, matches MATLAB `hamming(n)`.
pub fn hamming(n: usize) -> Vec<f32> {
    if n == 1 {
        return vec![1.0];
    }
    let denom = (n - 1) as f32;
    (0..n)
        .map(|i| 0.54 - 0.46 * (std::f32::consts::TAU * i as f32 / denom).cos())
        .collect()
}

/// Centered moving average of width `w` samples (MATLAB `smooth(x, w)` with
/// odd `w`; even widths are rounded up). Edges use shrinking windows.
pub fn moving_average(x: &[f32], w: usize) -> Vec<f32> {
    let n = x.len();
    if w <= 1 || n == 0 {
        return x.to_vec();
    }
    let half = w / 2;
    let mut prefix = vec![0f64; n + 1];
    for i in 0..n {
        prefix[i + 1] = prefix[i] + x[i] as f64;
    }
    (0..n)
        .map(|i| {
            // MATLAB smooth: at the edges use the largest symmetric odd window that fits
            let r = half.min(i).min(n - 1 - i);
            let a = i - r;
            let b = i + r + 1;
            ((prefix[b] - prefix[a]) / (b - a) as f64) as f32
        })
        .collect()
}

/// Correction factors for a window: `linear` scales amplitudes, `energy` scales power.
#[derive(Debug, Clone, Copy)]
pub struct WindowCorrection {
    pub linear: f32,
    pub energy: f32,
}

pub fn window_correction(w: &[f32]) -> WindowCorrection {
    let n = w.len() as f32;
    let mean = w.iter().sum::<f32>() / n;
    let mean_sq = w.iter().map(|v| v * v).sum::<f32>() / n;
    WindowCorrection {
        linear: 1.0 / mean,
        energy: 1.0 / mean_sq.sqrt(),
    }
}

/// Multiply `x` by `w` in place (lengths must match).
pub fn apply(x: &mut [f32], w: &[f32]) {
    debug_assert_eq!(x.len(), w.len());
    for (a, b) in x.iter_mut().zip(w) {
        *a *= *b;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    #[test]
    fn hann_endpoints_and_center() {
        let w = hann(9);
        assert_relative_eq!(w[0], 0.0, epsilon = 1e-6);
        assert_relative_eq!(w[8], 0.0, epsilon = 1e-6);
        assert_relative_eq!(w[4], 1.0, epsilon = 1e-6);
    }

    #[test]
    fn hann_energy_correction_is_about_1_633() {
        let w = hann(4096);
        let c = window_correction(&w);
        assert_relative_eq!(c.energy, 1.0 / (0.375f32).sqrt(), epsilon = 2e-3);
        assert_relative_eq!(c.linear, 2.0, epsilon = 2e-3);
    }
}
