//! `Index` → `domain::FlightLog`.

use crate::index::Index;
use crate::rates::{measure_rate_hz, plausible_mask};
use domain::ap_consts::*;
use domain::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, thiserror::Error)]
pub enum IngestError {
    #[error("not an ArduPilot DataFlash log")]
    NotDataflash,
    #[error("no usable rate-loop data (neither PIDR/PIDP/PIDY nor RATE messages)")]
    NoRateData,
    #[error("{0}")]
    Other(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub index: usize,
    pub firmware_revision: String,
    pub craft_name: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestOpts {
    /// Gap threshold as multiple of the nominal sample period.
    pub gap_mult: f64,
    /// Keep only [first arm − 2 s, last disarm + 2 s].
    pub clip_to_armed: bool,
}

impl Default for IngestOpts {
    fn default() -> Self {
        Self { gap_mult: 4.0, clip_to_armed: true }
    }
}

const MAX_SPAN_US: u64 = 24 * 3600 * 1_000_000;
const BACK_US: u64 = 1_000_000;

fn firmware_banner(ix: &Index) -> Option<String> {
    ix.messages.iter().find(|m| m.starts_with("Ardu") && m.contains(" V")).cloned()
}

fn firmware_of(banner: Option<&str>) -> Firmware {
    match banner {
        Some(b) => {
            let version = b.split_whitespace().nth(1).map(|v| v.trim_start_matches('V').to_string()).unwrap_or_default();
            if b.starts_with("ArduCopter") {
                Firmware::ArduCopter { version }
            } else {
                Firmware::Unknown { product: b.to_string() }
            }
        }
        None => Firmware::Unknown { product: "ArduPilot (no MSG banner)".into() },
    }
}

pub fn list_sessions(bytes: &[u8]) -> Vec<SessionInfo> {
    if !crate::looks_like_dataflash(bytes) {
        return vec![SessionInfo { index: 0, firmware_revision: String::new(), craft_name: None, error: Some("not a DataFlash log".into()) }];
    }
    let ix = Index::scan(bytes);
    vec![SessionInfo { index: 0, firmware_revision: firmware_banner(&ix).unwrap_or_else(|| "ArduPilot".into()), craft_name: None, error: None }]
}

/// Linear interpolation of `y(t_src)` onto `grid` (both in seconds, same base).
fn interp(t_src: &[f64], y: &[f64], grid: &[f32]) -> Vec<f32> {
    let n = t_src.len();
    let mut out = Vec::with_capacity(grid.len());
    if n == 0 {
        out.resize(grid.len(), 0.0);
        return out;
    }
    let mut j = 0usize;
    for &g in grid {
        let t = g as f64;
        while j + 1 < n && t_src[j + 1] <= t {
            j += 1;
        }
        if t <= t_src[0] {
            out.push(y[0] as f32);
        } else if j + 1 >= n {
            out.push(y[n - 1] as f32);
        } else {
            let (ta, tb) = (t_src[j], t_src[j + 1]);
            let f = if tb > ta { (t - ta) / (tb - ta) } else { 0.0 };
            out.push((y[j] + (y[j + 1] - y[j]) * f.clamp(0.0, 1.0)) as f32);
        }
    }
    out
}

struct Series {
    /// seconds relative to t0, plausible rows only, non-decreasing
    t: Vec<f64>,
    rows: Vec<usize>,
}

fn series(ix: &Index, msg: &str, t0: u64, clip: Option<(u64, u64)>) -> Option<Series> {
    let tu = ix.column_u64(msg, "TimeUS")?;
    if tu.len() < 2 {
        return None;
    }
    let mask = plausible_mask(&tu, t0, MAX_SPAN_US, BACK_US);
    let mut t = Vec::new();
    let mut rows = Vec::new();
    for (i, &ok) in mask.iter().enumerate() {
        if !ok {
            continue;
        }
        if let Some((a, b)) = clip {
            if tu[i] < a || tu[i] > b {
                continue;
            }
        }
        t.push((tu[i] as f64 - t0 as f64) / 1e6);
        rows.push(i);
    }
    (t.len() >= 2).then_some(Series { t, rows })
}

fn col_on(ix: &Index, msg: &str, col: &str, s: &Series, grid: &[f32], scale: f64) -> Option<Vec<f32>> {
    let c = ix.column_f64(msg, col)?;
    let y: Vec<f64> = s.rows.iter().map(|&i| c[i] * scale).collect();
    Some(interp(&s.t, &y, grid))
}

/// Armed intervals in TimeUS from `ARM` (fallback `EV`).
fn armed_intervals(ix: &Index) -> Vec<(u64, u64)> {
    let mut ev: Vec<(u64, bool)> = Vec::new();
    if let (Some(t), Some(s)) = (ix.column_u64("ARM", "TimeUS"), ix.column_f64("ARM", "ArmState")) {
        ev.extend(t.iter().zip(&s).map(|(t, s)| (*t, *s >= 0.5)));
    }
    if ev.is_empty() {
        if let (Some(t), Some(id)) = (ix.column_u64("EV", "TimeUS"), ix.column_f64("EV", "Id")) {
            for (t, id) in t.iter().zip(&id) {
                let id = *id as u8;
                if id == EV_ARMED {
                    ev.push((*t, true));
                } else if id == EV_DISARMED {
                    ev.push((*t, false));
                }
            }
        }
    }
    ev.sort_by_key(|e| e.0);
    let mut out = Vec::new();
    let mut open: Option<u64> = None;
    for (t, armed) in ev {
        match (armed, open) {
            (true, None) => open = Some(t),
            (false, Some(a)) => {
                out.push((a, t));
                open = None;
            }
            _ => {}
        }
    }
    if let Some(a) = open {
        out.push((a, u64::MAX));
    }
    out
}

pub fn ingest(bytes: &[u8], _session: usize, opts: &IngestOpts) -> Result<FlightLog, IngestError> {
    if !crate::looks_like_dataflash(bytes) {
        return Err(IngestError::NotDataflash);
    }
    let ix = Index::scan(bytes);
    let mut warnings: Vec<String> = ix.stats.warnings.clone();
    let banner = firmware_banner(&ix);
    let firmware = firmware_of(banner.as_deref());
    let p = |name: &str| ix.param(name);

    // ---- reference series & time base ---------------------------------------
    let have_pid = ["PIDR", "PIDP", "PIDY"].iter().all(|m| ix.count(m) >= 8);
    let ref_msg = if have_pid { "PIDR" } else if ix.count("RATE") >= 8 { "RATE" } else { return Err(IngestError::NoRateData) };
    let ref_tu = ix.column_u64(ref_msg, "TimeUS").ok_or(IngestError::NoRateData)?;
    // t0 = first plausible stamp of the reference series (start of file order)
    let t0 = *ref_tu.iter().find(|&&t| t > 0).ok_or(IngestError::NoRateData)?;
    let armed = armed_intervals(&ix);
    let clip = if opts.clip_to_armed && !armed.is_empty() {
        let a = armed.first().unwrap().0.saturating_sub(2_000_000);
        let b = armed.last().unwrap().1.saturating_add(2_000_000);
        Some((a, b))
    } else {
        None
    };
    let rs = series(&ix, ref_msg, t0, clip).ok_or(IngestError::NoRateData)?;
    let t_start = rs.t[0];
    let t_end = *rs.t.last().unwrap();
    let rate_hz = measure_rate_hz(&rs.rows.iter().map(|&i| ref_tu[i]).collect::<Vec<_>>()).unwrap_or(10.0);
    let fs = rate_hz.round().max(1.0);
    let n = ((t_end - t_start) * fs).floor() as usize + 1;
    let grid: Vec<f32> = (0..n).map(|i| (t_start + i as f64 / fs) as f32).collect();

    // ---- measured message rates ---------------------------------------------
    let mut msg_rates_hz = BTreeMap::new();
    for m in ["RATE", "PIDR", "PIDP", "PIDY", "ATT", "ANG", "IMU", "GYR", "ISBD", "ISBH", "CTUN", "RCOU", "ESC", "VIBE"] {
        if let Some(tu) = ix.column_u64(m, "TimeUS") {
            let mask = plausible_mask(&tu, t0, MAX_SPAN_US, BACK_US);
            let kept: Vec<u64> = tu.iter().zip(&mask).filter(|(_, ok)| **ok).map(|(t, _)| *t).collect();
            if let Some(r) = measure_rate_hz(&kept) {
                msg_rates_hz.insert(m.to_string(), r);
            }
        }
    }
    let loop_hz = p("SCHED_LOOP_RATE").map(|v| v as f64);
    let rate_thread = ix.count("RTDT") > 0;

    // ---- axes ---------------------------------------------------------------
    let mut axes: [AxisSeries; 3] = Default::default();
    if have_pid {
        for (k, msg) in ["PIDR", "PIDP", "PIDY"].iter().enumerate() {
            let s = series(&ix, msg, t0, clip).ok_or(IngestError::NoRateData)?;
            let ax = &mut axes[k];
            // PIDx Tar/Act are rad/s (AC_AttitudeControl_Multi::rate_controller_run passes _ang_vel_body / gyro in rad/s)
            ax.setpoint = col_on(&ix, msg, "Tar", &s, &grid, RAD_TO_DEG).unwrap_or_default();
            ax.gyro_filt = col_on(&ix, msg, "Act", &s, &grid, RAD_TO_DEG).unwrap_or_default();
            ax.p = col_on(&ix, msg, "P", &s, &grid, 1.0);
            ax.i = col_on(&ix, msg, "I", &s, &grid, 1.0);
            ax.d = col_on(&ix, msg, "D", &s, &grid, 1.0);
            ax.ff = col_on(&ix, msg, "FF", &s, &grid, 1.0);
            ax.srate = col_on(&ix, msg, "SRate", &s, &grid, 1.0);
            let dff = col_on(&ix, msg, "DFF", &s, &grid, 1.0);
            if let (Some(p), Some(i), Some(d), Some(f)) = (&ax.p, &ax.i, &ax.d, &ax.ff) {
                ax.pid_sum = Some((0..grid.len()).map(|n| p[n] + i[n] + d[n] + f[n] + dff.as_ref().map(|x| x[n]).unwrap_or(0.0)).collect());
            }
        }
        if let Some(r) = msg_rates_hz.get("PIDR") {
            if let Some(lh) = loop_hz {
                if *r < 0.9 * lh {
                    warnings.push(format!("PIDR/PIDP/PIDY logged at {r:.0} Hz, loop rate is {lh:.0} Hz: set LOG_BITMASK bit 0 (ATTITUDE_FAST) for loop-rate PID logging"));
                }
            }
        }
    } else {
        warnings.push("PIDR/PIDP/PIDY absent: set LOG_BITMASK bit 12 (PID); using RATE (no P/I/D terms)".into());
        let s = &rs;
        for (k, (des, act, out)) in [("RDes", "R", "ROut"), ("PDes", "P", "POut"), ("YDes", "Y", "YOut")].iter().enumerate() {
            axes[k].setpoint = col_on(&ix, "RATE", des, s, &grid, 1.0).unwrap_or_default();
            axes[k].gyro_filt = col_on(&ix, "RATE", act, s, &grid, 1.0).unwrap_or_default();
            axes[k].pid_sum = col_on(&ix, "RATE", out, s, &grid, 1.0);
        }
    }
    // RATE.*Out (mixer −1..1) as saturation indicator when PIDx present too
    if have_pid {
        if let Some(s) = series(&ix, "RATE", t0, clip) {
            for (k, out) in ["ROut", "POut", "YOut"].iter().enumerate() {
                if let Some(v) = col_on(&ix, "RATE", out, &s, &grid, 1.0) {
                    axes[k].pid_sum = Some(v);
                }
            }
        }
    }
    // loop-rate raw gyro if present (INS_RAW_LOG_OPT, H7 boards)
    if ix.count("GYR") >= 8 {
        let inst0 = ix.instances("GYR").first().copied().unwrap_or(0);
        let rows = ix.instance_rows("GYR", inst0);
        if let Some(tu) = ix.column_u64("GYR", "TimeUS") {
            let sub: Vec<u64> = rows.iter().map(|&i| tu[i]).collect();
            let mask = plausible_mask(&sub, t0, MAX_SPAN_US, BACK_US);
            let t: Vec<f64> = sub.iter().zip(&mask).filter(|(_, ok)| **ok).map(|(t, _)| (*t as f64 - t0 as f64) / 1e6).collect();
            for (k, c) in ["GyrX", "GyrY", "GyrZ"].iter().enumerate() {
                if let Some(all) = ix.column_f64("GYR", c) {
                    let y: Vec<f64> = rows.iter().zip(&mask).filter(|(_, ok)| **ok).map(|(&i, _)| all[i] * RAD_TO_DEG).collect();
                    axes[k].gyro_raw = Some(interp(&t, &y, &grid));
                }
            }
        }
    }

    // ---- batch sampler tracks -------------------------------------------------
    let mut gyro_hr: Vec<RawGyroTrack> = Vec::new();
    if ix.count("ISBH") > 0 && ix.count("ISBD") > 0 {
        let gyro_count = ix.instances("IMU").len().max(1) as u8;
        let bat_opt = p("INS_LOG_BAT_OPT").unwrap_or(0.0) as u32;
        let post_bits = bat_opt & (INS_LOG_BAT_OPT_POST_FILTER | INS_LOG_BAT_OPT_PRE_POST) != 0;
        let h_n = ix.column_u64("ISBH", "N").unwrap_or_default();
        let h_type = ix.column_f64("ISBH", "type").unwrap_or_default();
        let h_inst = ix.column_f64("ISBH", "instance").unwrap_or_default();
        let h_mul = ix.column_f64("ISBH", "mul").unwrap_or_default();
        let h_smp = ix.column_u64("ISBH", "SampleUS").unwrap_or_default();
        let h_rate = ix.column_f64("ISBH", "smp_rate").unwrap_or_default();
        let d_n = ix.column_u64("ISBD", "N").unwrap_or_default();
        let d_seq = ix.column_u64("ISBD", "seqno").unwrap_or_default();
        let d_x = ix.column_i16x32("ISBD", "x").unwrap_or_default();
        let d_y = ix.column_i16x32("ISBD", "y").unwrap_or_default();
        let d_z = ix.column_i16x32("ISBD", "z").unwrap_or_default();
        let mut by_n: BTreeMap<u64, Vec<usize>> = BTreeMap::new();
        for (i, n) in d_n.iter().enumerate() {
            by_n.entry(*n).or_default().push(i);
        }
        let mut tracks: BTreeMap<(u8, bool), RawGyroTrack> = BTreeMap::new();
        for h in 0..h_n.len() {
            if h_type[h] as u8 != ISBH_TYPE_GYRO || h_mul[h] <= 0.0 || h_rate[h] <= 0.0 {
                continue;
            }
            let Some(rows) = by_n.get(&h_n[h]) else { continue };
            let mut rows = rows.clone();
            rows.sort_by_key(|&i| d_seq[i]);
            let mul = h_mul[h];
            let mut xyz: [Vec<f32>; 3] = Default::default();
            for &i in &rows {
                for (k, src) in [&d_x, &d_y, &d_z].iter().enumerate() {
                    xyz[k].extend(src[i].iter().map(|v| (*v as f64 / mul * RAD_TO_DEG) as f32));
                }
            }
            if xyz[0].is_empty() {
                continue;
            }
            let inst = h_inst[h] as u8;
            let post = post_bits && inst >= gyro_count;
            let t0_s = ((h_smp[h] as f64 - t0 as f64) / 1e6) as f32;
            let tr = tracks.entry((inst, post)).or_insert_with(|| RawGyroTrack { fs_hz: h_rate[h], instance: inst, post_filter: post, batches: Vec::new() });
            tr.batches.push(GyroBatch { t0_s, xyz });
        }
        gyro_hr = tracks.into_values().collect();
    } else {
        warnings.push("no IMU batch-sampler data (ISBH/ISBD): set LOG_BITMASK bit 19 and INS_LOG_BAT_MASK=1 for gyro spectra".into());
    }

    // ---- motors / throttle / esc ----------------------------------------------
    let frame_class = p("FRAME_CLASS").unwrap_or(1.0) as i32;
    let nmot = match frame_class { 1 => 4, 2 | 5 => 6, 3 | 4 | 6 => 8, 12 => 12, _ => 4 };
    let (mut pmin, mut pmax) = (p("MOT_PWM_MIN").unwrap_or(0.0), p("MOT_PWM_MAX").unwrap_or(0.0));
    if pmin <= 0.0 || pmax <= pmin {
        pmin = p("RC3_MIN").unwrap_or(MOT_PWM_DEFAULT_MIN);
        pmax = p("RC3_MAX").unwrap_or(MOT_PWM_DEFAULT_MAX);
        if pmin <= 0.0 || pmax <= pmin {
            pmin = MOT_PWM_DEFAULT_MIN;
            pmax = MOT_PWM_DEFAULT_MAX;
        }
        warnings.push(format!("MOT_PWM_MIN/MAX unset; motors normalised with {pmin:.0}–{pmax:.0} µs"));
    }
    let mut motors = Vec::new();
    if let Some(s) = series(&ix, "RCOU", t0, clip) {
        for k in 1..=nmot {
            if let Some(v) = col_on(&ix, "RCOU", &format!("C{k}"), &s, &grid, 1.0) {
                motors.push(v.iter().map(|pwm| ((pwm - pmin) / (pmax - pmin)).clamp(0.0, 1.2)).collect());
            }
        }
    }
    let mut throttle = series(&ix, "CTUN", t0, clip).and_then(|s| col_on(&ix, "CTUN", "ThO", &s, &grid, 1.0));
    if throttle.is_none() {
        throttle = series(&ix, "RATE", t0, clip).and_then(|s| col_on(&ix, "RATE", "AOut", &s, &grid, 1.0));
    }
    let throttle = throttle.unwrap_or_else(|| vec![0.0; grid.len()]);
    let mut erpm = Vec::new();
    for inst in ix.instances("ESC") {
        let rows = ix.instance_rows("ESC", inst);
        if let (Some(tu), Some(rpm)) = (ix.column_u64("ESC", "TimeUS"), ix.column_f64("ESC", "RPM")) {
            let t: Vec<f64> = rows.iter().map(|&i| (tu[i] as f64 - t0 as f64) / 1e6).collect();
            let y: Vec<f64> = rows.iter().map(|&i| rpm[i]).collect();
            if t.len() >= 2 {
                erpm.push(interp(&t, &y, &grid));
            }
        }
    }

    // ---- tune & meta --------------------------------------------------------
    let mut tune = ApTune::default();
    for (n, v) in &ix.params {
        tune.params.insert(n.clone(), *v);
    }
    let mut headers: BTreeMap<String, String> = BTreeMap::new();
    if let Some(b) = &banner {
        headers.insert("Firmware revision".into(), b.clone());
    }
    for m in ix.messages.iter().take(6) {
        headers.entry("MSG".into()).and_modify(|v| { v.push_str(" | "); v.push_str(m) }).or_insert_with(|| m.clone());
    }
    let hp = |k: &str, v: Option<f32>, h: &mut BTreeMap<String, String>| { if let Some(v) = v { h.insert(k.into(), format!("{v}")); } };
    hp("ap.log_bitmask", p("LOG_BITMASK"), &mut headers);
    hp("ap.loop_rate", p("SCHED_LOOP_RATE"), &mut headers);
    hp("ap.batch.mask", p("INS_LOG_BAT_MASK"), &mut headers);
    hp("ap.batch.opt", p("INS_LOG_BAT_OPT"), &mut headers);
    hp("ap.batch.cnt", p("INS_LOG_BAT_CNT"), &mut headers);
    hp("ap.batch.lgin", p("INS_LOG_BAT_LGIN"), &mut headers);
    hp("ap.frame_class", p("FRAME_CLASS"), &mut headers);
    hp("ap.gyro_filter", p("INS_GYRO_FILTER"), &mut headers);
    headers.insert("ap.rate_thread".into(), rate_thread.to_string());
    headers.insert("ap.corrupt_regions".into(), format!("{:?}", ix.stats.corrupt_regions));
    headers.insert("ap.armed_intervals".into(), armed.iter().map(|(a, b)| format!("{:.2}-{}", (*a as f64 - t0 as f64) / 1e6, if *b == u64::MAX { "end".to_string() } else { format!("{:.2}", (*b as f64 - t0 as f64) / 1e6) })).collect::<Vec<_>>().join(","));
    if let (Some(tu), Some(mn)) = (ix.column_u64("MODE", "TimeUS"), ix.column_f64("MODE", "ModeNum")) {
        headers.insert("ap.modes".into(), tu.iter().zip(&mn).map(|(t, m)| format!("{:.2}:{}", (*t as f64 - t0 as f64) / 1e6, *m as i32)).collect::<Vec<_>>().join(","));
    }
    if let Some(th) = ix.column_f64("CTUN", "ThH") {
        let mut v: Vec<f64> = th.into_iter().filter(|x| x.is_finite() && *x > 0.0).collect();
        if !v.is_empty() {
            v.sort_by(|a, b| a.partial_cmp(b).unwrap());
            headers.insert("ap.hover_thr".into(), format!("{:.3}", v[v.len() / 2]));
        }
    }
    if !ix.stats.corrupt_regions.is_empty() {
        warnings.push(format!("{} corrupt region(s) skipped", ix.stats.corrupt_regions.len()));
    }
    let gaps = {
        let dt = 1.0 / fs;
        let mut g = Vec::new();
        for w in rs.t.windows(2) {
            if w[1] - w[0] > dt * opts.gap_mult {
                g.push(((w[0] - t_start) as f32, (w[1] - t_start) as f32));
            }
        }
        g
    };
    // grid is relative to t_start for FlightLog::t
    let t: Vec<f32> = grid.iter().map(|g| g - t_start as f32).collect();
    for tr in gyro_hr.iter_mut() {
        for b in tr.batches.iter_mut() {
            b.t0_s -= t_start as f32;
        }
    }

    let mut hasher = blake3::Hasher::new();
    hasher.update(bytes);
    let id = LogId(hasher.finalize().to_hex().to_string());

    Ok(FlightLog {
        id,
        firmware,
        fs_hz: fs,
        t,
        axes,
        motors,
        throttle,
        erpm: if erpm.is_empty() { None } else { Some(erpm) },
        gaps,
        meta: LogMeta {
            craft_name: None,
            loop_hz,
            source_rate_hz: rate_hz,
            debug_mode: None,
            duration_s: t_end - t_start,
            session_index: 0,
            session_count: 1,
            headers,
            warnings,
            msg_rates_hz,
        },
        tune_at_log: Tune::Ap(tune),
        gyro_hr,
    })
}
