//! Betaflight CHIRP sweep detection.
//!
//! Port of the segmentation in betaflight-configurator
//! `src/js/blackbox/chirp_bbl_parser.js` (GPL-3.0-or-later): coarse gate =
//! the BOXCHIRP flight-mode bit, fine gate = `debug[1]` (active axis, −1 when
//! off); frames whose axis value is outside −1..2 (decoder re-sync artefacts,
//! configurator PR #5257) are ignored without splitting the run.

use domain::*;
use std::collections::BTreeMap;

/// `boxId_e` index of BOXCHIRP (betaflight `src/main/fc/rc_modes.h`, 2026.6.1:
/// ARM 0, ANGLE 1, HORIZON 2, MAG 3, ALTHOLD 4, HEADFREE 5, CHIRP 6).
pub const BOXCHIRP_BIT: u32 = 6;
pub const BOXANGLE_BIT: u32 = 1;
pub const BOXHORIZON_BIT: u32 = 2;

pub fn config_from_headers(h: &BTreeMap<String, String>) -> Option<ChirpConfig> {
    let g = |k: &str| h.get(k).and_then(|v| v.trim().parse::<f32>().ok());
    let any = [
        "chirp_lag_freq_hz",
        "chirp_lead_freq_hz",
        "chirp_amplitude_roll",
        "chirp_frequency_start_deci_hz",
        "chirp_frequency_end_deci_hz",
        "chirp_time_seconds",
    ]
    .iter()
    .any(|k| h.contains_key(*k));
    if !any {
        return None;
    }
    let d = ChirpConfig::default();
    Some(ChirpConfig {
        lag_freq_hz: g("chirp_lag_freq_hz").unwrap_or(d.lag_freq_hz),
        lead_freq_hz: g("chirp_lead_freq_hz").unwrap_or(d.lead_freq_hz),
        amplitude: [
            g("chirp_amplitude_roll").unwrap_or(d.amplitude[0] as f32) as u16,
            g("chirp_amplitude_pitch").unwrap_or(d.amplitude[1] as f32) as u16,
            g("chirp_amplitude_yaw").unwrap_or(d.amplitude[2] as f32) as u16,
        ],
        f_start_hz: g("chirp_frequency_start_deci_hz")
            .map(|v| v / 10.0)
            .unwrap_or(d.f_start_hz),
        f_end_hz: g("chirp_frequency_end_deci_hz")
            .map(|v| v / 10.0)
            .unwrap_or(d.f_end_hz),
        time_s: g("chirp_time_seconds").unwrap_or(d.time_s),
    })
}

/// Does `debug[1]`/`debug[2]` look like DEBUG_CHIRP output? (`debug[1]` ∈ −1..2
/// for ≥ 99 % of samples, at least one run with axis ≥ 0, and `debug[2]/10`
/// non-decreasing inside such runs.)
pub fn looks_like_chirp(debug1: &[f32], debug2: Option<&[f32]>) -> bool {
    if debug1.len() < 100 {
        return false;
    }
    let ok = debug1
        .iter()
        .filter(|v| (-1.0..=2.0).contains(*v) && v.fract() == 0.0)
        .count();
    if (ok as f64) < 0.99 * debug1.len() as f64 {
        return false;
    }
    // Inside axis runs the frequency channel must sweep upwards; a run holds
    // several back-to-back sweeps, so restarts are allowed but must be rare.
    let mut runs = 0usize;
    let mut prev = -1.0f32;
    let mut up = 0usize;
    let mut down = 0usize;
    for (i, &a) in debug1.iter().enumerate() {
        if a >= 0.0 && prev < 0.0 {
            runs += 1;
        }
        if a >= 0.0 && prev >= 0.0 {
            if let Some(d2) = debug2 {
                let (f0, f1) = (d2[i - 1], d2[i]);
                if f1 > f0 + 1e-3 {
                    up += 1;
                } else if f1 + 1e-3 < f0 {
                    down += 1;
                }
            }
        }
        prev = a;
    }
    runs >= 1 && (debug2.is_none() || (up > 0 && down * 5 < up))
}

/// Locate sweeps on the uniform grid.
///
/// * `debug1` — active axis per sample (−1 off), nearest-resampled.
/// * `debug2` — chirp frequency ×10 per sample (optional, only for `f_start/f_end`).
/// * `mode_flags` — `flightModeFlags` per sample from slow frames (None when not decoded);
///   bit 6 (BOXCHIRP) gates the run, bits 1/2 (ANGLE/HORIZON) tag it and split it.
/// * `segment_size` — Welch window; shorter runs are dropped.
pub fn detect_segments(
    t: &[f32],
    debug1: Option<&[f32]>,
    debug2: Option<&[f32]>,
    mode_flags: Option<&[u32]>,
    segment_size: usize,
) -> Vec<ChirpSegment> {
    let n = t.len();
    let mut out = Vec::new();
    let chirp_flag = mode_flags.is_some();
    let source = match (debug1.is_some(), chirp_flag) {
        (true, true) => ChirpGate::Both,
        (true, false) => ChirpGate::Debug,
        (false, true) => ChirpGate::FlightMode,
        (false, false) => return out,
    };
    // Per-sample axis: from debug[1]; with only the flight-mode flag we cannot
    // tell the axis (Configurator needs debug[1] too), so gate-only logs yield nothing.
    let Some(d1) = debug1 else { return out };
    let mut cur_axis: i32 = -1;
    let mut cur_angle = false;
    let mut start = 0usize;
    let close = |out: &mut Vec<ChirpSegment>, axis: i32, angle: bool, i0: usize, i1: usize| {
        // trim one sample at each edge (resampling smears the transitions)
        let (a, b) = (i0 + 1, i1.saturating_sub(1));
        if axis < 0 || b <= a || b - a < segment_size {
            return;
        }
        // sweep range = min positive / max of the frequency channel over the run
        let (f_start_hz, f_end_hz) = match debug2 {
            Some(d) => {
                let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
                for v in &d[a..b] {
                    if *v > 0.0 {
                        lo = lo.min(*v);
                        hi = hi.max(*v);
                    }
                }
                if hi.is_finite() {
                    (lo / 10.0, hi / 10.0)
                } else {
                    (f32::NAN, f32::NAN)
                }
            }
            None => (f32::NAN, f32::NAN),
        };
        out.push(ChirpSegment {
            axis: Axis::ALL[axis as usize],
            i0: a,
            i1: b,
            t0_s: t[a],
            t1_s: t[b.min(n - 1)],
            f_start_hz,
            f_end_hz,
            source,
            angle_mode: angle,
        });
    };
    for i in 0..n {
        let raw = d1[i];
        let flags = mode_flags.map(|f| f[i]);
        let gate = flags.map(|m| m & (1 << BOXCHIRP_BIT) != 0).unwrap_or(true);
        let axis: i32 = if raw.is_finite() && raw.fract() == 0.0 && (-1.0..=2.0).contains(&raw) {
            raw as i32
        } else {
            i32::MIN
        };
        if axis == i32::MIN {
            continue; // corrupt frame: ignore, do not split
        }
        let axis = if gate { axis } else { -1 };
        // ANGLE/HORIZON level roll and pitch only; yaw stays a pure rate loop.
        let angle = axis >= 0
            && axis != 2
            && flags
                .map(|m| m & ((1 << BOXANGLE_BIT) | (1 << BOXHORIZON_BIT)) != 0)
                .unwrap_or(false);
        if axis != cur_axis || (axis >= 0 && angle != cur_angle) {
            close(&mut out, cur_axis, cur_angle, start, i);
            cur_axis = axis;
            cur_angle = angle;
            start = i;
        }
    }
    close(&mut out, cur_axis, cur_angle, start, n);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seq(parts: &[(f32, usize)]) -> Vec<f32> {
        parts
            .iter()
            .flat_map(|(v, n)| std::iter::repeat_n(*v, *n))
            .collect()
    }

    #[test]
    fn segments_from_debug_axis_with_corrupt_frames() {
        let fs = 2000.0;
        let d1 = seq(&[
            (-1.0, 100),
            (0.0, 5000),
            (-1.0, 50),
            (1.0, 5000),
            (7.0, 3),
            (1.0, 2000),
            (-1.0, 10),
        ]);
        let t: Vec<f32> = (0..d1.len()).map(|i| i as f32 / fs).collect();
        let segs = detect_segments(&t, Some(&d1), None, None, 1024);
        assert_eq!(segs.len(), 2, "{segs:?}");
        assert_eq!(segs[0].axis, Axis::Roll);
        assert_eq!(segs[0].i1 - segs[0].i0, 5000 - 2);
        assert_eq!(segs[1].axis, Axis::Pitch);
        // corrupt "7" frames are dropped, not split: 5000 + 3(ignored) + 2000 → one run of 7003 grid samples
        assert_eq!(segs[1].i1 - segs[1].i0, 7003 - 2);
        assert_eq!(segs[1].source, ChirpGate::Debug);
        assert!(segs[0].f_start_hz.is_nan());
    }

    #[test]
    fn short_runs_are_dropped_and_gate_masks_axis() {
        let d1 = seq(&[(0.0, 300), (-1.0, 10), (2.0, 3000)]);
        let t: Vec<f32> = (0..d1.len()).map(|i| i as f32 / 2000.0).collect();
        let segs = detect_segments(&t, Some(&d1), None, None, 1024);
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].axis, Axis::Yaw);
        // flight-mode gate never set → no segments
        let flag = vec![0u32; d1.len()];
        assert!(detect_segments(&t, Some(&d1), None, Some(&flag), 1024).is_empty());
        // gate set only during the yaw run → Both
        let flag: Vec<u32> = (0..d1.len())
            .map(|i| if i >= 310 { 1 << BOXCHIRP_BIT } else { 0 })
            .collect();
        let s = detect_segments(&t, Some(&d1), None, Some(&flag), 1024);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].source, ChirpGate::Both);
        assert!(!s[0].angle_mode);
        assert!(detect_segments(&t, None, None, Some(&flag), 1024).is_empty());
        // ANGLE mode does not affect yaw (still a rate loop): one segment, not tagged
        let flag: Vec<u32> = (0..d1.len())
            .map(|i| {
                if i >= 310 {
                    (1 << BOXCHIRP_BIT) | if i >= 1810 { 1 << BOXANGLE_BIT } else { 0 }
                } else {
                    0
                }
            })
            .collect();
        let s = detect_segments(&t, Some(&d1), None, Some(&flag), 1024);
        assert_eq!(s.len(), 1, "{s:?}");
        assert!(!s[0].angle_mode);
        // …but a pitch run is split where ANGLE switches on and the second part is tagged
        let d1p = seq(&[(1.0, 3300)]);
        let tp: Vec<f32> = (0..d1p.len()).map(|i| i as f32 / 2000.0).collect();
        let flag: Vec<u32> = (0..d1p.len())
            .map(|i| (1 << BOXCHIRP_BIT) | if i >= 1650 { 1 << BOXANGLE_BIT } else { 0 })
            .collect();
        let s = detect_segments(&tp, Some(&d1p), None, Some(&flag), 1024);
        assert_eq!(s.len(), 2, "{s:?}");
        assert!(!s[0].angle_mode && s[1].angle_mode);
    }

    #[test]
    fn frequency_endpoints_from_debug2() {
        let d1 = seq(&[(-1.0, 10), (0.0, 3000), (-1.0, 10)]);
        let d2: Vec<f32> = (0..d1.len())
            .map(|i| {
                if (10..3010).contains(&i) {
                    2.0 + (i - 10) as f32 * 2.0
                } else {
                    0.0
                }
            })
            .collect();
        let t: Vec<f32> = (0..d1.len()).map(|i| i as f32 / 2000.0).collect();
        let s = detect_segments(&t, Some(&d1), Some(&d2), None, 512);
        assert_eq!(s.len(), 1);
        assert!((s[0].f_start_hz - 0.4).abs() < 0.01, "{}", s[0].f_start_hz);
        assert!(s[0].f_end_hz > 590.0, "{}", s[0].f_end_hz);
        assert!(looks_like_chirp(&d1, Some(&d2)));
    }

    #[test]
    fn signature_rejects_non_chirp_debug() {
        let junk: Vec<f32> = (0..1000).map(|i| (i % 37) as f32 * 3.0).collect();
        assert!(!looks_like_chirp(&junk, None));
        let all_off = vec![-1.0f32; 1000];
        assert!(!looks_like_chirp(&all_off, None));
        // a frequency channel that mostly falls is not a chirp
        let d1 = seq(&[(0.0, 400)]);
        let d2: Vec<f32> = (0..400).map(|i| (6000 - i * 15) as f32).collect();
        assert!(!looks_like_chirp(&d1, Some(&d2)));
        // several back-to-back sweeps in one run are fine
        let d2: Vec<f32> = (0..400).map(|i| ((i % 100) * 60) as f32).collect();
        assert!(looks_like_chirp(&d1, Some(&d2)));
    }

    #[test]
    fn header_config_and_defaults() {
        let mut h = BTreeMap::new();
        assert!(config_from_headers(&h).is_none());
        h.insert("chirp_frequency_start_deci_hz".into(), "2".into());
        h.insert("chirp_frequency_end_deci_hz".into(), "6000".into());
        h.insert("chirp_time_seconds".into(), "20".into());
        h.insert("chirp_amplitude_yaw".into(), "150".into());
        let c = config_from_headers(&h).unwrap();
        assert!((c.f_start_hz - 0.2).abs() < 1e-6 && (c.f_end_hz - 600.0).abs() < 1e-6);
        assert_eq!(c.amplitude, [230, 230, 150]);
        assert_eq!(c.lag_freq_hz, 3.0);
    }
}
