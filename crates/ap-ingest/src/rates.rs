//! Measured logging rates and time-base sanity.

/// Median sample period → rate in Hz. Requires ≥ 8 samples with positive deltas.
pub fn measure_rate_hz(time_us: &[u64]) -> Option<f64> {
    let mut d: Vec<u64> = time_us
        .windows(2)
        .filter(|w| w[1] > w[0])
        .map(|w| w[1] - w[0])
        .collect();
    if d.len() < 8 {
        return None;
    }
    d.sort_unstable();
    let med = d[d.len() / 2] as f64;
    (med > 0.0).then(|| 1e6 / med)
}

/// Indices whose `TimeUS` is plausible: within `[t0, t0 + max_span_us]` and
/// not earlier than the previous kept sample by more than `back_us`.
/// Corrupt payloads that survive length validation show up as absurd stamps.
pub fn plausible_mask(time_us: &[u64], t0: u64, max_span_us: u64, back_us: u64) -> Vec<bool> {
    let mut last = t0;
    time_us
        .iter()
        .map(|&t| {
            let ok =
                t >= t0.saturating_sub(back_us) && t <= t0 + max_span_us && t + back_us >= last;
            if ok {
                last = t;
            }
            ok
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_from_median_ignores_gaps() {
        let mut t: Vec<u64> = (0..100).map(|i| i * 2500).collect(); // 400 Hz
        t.push(10_000_000); // one huge gap
        t.extend((0..50).map(|i| 10_000_000 + (i + 1) * 2500));
        assert!((measure_rate_hz(&t).unwrap() - 400.0).abs() < 0.01);
        assert!(measure_rate_hz(&[1, 2, 3]).is_none());
    }

    #[test]
    fn plausible_mask_drops_outliers() {
        let t = [100, 200, 300, 14_988_346_796_772_687_872, 400, 50, 500];
        let m = plausible_mask(&t, 100, 60 * 60 * 1_000_000, 1_000_000);
        assert_eq!(m, vec![true, true, true, false, true, true, true]);
    }
}
