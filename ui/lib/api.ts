import { invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";
import type { AnalysisBundle, ApplyPhase, ConnectKind, FcApplyResult, FcLogEntry, FcStatus, Flight, ImportResult, LogSummary, Mode, ParamValue, PidStrategy, PortInfo, Recommendation, SessionInfo, SessionSnapshot, SessionSummary, Step } from "./types";

export async function pickLogFile(): Promise<string | null> {
  const r = await open({
    multiple: false,
    filters: [{ name: "Blackbox / DataFlash logs", extensions: ["bbl", "bfl", "txt", "BBL", "BFL", "TXT", "bin", "BIN"] }],
  });
  return typeof r === "string" ? r : null;
}

/** Save dialog + write; returns the path or null when cancelled. */
export async function saveTextAs(defaultName: string, text: string): Promise<string | null> {
  const ext = defaultName.split(".").pop() ?? "txt";
  const path = await save({ defaultPath: defaultName, filters: [{ name: ext.toUpperCase(), extensions: [ext] }] });
  if (!path) return null;
  await invoke<void>("save_text_file", { path, text });
  return path;
}

export const api = {
  devAutoloadPath: () => invoke<string | null>("dev_autoload_path"),
  sessions: (path: string) => invoke<SessionInfo[]>("log_sessions", { path }),
  open: (path: string, session: number) => invoke<LogSummary>("log_open", { path, session }),
  analyze: (id: string, pidAnalyzer = false) => invoke<AnalysisBundle>("log_analyze", { id, pidAnalyzer }),
  recommend: (id: string, _bundle: AnalysisBundle, phase: "filters" | "pids") =>
    invoke<Recommendation[]>("log_recommend", { args: { id, phase } }),
  seriesWindow: (id: string, series: string, axis: number, t0: number, t1: number, pxWidth: number) =>
    invoke<{ x: number[]; y: number[] }>("series_window", { id, series, axis, t0, t1, pxWidth }),

  // ---- wizard ----
  sessionList: () => invoke<SessionSummary[]>("session_list"),
  sessionCreate: (name: string, mode: Mode) => invoke<SessionSnapshot>("session_create", { name, mode }),
  sessionOpen: (id: string) => invoke<SessionSnapshot>("session_open", { id }),
  sessionDelete: (id: string) => invoke<void>("session_delete", { id }),
  sessionSnapshot: () => invoke<SessionSnapshot>("session_snapshot"),
  sessionNotes: (notes: string) => invoke<SessionSnapshot>("session_notes", { notes }),
  next: () => invoke<SessionSnapshot>("wizard_next"),
  back: () => invoke<SessionSnapshot>("wizard_back"),
  goto: (step: Step) => invoke<SessionSnapshot>("wizard_goto", { step }),
  override: (guardId: string, reason: string) => invoke<SessionSnapshot>("wizard_override", { guardId, reason }),
  flightDone: (which: Flight, done: boolean) => invoke<SessionSnapshot>("flight_done", { which, done }),
  flightImport: (which: Flight, path: string, sessionIndex: number) =>
    invoke<ImportResult>("flight_import", { which, path, sessionIndex }),
  flightBundle: (which: Flight) => invoke<AnalysisBundle | null>("flight_bundle", { which }),
  recsSet: (phase: ApplyPhase, id: string, accepted: boolean, newValue?: ParamValue) =>
    invoke<SessionSnapshot>("recs_set", { update: { phase, id, accepted, new_value: newValue ?? null } }),
  applyConfirm: (phase: ApplyPhase, method: string, notes?: string) =>
    invoke<SessionSnapshot>("apply_confirm", { phase, method, notes: notes ?? null }),
  fcStatus: () => invoke<FcStatus>("fc_status"),
  fcPorts: () => invoke<PortInfo[]>("fc_ports"),
  fcConnect: (port: string, kind: ConnectKind = "auto") => invoke<FcStatus>("fc_connect", { port, kind }),
  fcDisconnect: () => invoke<FcStatus>("fc_disconnect"),
  fcPoll: () => invoke<FcStatus>("fc_poll"),
  fcRefresh: () => invoke<FcStatus>("fc_refresh"),
  fcBackupCli: () => invoke<FcStatus>("fc_backup_cli"),
  fcPreflightFix: () => invoke<FcStatus>("fc_preflight_fix"),
  fcListLogs: () => invoke<FcLogEntry[]>("fc_list_logs"),
  fcDownloadImport: (which: Flight, logId?: number) => invoke<ImportResult>("fc_download_import", { which, logId: logId ?? null }),
  fcExportText: (phase: ApplyPhase) => invoke<string>("fc_export_text", { phase }),
  pidStrategy: (strategy: PidStrategy) => invoke<SessionSnapshot>("wizard_pid_strategy", { strategy }),
  fcApply: (phase: ApplyPhase) => invoke<FcApplyResult>("fc_apply", { phase }),
  reportExport: (images: { title: string; data_url: string }[]) => invoke<string>("report_export", { images }),
};
