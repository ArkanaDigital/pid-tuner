//! Guard definitions per step. Each guard is a pure predicate over the
//! session and the (optional) live FC status.

use crate::model::*;
use domain::*;
use serde::{Deserialize, Serialize};

pub use domain::fc::FcStatus;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GuardOutcome {
    Pass,
    Fail { message: String, fix_hint: Option<String> },
    NeedsAction { message: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardResult {
    pub id: String,
    pub title: String,
    pub outcome: GuardOutcome,
    pub can_override: bool,
    pub overridden: bool,
}

impl GuardResult {
    pub fn satisfied(&self) -> bool {
        matches!(self.outcome, GuardOutcome::Pass) || self.overridden
    }
}

pub struct GuardCtx<'a> {
    pub session: &'a Session,
    pub fc: Option<&'a FcStatus>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Fw {
    Any,
    Bf,
    Ap,
}

struct GuardDef {
    id: &'static str,
    title: &'static str,
    can_override: bool,
    fw: Fw,
    eval: fn(&GuardCtx) -> GuardOutcome,
}

/// Firmware the session is about: from the FC when connected, else from the
/// first imported log, else unknown (both guard sets apply where sensible).
fn session_fw(c: &GuardCtx) -> Option<Fw> {
    let f = c.fc.and_then(|f| f.firmware.clone()).or_else(|| c.session.firmware.clone())?;
    Some(match f {
        Firmware::Betaflight { .. } => Fw::Bf,
        Firmware::ArduCopter { .. } => Fw::Ap,
        Firmware::Unknown { .. } => return None,
    })
}

fn pass() -> GuardOutcome {
    GuardOutcome::Pass
}
fn fail(msg: impl Into<String>, hint: Option<&str>) -> GuardOutcome {
    GuardOutcome::Fail { message: msg.into(), fix_hint: hint.map(str::to_string) }
}
fn action(msg: impl Into<String>) -> GuardOutcome {
    GuardOutcome::NeedsAction { message: msg.into() }
}

// ---------------------------------------------------------------------------
// Individual guards
// ---------------------------------------------------------------------------

fn fc_connected(c: &GuardCtx) -> GuardOutcome {
    match c.fc {
        Some(f) if f.connected => pass(),
        _ => action("Connect the flight controller over USB and select its port."),
    }
}

fn fc_supported(c: &GuardCtx) -> GuardOutcome {
    let Some(f) = c.fc.filter(|f| f.connected) else { return action("Not connected.") };
    match &f.firmware {
        Some(Firmware::Betaflight { version, .. }) => {
            let ok = version.split('.').next().and_then(|m| m.parse::<u32>().ok()).map(|m| m >= 4).unwrap_or(false);
            if ok { pass() } else { fail(format!("Betaflight {version} is not supported (need ≥ 4.3)."), None) }
        }
        Some(Firmware::ArduCopter { version }) => {
            let mut it = version.split('.').map(|x| x.parse::<u32>().unwrap_or(0));
            let (maj, min) = (it.next().unwrap_or(0), it.next().unwrap_or(0));
            if maj > 4 || (maj == 4 && min >= 4) { pass() } else { fail(format!("ArduCopter {version} is not supported (need ≥ 4.4: PIDx SRate and ISBH layouts)."), None) }
        }
        Some(Firmware::Unknown { product }) => fail(format!("Unsupported firmware: {product}"), None),
        None => action("Waiting for firmware identification…"),
    }
}

fn fc_tune_read(c: &GuardCtx) -> GuardOutcome {
    match c.fc {
        Some(f) if f.connected && f.tune.is_some() => pass(),
        _ => action("Reading the current tune from the flight controller…"),
    }
}

fn backup_taken(c: &GuardCtx) -> GuardOutcome {
    if c.session.snapshots.iter().any(|s| s.label.starts_with("00-")) || c.fc.map(|f| f.snapshot_taken).unwrap_or(false) {
        pass()
    } else {
        action("Take a full settings backup (diff all / param.pck) before changing anything.")
    }
}

fn fc_disarmed(c: &GuardCtx) -> GuardOutcome {
    match c.fc {
        Some(f) if f.connected && !f.armed && f.heartbeat_age_s < 2.0 => pass(),
        Some(f) if f.connected && f.armed => fail("Flight controller is ARMED. Remove props and disarm.", None),
        Some(f) if f.connected => fail(format!("No heartbeat for {:.1} s.", f.heartbeat_age_s), Some("Reconnect the USB cable.")),
        _ => action("Not connected."),
    }
}

fn log_rate_ok(c: &GuardCtx) -> GuardOutcome {
    let Some(f) = c.fc.filter(|f| f.connected) else { return action("Not connected.") };
    match (&f.firmware, f.log_rate_hz) {
        (Some(Firmware::Betaflight { .. }), Some(_)) if f.pid_logging_enabled == Some(false) => fail(
            "The blackbox field mask switches off PID, Setpoint or Gyro fields (blackbox_disable_*), so the log would be unusable.",
            Some("Preflight fix clears blackbox_disable_pids / _setpoint / _gyro / _gyrounfilt / _motors / _rc (fields_disabled_mask, blackbox_fielddefs.h)."),
        ),
        (Some(Firmware::Betaflight { .. }), Some(r)) if r >= 1900.0 => pass(),
        (Some(Firmware::Betaflight { .. }), Some(r)) => fail(
            format!("Blackbox rate is {r:.0} Hz; need ≥ 2 kHz for a 1 kHz spectrum."),
            Some("Set blackbox_sample_rate so that loop_rate × rate ≥ 2 kHz (e.g. 1/4 at 8 kHz)."),
        ),
        (Some(Firmware::ArduCopter { .. }), _) => match f.pid_logging_enabled {
            Some(true) => pass(),
            _ => fail("LOG_BITMASK must include Fast Attitude (bit 0) and PID (bit 12).", None),
        },
        _ => action("Reading logging configuration…"),
    }
}

fn raw_gyro_logging(c: &GuardCtx) -> GuardOutcome {
    let Some(f) = c.fc.filter(|f| f.connected) else { return action("Not connected.") };
    match f.raw_gyro_logging_enabled {
        Some(true) => pass(),
        Some(false) => fail(
            "Unfiltered gyro is not being logged.",
            Some("Betaflight ≤ 4.3: set debug_mode = GYRO_SCALED. ArduPilot: INS_RAW_LOG_OPT = 9 (H7) or INS_LOG_BAT_MASK = 1, INS_LOG_BAT_OPT = 4."),
        ),
        None => action("Reading logging configuration…"),
    }
}

fn storage_free(c: &GuardCtx) -> GuardOutcome {
    let Some(f) = c.fc.filter(|f| f.connected) else { return action("Not connected.") };
    match f.storage_free_bytes {
        Some(b) if b >= 4 * 1024 * 1024 => pass(),
        Some(b) => fail(format!("Only {:.1} MB free on the blackbox device.", b as f64 / 1e6), Some("Erase the flash / clear the SD card.")),
        None => action("Reading storage state…"),
    }
}

fn flight_done(which: Flight) -> fn(&GuardCtx) -> GuardOutcome {
    match which {
        Flight::A => |c| if c.session.flight_done.contains_key(&Flight::A) { pass() } else { action("Fly the protocol, land, disarm, then tick 'flight done'.") },
        Flight::B => |c| if c.session.flight_done.contains_key(&Flight::B) { pass() } else { action("Fly the protocol, land, disarm, then tick 'flight done'.") },
        Flight::C => |c| if c.session.flight_done.contains_key(&Flight::C) { pass() } else { action("Fly the protocol, land, disarm, then tick 'flight done'.") },
    }
}

fn record<'a>(c: &'a GuardCtx, which: Flight) -> Option<&'a FlightRecord> {
    c.session.flights.get(&which)
}

fn log_imported(which: Flight) -> fn(&GuardCtx) -> GuardOutcome {
    match which {
        Flight::A => |c| if record(c, Flight::A).is_some() { pass() } else { action("Import the blackbox log for this flight.") },
        Flight::B => |c| if record(c, Flight::B).is_some() { pass() } else { action("Import the blackbox log for this flight.") },
        Flight::C => |c| if record(c, Flight::C).is_some() { pass() } else { action("Import the blackbox log for this flight.") },
    }
}

fn log_rate_guard(r: &FlightRecord) -> GuardOutcome {
    if r.quality.fs_hz >= 1900.0 {
        pass()
    } else if r.quality.fs_hz >= 950.0 {
        fail(
            format!("Log rate is {:.0} Hz; spectrum only reaches {:.0} Hz. Step response is fine, filter analysis is limited.", r.quality.fs_hz, r.quality.fs_hz / 2.0),
            Some("Log at ≥ 2 kHz (blackbox_sample_rate 1/4 at 8 kHz loop)."),
        )
    } else {
        fail(format!("Log rate is only {:.0} Hz.", r.quality.fs_hz), Some("Log at ≥ 2 kHz."))
    }
}

fn duration_guard(r: &FlightRecord, min_s: f64) -> GuardOutcome {
    if r.quality.duration_s >= min_s {
        pass()
    } else {
        fail(format!("Log is {:.1} s; need ≥ {min_s:.0} s.", r.quality.duration_s), None)
    }
}

fn raw_gyro_guard(r: &FlightRecord) -> GuardOutcome {
    if r.quality.has_gyro_raw {
        pass()
    } else {
        fail(
            "No unfiltered gyro in this log, so pre/post filter comparison is impossible.",
            Some("Betaflight ≥ 4.4 logs gyroUnfilt natively; on 4.3 set debug_mode = GYRO_SCALED."),
        )
    }
}

fn hover_guard(r: &FlightRecord) -> GuardOutcome {
    if r.quality.hover_seconds >= 20.0 {
        pass()
    } else {
        fail(
            format!("Only {:.1} s of steady hover (around {:.0} % throttle); need ≥ 20 s.", r.quality.hover_seconds, r.quality.hover_throttle_pct),
            Some("Hover steadily for 30 s at a normal hover throttle."),
        )
    }
}

fn saturation_guard(r: &FlightRecord) -> GuardOutcome {
    if r.quality.motor_saturation_pct < 5.0 {
        pass()
    } else {
        fail(format!("Motors saturated {:.1} % of the time; the response is not representative.", r.quality.motor_saturation_pct), Some("Fly with less throttle / lower rates."))
    }
}

fn steps_guard(r: &FlightRecord) -> GuardOutcome {
    let s = r.quality.step_segments_per_axis;
    let ms = r.quality.max_setpoint_per_axis;
    let mut problems = Vec::new();
    for (k, name, need) in [(0, "roll", 30usize), (1, "pitch", 30), (2, "yaw", 10)] {
        if s[k] < need {
            let why = if ms[k] < 150.0 {
                format!("{name}: only {} usable segments — stick input too small (max {:.0} °/s)", s[k], ms[k])
            } else if ms[k] > 900.0 && k == 2 {
                format!("{name}: only {} usable segments — full-rate spins saturate the estimate, use moderate yaw inputs (200–500 °/s)", s[k])
            } else {
                format!("{name}: only {} usable segments (need ≥ {need})", s[k])
            };
            problems.push(why);
        }
    }
    let reconstructed = r.warnings.iter().any(|w| w.contains("setpoint is not logged"));
    if problems.is_empty() {
        if reconstructed {
            // usable, but the pilot should log the real setpoint next time
            return fail(
                "setpoint was not logged (blackbox_disable_setpoint = ON); the step response uses a setpoint rebuilt from rcCommand without RC smoothing, so latency reads a few ms high.",
                Some("Enable the Setpoint field in Betaflight Blackbox (CLI: set blackbox_disable_setpoint = OFF, save) and re-fly, or override to continue with the rebuilt setpoint."),
            );
        }
        pass()
    } else {
        let hint = if reconstructed {
            "Setpoint is not logged (blackbox_disable_setpoint = ON) — enable the Setpoint field in Blackbox, then do sharp, isolated stick snaps on ONE axis at a time (roll ×10, pitch ×10, yaw ×5), each held ~½ s."
        } else {
            "Do sharp, isolated stick snaps on ONE axis at a time (roll ×10, pitch ×10, yaw ×5), each held ~½ s. Below ~30 segments the averaged curve is still noisy."
        };
        fail(problems.join("; "), Some(hint))
    }
}

fn tune_matches(c: &GuardCtx, which: Flight, phase: Option<ApplyPhase>) -> GuardOutcome {
    let Some(r) = record(c, which) else { return action("Import the log first.") };
    // Reference tune: what was last applied (if any), else the FC tune, else nothing to check.
    if let Some(p) = phase {
        if let Some(a) = c.session.apply_for(p) {
            let mismatches: Vec<String> = a
                .applied
                .iter()
                .filter(|rec| rec.accepted)
                .filter_map(|rec| {
                    let logged = tune_param(&r.tune, rec.param.name())?;
                    if (logged - rec.new.as_f64()).abs() > 1e-6 {
                        Some(format!("{} is {} in the log, expected {}", rec.param.name(), logged, rec.new))
                    } else {
                        None
                    }
                })
                .collect();
            if !mismatches.is_empty() {
                return fail(format!("Log was not flown with the applied settings: {}", mismatches.join("; ")), Some("Make sure the settings were saved and re-fly."));
            }
            return pass();
        }
    }
    if let (Some(Tune::Bf(ft)), Tune::Bf(lt)) = (&c.session.fc_tune, &r.tune) {
        if ft.pids != lt.pids {
            return fail("PIDs in the log differ from the flight controller's current PIDs.", Some("Re-fly with the current settings."));
        }
    }
    if let (Some(Tune::Ap(ft)), Tune::Ap(lt)) = (&c.session.fc_tune, &r.tune) {
        let diff: Vec<String> = ["ATC_RAT_RLL_P", "ATC_RAT_RLL_I", "ATC_RAT_RLL_D", "ATC_RAT_PIT_P", "ATC_RAT_PIT_I", "ATC_RAT_PIT_D", "ATC_RAT_YAW_P", "ATC_RAT_YAW_I", "ATC_RAT_YAW_D", "INS_GYRO_FILTER", "INS_HNTCH_FREQ"]
            .iter()
            .filter_map(|n| match (ft.get(n), lt.get(n)) {
                (Some(a), Some(b)) if (a - b).abs() > 1e-6 * a.abs().max(1.0) => Some(format!("{n} log {b} vs FC {a}")),
                _ => None,
            })
            .collect();
        if !diff.is_empty() {
            return fail(format!("Log tune differs from the flight controller: {}", diff.join("; ")), Some("Re-fly with the current settings."));
        }
    }
    pass()
}

/// Look up a CLI/param name in a tune parsed from a log header.
pub fn tune_param(t: &Tune, name: &str) -> Option<f64> {
    match t {
        Tune::Bf(b) => {
            let ax = |n: &str| ["roll", "pitch", "yaw"].iter().position(|a| n.ends_with(a));
            let legacy = b.legacy_d_naming();
            let v = match name {
                n if n.starts_with("p_") => b.pids[ax(n)?].p as f64,
                n if n.starts_with("i_") => b.pids[ax(n)?].i as f64,
                n if n.starts_with("d_max_") => b.pids[ax(n)?].d_max as f64,
                n if n.starts_with("d_min_") => b.pids[ax(n)?].d as f64,
                n if n.starts_with("d_") && legacy => b.pids[ax(n)?].d_max as f64,
                n if n.starts_with("d_") => b.pids[ax(n)?].d as f64,
                n if n.starts_with("f_") => b.pids[ax(n)?].ff as f64,
                "simplified_pids_mode" => b.simplified.pids_mode as f64,
                "simplified_gyro_filter" => b.simplified.gyro_filter as u8 as f64,
                "simplified_dterm_filter" => b.simplified.dterm_filter as u8 as f64,
                "dyn_notch_count" => b.filters.dyn_notch_count as f64,
                "dyn_notch_min_hz" => b.filters.dyn_notch_min_hz as f64,
                "dyn_notch_max_hz" => b.filters.dyn_notch_max_hz as f64,
                "dyn_notch_q" => b.filters.dyn_notch_q as f64,
                "gyro_lpf1_dyn_max_hz" => b.filters.gyro_lpf1_dyn_max_hz as f64,
                "gyro_lpf1_static_hz" => b.filters.gyro_lpf1_static_hz as f64,
                "dterm_lpf1_dyn_max_hz" => b.filters.dterm_lpf1_dyn_max_hz as f64,
                "dterm_lpf1_static_hz" => b.filters.dterm_lpf1_static_hz as f64,
                "rpm_filter_harmonics" => b.filters.rpm_filter_harmonics as f64,
                other => b.raw.get(other)?.parse().ok()?,
            };
            Some(v)
        }
        Tune::Ap(a) => a.get(name).map(|v| v as f64),
        Tune::Unknown => None,
    }
}

fn analysis_done(which: Flight) -> fn(&GuardCtx) -> GuardOutcome {
    match which {
        Flight::A => |c| if record(c, Flight::A).is_some() { pass() } else { action("Analysis runs automatically after import.") },
        Flight::B => |c| if record(c, Flight::B).is_some() { pass() } else { action("Analysis runs automatically after import.") },
        Flight::C => |c| if record(c, Flight::C).is_some() { pass() } else { action("Analysis runs automatically after import.") },
    }
}

fn applied(phase: ApplyPhase) -> fn(&GuardCtx) -> GuardOutcome {
    match phase {
        ApplyPhase::Filters => |c| applied_impl(c, ApplyPhase::Filters),
        ApplyPhase::Pids => |c| applied_impl(c, ApplyPhase::Pids),
    }
}

fn applied_impl(c: &GuardCtx, phase: ApplyPhase) -> GuardOutcome {
    let recs = c.session.recs(phase);
    match c.session.apply_for(phase) {
        Some(a) if a.verified => pass(),
        Some(_) => fail("Settings were written but read-back verification failed.", Some("Reconnect and apply again, or roll back from the snapshot.")),
        None if recs.iter().all(|r| !r.accepted) => pass(), // explicit skip: nothing accepted
        None => action(match c.session.mode {
            Mode::Online => "Write the accepted settings to the flight controller.",
            Mode::Offline => "Send the CLI / param file to the pilot and confirm it was applied (save).",
        }),
    }
}

// ---------------------------------------------------------------------------
// ArduPilot guards. Every ArduPilot constant comes from `domain::ap_consts`
// (cited there); thresholds marked "ours" are ours.
// ---------------------------------------------------------------------------

use domain::ap_consts::*;

/// ours: measured PIDx rate must reach this fraction of the loop rate.
const AP_PID_RATE_FRACTION: f64 = 0.9;
/// ours: batches needed for a trustworthy averaged spectrum.
const AP_MIN_GYRO_BATCHES: usize = 20;
/// ours: ~30 % of default Stabilize rates (ANGLE_MAX 4500 cd × ATC_ANG_*_P 4.5 ≈ 200 °/s) — small but real inputs.
const AP_MIN_SETPOINT_RP_DPS: f32 = 60.0;
/// ours: yaw is slower (PILOT_Y_RATE default 202.5 °/s).
const AP_MIN_SETPOINT_YAW_DPS: f32 = 40.0;
/// ours: mixer output within this fraction of saturation is fine.
const AP_MAX_PID_OUT: f32 = 0.9;

fn ap_bits(mask: u32, wanted: u32) -> bool {
    mask & wanted == wanted
}

fn ap_log_bitmask(c: &GuardCtx) -> GuardOutcome {
    let Some(f) = c.fc.filter(|f| f.connected) else { return action("Not connected.") };
    match f.log_bitmask {
        Some(m) if ap_bits(m, LOG_BIT_ATTITUDE_FAST | LOG_BIT_PID) => pass(),
        Some(m) => fail(
            format!("LOG_BITMASK = {m} lacks bit 0 (ATTITUDE_FAST) and/or bit 12 (PID); RATE/PIDx would log at 10 Hz instead of the loop rate."),
            Some("Preflight fix sets LOG_BITMASK |= bit 0 | bit 12 | bit 19 (ArduCopter/defines.h MASK_LOG_*)."),
        ),
        None => action("Reading LOG_BITMASK…"),
    }
}

fn ap_batch_configured(c: &GuardCtx) -> GuardOutcome {
    let Some(f) = c.fc.filter(|f| f.connected) else { return action("Not connected.") };
    match f.batch_configured {
        Some(true) => pass(),
        Some(false) => fail(
            "IMU batch sampler is off: no gyro spectrum will be logged.",
            Some("Set LOG_BITMASK bit 19 (IMU_RAW), INS_LOG_BAT_MASK = 1, INS_LOG_BAT_OPT = 4 (pre + post filter), INS_LOG_BAT_CNT = 1024, INS_LOG_BAT_LGIN = 20. INS_LOG_BAT_MASK needs a reboot."),
        ),
        None => action("Reading INS_LOG_BAT_* …"),
    }
}

fn ap_pid_rate_guard(r: &FlightRecord) -> GuardOutcome {
    let loop_hz = r.tune_loop_rate();
    match (r.quality.pid_rate_hz, loop_hz) {
        (None, _) => fail("PIDR/PIDP/PIDY are not in the log (LOG_BITMASK bit 12 PID).", Some("Enable bit 12 and bit 0 of LOG_BITMASK and re-fly.")),
        (Some(pr), Some(lh)) if pr >= AP_PID_RATE_FRACTION * lh => pass(),
        (Some(pr), Some(lh)) => fail(
            format!("PIDx logged at {pr:.0} Hz but the loop runs at {lh:.0} Hz (SCHED_LOOP_RATE): LOG_BITMASK bit 0 (ATTITUDE_FAST) is off, so the step response cannot be estimated."),
            Some("Set LOG_BITMASK bit 0 (MASK_LOG_ATTITUDE_FAST, ArduCopter/Copter.cpp loop_rate_logging) and re-fly."),
        ),
        (Some(pr), None) if pr >= 200.0 => pass(),
        (Some(pr), None) => fail(format!("PIDx logged at only {pr:.0} Hz."), Some("Set LOG_BITMASK bit 0 (ATTITUDE_FAST).")),
    }
}

fn ap_isbh_guard(r: &FlightRecord) -> GuardOutcome {
    if r.quality.gyro_hr_batches >= AP_MIN_GYRO_BATCHES {
        pass()
    } else if r.quality.gyro_hr_batches == 0 {
        fail("No IMU batch-sampler data (ISBH/ISBD) in the log.", Some("LOG_BITMASK bit 19 + INS_LOG_BAT_MASK = 1, INS_LOG_BAT_OPT = 4 (needs reboot), then re-fly."))
    } else {
        fail(format!("Only {} gyro batches; need ≥ {AP_MIN_GYRO_BATCHES} for an averaged spectrum.", r.quality.gyro_hr_batches), Some("Fly longer (≥ 40 s) or lower INS_LOG_BAT_LGIN."))
    }
}

fn ap_steps_guard(r: &FlightRecord) -> GuardOutcome {
    let s = r.quality.step_segments_per_axis;
    let ms = r.quality.max_setpoint_per_axis;
    let mut problems = Vec::new();
    for (k, name, need, min_sp) in [(0, "roll", 30usize, AP_MIN_SETPOINT_RP_DPS), (1, "pitch", 30, AP_MIN_SETPOINT_RP_DPS), (2, "yaw", 10, AP_MIN_SETPOINT_YAW_DPS)] {
        if s[k] < need {
            let why = if ms[k] < min_sp {
                format!("{name}: only {} usable segments — stick input too small (max {:.0} °/s, need ≥ {min_sp:.0})", s[k], ms[k])
            } else {
                format!("{name}: only {} usable segments (need ≥ {need})", s[k])
            };
            problems.push(why);
        }
    }
    if problems.is_empty() {
        pass()
    } else {
        fail(problems.join("; "), Some("In Stabilize: crisp stick steps on ONE axis at a time (roll ×10, pitch ×10, yaw ×5), hold ~½ s. ATC_INPUT_TC smooths the target, so keep the moves sharp."))
    }
}

fn ap_hover_guard(r: &FlightRecord) -> GuardOutcome {
    if r.quality.hover_seconds >= 30.0 {
        pass()
    } else {
        fail(format!("Only {:.1} s of steady hover (need ≥ 30 s).", r.quality.hover_seconds), Some("Hover in AltHold/Loiter for 30–40 s without stick input."))
    }
}

fn ap_saturation_guard(r: &FlightRecord) -> GuardOutcome {
    match r.quality.max_pid_out {
        Some(m) if m.iter().all(|v| *v < AP_MAX_PID_OUT) => pass(),
        Some(m) => fail(
            format!("Mixer output saturated (RATE.*Out max R/P/Y = {:.2}/{:.2}/{:.2}); gains cannot be judged.", m[0], m[1], m[2]),
            Some("Reduce ATC_RAT_* gains or check motor/prop sizing before tuning."),
        ),
        None => pass(),
    }
}

impl FlightRecord {
    /// `SCHED_LOOP_RATE` from the tune parsed out of the log.
    pub fn tune_loop_rate(&self) -> Option<f64> {
        match &self.tune {
            Tune::Ap(t) => t.get("SCHED_LOOP_RATE").map(|v| v as f64),
            _ => None,
        }
    }
}

/// AUTOTUNE branch: the verification log must show new ATC_RAT_{axis}_{P,D}
/// for every axis in AUTOTUNE_AXES and D above AUTOTUNE_MIN_D (a D that ends
/// at the floor means the tune failed — ArduCopter AUTOTUNE docs / AC_AutoTune_Multi).
fn autotune_result(c: &GuardCtx) -> GuardOutcome {
    if c.session.pid_strategy != PidStrategy::Autotune {
        return pass();
    }
    let (Some(b), Some(cc)) = (record(c, Flight::B), record(c, Flight::C)) else { return action("Import logs B and C.") };
    let (Tune::Ap(before), Tune::Ap(after)) = (&b.tune, &cc.tune) else { return pass() };
    let axes = after.get("AUTOTUNE_AXES").unwrap_or(7.0) as u32;
    let min_d = after.get("AUTOTUNE_MIN_D").unwrap_or(0.001);
    let mut problems = Vec::new();
    for (bit, ax) in [(1u32, "RLL"), (2, "PIT"), (4, "YAW")] {
        if axes & bit == 0 {
            continue;
        }
        let p = format!("ATC_RAT_{ax}_P");
        let d = format!("ATC_RAT_{ax}_D");
        let (bp, ap_) = (before.get(&p).unwrap_or(0.0), after.get(&p).unwrap_or(0.0));
        let (bd, ad) = (before.get(&d).unwrap_or(0.0), after.get(&d).unwrap_or(0.0));
        if (bp - ap_).abs() < 1e-6 && (bd - ad).abs() < 1e-6 {
            problems.push(format!("{ax}: P/D unchanged — AUTOTUNE was not saved for this axis"));
        } else if ax != "YAW" && ad <= min_d + 1e-9 {
            problems.push(format!("{ax}: D = {ad} is at AUTOTUNE_MIN_D — the tune failed (lower AUTOTUNE_AGGR, check frame stiffness)"));
        }
    }
    if problems.is_empty() { pass() } else { fail(problems.join("; "), Some("Re-run AUTOTUNE and land/disarm without touching the sticks to save it, or switch to the heuristic path.")) }
}

fn report_written(c: &GuardCtx) -> GuardOutcome {
    if c.session.report_file.is_some() { pass() } else { action("Export the report.") }
}

// ---------------------------------------------------------------------------
// Table
// ---------------------------------------------------------------------------

fn defs(step: Step) -> Vec<GuardDef> {
    macro_rules! g {
        ($id:expr, $title:expr, $ov:expr, $f:expr) => {
            GuardDef { id: $id, title: $title, can_override: $ov, fw: Fw::Any, eval: $f }
        };
        ($id:expr, $title:expr, $ov:expr, $fw:expr, $f:expr) => {
            GuardDef { id: $id, title: $title, can_override: $ov, fw: $fw, eval: $f }
        };
    }
    match step {
        Step::Connect => vec![
            g!("fc_connected", "Flight controller connected", false, fc_connected),
            g!("fc_supported", "Firmware supported", false, fc_supported),
            g!("fc_tune_read", "Current tune read", false, fc_tune_read),
            g!("backup_taken", "Settings backup saved", false, backup_taken),
        ],
        Step::Preflight => vec![
            g!("fc_disarmed", "Disarmed, props off", false, fc_disarmed),
            g!("log_rate", "Logging rate ≥ 2 kHz", true, Fw::Bf, log_rate_ok),
            g!("raw_gyro_logging", "Unfiltered gyro logged", true, Fw::Bf, raw_gyro_logging),
            g!("ap_log_bitmask", "LOG_BITMASK bits 0 + 12 (loop-rate RATE/PIDx)", true, Fw::Ap, ap_log_bitmask),
            g!("ap_batch", "IMU batch sampler configured", true, Fw::Ap, ap_batch_configured),
            g!("storage_free", "≥ 4 MB log storage free", true, storage_free),
        ],
        Step::FlightA => vec![g!("flight_done", "Flight A completed", false, flight_done(Flight::A))],
        Step::ImportA => vec![
            g!("imported", "Log imported", false, log_imported(Flight::A)),
            g!("log_rate", "Log rate ≥ 2 kHz", true, Fw::Bf, |c| record(c, Flight::A).map(log_rate_guard).unwrap_or_else(|| action("Import first."))),
            g!("duration", "≥ 40 s of data", true, |c| record(c, Flight::A).map(|r| duration_guard(r, 40.0)).unwrap_or_else(|| action("Import first."))),
            g!("raw_gyro", "Unfiltered gyro present", true, Fw::Bf, |c| record(c, Flight::A).map(raw_gyro_guard).unwrap_or_else(|| action("Import first."))),
            g!("ap_isbh", "IMU batch-sampler gyro data present", true, Fw::Ap, |c| record(c, Flight::A).map(ap_isbh_guard).unwrap_or_else(|| action("Import first."))),
            g!("hover", "≥ 20 s of steady hover", true, Fw::Bf, |c| record(c, Flight::A).map(hover_guard).unwrap_or_else(|| action("Import first."))),
            g!("ap_hover", "≥ 30 s of steady hover", true, Fw::Ap, |c| record(c, Flight::A).map(ap_hover_guard).unwrap_or_else(|| action("Import first."))),
            g!("saturation", "Motor saturation < 5 %", true, |c| record(c, Flight::A).map(saturation_guard).unwrap_or_else(|| action("Import first."))),
            g!("tune_match", "Log matches current tune", true, |c| tune_matches(c, Flight::A, None)),
        ],
        Step::FilterAnalysis => vec![g!("analysis", "Spectrum analysis complete", false, analysis_done(Flight::A))],
        Step::ApplyFilters => vec![
            g!("fc_disarmed", "Disarmed (online) ", true, |c| if c.session.mode == Mode::Offline { pass() } else { fc_disarmed(c) }),
            g!("applied", "Filter settings applied & verified", false, applied(ApplyPhase::Filters)),
        ],
        Step::FlightB => vec![g!("flight_done", "Flight B completed", false, flight_done(Flight::B))],
        Step::ImportB => vec![
            g!("imported", "Log imported", false, log_imported(Flight::B)),
            g!("log_rate", "Log rate ≥ 1 kHz", true, Fw::Bf, |c| record(c, Flight::B).map(|r| if r.quality.fs_hz >= 950.0 { pass() } else { log_rate_guard(r) }).unwrap_or_else(|| action("Import first."))),
            g!("ap_pid_rate", "PIDx logged at loop rate", true, Fw::Ap, |c| record(c, Flight::B).map(ap_pid_rate_guard).unwrap_or_else(|| action("Import first."))),
            g!("duration", "≥ 30 s of data", true, |c| record(c, Flight::B).map(|r| duration_guard(r, 30.0)).unwrap_or_else(|| action("Import first."))),
            g!("steps", "Enough stick steps per axis", true, Fw::Bf, |c| record(c, Flight::B).map(steps_guard).unwrap_or_else(|| action("Import first."))),
            g!("ap_steps", "Enough stick steps per axis", true, Fw::Ap, |c| record(c, Flight::B).map(ap_steps_guard).unwrap_or_else(|| action("Import first."))),
            g!("saturation", "Motor saturation < 5 %", true, |c| record(c, Flight::B).map(saturation_guard).unwrap_or_else(|| action("Import first."))),
            g!("ap_saturation", "Mixer output not saturated", true, Fw::Ap, |c| record(c, Flight::B).map(ap_saturation_guard).unwrap_or_else(|| action("Import first."))),
            g!("tune_match", "Log flown with the applied filters", true, |c| tune_matches(c, Flight::B, Some(ApplyPhase::Filters))),
        ],
        Step::PidAnalysis => vec![g!("analysis", "Step-response analysis complete", false, analysis_done(Flight::B))],
        Step::ApplyPids => vec![
            g!("fc_disarmed", "Disarmed (online)", true, |c| if c.session.mode == Mode::Offline { pass() } else { fc_disarmed(c) }),
            g!("applied", "PID settings applied & verified", false, applied(ApplyPhase::Pids)),
        ],
        Step::FlightC => vec![g!("flight_done", "Verification flight completed", false, flight_done(Flight::C))],
        Step::ImportC => vec![
            g!("imported", "Log imported", false, log_imported(Flight::C)),
            g!("ap_pid_rate", "PIDx logged at loop rate", true, Fw::Ap, |c| record(c, Flight::C).map(ap_pid_rate_guard).unwrap_or_else(|| action("Import first."))),
            g!("steps", "Enough stick steps per axis", true, Fw::Bf, |c| record(c, Flight::C).map(steps_guard).unwrap_or_else(|| action("Import first."))),
            g!("ap_steps", "Enough stick steps per axis", true, Fw::Ap, |c| record(c, Flight::C).map(ap_steps_guard).unwrap_or_else(|| action("Import first."))),
            g!("tune_match", "Log flown with the applied PIDs", true, |c| tune_matches(c, Flight::C, Some(ApplyPhase::Pids))),
            g!("autotune_result", "AUTOTUNE changed the rate gains", true, Fw::Ap, autotune_result),
        ],
        Step::Compare => vec![g!("have_logs", "Before and after logs available", false, |c| if c.session.flights.contains_key(&Flight::B) && c.session.flights.contains_key(&Flight::C) { pass() } else { action("Logs B and C are required.") })],
        Step::Report => vec![g!("report", "Report exported", false, report_written)],
    }
}

/// Evaluate every guard of `step`.
pub fn evaluate(step: Step, ctx: &GuardCtx) -> Vec<GuardResult> {
    let fw = session_fw(ctx);
    defs(step)
        .into_iter()
        .filter(|d| match (d.fw, fw) {
            (Fw::Any, _) => true,
            (Fw::Bf, Some(Fw::Bf)) | (Fw::Ap, Some(Fw::Ap)) => true,
            // Firmware not known yet: keep Betaflight guards (the default path) hidden for AP-only ids
            (Fw::Bf, None) => true,
            _ => false,
        })
        .map(|d| GuardResult {
            id: d.id.to_string(),
            title: d.title.to_string(),
            outcome: (d.eval)(ctx),
            can_override: d.can_override,
            overridden: ctx.session.is_overridden(step, d.id),
        })
        .collect()
}

pub fn can_override(step: Step, guard_id: &str) -> bool {
    defs(step).iter().any(|d| d.id == guard_id && d.can_override)
}
