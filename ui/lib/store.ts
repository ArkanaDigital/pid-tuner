import { create } from "zustand";
import type { AnalysisBundle, FcStatus, Flight, LogSummary, Recommendation, SessionInfo, SessionSnapshot, SessionSummary } from "./types";

type View = "home" | "wizard" | "quick";

interface State {
  view: View;
  busy: string | null;
  error: string | null;
  // quick look
  path: string | null;
  sessions: SessionInfo[];
  log: LogSummary | null;
  bundle: AnalysisBundle | null;
  recs: Recommendation[];
  // wizard
  list: SessionSummary[];
  snap: SessionSnapshot | null;
  bundles: Partial<Record<Flight, AnalysisBundle>>;
  fc: FcStatus | null;
  stepSmoothMs: number;
  stepBand: boolean;
  set: (p: Partial<State>) => void;
  setBundle: (f: Flight, b: AnalysisBundle) => void;
}

export const useStore = create<State>((set) => ({
  view: "home",
  busy: null,
  error: null,
  path: null,
  sessions: [],
  log: null,
  bundle: null,
  recs: [],
  list: [],
  snap: null,
  bundles: {},
  fc: null,
  stepSmoothMs: 20,
  stepBand: false,
  set: (p) => set(p),
  setBundle: (f, b) => set((s) => ({ bundles: { ...s.bundles, [f]: b } })),
}));

/** Run an async action with busy/error handling; returns the value or undefined on error. */
export async function run<T>(label: string, fn: () => Promise<T>): Promise<T | undefined> {
  const s = useStore.getState();
  s.set({ busy: label, error: null });
  try {
    const v = await fn();
    useStore.getState().set({ busy: null });
    return v;
  } catch (e) {
    useStore.getState().set({ busy: null, error: String(e) });
    return undefined;
  }
}
