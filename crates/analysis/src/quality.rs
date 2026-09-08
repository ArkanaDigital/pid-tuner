//! Measured facts about a log that wizard guards evaluate.

use domain::{Firmware, FlightLog, LogQuality};

/// First/last time the quad is airborne: throttle above 12 % sustained for 1 s,
/// trimmed by 1 s on each side so arming spin-up and touchdown are excluded.
pub fn airborne_range(log: &FlightLog) -> Option<(f32, f32)> {
    let fs = log.fs_hz as f32;
    let need = (fs * 1.0) as usize;
    let thr = &log.throttle;
    if thr.len() < 3 * need {
        return None;
    }
    let above: Vec<bool> = thr.iter().map(|t| *t > 0.12).collect();
    let mut run = 0usize;
    let mut first = None;
    for (i, a) in above.iter().enumerate() {
        run = if *a { run + 1 } else { 0 };
        if run >= need {
            first = Some(i + 1 - need);
            break;
        }
    }
    run = 0;
    let mut last = None;
    for (i, a) in above.iter().enumerate().rev() {
        run = if *a { run + 1 } else { 0 };
        if run >= need {
            last = Some(i + need - 1);
            break;
        }
    }
    let (f, l) = (first?, last?);
    let t0 = log.t[f] + 1.0;
    let t1 = log.t[l.min(log.t.len() - 1)] - 1.0;
    (t1 - t0 > 2.0).then_some((t0, t1))
}

pub fn log_quality(log: &FlightLog) -> LogQuality {
    let n = log.len().max(1);
    let dt = 1.0 / log.fs_hz;
    let air = airborne_range(log);
    // Hover point = modal throttle (2 % bins) over the airborne part; hover time = within ±8 % of it.
    let (i0, i1) = match air {
        Some((a, b)) => dsp::decimate::range_indices(&log.t, a, b),
        None => (0, log.len()),
    };
    let thr_air = &log.throttle[i0..i1.max(i0)];
    let mut hist = [0usize; 50];
    for t in thr_air {
        hist[((t * 50.0) as usize).min(49)] += 1;
    }
    let modal = hist
        .iter()
        .enumerate()
        .skip(6)
        .max_by_key(|(_, c)| **c)
        .map(|(b, _)| (b as f64 + 0.5) / 50.0)
        .unwrap_or(0.0);
    let hover = thr_air
        .iter()
        .filter(|t| (**t as f64 - modal).abs() <= 0.08)
        .count() as f64
        * dt;
    let sat = if log.motors.is_empty() {
        0.0
    } else {
        let mut c = 0usize;
        for k in 0..log.motors[0].len() {
            if log
                .motors
                .iter()
                .any(|m| m.get(k).copied().unwrap_or(0.0) >= 0.98)
            {
                c += 1;
            }
        }
        100.0 * c as f64 / n as f64
    };
    let mut max_sp = [0f32; 3];
    for (k, ax) in log.axes.iter().enumerate() {
        max_sp[k] = ax.setpoint.iter().fold(0f32, |m, v| m.max(v.abs()));
    }
    LogQuality {
        fs_hz: log.fs_hz,
        duration_s: log.duration_s(),
        has_gyro_raw: log.axes.iter().all(|a| a.gyro_raw.is_some()),
        has_pid_terms: log.axes.iter().all(|a| a.p.is_some() && a.d.is_some()),
        hover_seconds: hover,
        hover_throttle_pct: modal * 100.0,
        airborne_range_s: air,
        motor_saturation_pct: sat,
        max_setpoint_per_axis: max_sp,
        step_segments_per_axis: [0; 3],
        gap_seconds: log.gaps.iter().map(|(a, b)| (b - a) as f64).sum(),
        pid_rate_hz: log.meta.msg_rates_hz.get("PIDR").copied(),
        max_pid_out: {
            let m: Vec<f32> = log
                .axes
                .iter()
                .map(|a| {
                    a.pid_sum
                        .as_ref()
                        .map(|s| s.iter().fold(0f32, |x, v| x.max(v.abs())))
                        .unwrap_or(0.0)
                })
                .collect();
            matches!(log.firmware, Firmware::ArduCopter { .. }).then(|| [m[0], m[1], m[2]])
        },
        gyro_hr_batches: log.gyro_hr.iter().map(|t| t.batches.len()).sum(),
        chirp_sweeps_per_axis: [0; 3],
        chirp_windows_per_axis: [0; 3],
        chirp_coherence_per_axis: [0.0; 3],
    }
}
