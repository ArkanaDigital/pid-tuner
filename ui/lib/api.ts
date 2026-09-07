import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import type { AnalysisBundle, ApplyPhase, FcApplyResult, FcStatus, Flight, ImportResult, LogSummary, Mode, ParamValue, PortInfo, Recommendation, SessionInfo, SessionSnapshot, SessionSummary, Step } from "./types";

export async function pickLogFile(): Promise<string | null> {
  const r = await open({
    multiple: false,
    filters: [{ name: "Blackbox logs", extensions: ["bbl", "bfl", "txt", "BBL", "BFL", "TXT", "bin", "BIN"] }],
  });
  return typeof r === "string" ? r : null;
}

export const api = {
  devAutoloadPath: () => invoke<string | null>("dev_autoload_path"),
  sessions: (path: string) => invoke<SessionInfo[]>("log_sessions", { path }),
  open: (path: string, session: number) => invoke<LogSummary>("log_open", { path, session }),
  analyze: (id: string, pidAnalyzer = false) => invoke<AnalysisBundle>("log_analyze", { id, pidAnalyzer }),
  recommend: (id: string, bundle: AnalysisBundle, phase: "filters" | "pids") =>
    invoke<Recommendation[]>("log_recommend", { args: { id, bundle, phase } }),
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
  fcConnect: (port: string) => invoke<FcStatus>("fc_connect", { port }),
  fcDisconnect: () => invoke<FcStatus>("fc_disconnect"),
  fcPoll: () => invoke<FcStatus>("fc_poll"),
  fcRefresh: () => invoke<FcStatus>("fc_refresh"),
  fcBackupCli: () => invoke<FcStatus>("fc_backup_cli"),
  fcPreflightFix: () => invoke<FcStatus>("fc_preflight_fix"),
  fcDownloadImport: (which: Flight) => invoke<ImportResult>("fc_download_import", { which }),
  fcApply: (phase: ApplyPhase) => invoke<FcApplyResult>("fc_apply", { phase }),
  reportExport: (images: { title: string; data_url: string }[]) => invoke<string>("report_export", { images }),
};
