//! Betaflight blackbox (.BBL/.BFL/.TXT) → [`FlightLog`].
//!
//! Decoding is delegated to the vendored `blackbox-log` crate; this crate
//! handles session selection, unit scaling (`blackbox_high_resolution`),
//! resampling to a uniform grid, and extraction of the tune from the header.

pub mod headers;
pub mod rates;
pub mod tune;

use blackbox_log::frame::{Frame as _, FrameDef as _, MainValue};
use blackbox_log::units::si::angular_velocity::degree_per_second;
use blackbox_log::units::Flag as _;
use blackbox_log::{File, Filter, FilterSet, ParserEvent};
use domain::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, thiserror::Error)]
pub enum IngestError {
    #[error("no blackbox session at index {0}")]
    NoSuchSession(usize),
    #[error("header parse error: {0}")]
    Header(String),
    #[error("log has no usable frames")]
    Empty,
    #[error("required field missing: {0}")]
    MissingField(&'static str),
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
    /// Output sample rate; `None` = round(native log rate).
    pub fs_hz: Option<f64>,
    /// Gap threshold as multiple of the nominal frame period.
    pub gap_mult: f64,
}

impl Default for IngestOpts {
    fn default() -> Self {
        Self { fs_hz: None, gap_mult: 4.0 }
    }
}

/// List the sessions (logs) contained in a file, without decoding data.
pub fn list_sessions(bytes: &[u8]) -> Vec<SessionInfo> {
    let file = File::new(bytes);
    file.iter()
        .enumerate()
        .map(|(index, h)| match h {
            Ok(h) => SessionInfo {
                index,
                firmware_revision: h.firmware_revision().to_string(),
                craft_name: h.craft_name().map(str::to_string),
                error: None,
            },
            Err(e) => SessionInfo {
                index,
                firmware_revision: String::new(),
                craft_name: None,
                error: Some(e.to_string()),
            },
        })
        .collect()
}

/// Names of the main-frame fields we decode (base names; array suffixes implied).
const WANTED: &[&str] = &[
    "gyroADC", "gyroUnfilt", "setpoint", "rcCommand", "axisP", "axisI", "axisD", "axisF",
    "motor", "debug", "eRPM",
];

struct Column {
    idx: usize,
    scale: f32,
    data: Vec<f32>,
}

pub fn ingest(bytes: &[u8], session: usize, opts: &IngestOpts) -> Result<FlightLog, IngestError> {
    let file = File::new(bytes);
    let hdr = file
        .parse(session)
        .ok_or(IngestError::NoSuchSession(session))?
        .map_err(|e| IngestError::Header(e.to_string()))?;
    let session_count = file.log_count();

    let mut raw_headers = headers::raw_headers(bytes, session);
    let high_res = raw_headers.get("blackbox_high_resolution").map(|v| v == "1").unwrap_or(false);
    let hr_scale = if high_res { 0.1 } else { 1.0 };
    let debug_mode = hdr.debug_mode().as_name().to_string();

    // ---- column map ---------------------------------------------------------
    let filters = FilterSet {
        main: Filter::OnlyFields(WANTED.iter().copied().collect()),
        slow: Filter::OnlyFields(std::iter::empty::<&str>().collect()),
        gps: Filter::OnlyFields(std::iter::empty::<&str>().collect()),
    };
    let mut parser = hdr.data_parser_with_filters(&filters);
    let def = parser.main_frame_def();
    let mut cols: BTreeMap<String, Column> = BTreeMap::new();
    for (i, f) in def.iter().enumerate() {
        let base = f.name.split('[').next().unwrap_or(f.name);
        let scale = match base {
            "gyroADC" | "gyroUnfilt" | "rcCommand" => hr_scale,
            "setpoint" if !f.name.ends_with("[3]") => hr_scale,
            _ => 1.0,
        };
        cols.insert(f.name.to_string(), Column { idx: i, scale, data: Vec::new() });
    }
    if !cols.contains_key("gyroADC[0]") {
        return Err(IngestError::MissingField("gyroADC"));
    }
    // setpoint may be disabled in the blackbox field mask; it is rebuilt from rcCommand below.
    let setpoint_logged = cols.contains_key("setpoint[0]");
    if !setpoint_logged && !cols.contains_key("rcCommand[0]") {
        return Err(IngestError::MissingField("setpoint (and rcCommand)"));
    }

    // ---- decode -------------------------------------------------------------
    let mut t_us: Vec<f64> = Vec::new();
    let mut vals: Vec<f32> = vec![0.0; cols.len()];
    let mut last_t: u64 = 0;
    while let Some(ev) = parser.next() {
        if let ParserEvent::Main(main) = ev {
            let t = main.time_raw();
            if !t_us.is_empty() && t < last_t {
                // 32-bit wrap or corrupt frame — skip out-of-order samples.
                continue;
            }
            last_t = t;
            for (i, v) in main.iter().enumerate() {
                vals[i] = match v {
                    MainValue::Rotation(r) => r.get::<degree_per_second>() as f32,
                    MainValue::Signed(s) => s as f32,
                    MainValue::Unsigned(u) => u as f32,
                    MainValue::Amperage(_) | MainValue::Voltage(_) | MainValue::Acceleration(_) => 0.0,
                };
            }
            t_us.push(t as f64 * 1e-6);
            for c in cols.values_mut() {
                c.data.push(vals[c.idx] * c.scale);
            }
        }
    }
    if t_us.len() < 16 {
        return Err(IngestError::Empty);
    }

    // ---- resample -----------------------------------------------------------
    let dt = dsp::resample::median_dt(&t_us);
    if dt <= 0.0 {
        return Err(IngestError::Empty);
    }
    let source_rate = 1.0 / dt;
    let fs = opts.fs_hz.unwrap_or_else(|| source_rate.round());
    let t0 = t_us[0];
    let t_end = *t_us.last().unwrap();
    let grid = dsp::resample::uniform_grid(t0, t_end, fs);
    let gaps = dsp::resample::find_gaps(&t_us, dt, opts.gap_mult);
    let rs = |name: &str| -> Option<Vec<f32>> {
        cols.get(name).map(|c| dsp::resample::interp_linear(&t_us, &c.data, &grid))
    };
    let rs3 = |base: &str| -> Option<[Vec<f32>; 3]> {
        Some([rs(&format!("{base}[0]"))?, rs(&format!("{base}[1]"))?, rs(&format!("{base}[2]"))?])
    };

    let gyro_filt = rs3("gyroADC").ok_or(IngestError::MissingField("gyroADC"))?;
    let mut warnings = Vec::new();
    let mut extra_headers: Vec<(String, String)> = Vec::new();
    let setpoint: [Vec<f32>; 3] = match rs3("setpoint") {
        Some(sp) => sp,
        None => {
            let rc = rs3("rcCommand").ok_or(IngestError::MissingField("setpoint"))?;
            let prof = rates::RatesProfile::from_headers(&raw_headers);
            match prof {
                Some(pr) if pr.supported() => {
                    warnings.push(format!(
                        "setpoint is not logged (blackbox_disable_setpoint = ON, fields_disabled_mask bit 2): rebuilt from rcCommand with the {} rates (rc_rates {:?}, rates {:?}, expo {:?}) without RC smoothing — step-response latency reads a few ms high. Enable the Setpoint field in Blackbox for exact results.",
                        pr.type_name(), pr.rc_rates, pr.rates, pr.rc_expo
                    ));
                    extra_headers.push(("bf.setpoint_reconstructed".into(), pr.type_name().to_string()));
                    let mut out: [Vec<f32>; 3] = Default::default();
                    for k in 0..3 {
                        out[k] = rc[k].iter().map(|&v| pr.setpoint(k, v)).collect();
                    }
                    out
                }
                Some(pr) => {
                    warnings.push(format!("setpoint is not logged and the {} rates type is not modelled: no step response possible. Enable the Setpoint field in Blackbox (set blackbox_disable_setpoint = OFF).", pr.type_name()));
                    let z = vec![0.0f32; grid.len()];
                    [z.clone(), z.clone(), z]
                }
                None => {
                    warnings.push("setpoint is not logged and the rates headers are missing: no step response possible. Enable the Setpoint field in Blackbox.".into());
                    let z = vec![0.0f32; grid.len()];
                    [z.clone(), z.clone(), z]
                }
            }
        }
    };
    let gyro_raw = match rs3("gyroUnfilt") {
        Some(g) => Some(g),
        None if debug_mode == "GYRO_SCALED" => rs3("debug"),
        None => {
            warnings.push(
                "no unfiltered gyro: log gyroUnfilt (BF ≥ 4.4) or set debug_mode = GYRO_SCALED".into(),
            );
            None
        }
    };
    let p = rs3("axisP");
    let i = rs3("axisI");
    let d = rs3("axisD").or_else(|| {
        // yaw D is often not logged; substitute zeros when roll/pitch exist.
        let r = rs("axisD[0]")?;
        let pch = rs("axisD[1]")?;
        let z = vec![0.0; r.len()];
        Some([r, pch, z])
    });
    let ff = rs3("axisF");

    let mut axes: [AxisSeries; 3] = Default::default();
    for (k, ax) in axes.iter_mut().enumerate() {
        ax.setpoint = setpoint[k].clone();
        ax.gyro_filt = gyro_filt[k].clone();
        ax.gyro_raw = gyro_raw.as_ref().map(|g| g[k].clone());
        ax.p = p.as_ref().map(|v| v[k].clone());
        ax.i = i.as_ref().map(|v| v[k].clone());
        ax.d = d.as_ref().map(|v| v[k].clone());
        ax.ff = ff.as_ref().map(|v| v[k].clone());
        if let (Some(p), Some(i), Some(d)) = (&ax.p, &ax.i, &ax.d) {
            let ffv = ax.ff.as_ref();
            ax.pid_sum = Some(
                (0..p.len())
                    .map(|n| p[n] + i[n] + d[n] + ffv.map(|f| f[n]).unwrap_or(0.0))
                    .collect(),
            );
        }
    }

    // motors normalised by the motorOutput range header (e.g. "0,1000" for DShot).
    let (m_lo, m_hi) = raw_headers
        .get("motorOutput")
        .and_then(|v| {
            let mut it = v.split(',');
            Some((it.next()?.trim().parse::<f32>().ok()?, it.next()?.trim().parse::<f32>().ok()?))
        })
        .unwrap_or((1000.0, 2000.0));
    let m_span = (m_hi - m_lo).max(1.0);
    let mut motors = Vec::new();
    for k in 0..8 {
        match rs(&format!("motor[{k}]")) {
            Some(m) => motors.push(m.iter().map(|v| ((v - m_lo) / m_span).clamp(0.0, 1.2)).collect()),
            None => break,
        }
    }
    let mut erpm = Vec::new();
    for k in 0..8 {
        match rs(&format!("eRPM[{k}]")) {
            Some(m) => erpm.push(m),
            None => break,
        }
    }
    let throttle = match rs("setpoint[3]") {
        Some(t) => t.iter().map(|v| (v / 1000.0).clamp(0.0, 1.0)).collect(),
        None => rs("rcCommand[3]")
            .map(|t| t.iter().map(|v| ((v - 1000.0) / 1000.0).clamp(0.0, 1.0)).collect())
            .unwrap_or_else(|| vec![0.0; grid.len()]),
    };

    let bf_tune = tune::parse_bf_tune(&raw_headers);
    let version = headers::firmware_version(&raw_headers).unwrap_or_default();
    let looptime_us: Option<f64> = raw_headers.get("looptime").and_then(|v| v.parse().ok());
    let pid_denom: f64 = raw_headers.get("pid_process_denom").and_then(|v| v.parse().ok()).unwrap_or(1.0);
    let loop_hz = looptime_us.map(|lt| 1e6 / (lt * pid_denom));

    let mut hasher = blake3::Hasher::new();
    hasher.update(bytes);
    hasher.update(&session.to_le_bytes());
    let id = LogId(hasher.finalize().to_hex().to_string());

    Ok(FlightLog {
        id,
        firmware: Firmware::Betaflight { version, api: (0, 0) },
        fs_hz: fs,
        t: grid,
        axes,
        motors,
        throttle,
        erpm: if erpm.is_empty() { None } else { Some(erpm) },
        gaps,
        meta: LogMeta {
            craft_name: hdr.craft_name().map(str::to_string),
            loop_hz,
            source_rate_hz: source_rate,
            debug_mode: Some(debug_mode),
            duration_s: t_end - t0,
            session_index: session,
            session_count,
            headers: { raw_headers.extend(extra_headers); raw_headers },
            warnings,
            msg_rates_hz: Default::default(),
        },
        tune_at_log: Tune::Bf(bf_tune),
        gyro_hr: Vec::new(),
    })
}
