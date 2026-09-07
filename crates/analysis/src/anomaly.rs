//! Flight anomaly detectors. Each detector scans the airborne part of the log
//! and emits [`Anomaly`] events. Thresholds ("ours") are conservative so a
//! healthy log produces nothing; they are unit-tested on synthetic signals and
//! checked against the real fixtures (which must stay clean of Critical events).

use domain::{Anomaly, AnomalyKind, Axis, FlightLog, Severity};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnomalyOpts {
    /// Motor output treated as "pinned at max".
    pub motor_max: f32,
    /// Motor output treated as "at the floor" (BF idle ≈ 5 %).
    pub motor_floor: f32,
    /// Minimum duration for a pinned/floor motor to count (seconds).
    pub pin_min_s: f32,
    /// Gyro magnitude treated as sensor clipping (°/s). MPU6000/ICM ±2000 °/s range.
    pub gyro_clip_dps: f32,
    /// Oscillation: RMS of (gyro − setpoint) over a 0.5 s window, with little stick input (°/s).
    pub osc_rms_dps: f32,
    pub osc_max_setpoint_rms_dps: f32,
    /// Hover motor mean spread that counts as imbalance (fraction of full scale).
    pub imbalance_frac: f32,
    /// eRPM deviation from the median of the other motors (fraction).
    pub rpm_imbalance_frac: f32,
    /// Yaw spin: |gyro yaw| above this for `yaw_spin_min_s` with little yaw stick (°/s).
    pub yaw_spin_dps: f32,
    pub yaw_spin_min_s: f32,
    /// High-frequency raw gyro RMS (> 300 Hz) that counts as excessive vibration (°/s).
    pub vibration_rms_dps: f32,
    /// Gap longer than this is reported (seconds).
    pub gap_min_s: f32,
}

impl Default for AnomalyOpts {
    fn default() -> Self {
        Self {
            motor_max: 0.98,
            motor_floor: 0.06,
            pin_min_s: 0.10,
            gyro_clip_dps: 1950.0,
            osc_rms_dps: 40.0,
            osc_max_setpoint_rms_dps: 25.0,
            imbalance_frac: 0.15,
            rpm_imbalance_frac: 0.15,
            yaw_spin_dps: 1000.0,
            yaw_spin_min_s: 0.2,
            vibration_rms_dps: 40.0,
            gap_min_s: 0.02,
        }
    }
}

fn mk(kind: AnomalyKind, severity: Severity, t0: f32, t1: f32, axis: Option<Axis>, motor: Option<usize>, value: f32, detail: String) -> Anomaly {
    Anomaly { kind, severity, t_start_s: t0, t_end_s: t1, axis, motor, value, detail }
}

/// Runs of consecutive `true` at least `min_len` long → (start, end) index pairs.
fn runs(mask: &[bool], min_len: usize) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, &m) in mask.iter().enumerate() {
        match (m, start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                if i - s >= min_len {
                    out.push((s, i));
                }
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        if mask.len() - s >= min_len {
            out.push((s, mask.len()));
        }
    }
    out
}

fn rms(x: &[f32]) -> f32 {
    if x.is_empty() {
        return 0.0;
    }
    (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
}

fn median(v: &mut [f32]) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    v[v.len() / 2]
}

/// Merge events of the same kind/motor/axis that are closer than `gap_s`.
fn merge(mut v: Vec<Anomaly>, gap_s: f32) -> Vec<Anomaly> {
    v.sort_by(|a, b| a.t_start_s.partial_cmp(&b.t_start_s).unwrap());
    let mut out: Vec<Anomaly> = Vec::new();
    for a in v {
        if let Some(last) = out.last_mut() {
            if last.kind == a.kind && last.motor == a.motor && last.axis == a.axis && a.t_start_s - last.t_end_s <= gap_s {
                last.t_end_s = last.t_end_s.max(a.t_end_s);
                if a.severity > last.severity {
                    last.severity = a.severity;
                }
                last.value = last.value.max(a.value);
                continue;
            }
        }
        out.push(a);
    }
    out
}

pub fn detect(log: &FlightLog, airborne: Option<(f32, f32)>, opts: &AnomalyOpts) -> Vec<Anomaly> {
    let n = log.len();
    if n < 10 {
        return Vec::new();
    }
    let fs = log.fs_hz as f32;
    let (i0, i1) = match airborne {
        Some((a, b)) => dsp::decimate::range_indices(&log.t, a, b),
        None => (0, n),
    };
    if i1 <= i0 + 10 {
        return Vec::new();
    }
    let t = |i: usize| log.t[i.min(n - 1)];
    let mut out = Vec::new();

    // ---- log gaps ---------------------------------------------------------
    let mut gap_total = 0.0f32;
    for &(a, b) in &log.gaps {
        let d = b - a;
        gap_total += d;
        if d >= opts.gap_min_s {
            out.push(mk(AnomalyKind::LogGap, if d >= 0.5 { Severity::Critical } else { Severity::Warning }, a, b, None, None, d * 1000.0, format!("{:.0} ms of missing frames — logging device too slow for the rate (lower blackbox rate, faster SD card, or flash instead of SD).", d * 1000.0)));
        }
    }
    if gap_total > 1.0 && out.iter().all(|a| a.kind != AnomalyKind::LogGap || a.severity < Severity::Critical) {
        out.push(mk(AnomalyKind::LogGap, Severity::Critical, log.t[0], log.t[n - 1], None, None, gap_total * 1000.0, format!("{gap_total:.2} s of frames missing in total.")));
    }

    // ---- gyro clipping / yaw spin / control reversed ------------------------
    let pin_len = ((opts.pin_min_s * fs) as usize).max(2);
    for (k, ax) in log.axes.iter().enumerate() {
        let axis = Axis::ALL[k];
        let g = &ax.gyro_filt;
        let clip: Vec<bool> = (i0..i1).map(|i| g[i].abs() >= opts.gyro_clip_dps || ax.gyro_raw.as_ref().map(|r| r[i].abs() >= opts.gyro_clip_dps).unwrap_or(false)).collect();
        for (a, b) in runs(&clip, 3) {
            let peak = (a..b).map(|i| g[i0 + i].abs()).fold(0f32, f32::max);
            out.push(mk(AnomalyKind::GyroClipping, Severity::Warning, t(i0 + a), t(i0 + b), Some(axis), None, peak, format!("{} gyro at the sensor limit ({peak:.0} °/s) — crash, prop strike or a hit; data around here is unusable.", axis.name())));
        }
        if axis == Axis::Yaw {
            let spin: Vec<bool> = (i0..i1).map(|i| g[i].abs() >= opts.yaw_spin_dps && ax.setpoint[i].abs() < 300.0).collect();
            for (a, b) in runs(&spin, ((opts.yaw_spin_min_s * fs) as usize).max(2)) {
                let peak = (a..b).map(|i| g[i0 + i].abs()).fold(0f32, f32::max);
                out.push(mk(AnomalyKind::YawSpin, Severity::Critical, t(i0 + a), t(i0 + b), Some(axis), None, peak, format!("Un-commanded yaw rotation of {peak:.0} °/s for {:.0} ms — yaw spin (motor/ESC failure, prop loss or crash).", (b - a) as f32 / fs * 1000.0)));
            }
        }
        // reversed control: over strong stick input the gyro must follow the setpoint sign
        let mut num = 0.0f64;
        let mut den = 0.0f64;
        let mut cnt = 0usize;
        for i in i0..i1 {
            let sp = ax.setpoint[i];
            if sp.abs() >= 100.0 {
                num += (sp * g[i]) as f64;
                den += (sp * sp) as f64;
                cnt += 1;
            }
        }
        if cnt >= (fs * 0.5) as usize && den > 0.0 {
            let gain = num / den; // ≈ 1 when tracking, < 0 when reversed
            if gain < -0.2 {
                out.push(mk(AnomalyKind::ControlReversed, Severity::Critical, t(i0), t(i1), Some(axis), None, gain as f32, format!("{} gyro moves against the setpoint (gain {gain:.2}) — check board orientation / gyro alignment and motor direction before flying again.", axis.name())));
            }
        }
    }

    // ---- oscillation (0.5 s windows, hop 0.25 s) ------------------------------
    let win = ((0.5 * fs) as usize).max(16);
    let hop = win / 2;
    for (k, ax) in log.axes.iter().enumerate() {
        let axis = Axis::ALL[k];
        let mut mask = vec![false; (i1 - i0).div_ceil(hop)];
        let mut freqs = vec![0f32; mask.len()];
        let mut amps = vec![0f32; mask.len()];
        let mut w = 0;
        let mut s = i0;
        while s + win <= i1 {
            let err: Vec<f32> = (s..s + win).map(|i| ax.gyro_filt[i] - ax.setpoint[i]).collect();
            let sp_rms = rms(&ax.setpoint[s..s + win]);
            let e_rms = rms(&err);
            if e_rms >= opts.osc_rms_dps && sp_rms <= opts.osc_max_setpoint_rms_dps {
                // dominant frequency from zero crossings of the error
                let mean = err.iter().sum::<f32>() / err.len() as f32;
                let zc = err.windows(2).filter(|p| (p[0] - mean).signum() != (p[1] - mean).signum()).count();
                let f = zc as f32 / 2.0 / (win as f32 / fs);
                if (4.0..=200.0).contains(&f) {
                    mask[w] = true;
                    freqs[w] = f;
                    amps[w] = e_rms;
                }
            }
            w += 1;
            s += hop;
        }
        for (a, b) in runs(&mask, 2) {
            let f = freqs[a..b].iter().sum::<f32>() / (b - a) as f32;
            let amp = amps[a..b].iter().cloned().fold(0f32, f32::max);
            let t0 = t(i0 + a * hop);
            let t1 = t(i0 + b * hop + win - hop);
            let sev = if amp >= 3.0 * opts.osc_rms_dps { Severity::Critical } else { Severity::Warning };
            let why = if f < 15.0 { "slow wobble — P too low or I-term/attitude issue" } else if f < 80.0 { "P/D oscillation — gains too high for this frame or filtering delay" } else { "fast oscillation — D-term noise / filter too loose" };
            out.push(mk(AnomalyKind::Oscillation, sev, t0, t1, Some(axis), None, f, format!("{} oscillates at ≈{f:.0} Hz (error RMS {amp:.0} °/s) without stick input for {:.1} s — {why}.", axis.name(), t1 - t0)));
        }
    }

    // ---- motors -------------------------------------------------------------
    let nm = log.motors.len();
    if nm >= 2 {
        let m = &log.motors;
        let roll_pitch_hit = |a: usize, b: usize| -> f32 { (a..b).map(|i| log.axes[0].gyro_filt[i].abs().max(log.axes[1].gyro_filt[i].abs())).fold(0f32, f32::max) };
        for k in 0..nm {
            let pinned: Vec<bool> = (i0..i1).map(|i| m[k][i] >= opts.motor_max).collect();
            for (a, b) in runs(&pinned, pin_len) {
                let (s, e) = (i0 + a, i0 + b);
                let others_mean = (s..e).map(|i| (0..nm).filter(|j| *j != k).map(|j| m[j][i]).sum::<f32>() / (nm - 1) as f32).sum::<f32>() / (e - s) as f32;
                let hit = roll_pitch_hit(s, e);
                let rpm_collapse = log.erpm.as_ref().and_then(|r| {
                    if r.len() != nm {
                        return None;
                    }
                    let mine = (s..e).map(|i| r[k][i]).sum::<f32>() / (e - s) as f32;
                    let mut others: Vec<f32> = (0..nm).filter(|j| *j != k).map(|j| (s..e).map(|i| r[j][i]).sum::<f32>() / (e - s) as f32).collect();
                    let med = median(&mut others);
                    (med > 1000.0).then_some(mine / med)
                });
                let dur_ms = (e - s) as f32 / fs * 1000.0;
                match rpm_collapse {
                    Some(ratio) if ratio < 0.4 => out.push(mk(AnomalyKind::MotorDesync, Severity::Critical, t(s), t(e), None, Some(k), ratio, format!("Motor {} commanded 100 % for {dur_ms:.0} ms while its eRPM fell to {:.0} % of the others — ESC desync. Check ESC firmware/timing, motor wiring and solder joints; lower dshot rate / enable bidirectional DShot demag settings.", k + 1, ratio * 100.0))),
                    None if others_mean < 0.6 && hit > 300.0 => out.push(mk(AnomalyKind::MotorDesync, Severity::Critical, t(s), t(e), None, Some(k), hit, format!("Motor {} pinned at 100 % for {dur_ms:.0} ms while the others sat at {:.0} % and the quad rolled/pitched {hit:.0} °/s — desync-like event (no RPM telemetry to confirm). Check that motor/ESC.", k + 1, others_mean * 100.0))),
                    _ => out.push(mk(AnomalyKind::MotorSaturation, Severity::Warning, t(s), t(e), None, Some(k), dur_ms, format!("Motor {} at 100 % for {dur_ms:.0} ms — no headroom left (too heavy / low voltage / gains asking for more than the motor has).", k + 1))),
                }
            }
            let floor: Vec<bool> = (i0..i1).map(|i| m[k][i] <= opts.motor_floor && (0..nm).any(|j| m[j][i] > 0.5)).collect();
            for (a, b) in runs(&floor, pin_len) {
                let dur_ms = (b - a) as f32 / fs * 1000.0;
                out.push(mk(AnomalyKind::MotorFloor, Severity::Warning, t(i0 + a), t(i0 + b), None, Some(k), dur_ms, format!("Motor {} at the idle floor for {dur_ms:.0} ms while others were high — authority lost on the low side (raise idle, or D/P too high / prop wash).", k + 1)));
            }
        }
        // hover imbalance: motor means over the airborne range where throttle is near hover
        let means: Vec<f32> = (0..nm).map(|k| m[k][i0..i1].iter().sum::<f32>() / (i1 - i0) as f32).collect();
        let (lo, hi) = means.iter().fold((f32::MAX, f32::MIN), |(l, h), v| (l.min(*v), h.max(*v)));
        if hi - lo >= opts.imbalance_frac && hi > 0.1 {
            let kmax = means.iter().position(|v| *v == hi).unwrap();
            let kmin = means.iter().position(|v| *v == lo).unwrap();
            out.push(mk(AnomalyKind::MotorImbalance, Severity::Warning, t(i0), t(i1), None, Some(kmax), (hi - lo) * 100.0, format!("Motor {} averages {:.0} % but motor {} only {:.0} % over the flight — CG offset, bent prop, tilted arm or a weak motor.", kmax + 1, hi * 100.0, kmin + 1, lo * 100.0)));
        }
        if let Some(r) = &log.erpm {
            if r.len() == nm {
                // RPM per unit of motor command: a motor that is simply commanded less (CG
                // offset) keeps the same ratio; a damaged prop / dragging bearing / ESC
                // mismatch turns less for the same command.
                let ratio: Vec<f32> = (0..nm)
                    .map(|k| {
                        let (mut rs, mut ms) = (0f64, 0f64);
                        for i in i0..i1 {
                            if m[k][i] > 0.15 {
                                rs += r[k][i] as f64;
                                ms += m[k][i] as f64;
                            }
                        }
                        if ms > 0.0 { (rs / ms) as f32 } else { 0.0 }
                    })
                    .collect();
                let mut sorted = ratio.clone();
                let med = median(&mut sorted);
                if med > 1000.0 {
                    for k in 0..nm {
                        let dev = (ratio[k] - med) / med;
                        if dev.abs() >= opts.rpm_imbalance_frac {
                            out.push(mk(AnomalyKind::RpmImbalance, Severity::Warning, t(i0), t(i1), None, Some(k), dev * 100.0, format!("Motor {} turns {:+.0} % RPM per unit of command compared with the others — damaged/unbalanced prop, dragging bearing, or ESC/motor mismatch.", k + 1, dev * 100.0)));
                        }
                    }
                }
                for k in 0..nm {
                    let drop: Vec<bool> = (i0..i1).map(|i| r[k][i] < 100.0 && m[k][i] > 0.2).collect();
                    for (a, b) in runs(&drop, pin_len) {
                        out.push(mk(AnomalyKind::RpmDropout, Severity::Warning, t(i0 + a), t(i0 + b), None, Some(k), (b - a) as f32 / fs * 1000.0, format!("Motor {} eRPM reads 0 for {:.0} ms while commanded — bidirectional DShot telemetry loss (ESC firmware, dshot rate, wiring) or the motor really stopped.", k + 1, (b - a) as f32 / fs * 1000.0)));
                    }
                }
            }
        }
    }

    // ---- vibration: raw gyro high-frequency RMS ------------------------------
    if fs >= 1000.0 {
        for (k, ax) in log.axes.iter().enumerate() {
            let Some(raw) = &ax.gyro_raw else { continue };
            // crude high-pass: raw minus 8-sample moving average (≈ cutoff fs/16 ≈ 125 Hz @ 2 kHz)
            let seg = &raw[i0..i1];
            let mut hp = Vec::with_capacity(seg.len());
            let w = 8usize;
            let mut acc: f32 = seg.iter().take(w).sum();
            for i in 0..seg.len() {
                if i >= w {
                    acc += seg[i] - seg[i - w];
                }
                hp.push(seg[i] - acc / w as f32);
            }
            let v = rms(&hp);
            if v >= opts.vibration_rms_dps {
                out.push(mk(AnomalyKind::Vibration, if v >= 2.0 * opts.vibration_rms_dps { Severity::Critical } else { Severity::Warning }, t(i0), t(i1), Some(Axis::ALL[k]), None, v, format!("{} raw gyro high-frequency RMS {v:.0} °/s — heavy vibration (bent/unbalanced prop, bearing, loose stack or frame). Filters will not fix the source.", Axis::ALL[k].name())));
            }
        }
    }

    let mut out = merge(out, 0.2);
    out.sort_by(|a, b| b.severity.cmp(&a.severity).then(a.t_start_s.partial_cmp(&b.t_start_s).unwrap()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::*;

    fn base(fs: f32, secs: f32) -> FlightLog {
        let n = (fs * secs) as usize;
        let t: Vec<f32> = (0..n).map(|i| i as f32 / fs).collect();
        let z = vec![0f32; n];
        let axes: [AxisSeries; 3] = std::array::from_fn(|_| AxisSeries { setpoint: z.clone(), gyro_filt: z.clone(), gyro_raw: Some(z.clone()), ..Default::default() });
        FlightLog {
            id: LogId("t".into()),
            firmware: Firmware::Betaflight { version: "4.5".into(), api: (1, 46) },
            fs_hz: fs as f64,
            t,
            axes,
            motors: vec![vec![0.4; n]; 4],
            throttle: vec![0.4; n],
            erpm: None,
            gaps: vec![],
            meta: Default::default(),
            tune_at_log: Tune::Unknown,
            gyro_hr: vec![],
        }
    }

    #[test]
    fn clean_hover_has_no_anomalies() {
        let log = base(2000.0, 10.0);
        assert!(detect(&log, None, &AnomalyOpts::default()).is_empty());
    }

    #[test]
    fn desync_with_rpm_collapse_is_critical() {
        let mut log = base(2000.0, 10.0);
        let n = log.len();
        let mut rpm = vec![vec![8000f32; n]; 4];
        for i in 6000..6600 {
            log.motors[2][i] = 1.0;
            rpm[2][i] = 500.0;
        }
        log.erpm = Some(rpm);
        let a = detect(&log, None, &AnomalyOpts::default());
        let d = a.iter().find(|x| x.kind == AnomalyKind::MotorDesync).expect("desync");
        assert_eq!(d.severity, Severity::Critical);
        assert_eq!(d.motor, Some(2));
        assert!((d.t_start_s - 3.0).abs() < 0.01 && (d.t_end_s - 3.3).abs() < 0.01, "{d:?}");
        assert!(a.iter().all(|x| x.kind != AnomalyKind::MotorSaturation));
    }

    #[test]
    fn desync_without_rpm_needs_others_low_and_attitude_hit() {
        let mut log = base(2000.0, 10.0);
        for i in 6000..6400 {
            log.motors[0][i] = 1.0;
            log.motors[1][i] = 0.2;
            log.motors[2][i] = 0.2;
            log.motors[3][i] = 0.2;
            log.axes[0].gyro_filt[i] = 450.0;
        }
        let a = detect(&log, None, &AnomalyOpts::default());
        assert!(a.iter().any(|x| x.kind == AnomalyKind::MotorDesync && x.motor == Some(0)), "{a:?}");
        // plain saturation: others also high, no attitude hit
        let mut log = base(2000.0, 10.0);
        for i in 6000..6400 {
            log.motors[0][i] = 1.0;
            log.motors[1][i] = 0.9;
        }
        let a = detect(&log, None, &AnomalyOpts::default());
        assert!(a.iter().any(|x| x.kind == AnomalyKind::MotorSaturation && x.motor == Some(0)));
        assert!(a.iter().all(|x| x.kind != AnomalyKind::MotorDesync));
    }

    #[test]
    fn short_spikes_are_ignored() {
        let mut log = base(2000.0, 10.0);
        for i in 6000..6050 {
            log.motors[0][i] = 1.0; // 25 ms < pin_min_s
        }
        assert!(detect(&log, None, &AnomalyOpts::default()).is_empty());
    }

    #[test]
    fn oscillation_frequency_and_severity() {
        let mut log = base(2000.0, 10.0);
        for i in 4000..12000 {
            let tt = i as f32 / 2000.0;
            log.axes[1].gyro_filt[i] = 80.0 * (2.0 * std::f32::consts::PI * 30.0 * tt).sin();
        }
        let a = detect(&log, None, &AnomalyOpts::default());
        let o = a.iter().find(|x| x.kind == AnomalyKind::Oscillation).expect("osc");
        assert_eq!(o.axis, Some(Axis::Pitch));
        assert!((o.value - 30.0).abs() < 3.0, "{}", o.value);
        assert_eq!(o.severity, Severity::Warning);
        assert!(o.t_start_s >= 1.7 && o.t_end_s <= 6.3, "{o:?}"); // ±1 hop (0.25 s)
        // same movement but commanded by the sticks is not an oscillation
        for i in 4000..12000 {
            log.axes[1].setpoint[i] = log.axes[1].gyro_filt[i];
        }
        assert!(detect(&log, None, &AnomalyOpts::default()).iter().all(|x| x.kind != AnomalyKind::Oscillation));
    }

    #[test]
    fn clipping_yaw_spin_reversed_gap_imbalance_vibration() {
        let mut log = base(2000.0, 10.0);
        for i in 1000..1010 {
            log.axes[0].gyro_filt[i] = 1999.0;
        }
        for i in 8000..9000 {
            log.axes[2].gyro_filt[i] = 1400.0;
        }
        for i in 12000..14000 {
            log.axes[1].setpoint[i] = 300.0;
            log.axes[1].gyro_filt[i] = -250.0;
        }
        log.gaps = vec![(7.0, 7.05)];
        for i in 0..log.len() {
            log.motors[3][i] = 0.7;
        }
        for i in 0..log.len() {
            log.axes[2].gyro_raw.as_mut().unwrap()[i] = if i % 2 == 0 { 60.0 } else { -60.0 };
        }
        let a = detect(&log, None, &AnomalyOpts::default());
        let kinds: Vec<AnomalyKind> = a.iter().map(|x| x.kind).collect();
        for k in [AnomalyKind::GyroClipping, AnomalyKind::YawSpin, AnomalyKind::ControlReversed, AnomalyKind::LogGap, AnomalyKind::MotorImbalance, AnomalyKind::Vibration] {
            assert!(kinds.contains(&k), "missing {k:?} in {kinds:?}");
        }
        assert_eq!(a[0].severity, Severity::Critical, "sorted critical first");
        assert!(a.iter().find(|x| x.kind == AnomalyKind::MotorImbalance).unwrap().motor == Some(3));
    }

    #[test]
    fn rpm_imbalance_and_dropout() {
        let mut log = base(2000.0, 10.0);
        let n = log.len();
        let mut rpm = vec![vec![8000f32; n]; 4];
        for v in rpm[1].iter_mut() {
            *v = 6000.0;
        }
        for i in 3000..3400 {
            rpm[0][i] = 0.0;
        }
        log.erpm = Some(rpm);
        let a = detect(&log, None, &AnomalyOpts::default());
        assert!(a.iter().any(|x| x.kind == AnomalyKind::RpmImbalance && x.motor == Some(1)), "{a:?}");
        assert!(a.iter().any(|x| x.kind == AnomalyKind::RpmDropout && x.motor == Some(0)), "{a:?}");
    }
}
