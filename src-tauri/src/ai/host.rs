//! The tools the AI helper can call. Read-only views of the session plus two
//! bounded recommendation tools; nothing here touches the flight controller,
//! the wizard position or the file system.

use crate::AppState;
use domain::*;
use llm::{ToolDef, ToolHost};
use recommend::ap::{ap_bounds, clamp_ap};
use recommend::bf_param_meta::{bf_param_meta, clamp_bf, BfPhase};
use serde::Deserialize;
use serde_json::{json, Value};
use session::{ApplyPhase, Flight, GuardOutcome, Step};
use tauri::{AppHandle, Manager};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    Session,
    Quick,
}

/// Data access the tools need; implemented for the live Tauri state and, in
/// tests, for a plain `SessionEngine`.
pub trait Backend: Send + Sync {
    fn scope(&self) -> Scope;
    fn bundle(&self, f: Flight) -> Result<AnalysisBundle, String>;
    fn record(&self, f: Flight) -> Result<(LogQuality, Vec<String>, Vec<Anomaly>), String>;
    fn firmware(&self) -> Option<Firmware>;
    fn tune(&self, source: Option<&str>) -> Option<Tune>;
    fn recs(&self, phase: ApplyPhase) -> Result<Vec<Recommendation>, String>;
    fn store_recs(&self, phase: ApplyPhase, recs: Vec<Recommendation>) -> Result<(), String>;
    fn overview(&self) -> Result<Value, String>;
    fn guards(&self, step: Option<Step>) -> Result<Value, String>;
    fn fc_status(&self) -> Result<Value, String>;
}

pub struct AiHost<B: Backend> {
    pub backend: B,
}

impl AiHost<TauriBackend> {
    pub fn tauri(app: AppHandle, scope: Scope) -> Self {
        Self {
            backend: TauriBackend { app, scope },
        }
    }
}

pub struct TauriBackend {
    pub app: AppHandle,
    pub scope: Scope,
}

/// Quick-look context (no wizard session): recommendations live here.
#[derive(Default)]
pub struct QuickCtx {
    pub log_id: Option<String>,
    pub recs_filters: Vec<Recommendation>,
    pub recs_pids: Vec<Recommendation>,
    pub transcript: llm::Transcript,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FlightArg {
    flight: Flight,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PeaksArg {
    flight: Flight,
    #[serde(default)]
    max: Option<usize>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PhaseArg {
    phase: ApplyPhase,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GuardsArg {
    #[serde(default)]
    step: Option<Step>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetRecArg {
    phase: ApplyPhase,
    id: uuid::Uuid,
    #[serde(default)]
    accepted: Option<bool>,
    #[serde(default)]
    value: Option<Value>,
    #[serde(default)]
    reason: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AddRecArg {
    phase: ApplyPhase,
    param: String,
    value: Value,
    reason: String,
    #[serde(default)]
    confidence: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NameArg {
    name: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TuneArg {
    #[serde(default)]
    source: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}

fn r2(x: f32) -> Value {
    if x.is_finite() {
        json!((x * 100.0).round() / 100.0)
    } else {
        Value::Null
    }
}
fn r3(x: f64) -> Value {
    if x.is_finite() {
        json!((x * 1000.0).round() / 1000.0)
    } else {
        Value::Null
    }
}

fn flight_schema() -> Value {
    json!({"type": "object", "properties": {"flight": {"type": "string", "enum": ["a", "b", "c"], "description": "a = hover/noise flight, b = step/chirp flight, c = verification flight"}}, "required": ["flight"], "additionalProperties": false})
}

pub fn tool_defs(scope: Scope) -> Vec<ToolDef> {
    let phase = json!({"type": "string", "enum": ["filters", "pids"]});
    let mut v = vec![
        ToolDef::new("get_log_quality", "Measured facts about a flight log: sample rate, duration, hover time, saturation, step segments, chirp windows/coherence, warnings.", flight_schema()),
        ToolDef::new("get_step_response", "Step-response metrics per axis (overshoot, latency, steady state, segments) — no curves.", flight_schema()),
        ToolDef::new("get_frequency_response", "CHIRP frequency-response metrics per axis (bandwidth, phase margin, resonant peak, coherence, targets) — no curves.", flight_schema()),
        ToolDef::new("get_spectrum_peaks", "Strongest noise peaks of the gyro/D-term spectra, sorted by prominence.", json!({"type": "object", "properties": {"flight": {"type": "string", "enum": ["a", "b", "c"]}, "max": {"type": "integer", "minimum": 1, "maximum": 30}}, "required": ["flight"], "additionalProperties": false})),
        ToolDef::new("get_anomalies", "Flight anomalies found in the log (desync, saturation, clipping, oscillation, imbalance, gaps …), critical first.", flight_schema()),
        ToolDef::new("get_recommendations", "Current recommendation list of a phase with ids, old/new values, reasons, accepted flags.", json!({"type": "object", "properties": {"phase": phase}, "required": ["phase"], "additionalProperties": false})),
        ToolDef::new("set_recommendation", "Change one existing recommendation: accept/unaccept, set a new value (clamped to the parameter bounds) and/or append a reason. Proposal only — the human still writes to the FC.", json!({"type": "object", "properties": {"phase": phase, "id": {"type": "string", "description": "recommendation uuid"}, "accepted": {"type": "boolean"}, "value": {"type": ["number", "boolean"]}, "reason": {"type": "string"}}, "required": ["phase", "id"], "additionalProperties": false})),
        ToolDef::new("add_recommendation", "Propose a new parameter change (unaccepted until the human ticks it). Rejected when the parameter is unknown for this firmware, belongs to the other phase, or already has a recommendation.", json!({"type": "object", "properties": {"phase": phase, "param": {"type": "string", "description": "exact CLI / parameter name"}, "value": {"type": ["number", "boolean"]}, "reason": {"type": "string"}, "confidence": {"type": "string", "enum": ["low", "medium", "high"]}}, "required": ["phase", "param", "value", "reason"], "additionalProperties": false})),
        ToolDef::new("get_tune", "Current tune (PIDs, filters, simplified sliders for Betaflight; ATC_RAT_*, INS_* for ArduCopter).", json!({"type": "object", "properties": {"source": {"type": "string", "enum": ["fc", "a", "b", "c"], "description": "fc = live flight controller (online), else the tune logged in that flight"}}, "additionalProperties": false})),
        ToolDef::new("get_param_bounds", "Allowed range/type of a parameter.", json!({"type": "object", "properties": {"name": {"type": "string"}}, "required": ["name"], "additionalProperties": false})),
    ];
    if scope == Scope::Session {
        v.extend([
            ToolDef::new("get_session_overview", "Session name, mode, firmware, current step, per-step status, imported flights, applies, PID strategy, notes.", json!({"type": "object", "properties": {}, "additionalProperties": false})),
            ToolDef::new("get_guards", "Guard results of a wizard step (default: the current step): id, title, pass/fail/needs_action, message, hint, overridable, overridden.", json!({"type": "object", "properties": {"step": {"type": "string"}}, "additionalProperties": false})),
            ToolDef::new("get_flight_protocol", "The flight procedure the pilot must fly for a flight.", flight_schema()),
            ToolDef::new("get_fc_status", "Live flight-controller status (connection, armed, firmware, logging configuration).", json!({"type": "object", "properties": {}, "additionalProperties": false})),
            ToolDef::new("get_compare", "Before/after (Flight B vs C) step-response and frequency-response metrics.", json!({"type": "object", "properties": {}, "additionalProperties": false})),
        ]);
    }
    v
}

fn quality_json(q: &LogQuality, warnings: &[String]) -> Value {
    json!({
        "fs_hz": q.fs_hz, "duration_s": r3(q.duration_s), "has_gyro_raw": q.has_gyro_raw, "has_pid_terms": q.has_pid_terms,
        "hover_seconds": r3(q.hover_seconds), "hover_throttle_pct": r3(q.hover_throttle_pct), "airborne_range_s": q.airborne_range_s,
        "motor_saturation_pct": r3(q.motor_saturation_pct), "max_setpoint_dps": q.max_setpoint_per_axis.map(r2), "step_segments": q.step_segments_per_axis,
        "gap_seconds": r3(q.gap_seconds), "pid_rate_hz": q.pid_rate_hz, "max_pid_out": q.max_pid_out, "gyro_hr_batches": q.gyro_hr_batches,
        "chirp_sweeps": q.chirp_sweeps_per_axis, "chirp_windows": q.chirp_windows_per_axis, "chirp_coherence": q.chirp_coherence_per_axis.map(r2),
        "warnings": warnings,
    })
}

fn steps_json(b: &AnalysisBundle) -> Value {
    Value::Array(b.steps.iter().map(|s| json!({"axis": s.axis.name(), "segments": s.n_segments, "rejected": s.rejected, "overshoot": r2(s.overshoot), "latency_ms": r2(s.latency_ms), "settle_ms": s.settle_ms.map(r2), "steady_state": r2(s.steady_state)})).collect())
}

fn freq_json(b: &AnalysisBundle) -> Value {
    Value::Array(
        b.freq_resp
            .iter()
            .map(|f| {
                let m = &f.metrics;
                json!({"axis": f.axis.name(), "angle_mode": f.angle_mode, "sweeps": f.n_sweeps, "windows": f.n_windows, "coherence_mean": r2(m.coherence_mean), "bandwidth_hz": r2(m.bandwidth_hz), "crossover_hz": r2(m.crossover_hz), "phase_margin_deg": r2(m.phase_margin_deg), "max_phase_margin_deg": r2(m.max_phase_margin_deg), "resonant_peak_db": r2(m.resonant_peak_db), "resonant_peak_hz": r2(m.resonant_peak_hz), "loop_delay_ms": r2(m.loop_delay_ms), "low_freq_err_db": r2(m.low_freq_err_db), "sens_peak_db": r2(m.sens_peak_db), "step_overshoot": r2(m.step_overshoot), "step_rise_ms": r2(m.step_rise_ms),
                    "targets": m.targets.iter().map(|t| json!({"pm_deg": t.pm_deg, "crossover_hz": r2(t.crossover_hz), "gain_to_target": r2(t.gain_to_target), "gain_for_sens_limit": r2(t.gain_for_sens_limit)})).collect::<Vec<_>>()})
            })
            .collect(),
    )
}

fn recs_json(recs: &[Recommendation]) -> Value {
    Value::Array(
        recs.iter()
            .map(|r| json!({"id": r.id, "param": r.param.name(), "old": r.old.to_string(), "new": r.new.to_string(), "reason": r.reason, "confidence": r.confidence, "requires_reboot": r.requires_reboot, "accepted": r.accepted, "evidence": r.evidence}))
            .collect(),
    )
}

fn anomalies_json(a: &[Anomaly]) -> Value {
    Value::Array(a.iter().take(30).map(|x| json!({"kind": x.kind.title(), "severity": x.severity, "t_start_s": r2(x.t_start_s), "t_end_s": r2(x.t_end_s), "axis": x.axis.map(|a| a.name()), "motor": x.motor.map(|m| m + 1), "value": r2(x.value), "detail": x.detail})).collect())
}

fn peaks_json(b: &AnalysisBundle, max: usize) -> Value {
    let mut p: Vec<&NoisePeak> = b.peaks.iter().collect();
    p.sort_by(|a, c| {
        c.prominence_db
            .partial_cmp(&a.prominence_db)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Value::Array(p.iter().take(max).map(|x| json!({"axis": x.axis.name(), "kind": x.kind, "f_hz": r2(x.f_hz), "psd_db": r2(x.psd_db), "prominence_db": r2(x.prominence_db), "band": x.band})).collect())
}

fn tune_json(t: &Tune) -> Value {
    match t {
        Tune::Bf(b) => json!({
            "firmware": "betaflight",
            "pids": Axis::ALL.iter().map(|a| { let p = b.pids[a.index()]; json!({"axis": a.name(), "p": p.p, "i": p.i, "d": p.d, "d_max": p.d_max, "ff": p.ff}) }).collect::<Vec<_>>(),
            "d_naming": if b.legacy_d_naming() { "d_* = D max, d_min_* = D (BF <= 4.5)" } else { "d_* = D, d_max_* = D max" },
            "filters": b.filters, "simplified": b.simplified,
            "d_max_gain": b.d_max_gain, "d_max_advance": b.d_max_advance, "anti_gravity_gain": b.anti_gravity_gain,
            "feedforward_smooth_factor": b.feedforward_smooth_factor, "feedforward_jitter_factor": b.feedforward_jitter_factor, "feedforward_boost": b.feedforward_boost,
            "tpa_rate": b.tpa_rate, "tpa_breakpoint": b.tpa_breakpoint,
        }),
        Tune::Ap(a) => {
            let keep: serde_json::Map<String, Value> = a
                .params
                .iter()
                .filter(|(k, _)| {
                    k.starts_with("ATC_RAT_")
                        || k.starts_with("ATC_ANG_")
                        || k.starts_with("ATC_INPUT")
                        || k.starts_with("INS_GYRO_FILTER")
                        || k.starts_with("INS_HNTCH")
                        || k.starts_with("INS_HNTC2")
                        || k.starts_with("AUTOTUNE_")
                        || k.starts_with("MOT_THST")
                        || k.starts_with("INS_LOG_BAT")
                        || k.starts_with("SCHED_LOOP")
                        || *k == "LOG_BITMASK"
                })
                .map(|(k, v)| (k.clone(), json!(v)))
                .collect();
            json!({"firmware": "ardupilot", "params": keep})
        }
        Tune::Unknown => json!({"firmware": "unknown"}),
    }
}

fn ap_phase(name: &str) -> BfPhase {
    if name.starts_with("INS_") || name.contains("_FLT") || name.starts_with("FILT") {
        BfPhase::Filters
    } else {
        BfPhase::Pids
    }
}

fn old_value(tune: Option<&Tune>, name: &str) -> Option<ParamValue> {
    match tune? {
        Tune::Bf(b) => {
            if let Some(v) = b.get_raw(name) {
                if let Ok(f) = v.trim().parse::<f64>() {
                    return clamp_bf(name, f);
                }
                return match v.trim().to_ascii_uppercase().as_str() {
                    "ON" => Some(ParamValue::Bool(true)),
                    "OFF" => Some(ParamValue::Bool(false)),
                    _ => None,
                };
            }
            let (k, base) = ["roll", "pitch", "yaw"]
                .iter()
                .enumerate()
                .find_map(|(k, ax)| name.strip_suffix(&format!("_{ax}")).map(|b| (k, b)))?;
            let p = b.pids[k];
            let v = match base {
                "p" => p.p as f64,
                "i" => p.i as f64,
                "f" => p.ff as f64,
                "d" => {
                    if b.legacy_d_naming() {
                        p.d_max as f64
                    } else {
                        p.d as f64
                    }
                }
                "d_min" => p.d as f64,
                "d_max" => p.d_max as f64,
                _ => return None,
            };
            clamp_bf(name, v)
        }
        Tune::Ap(a) => a.get(name).map(ParamValue::F32),
        Tune::Unknown => None,
    }
}

impl TauriBackend {
    fn state(&self) -> tauri::State<'_, AppState> {
        self.app.state::<AppState>()
    }
}

impl Backend for TauriBackend {
    fn scope(&self) -> Scope {
        self.scope
    }

    fn bundle(&self, f: Flight) -> Result<AnalysisBundle, String> {
        let st = self.state();
        match self.scope {
            Scope::Session => {
                let g = st.engine.lock().unwrap();
                let e = g.as_ref().ok_or("no session open")?;
                e.bundle(f)
                    .map_err(|e| e.to_string())?
                    .ok_or_else(|| format!("flight {f:?} has not been imported yet"))
            }
            Scope::Quick => {
                let q = st.quick.lock().unwrap();
                let id = q
                    .as_ref()
                    .and_then(|q| q.log_id.clone())
                    .ok_or("no log open in Quick look")?;
                st.bundles
                    .lock()
                    .unwrap()
                    .get(&id)
                    .map(|b| (**b).clone())
                    .ok_or("analysis not cached; open the log again".into())
            }
        }
    }

    fn record(&self, f: Flight) -> Result<(LogQuality, Vec<String>, Vec<Anomaly>), String> {
        let st = self.state();
        match self.scope {
            Scope::Session => {
                let g = st.engine.lock().unwrap();
                let e = g.as_ref().ok_or("no session open")?;
                let r = e
                    .session
                    .flights
                    .get(&f)
                    .ok_or_else(|| format!("flight {f:?} has not been imported yet"))?;
                Ok((r.quality.clone(), r.warnings.clone(), r.anomalies.clone()))
            }
            Scope::Quick => {
                let b = self.bundle(f)?;
                let warnings = {
                    let q = st.quick.lock().unwrap();
                    let id = q
                        .as_ref()
                        .and_then(|q| q.log_id.clone())
                        .unwrap_or_default();
                    st.logs
                        .lock()
                        .unwrap()
                        .get(&id)
                        .map(|l| l.meta.warnings.clone())
                        .unwrap_or_default()
                };
                Ok((b.quality.clone(), warnings, b.anomalies.clone()))
            }
        }
    }

    fn firmware(&self) -> Option<Firmware> {
        let st = self.state();
        match self.scope {
            Scope::Session => st
                .engine
                .lock()
                .unwrap()
                .as_ref()
                .and_then(|e| e.session.firmware.clone())
                .or_else(|| st.fc.lock().unwrap().firmware.clone()),
            Scope::Quick => {
                let q = st.quick.lock().unwrap();
                let id = q
                    .as_ref()
                    .and_then(|q| q.log_id.clone())
                    .unwrap_or_default();
                st.logs.lock().unwrap().get(&id).map(|l| l.firmware.clone())
            }
        }
    }

    fn tune(&self, source: Option<&str>) -> Option<Tune> {
        let st = self.state();
        match self.scope {
            Scope::Session => {
                let g = st.engine.lock().unwrap();
                let e = g.as_ref()?;
                match source {
                    Some("fc") => e
                        .session
                        .fc_tune
                        .clone()
                        .or_else(|| st.fc.lock().unwrap().tune.clone()),
                    Some(f) => {
                        let fl = match f {
                            "a" => Flight::A,
                            "b" => Flight::B,
                            _ => Flight::C,
                        };
                        e.session.flights.get(&fl).map(|r| r.tune.clone())
                    }
                    None => e.session.fc_tune.clone().or_else(|| {
                        [Flight::C, Flight::B, Flight::A]
                            .iter()
                            .find_map(|f| e.session.flights.get(f).map(|r| r.tune.clone()))
                    }),
                }
            }
            Scope::Quick => {
                let q = st.quick.lock().unwrap();
                let id = q
                    .as_ref()
                    .and_then(|q| q.log_id.clone())
                    .unwrap_or_default();
                st.logs
                    .lock()
                    .unwrap()
                    .get(&id)
                    .map(|l| l.tune_at_log.clone())
            }
        }
    }

    fn recs(&self, phase: ApplyPhase) -> Result<Vec<Recommendation>, String> {
        let st = self.state();
        match self.scope {
            Scope::Session => Ok(st
                .engine
                .lock()
                .unwrap()
                .as_ref()
                .ok_or("no session open")?
                .session
                .recs(phase)
                .clone()),
            Scope::Quick => {
                let q = st.quick.lock().unwrap();
                let q = q.as_ref().ok_or("no log open in Quick look")?;
                Ok(match phase {
                    ApplyPhase::Filters => q.recs_filters.clone(),
                    ApplyPhase::Pids => q.recs_pids.clone(),
                })
            }
        }
    }

    fn store_recs(&self, phase: ApplyPhase, recs: Vec<Recommendation>) -> Result<(), String> {
        let st = self.state();
        match self.scope {
            Scope::Session => st
                .engine
                .lock()
                .unwrap()
                .as_mut()
                .ok_or("no session open")?
                .set_recs(phase, recs)
                .map_err(|e| e.to_string()),
            Scope::Quick => {
                let mut q = st.quick.lock().unwrap();
                let q = q.as_mut().ok_or("no log open in Quick look")?;
                match phase {
                    ApplyPhase::Filters => q.recs_filters = recs,
                    ApplyPhase::Pids => q.recs_pids = recs,
                }
                Ok(())
            }
        }
    }

    fn overview(&self) -> Result<Value, String> {
        let st = self.state();
        let g = st.engine.lock().unwrap();
        let e = g.as_ref().ok_or("no session open")?;
        Ok(overview_json(&e.session))
    }

    fn guards(&self, step: Option<Step>) -> Result<Value, String> {
        let st = self.state();
        let fc = st.fc.lock().unwrap().clone();
        let g = st.engine.lock().unwrap();
        let e = g.as_ref().ok_or("no session open")?;
        let step = step.unwrap_or(e.session.current);
        Ok(guards_json(
            step,
            &e.guards(step, if fc.connected { Some(&fc) } else { None }),
        ))
    }

    fn fc_status(&self) -> Result<Value, String> {
        let mut fc = self.state().fc.lock().unwrap().clone();
        fc.tune = None;
        Ok(serde_json::to_value(fc).unwrap())
    }
}

pub fn overview_json(s: &session::Session) -> Value {
    json!({
        "name": s.name, "mode": s.mode, "firmware": s.firmware, "current_step": s.current, "current_step_title": s.current.title(),
        "steps": Step::ALL.iter().map(|st| json!({"step": st, "title": st.title(), "status": s.status.get(st)})).collect::<Vec<_>>(),
        "flights": s.flights.iter().map(|(f, r)| json!({"flight": f, "fs_hz": r.fs_hz, "duration_s": r3(r.duration_s), "imported_at": r.imported_at, "craft": r.craft_name, "n_anomalies": r.anomalies.len(), "critical_anomalies": r.anomalies.iter().filter(|a| a.severity == Severity::Critical).count()})).collect::<Vec<_>>(),
        "applies": s.applies.iter().map(|a| json!({"phase": a.phase, "at": a.at, "verified": a.verified, "method": a.method, "n": a.applied.len()})).collect::<Vec<_>>(),
        "pid_strategy": s.pid_strategy, "pid_source": s.pid_source, "notes": s.notes, "overrides": s.overrides.iter().map(|o| json!({"step": o.step, "guard": o.guard_id, "reason": o.reason})).collect::<Vec<_>>(),
    })
}

pub fn guards_json(step: Step, gs: &[session::GuardResult]) -> Value {
    json!({"step": step, "title": step.title(), "guards": gs.iter().map(|g| {
        let (outcome, message, hint) = match &g.outcome { GuardOutcome::Pass => ("pass", String::new(), None), GuardOutcome::Fail { message, fix_hint } => ("fail", message.clone(), fix_hint.clone()), GuardOutcome::NeedsAction { message } => ("needs_action", message.clone(), None) };
        json!({"id": g.id, "title": g.title, "outcome": outcome, "message": message, "fix_hint": hint, "can_override": g.can_override, "overridden": g.overridden})
    }).collect::<Vec<_>>()})
}

impl<B: Backend> AiHost<B> {
    fn bundle(&self, f: Flight) -> Result<AnalysisBundle, String> {
        self.backend.bundle(f)
    }
    fn record(&self, f: Flight) -> Result<(LogQuality, Vec<String>, Vec<Anomaly>), String> {
        self.backend.record(f)
    }
    fn firmware(&self) -> Option<Firmware> {
        self.backend.firmware()
    }
    fn tune(&self, source: Option<&str>) -> Option<Tune> {
        self.backend.tune(source)
    }
    fn recs(&self, phase: ApplyPhase) -> Result<Vec<Recommendation>, String> {
        self.backend.recs(phase)
    }
    fn store_recs(&self, phase: ApplyPhase, recs: Vec<Recommendation>) -> Result<(), String> {
        self.backend.store_recs(phase, recs)
    }

    /// Typed + clamped value for `name` under the session firmware.
    fn typed_value(
        &self,
        fw: &Firmware,
        name: &str,
        v: &Value,
        like: Option<&ParamValue>,
    ) -> Result<(ParamValue, bool), String> {
        let num = match v {
            Value::Bool(b) => {
                if let Some(ParamValue::Bool(_)) | None = like {
                    if matches!(fw, Firmware::Betaflight { .. })
                        && matches!(
                            bf_param_meta(name).map(|m| m.kind),
                            Some(recommend::bf_param_meta::BfKind::Bool)
                        )
                    {
                        return Ok((ParamValue::Bool(*b), false));
                    }
                }
                return Err(format!("{name} is numeric; send a number"));
            }
            Value::Number(n) => n.as_f64().ok_or("bad number")?,
            _ => return Err("value must be a number or boolean".into()),
        };
        if like.is_some_and(|l| matches!(l, ParamValue::Bool(_))) {
            return Err(format!("{name} is a boolean; send true/false"));
        }
        match fw {
            Firmware::Betaflight { .. } => {
                let pv = clamp_bf(name, num)
                    .ok_or_else(|| format!("{name} is not a parameter this app may change"))?;
                Ok((pv, (pv.as_f64() - num).abs() > 1e-9))
            }
            Firmware::ArduCopter { .. } => {
                let c = clamp_ap(name, num as f32);
                Ok((ParamValue::F32(c), (c as f64 - num).abs() > 1e-6))
            }
            Firmware::Unknown { .. } => {
                Err("firmware unknown; import a log or connect the FC first".into())
            }
        }
    }

    fn set_rec(&self, a: SetRecArg) -> Result<Value, String> {
        let mut recs = self.recs(a.phase)?;
        let Some(r) = recs.iter_mut().find(|r| r.id == a.id) else {
            return Ok(json!({"ok": false, "message": "unknown recommendation id"}));
        };
        let fw = self.firmware().ok_or("firmware unknown")?;
        let mut clamped = false;
        if let Some(v) = &a.value {
            let (pv, c) = self.typed_value(&fw, r.param.name(), v, Some(&r.new))?;
            r.new = pv;
            clamped = c;
        }
        if let Some(acc) = a.accepted {
            r.accepted = acc;
        }
        if let Some(reason) = a.reason.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            r.reason = format!("{} [AI: {}]", r.reason, reason);
        }
        let out = json!({"ok": true, "id": r.id, "param": r.param.name(), "applied": {"accepted": r.accepted, "new": r.new.to_string()}, "clamped": clamped, "message": "updated (proposal only; the human applies it)"});
        self.store_recs(a.phase, recs)?;
        Ok(out)
    }

    fn add_rec(&self, a: AddRecArg) -> Result<Value, String> {
        let fw = self
            .firmware()
            .ok_or("firmware unknown; import a log or connect the FC first")?;
        let name = a.param.trim().to_string();
        // phase gate
        let phase_ok = match &fw {
            Firmware::Betaflight { .. } => {
                let m = bf_param_meta(&name).ok_or_else(|| {
                    format!("{name} is not a Betaflight parameter this app may change")
                })?;
                (m.phase == BfPhase::Filters) == (a.phase == ApplyPhase::Filters)
            }
            Firmware::ArduCopter { .. } => {
                if ap_bounds(&name).is_none() {
                    return Err(format!(
                        "{name} is not an ArduPilot parameter this app may change"
                    ));
                }
                (ap_phase(&name) == BfPhase::Filters) == (a.phase == ApplyPhase::Filters)
            }
            Firmware::Unknown { .. } => return Err("firmware unknown".into()),
        };
        if !phase_ok {
            return Ok(
                json!({"ok": false, "message": format!("{name} belongs to the other phase")}),
            );
        }
        let mut recs = self.recs(a.phase)?;
        if let Some(existing) = recs.iter().find(|r| r.param.name() == name) {
            return Ok(
                json!({"ok": false, "existing_id": existing.id, "message": "a recommendation for this parameter exists; use set_recommendation"}),
            );
        }
        let (new, clamped) = self.typed_value(&fw, &name, &a.value, None)?;
        let old = old_value(self.tune(None).as_ref(), &name).unwrap_or(new);
        let confidence = match a.confidence.as_deref() {
            Some("high") => Confidence::High,
            Some("medium") => Confidence::Medium,
            _ => Confidence::Low,
        };
        let requires_reboot = match &fw {
            Firmware::ArduCopter { .. } => {
                ap_bounds(&name).map(|m| m.reboot_required).unwrap_or(false)
                    || (name.starts_with("INS_HNTC") && name.ends_with("_ENABLE"))
            }
            _ => false,
        };
        let param = match &fw {
            Firmware::ArduCopter { .. } => ParamRef::Ap(name.clone()),
            _ => ParamRef::Bf(name.clone()),
        };
        let rec = Recommendation {
            id: uuid::Uuid::new_v4(),
            param,
            old,
            new,
            reason: a.reason.trim().to_string(),
            evidence: vec![EvidenceRef::Text {
                note: format!("AI proposal: {}", a.reason.trim()),
            }],
            confidence,
            requires_reboot,
            accepted: false,
        };
        let out = json!({"ok": true, "id": rec.id, "param": name, "old": old.to_string(), "new": new.to_string(), "clamped": clamped, "accepted": false, "message": "added as an unaccepted proposal; the human ticks it in the recommendations table"});
        recs.push(rec);
        self.store_recs(a.phase, recs)?;
        Ok(out)
    }

    fn compare(&self) -> Result<Value, String> {
        let b = self.bundle(Flight::B)?;
        let c = self.bundle(Flight::C)?;
        Ok(
            json!({"before_steps": steps_json(&b), "after_steps": steps_json(&c), "before_freq": freq_json(&b), "after_freq": freq_json(&c)}),
        )
    }
}

fn parse<T: for<'de> Deserialize<'de>>(args: Value) -> Result<T, String> {
    serde_json::from_value(args).map_err(|e| format!("invalid arguments: {e}"))
}

#[async_trait::async_trait]
impl<B: Backend> ToolHost for AiHost<B> {
    fn defs(&self) -> Vec<ToolDef> {
        tool_defs(self.backend.scope())
    }

    async fn call(&self, name: &str, args: Value) -> Result<String, String> {
        let v: Value = match name {
            "get_log_quality" => {
                let a: FlightArg = parse(args)?;
                let (q, w, _) = self.record(a.flight)?;
                quality_json(&q, &w)
            }
            "get_step_response" => steps_json(&self.bundle(parse::<FlightArg>(args)?.flight)?),
            "get_frequency_response" => {
                let b = self.bundle(parse::<FlightArg>(args)?.flight)?;
                if b.freq_resp.is_empty() {
                    json!({"note": "no CHIRP sweeps in this log"})
                } else {
                    freq_json(&b)
                }
            }
            "get_spectrum_peaks" => {
                let a: PeaksArg = parse(args)?;
                peaks_json(&self.bundle(a.flight)?, a.max.unwrap_or(12).clamp(1, 30))
            }
            "get_anomalies" => {
                let (_, _, an) = self.record(parse::<FlightArg>(args)?.flight)?;
                anomalies_json(&an)
            }
            "get_recommendations" => recs_json(&self.recs(parse::<PhaseArg>(args)?.phase)?),
            "set_recommendation" => self.set_rec(parse(args)?)?,
            "add_recommendation" => self.add_rec(parse(args)?)?,
            "get_tune" => {
                let a: TuneArg = parse(args)?;
                self.tune(a.source.as_deref())
                    .map(|t| tune_json(&t))
                    .unwrap_or(json!({"note": "no tune available"}))
            }
            "get_param_bounds" => {
                let a: NameArg = parse(args)?;
                match self.firmware() {
                    Some(Firmware::ArduCopter { .. }) => match ap_bounds(&a.name) {
                        Some(m) => {
                            json!({"name": m.name, "min": m.min, "max": m.max, "increment": m.increment, "requires_reboot": m.reboot_required, "units": m.units, "integer": m.integer})
                        }
                        None => json!({"error": "unknown parameter"}),
                    },
                    _ => match bf_param_meta(&a.name) {
                        Some(m) => {
                            json!({"name": a.name, "min": m.min, "max": m.max, "type": format!("{:?}", m.kind), "phase": format!("{:?}", m.phase), "requires_reboot": false})
                        }
                        None => json!({"error": "unknown parameter or not changeable by this app"}),
                    },
                }
            }
            "get_session_overview" => {
                let _: Empty = parse(args)?;
                self.backend.overview()?
            }
            "get_guards" => self.backend.guards(parse::<GuardsArg>(args)?.step)?,
            "get_flight_protocol" => serde_json::to_value(session::protocol::text(
                self.firmware().as_ref(),
                parse::<FlightArg>(args)?.flight,
            ))
            .unwrap(),
            "get_fc_status" => {
                let _: Empty = parse(args)?;
                self.backend.fc_status()?
            }
            "get_compare" => {
                let _: Empty = parse(args)?;
                self.compare()?
            }
            other => return Err(format!("unknown tool `{other}`")),
        };
        Ok(v.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use session::{Mode, SessionEngine, SessionStore};
    use std::sync::Mutex;

    /// Backend over a plain engine (no Tauri).
    struct EngineBackend {
        engine: Mutex<SessionEngine>,
        bundles: Mutex<std::collections::HashMap<Flight, AnalysisBundle>>,
    }

    impl Backend for EngineBackend {
        fn scope(&self) -> Scope {
            Scope::Session
        }
        fn bundle(&self, f: Flight) -> Result<AnalysisBundle, String> {
            self.bundles
                .lock()
                .unwrap()
                .get(&f)
                .cloned()
                .ok_or_else(|| format!("flight {f:?} has not been imported yet"))
        }
        fn record(&self, f: Flight) -> Result<(LogQuality, Vec<String>, Vec<Anomaly>), String> {
            let e = self.engine.lock().unwrap();
            let r = e.session.flights.get(&f).ok_or("not imported")?;
            Ok((r.quality.clone(), r.warnings.clone(), r.anomalies.clone()))
        }
        fn firmware(&self) -> Option<Firmware> {
            self.engine.lock().unwrap().session.firmware.clone()
        }
        fn tune(&self, _source: Option<&str>) -> Option<Tune> {
            self.engine.lock().unwrap().session.fc_tune.clone()
        }
        fn recs(&self, phase: ApplyPhase) -> Result<Vec<Recommendation>, String> {
            Ok(self.engine.lock().unwrap().session.recs(phase).clone())
        }
        fn store_recs(&self, phase: ApplyPhase, recs: Vec<Recommendation>) -> Result<(), String> {
            self.engine
                .lock()
                .unwrap()
                .set_recs(phase, recs)
                .map_err(|e| e.to_string())
        }
        fn overview(&self) -> Result<Value, String> {
            Ok(overview_json(&self.engine.lock().unwrap().session))
        }
        fn guards(&self, step: Option<Step>) -> Result<Value, String> {
            let e = self.engine.lock().unwrap();
            let step = step.unwrap_or(e.session.current);
            Ok(guards_json(step, &e.guards(step, None)))
        }
        fn fc_status(&self) -> Result<Value, String> {
            Ok(json!({"connected": false}))
        }
    }

    fn rec(name: &str, old: ParamValue, new: ParamValue) -> Recommendation {
        Recommendation {
            id: uuid::Uuid::new_v4(),
            param: ParamRef::Bf(name.into()),
            old,
            new,
            reason: "rule".into(),
            evidence: vec![],
            confidence: Confidence::Medium,
            requires_reboot: false,
            accepted: true,
        }
    }

    fn host(fw: Firmware) -> (tempfile::TempDir, AiHost<EngineBackend>) {
        let d = tempfile::tempdir().unwrap();
        let mut e =
            SessionEngine::create(SessionStore::new(d.path()), "t".into(), Mode::Offline).unwrap();
        let mut tune = BfTune::default();
        tune.pids[0].p = 45;
        tune.pids[0].d = 30;
        tune.pids[1].d = 27;
        tune.filters.gyro_lpf1_static_hz = 250;
        tune.raw.insert("dyn_notch_q".into(), "300".into());
        e.set_fc_tune(Some(Tune::Bf(tune)), Some(fw)).unwrap();
        e.set_recs(
            ApplyPhase::Pids,
            vec![rec("d_roll", ParamValue::U8(30), ParamValue::U8(40))],
        )
        .unwrap();
        let b = AnalysisBundle {
            log: LogId("x".into()),
            quality: LogQuality::default(),
            steps: vec![StepResponse {
                axis: Axis::Roll,
                variant: StepVariant::PtStep,
                t_ms: vec![0.0; 1000],
                mean: vec![1.0; 1000],
                p10: vec![],
                p90: vec![],
                n_segments: 40,
                rejected: 1,
                overshoot: 1.1,
                latency_ms: 12.0,
                settle_ms: None,
                steady_state: 1.0,
            }],
            spectra: vec![],
            spectrograms: vec![],
            peaks: (0..20)
                .map(|i| NoisePeak {
                    axis: Axis::Roll,
                    kind: SpectrumKind::GyroRaw,
                    f_hz: 100.0 + i as f32 * 10.0,
                    psd_db: 0.0,
                    prominence_db: i as f32,
                    band: NoiseBand::Frame,
                })
                .collect(),
            anomalies: vec![],
            freq_resp: vec![],
        };
        let mut bundles = std::collections::HashMap::new();
        bundles.insert(Flight::A, b);
        (
            d,
            AiHost {
                backend: EngineBackend {
                    engine: Mutex::new(e),
                    bundles: Mutex::new(bundles),
                },
            },
        )
    }

    fn bf() -> Firmware {
        Firmware::Betaflight {
            version: "4.5.1".into(),
            api: (1, 46),
        }
    }

    async fn call(h: &AiHost<EngineBackend>, name: &str, args: Value) -> Value {
        serde_json::from_str(&h.call(name, args).await.unwrap()).unwrap()
    }

    #[tokio::test]
    async fn set_recommendation_toggles_sets_and_clamps() {
        let (_d, h) = host(bf());
        let id = h.recs(ApplyPhase::Pids).unwrap()[0].id;
        let r = call(&h, "set_recommendation", json!({"phase": "pids", "id": id, "accepted": false, "value": 999, "reason": "too much"})).await;
        assert_eq!(r["ok"], true);
        assert_eq!(r["clamped"], true);
        assert_eq!(r["applied"]["new"], "250");
        let recs = h.recs(ApplyPhase::Pids).unwrap();
        assert!(!recs[0].accepted);
        assert_eq!(recs[0].new, ParamValue::U8(250));
        assert!(recs[0].reason.contains("[AI: too much]"));
        let r = call(
            &h,
            "set_recommendation",
            json!({"phase": "pids", "id": uuid::Uuid::new_v4(), "accepted": true}),
        )
        .await;
        assert_eq!(r["ok"], false);
        let e = h
            .call(
                "set_recommendation",
                json!({"phase": "pids", "id": id, "value": true}),
            )
            .await
            .unwrap_err();
        assert!(e.contains("numeric"), "{e}");
        let e = h
            .call(
                "set_recommendation",
                json!({"phase": "pids", "id": id, "bogus": 1}),
            )
            .await
            .unwrap_err();
        assert!(e.contains("invalid arguments"));
    }

    #[tokio::test]
    async fn add_recommendation_validates_param_phase_duplicates_and_bounds() {
        let (_d, h) = host(bf());
        assert!(h
            .call(
                "add_recommendation",
                json!({"phase": "pids", "param": "nope_param", "value": 1, "reason": "x"})
            )
            .await
            .is_err());
        assert!(h
            .call(
                "add_recommendation",
                json!({"phase": "pids", "param": "ATC_RAT_RLL_P", "value": 0.1, "reason": "x"})
            )
            .await
            .is_err());
        let r = call(
            &h,
            "add_recommendation",
            json!({"phase": "filters", "param": "p_roll", "value": 50, "reason": "x"}),
        )
        .await;
        assert_eq!(r["ok"], false, "wrong phase: {r}");
        let r = call(
            &h,
            "add_recommendation",
            json!({"phase": "pids", "param": "d_roll", "value": 50, "reason": "x"}),
        )
        .await;
        assert_eq!(r["ok"], false);
        assert!(r["existing_id"].is_string(), "{r}");
        let r = call(&h, "add_recommendation", json!({"phase": "pids", "param": "p_roll", "value": 999, "reason": "raise P", "confidence": "high"})).await;
        assert_eq!(r["ok"], true);
        assert_eq!(r["clamped"], true);
        assert_eq!(r["old"], "45");
        assert_eq!(r["new"], "250");
        let recs = h.recs(ApplyPhase::Pids).unwrap();
        assert_eq!(recs.len(), 2);
        let added = recs.iter().find(|r| r.param.name() == "p_roll").unwrap();
        assert!(!added.accepted);
        assert_eq!(added.confidence, Confidence::High);
        assert!(
            matches!(&added.evidence[0], EvidenceRef::Text { note } if note.starts_with("AI proposal"))
        );
        // pitch axis resolves (not only roll)
        let r = call(
            &h,
            "add_recommendation",
            json!({"phase": "pids", "param": "d_pitch", "value": 33, "reason": "x"}),
        )
        .await;
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(r["old"], "27");
        // raw-header value is used for old
        let r = call(
            &h,
            "add_recommendation",
            json!({"phase": "filters", "param": "dyn_notch_q", "value": 400, "reason": "x"}),
        )
        .await;
        assert_eq!(r["old"], "300");
        assert_eq!(r["new"], "400");
    }

    #[tokio::test]
    async fn ardupilot_uses_pdef_bounds() {
        let (_d, h) = host(Firmware::ArduCopter {
            version: "4.5.5".into(),
        });
        let r = call(
            &h,
            "add_recommendation",
            json!({"phase": "pids", "param": "ATC_RAT_PIT_D", "value": 0.5, "reason": "x"}),
        )
        .await;
        assert_eq!(r["ok"], true);
        assert_eq!(r["clamped"], true);
        assert_eq!(r["new"], "0.03");
        let r = call(
            &h,
            "add_recommendation",
            json!({"phase": "pids", "param": "INS_GYRO_FILTER", "value": 40, "reason": "x"}),
        )
        .await;
        assert_eq!(r["ok"], false, "{r}");
        let r = call(&h, "get_param_bounds", json!({"name": "INS_HNTCH_ENABLE"})).await;
        assert_eq!(r["integer"], true);
        let r = call(&h, "get_param_bounds", json!({"name": "FFT_ENABLE"})).await;
        assert_eq!(r["requires_reboot"], true);
    }

    #[tokio::test]
    async fn read_tools_are_compact_and_default_to_current_step() {
        let (_d, h) = host(bf());
        let r = h
            .call("get_step_response", json!({"flight": "a"}))
            .await
            .unwrap();
        assert!(r.len() < 2000 && !r.contains("\"mean\""), "{r}");
        let p = call(&h, "get_spectrum_peaks", json!({"flight": "a", "max": 3})).await;
        assert_eq!(p.as_array().unwrap().len(), 3);
        assert_eq!(p[0]["prominence_db"], 19.0);
        let o = call(&h, "get_session_overview", json!({})).await;
        let g = call(&h, "get_guards", json!({})).await;
        assert_eq!(g["step"], o["current_step"]);
        assert!(!g["guards"].as_array().unwrap().is_empty());
        let g = call(&h, "get_guards", json!({"step": "connect"})).await;
        assert!(g["guards"].as_array().unwrap().len() >= 2, "{g}");
        assert_eq!(o["mode"], "offline");
        assert!(h
            .call("get_step_response", json!({"flight": "b"}))
            .await
            .is_err());
        assert!(h
            .call("get_step_response", json!({"flight": "z"}))
            .await
            .is_err());
        assert!(h.call("nope", json!({})).await.is_err());
        let t = call(&h, "get_tune", json!({})).await;
        assert_eq!(t["firmware"], "betaflight");
        assert_eq!(t["pids"][0]["p"], 45);
        let d = tool_defs(Scope::Session);
        assert_eq!(d.len(), 15);
        assert_eq!(tool_defs(Scope::Quick).len(), 10);
        for def in d {
            assert_eq!(def.schema["additionalProperties"], false, "{}", def.name);
        }
    }
}
