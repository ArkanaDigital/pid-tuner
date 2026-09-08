//! Betaflight header → [`BfTune`]. Lenient: missing keys keep defaults.

use domain::{BfAxisPid, BfFilterConfig, BfFilterType, BfSimplified, BfTune};
use std::collections::BTreeMap;

type H = BTreeMap<String, String>;

fn num<T: std::str::FromStr>(h: &H, key: &str) -> Option<T> {
    h.get(key)?.trim().parse().ok()
}

fn list<T: std::str::FromStr + Copy>(h: &H, key: &str, n: usize) -> Option<Vec<T>> {
    let v: Vec<T> = h
        .get(key)?
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    (v.len() >= n).then_some(v)
}

fn ftype(h: &H, key: &str, default: BfFilterType) -> BfFilterType {
    h.get(key)
        .and_then(|v| BfFilterType::from_cli(v))
        .unwrap_or(default)
}

fn u16_or(h: &H, key: &str, alt: &str, default: u16) -> u16 {
    num(h, key).or_else(|| num(h, alt)).unwrap_or(default)
}

pub fn parse_bf_tune(h: &H) -> BfTune {
    let mut t = BfTune::default();
    t.raw = h.clone();

    for (k, key) in ["rollPID", "pitchPID", "yawPID"].iter().enumerate() {
        if let Some(v) = list::<u16>(h, key, 3) {
            t.pids[k].p = v[0].min(255) as u8;
            t.pids[k].i = v[1].min(255) as u8;
            t.pids[k].d = v[2].min(255) as u8;
        }
    }
    // 2025.12+: `rollPID` D = Derivative, `d_max:` = D Max.
    // ≤ 4.5:     `rollPID` D = D Max,     `d_min:` = Derivative.
    if let Some(v) = list::<u16>(h, "d_max", 3) {
        for k in 0..3 {
            t.pids[k].d_max = v[k].min(255) as u8;
        }
    } else if let Some(v) = list::<u16>(h, "d_min", 3) {
        for k in 0..3 {
            t.pids[k].d_max = t.pids[k].d;
            t.pids[k].d = v[k].min(255) as u8;
        }
    }
    if let Some(v) = list::<u16>(h, "ff_weight", 3) {
        for k in 0..3 {
            t.pids[k].ff = v[k];
        }
    }
    t.d_max_gain = num(h, "d_max_gain").unwrap_or(t.d_max_gain);
    t.d_max_advance = num(h, "d_max_advance").unwrap_or(t.d_max_advance);
    t.anti_gravity_gain = num::<u32>(h, "anti_gravity_gain")
        .map(|v| v.min(u16::MAX as u32) as u16)
        .unwrap_or(t.anti_gravity_gain);
    t.iterm_relax = num(h, "iterm_relax").unwrap_or(t.iterm_relax);
    t.iterm_relax_type = num(h, "iterm_relax_type").unwrap_or(t.iterm_relax_type);
    t.iterm_relax_cutoff = num(h, "iterm_relax_cutoff").unwrap_or(t.iterm_relax_cutoff);
    t.feedforward_smooth_factor =
        num(h, "feedforward_smooth_factor").unwrap_or(t.feedforward_smooth_factor);
    t.feedforward_jitter_factor =
        num(h, "feedforward_jitter_factor").unwrap_or(t.feedforward_jitter_factor);
    t.feedforward_boost = num(h, "feedforward_boost").unwrap_or(t.feedforward_boost);
    t.tpa_rate = num(h, "tpa_rate").unwrap_or(t.tpa_rate);
    t.tpa_breakpoint = num(h, "tpa_breakpoint").unwrap_or(t.tpa_breakpoint);

    let f = &mut t.filters;
    f.gyro_lpf1_static_hz = u16_or(
        h,
        "gyro_lpf1_static_hz",
        "gyro_lowpass_hz",
        f.gyro_lpf1_static_hz,
    );
    f.gyro_lpf1_type = ftype(h, "gyro_lpf1_type", f.gyro_lpf1_type);
    if let Some(v) = list::<u16>(h, "gyro_lpf1_dyn_hz", 2) {
        f.gyro_lpf1_dyn_min_hz = v[0];
        f.gyro_lpf1_dyn_max_hz = v[1];
    } else {
        f.gyro_lpf1_dyn_min_hz = num(h, "dyn_lpf_gyro_min_hz").unwrap_or(f.gyro_lpf1_dyn_min_hz);
        f.gyro_lpf1_dyn_max_hz = num(h, "dyn_lpf_gyro_max_hz").unwrap_or(f.gyro_lpf1_dyn_max_hz);
    }
    f.gyro_lpf2_static_hz = u16_or(
        h,
        "gyro_lpf2_static_hz",
        "gyro_lowpass2_hz",
        f.gyro_lpf2_static_hz,
    );
    f.gyro_lpf2_type = ftype(h, "gyro_lpf2_type", f.gyro_lpf2_type);
    if let Some(v) = list::<u16>(h, "gyro_notch_hz", 2) {
        f.gyro_notch1_hz = v[0];
        f.gyro_notch2_hz = v[1];
    }
    if let Some(v) = list::<u16>(h, "gyro_notch_cutoff", 2) {
        f.gyro_notch1_cutoff = v[0];
        f.gyro_notch2_cutoff = v[1];
    }
    f.dterm_lpf1_static_hz = u16_or(
        h,
        "dterm_lpf1_static_hz",
        "dterm_lowpass_hz",
        f.dterm_lpf1_static_hz,
    );
    f.dterm_lpf1_type = ftype(h, "dterm_lpf1_type", f.dterm_lpf1_type);
    if let Some(v) = list::<u16>(h, "dterm_lpf1_dyn_hz", 2) {
        f.dterm_lpf1_dyn_min_hz = v[0];
        f.dterm_lpf1_dyn_max_hz = v[1];
    } else {
        f.dterm_lpf1_dyn_min_hz = num(h, "dyn_lpf_dterm_min_hz").unwrap_or(f.dterm_lpf1_dyn_min_hz);
        f.dterm_lpf1_dyn_max_hz = num(h, "dyn_lpf_dterm_max_hz").unwrap_or(f.dterm_lpf1_dyn_max_hz);
    }
    f.dterm_lpf2_static_hz = u16_or(
        h,
        "dterm_lpf2_static_hz",
        "dterm_lowpass2_hz",
        f.dterm_lpf2_static_hz,
    );
    f.dterm_lpf2_type = ftype(h, "dterm_lpf2_type", f.dterm_lpf2_type);
    f.dterm_notch_hz = num(h, "dterm_notch_hz").unwrap_or(f.dterm_notch_hz);
    f.dterm_notch_cutoff = num(h, "dterm_notch_cutoff").unwrap_or(f.dterm_notch_cutoff);
    f.dyn_notch_count = num(h, "dyn_notch_count").unwrap_or(f.dyn_notch_count);
    f.dyn_notch_q = num(h, "dyn_notch_q").unwrap_or(f.dyn_notch_q);
    f.dyn_notch_min_hz = num(h, "dyn_notch_min_hz").unwrap_or(f.dyn_notch_min_hz);
    f.dyn_notch_max_hz = num(h, "dyn_notch_max_hz").unwrap_or(f.dyn_notch_max_hz);
    f.rpm_filter_harmonics = num(h, "rpm_filter_harmonics")
        .or_else(|| num(h, "gyro_rpm_notch_harmonics"))
        .unwrap_or(f.rpm_filter_harmonics);
    f.rpm_filter_min_hz = num(h, "rpm_filter_min_hz")
        .or_else(|| num(h, "gyro_rpm_notch_min"))
        .unwrap_or(f.rpm_filter_min_hz);
    f.rpm_filter_q = num(h, "rpm_filter_q")
        .or_else(|| num(h, "gyro_rpm_notch_q"))
        .unwrap_or(f.rpm_filter_q);
    f.rpm_filter_fade_range_hz =
        num(h, "rpm_filter_fade_range_hz").unwrap_or(f.rpm_filter_fade_range_hz);
    f.rpm_filter_lpf_hz = num(h, "rpm_filter_lpf_hz").unwrap_or(f.rpm_filter_lpf_hz);

    let s = &mut t.simplified;
    s.pids_mode = num(h, "simplified_pids_mode").unwrap_or(s.pids_mode);
    s.master_multiplier = num(h, "simplified_master_multiplier").unwrap_or(s.master_multiplier);
    s.pi_gain = num(h, "simplified_pi_gain").unwrap_or(s.pi_gain);
    s.i_gain = num(h, "simplified_i_gain").unwrap_or(s.i_gain);
    s.d_gain = num(h, "simplified_d_gain").unwrap_or(s.d_gain);
    s.d_max_gain = num(h, "simplified_d_max_gain").unwrap_or(s.d_max_gain);
    s.feedforward_gain = num(h, "simplified_feedforward_gain").unwrap_or(s.feedforward_gain);
    s.pitch_pi_gain = num(h, "simplified_pitch_pi_gain").unwrap_or(s.pitch_pi_gain);
    s.pitch_d_gain = num(h, "simplified_pitch_d_gain").unwrap_or(s.pitch_d_gain);
    s.gyro_filter = num::<u8>(h, "simplified_gyro_filter")
        .map(|v| v != 0)
        .unwrap_or(s.gyro_filter);
    s.gyro_filter_multiplier =
        num(h, "simplified_gyro_filter_multiplier").unwrap_or(s.gyro_filter_multiplier);
    s.dterm_filter = num::<u8>(h, "simplified_dterm_filter")
        .map(|v| v != 0)
        .unwrap_or(s.dterm_filter);
    s.dterm_filter_multiplier =
        num(h, "simplified_dterm_filter_multiplier").unwrap_or(s.dterm_filter_multiplier);
    t
}

#[allow(dead_code)]
fn _assert_types(_: BfAxisPid, _: BfFilterConfig, _: BfSimplified) {}
