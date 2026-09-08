// Mirrors crates/domain. Keep in sync by hand (serde snake_case tags).

export type Axis = "roll" | "pitch" | "yaw";
export const AXES: Axis[] = ["roll", "pitch", "yaw"];
export const AXIS_LABEL: Record<Axis, string> = { roll: "Roll", pitch: "Pitch", yaw: "Yaw" };

export type Firmware =
  | { kind: "betaflight"; version: string; api: [number, number] }
  | { kind: "ardu_copter"; version: string }
  | { kind: "unknown"; product: string };

export type SpectrumKind = "gyro_raw" | "gyro_filt" | "d_term" | "setpoint" | "pid_sum" | "predicted";

export interface StepResponse {
  axis: Axis;
  variant: "pt_step" | "pid_analyzer";
  t_ms: number[];
  mean: number[];
  p10: number[];
  p90: number[];
  n_segments: number;
  rejected: number;
  overshoot: number;
  latency_ms: number;
  settle_ms: number | null;
  steady_state: number;
}

export interface Spectrum {
  axis: Axis;
  kind: SpectrumKind;
  f_hz: number[];
  psd_db: number[];
  nfft: number;
  fs_hz: number;
}

export interface Spectrogram {
  axis: Axis;
  kind: SpectrumKind;
  f_hz: number[];
  throttle_bins: number[];
  db: number[];
  counts: number[];
}

export interface NoisePeak {
  axis: Axis;
  kind: SpectrumKind;
  f_hz: number;
  psd_db: number;
  prominence_db: number;
  band: "control" | "frame" | "motor";
}

export interface LogQuality {
  fs_hz: number;
  duration_s: number;
  has_gyro_raw: boolean;
  has_pid_terms: boolean;
  hover_seconds: number;
  hover_throttle_pct: number;
  airborne_range_s: [number, number] | null;
  motor_saturation_pct: number;
  max_setpoint_per_axis: [number, number, number];
  step_segments_per_axis: [number, number, number];
  gap_seconds: number;
  chirp_sweeps_per_axis?: [number, number, number];
  chirp_windows_per_axis?: [number, number, number];
  chirp_coherence_per_axis?: [number, number, number];
}

export interface FrTarget {
  pm_deg: number;
  crossover_hz: number | null;
  gain_to_target: number | null;
  gain_for_sens_limit: number | null;
}

export interface FrMetrics {
  bandwidth_hz: number | null;
  crossover_hz: number | null;
  phase_margin_deg: number | null;
  max_phase_margin_deg: number | null;
  resonant_peak_db: number | null;
  resonant_peak_hz: number | null;
  loop_delay_ms: number | null;
  low_freq_err_db: number | null;
  coherence_mean: number | null;
  noise_floor_hz: number | null;
  sens_peak_db: number | null;
  sens_peak_hz: number | null;
  step_overshoot: number | null;
  step_rise_ms: number | null;
  step_settle_ms: number | null;
  targets: FrTarget[];
}

/** Closed-loop frequency response of one axis from Betaflight CHIRP sweeps. */
export interface FrequencyResponse {
  axis: Axis;
  angle_mode: boolean;
  f_hz: number[];
  h_mag_db: (number | null)[];
  h_phase_deg: (number | null)[];
  coherence: (number | null)[];
  l_mag_db: (number | null)[];
  l_phase_deg: (number | null)[];
  s_mag_db: (number | null)[];
  step_t_ms: number[];
  step: number[];
  fs_hz: number;
  segment_size: number;
  n_windows: number;
  n_sweeps: number;
  sweep_seconds: number;
  metrics: FrMetrics;
}

export interface Protocol {
  title: string;
  steps: string[];
  note: string;
  alternative: { title: string; steps: string[]; note: string } | null;
}

export type AnomalyKind =
  | "motor_desync" | "motor_saturation" | "motor_floor" | "gyro_clipping" | "oscillation" | "motor_imbalance"
  | "rpm_imbalance" | "rpm_dropout" | "log_gap" | "yaw_spin" | "control_reversed" | "vibration";
export type Severity = "info" | "warning" | "critical";

export interface Anomaly {
  kind: AnomalyKind;
  severity: Severity;
  t_start_s: number;
  t_end_s: number;
  axis: Axis | null;
  motor: number | null;
  value: number;
  detail: string;
}

export const ANOMALY_TITLE: Record<AnomalyKind, string> = {
  motor_desync: "Motor desync",
  motor_saturation: "Motor saturation",
  motor_floor: "Motor at idle floor",
  gyro_clipping: "Gyro clipping",
  oscillation: "Oscillation",
  motor_imbalance: "Motor imbalance",
  rpm_imbalance: "RPM imbalance",
  rpm_dropout: "RPM telemetry dropout",
  log_gap: "Log gap",
  yaw_spin: "Yaw spin",
  control_reversed: "Control reversed",
  vibration: "Vibration",
};

export interface AnalysisBundle {
  log: string;
  quality: LogQuality;
  steps: StepResponse[];
  spectra: Spectrum[];
  spectrograms: Spectrogram[];
  peaks: NoisePeak[];
  anomalies: Anomaly[];
  freq_resp: FrequencyResponse[];
}

export type ParamValue =
  | { type: "f32"; value: number }
  | { type: "i32"; value: number }
  | { type: "u8"; value: number }
  | { type: "u16"; value: number }
  | { type: "bool"; value: boolean }
  | { type: "enum"; value: number };

export type EvidenceRef =
  | { kind: "peak"; axis: Axis; f_hz: number; psd_db: number }
  | { kind: "step"; axis: Axis; overshoot: number; latency_ms: number }
  | { kind: "quality"; field: string; value: number }
  | { kind: "text"; note: string }
  | { kind: "freq_resp"; axis: Axis; bandwidth_hz: number; phase_margin_deg: number; coherence: number };

export interface Recommendation {
  id: string;
  param: { firmware: "bf" | "ap"; name: string };
  old: ParamValue;
  new: ParamValue;
  reason: string;
  evidence: EvidenceRef[];
  confidence: "low" | "medium" | "high";
  requires_reboot: boolean;
  accepted: boolean;
}

export interface BfAxisPid { p: number; i: number; d: number; ff: number; d_max: number }

export interface LogSummary {
  id: string;
  path: string;
  firmware: Firmware;
  craft_name: string | null;
  fs_hz: number;
  duration_s: number;
  samples: number;
  session_index: number;
  session_count: number;
  debug_mode: string | null;
  warnings: string[];
  has_gyro_raw: boolean;
  tune: { firmware: "bf"; pids: BfAxisPid[]; filters: Record<string, unknown>; simplified: Record<string, unknown> } | { firmware: "ap" } | { firmware: "unknown" };
}

export interface SessionInfo {
  index: number;
  firmware_revision: string;
  craft_name: string | null;
  error: string | null;
}

export function paramValueText(v: ParamValue): string {
  if (v.type === "bool") return v.value ? "ON" : "OFF";
  return String(v.value);
}

export function isArduPilot(f: Firmware | null | undefined): boolean {
  return f?.kind === "ardu_copter";
}

export function firmwareText(f: Firmware): string {
  if (f.kind === "betaflight") return `Betaflight ${f.version}`;
  if (f.kind === "ardu_copter") return `ArduCopter ${f.version}`;
  return f.product;
}

// ---- session / wizard (crates/session) ----
export type Step =
  | "connect" | "preflight" | "flight_a" | "import_a" | "filter_analysis" | "apply_filters"
  | "flight_b" | "import_b" | "pid_analysis" | "apply_pids" | "flight_c" | "import_c" | "compare" | "report";
export type StepStatus = "locked" | "active" | "passed" | "skipped";
export type Flight = "a" | "b" | "c";
export type Mode = "online" | "offline";
export type ApplyPhase = "filters" | "pids";

export type GuardOutcome =
  | { kind: "pass" }
  | { kind: "fail"; message: string; fix_hint: string | null }
  | { kind: "needs_action"; message: string };

export interface GuardResult {
  id: string;
  title: string;
  outcome: GuardOutcome;
  can_override: boolean;
  overridden: boolean;
}

export interface StepView {
  step: Step;
  title: string;
  status: StepStatus;
  needs_fc: boolean;
  guards: GuardResult[];
}

export interface FlightRecord {
  log_file: string;
  original_path: string;
  session_index: number;
  log_id: string;
  imported_at: string;
  firmware: Firmware;
  craft_name: string | null;
  fs_hz: number;
  duration_s: number;
  quality: LogQuality;
  warnings: string[];
  anomalies?: Anomaly[];
  tune: unknown;
  bundle_file: string;
}

export interface ApplyRecord {
  phase: ApplyPhase;
  at: string;
  applied: Recommendation[];
  verified: boolean;
  method: string;
  notes: string | null;
}

export interface Session {
  id: string;
  name: string;
  mode: Mode;
  created_at: string;
  updated_at: string;
  current: Step;
  status: Record<Step, StepStatus>;
  overrides: { step: Step; guard_id: string; reason: string; at: string }[];
  flight_done: Partial<Record<Flight, string>>;
  flights: Partial<Record<Flight, FlightRecord>>;
  recs_filters: Recommendation[];
  recs_pids: Recommendation[];
  applies: ApplyRecord[];
  firmware: Firmware | null;
  notes: string;
  report_file: string | null;
  pid_strategy?: PidStrategy;
  pid_source?: "step_response" | "chirp";
}

export interface SessionSnapshot {
  session: Session;
  steps: StepView[];
  can_next: boolean;
  can_back: boolean;
}

export interface SessionSummary {
  id: string;
  name: string;
  mode: Mode;
  current: Step;
  updated_at: string;
  firmware: Firmware | null;
  craft_name: string | null;
}

export type FcKind = "msp" | "mavlink";
export type ConnectKind = "auto" | "msp" | "mavlink";
export type PidStrategy = "heuristic" | "autotune";

export interface FcLogEntry {
  id: number;
  size: number;
  time_utc: number | null;
}

export interface FcStatus {
  connected: boolean;
  port: string | null;
  kind: FcKind | null;
  firmware: Firmware | null;
  armed: boolean;
  heartbeat_age_s: number;
  tune: unknown;
  log_rate_hz: number | null;
  debug_mode: string | null;
  storage_free_bytes: number | null;
  pid_logging_enabled: boolean | null;
  raw_gyro_logging_enabled: boolean | null;
  snapshot_taken: boolean;
  // ArduPilot
  log_bitmask: number | null;
  batch_configured: boolean | null;
  loop_rate_hz: number | null;
  autotune_axes: number | null;
}

export interface PortInfo {
  path: string;
  vid: number | null;
  pid: number | null;
  product: string | null;
  manufacturer: string | null;
  likely_fc: boolean;
}

export interface ApplyOutcome {
  param: string;
  wanted: string;
  read_back: string | null;
  ok: boolean;
  via: string;
}

export interface FcApplyResult {
  result: { outcomes: ApplyOutcome[]; verified: boolean; rebooted: boolean };
  snapshot: SessionSnapshot;
}

export interface ImportResult {
  snapshot: SessionSnapshot;
  bundle: AnalysisBundle;
  log_id: string;
}

export const STEP_FLIGHT: Partial<Record<Step, Flight>> = {
  flight_a: "a", import_a: "a", flight_b: "b", import_b: "b", flight_c: "c", import_c: "c",
};
