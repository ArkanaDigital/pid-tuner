import { useState } from "react";
import { api, pickLogFile, saveTextAs } from "../lib/api";
import { run, useStore } from "../lib/store";
import { AXES, STEP_FLIGHT, isArduPilot, type AnalysisBundle, type ApplyPhase, type Flight, type PidStrategy, type SessionSnapshot, type Step } from "../lib/types";
import StepResponseChart from "../charts/StepResponseChart";
import SpectrumChart from "../charts/SpectrumChart";
import SpectrogramCanvas from "../charts/SpectrogramCanvas";
import RecsTable, { paramText } from "./RecsTable";
import { ConnectStep, DownloadFromFlash, FcApplyButton, PreflightStep } from "./fcsteps";
import AnomalyList from "./AnomalyList";

type Protocol = { title: string; steps: string[]; note: string };

/** ArduPilot Copter protocol: AltHold hover for the spectra, Stabilize stick steps for the step response. */
const AP_FLIGHT_PROTOCOL: Record<Flight, Protocol> = {
  a: {
    title: "Flight A — noise / filter data (ArduCopter)",
    steps: [
      "Props on, battery fresh, GPS not required. Arm in AltHold (or Loiter) in a safe open area.",
      "Hover steadily for 30 seconds at hover throttle (stick centred). No pitch/roll input.",
      "Then 20 seconds of gentle rocking (small roll/pitch, throttle 30–70 %) so the spectrogram covers a throttle range.",
      "Land, disarm. Wait ~5 s before power-off so the .bin log is closed.",
    ],
    note: "Needs LOG_BITMASK bits 0+12+19 and the IMU batch sampler (INS_LOG_BAT_MASK=1, INS_LOG_BAT_OPT=4) — Preflight sets them. The gyro spectrum comes from ISBH/ISBD batches; ≥ 20 batches are required.",
  },
  b: {
    title: "Flight B — step response data (ArduCopter)",
    steps: [
      "Take off in Stabilize (rate response is what we measure; AltHold/Loiter add position loops). Hover 5 s.",
      "Roll: sharp stick snap left, hold ½ s, centre, pause 1 s. Repeat right. Do 15 pairs.",
      "Pitch: same pattern forward / back, 15 pairs.",
      "Yaw: same pattern, 5 pairs.",
      "One axis at a time. Land and disarm.",
    ],
    note: "ATC_INPUT_TC shapes the pilot input, so the target seen by the rate loop (PIDx.Tar) is already filtered — snaps ≥ 60 °/s on roll/pitch, ≥ 40 °/s on yaw are still needed. ≥ 30 segments per axis for a trustworthy curve.",
  },
  c: {
    title: "Flight C — verification (ArduCopter)",
    steps: [
      "Heuristic path: fly the same Stabilize protocol as Flight B with the new gains.",
      "AUTOTUNE path: fly AUTOTUNE (AUTOTUNE_AXES / AUTOTUNE_AGGR as set), let it finish, land and disarm WITHOUT touching the sticks so the gains are saved — then fly the Flight B protocol once more.",
      "Land and disarm.",
    ],
    note: "The Import C guard checks ATC_RAT_*_P/D actually changed and D did not end at AUTOTUNE_MIN_D (a failed autotune).",
  },
};

const FLIGHT_PROTOCOL: Record<Flight, Protocol> = {
  a: {
    title: "Flight A — noise / filter data",
    steps: [
      "Props on, battery fresh, arm in a safe open area (or over a bed indoors for a tiny whoop).",
      "Hover steadily for 30 seconds at normal hover throttle. No stick input.",
      "Then hover with gentle wobbles for 20 seconds (small roll/pitch rocking, throttle 30–70 %).",
      "Land, disarm. Do not power off before the log is saved.",
    ],
    note: "This flight only needs to be smooth. The spectrum analysis needs ≥ 20 s of clean hover and a spread of throttle values.",
  },
  b: {
    title: "Flight B — step response data",
    steps: [
      "Take off and hover for 5 seconds.",
      "Roll: sharp stick snap left, hold ½ s, centre, pause 1 s. Repeat right. Do 5 pairs.",
      "Pitch: same pattern forward / back, 5 pairs.",
      "Yaw: same pattern, 3 pairs.",
      "Keep axes separate — one axis moving at a time. Land and disarm.",
    ],
    note: "Sharp, isolated stick moves ≥ 200 °/s on each axis give the deconvolution what it needs.",
  },
  c: {
    title: "Flight C — verification",
    steps: [
      "Fly the same protocol as Flight B with the new PIDs.",
      "Optionally add a few flips/rolls and throttle punches.",
      "Land and disarm.",
    ],
    note: "This log becomes the 'after' in the before/after comparison and the report.",
  },
};

function flightOf(step: Step): Flight {
  return STEP_FLIGHT[step]!;
}

export function StepPanel({ snap }: { snap: SessionSnapshot }) {
  const step = snap.session.current;
  switch (step) {
    case "connect":
      return <ConnectStep />;
    case "preflight":
      return <PreflightStep />;
    case "flight_a":
    case "flight_b":
    case "flight_c":
      return <FlightStep snap={snap} which={flightOf(step)} />;
    case "import_a":
    case "import_b":
    case "import_c":
      return <ImportStep snap={snap} which={flightOf(step)} />;
    case "filter_analysis":
      return <AnalysisStep snap={snap} which="a" phase="filters" />;
    case "pid_analysis":
      return <AnalysisStep snap={snap} which="b" phase="pids" />;
    case "apply_filters":
      return <ApplyStep snap={snap} phase="filters" />;
    case "apply_pids":
      return <ApplyStep snap={snap} phase="pids" />;
    case "compare":
      return <CompareStep />;
    case "report":
      return <ReportStep snap={snap} />;
  }
}

function FlightStep({ snap, which }: { snap: SessionSnapshot; which: Flight }) {
  const s = useStore();
  const ap = isArduPilot(snap.session.firmware ?? s.fc?.firmware);
  const p = (ap ? AP_FLIGHT_PROTOCOL : FLIGHT_PROTOCOL)[which];
  const done = !!snap.session.flight_done[which];
  async function toggle() {
    const n = await run("Saving…", () => api.flightDone(which, !done));
    if (n) s.set({ snap: n });
  }
  return (
    <div className="panel">
      <h2>{p.title}</h2>
      <ol className="protocol">
        {p.steps.map((x, i) => <li key={i}>{x}</li>)}
      </ol>
      <p className="muted">{p.note}</p>
      <label className="check">
        <input type="checkbox" checked={done} onChange={toggle} /> Flight done — landed and disarmed
      </label>
    </div>
  );
}

function ImportStep({ snap, which }: { snap: SessionSnapshot; which: Flight }) {
  const s = useStore();
  const rec = snap.session.flights[which];
  const bundle = s.bundles[which];
  const [sessions, setSessions] = useState<{ path: string; list: { index: number; firmware_revision: string; craft_name: string | null; error: string | null }[] } | null>(null);

  async function pick() {
    const path = await pickLogFile();
    if (!path) return;
    const list = await run("Reading sessions…", () => api.sessions(path));
    if (!list) return;
    const ok = list.filter((x) => !x.error);
    if (ok.length === 1) return importIdx(path, ok[0].index);
    setSessions({ path, list });
  }
  async function importIdx(path: string, idx: number) {
    setSessions(null);
    const r = await run("Decoding and analysing…", () => api.flightImport(which, path, idx));
    if (r) {
      s.setBundle(which, r.bundle);
      s.set({ snap: r.snapshot });
    }
  }

  return (
    <div className="panel">
      <h2>Import log {which.toUpperCase()}</h2>
      <p>
        Betaflight: pull the .BBL/.BFL from the flash or SD card (Configurator → Blackbox → Save flash to file).
        ArduPilot: copy the .BIN from the SD card (fastest) or download it over MAVLink below. Or use the file the pilot sent you.
        Betaflight 2025+ can switch off blackbox fields — keep <b>Setpoint</b>, PID, Gyro and Motors enabled (Blackbox tab → fields, or CLI <code>set blackbox_disable_setpoint = OFF</code>); without Setpoint the app rebuilds it from rcCommand and says so.
      </p>
      {snap.session.mode === "online" && <DownloadFromFlash which={which} />}
      <div className="row">
        <button className="primary" onClick={pick} disabled={!!s.busy}>{rec ? "Replace log…" : "Choose log file…"}</button>
        {rec && <span className="muted">{rec.original_path} · session {rec.session_index + 1} · {rec.fs_hz.toFixed(0)} Hz · {rec.duration_s.toFixed(1)} s</span>}
      </div>
      {sessions && (
        <div className="session-pick">
          <div>This file contains several logs — pick the right flight:</div>
          {sessions.list.map((x) => (
            <button key={x.index} disabled={!!x.error} onClick={() => importIdx(sessions.path, x.index)}>
              #{x.index + 1} {x.firmware_revision} {x.craft_name ?? ""} {x.error ? "(error)" : ""}
            </button>
          ))}
        </div>
      )}
      {rec && rec.warnings.length > 0 && <div className="notice">{rec.warnings.join(" · ")}</div>}
      {bundle && <AnomalyList list={bundle.anomalies} compact />}
      {bundle && which === "a" && <SpectrumPreview bundle={bundle} />}
      {bundle && which !== "a" && <StepPreview bundle={bundle} />}
    </div>
  );
}

function SpectrumPreview({ bundle }: { bundle: AnalysisBundle }) {
  return (
    <>
      {AXES.map((a) => (
        <div className="chart-card" key={a} data-report={`Full spectrum ${a}`}>
          <SpectrumChart
            axis={a}
            filt={bundle.spectra.find((x) => x.axis === a && x.kind === "gyro_filt") ?? null}
            raw={bundle.spectra.find((x) => x.axis === a && x.kind === "gyro_raw") ?? null}
            dterm={bundle.spectra.find((x) => x.axis === a && x.kind === "d_term") ?? null}
            peaks={bundle.peaks}
            maxHz={Math.min(1000, Math.floor(bundle.quality.fs_hz / 2))}
            height={200}
          />
        </div>
      ))}
    </>
  );
}

export function StepControls() {
  const s = useStore();
  return (
    <div className="row step-controls">
      <label>
        Smoothing
        <select value={s.stepSmoothMs} onChange={(e) => s.set({ stepSmoothMs: Number(e.target.value) })}>
          <option value={0}>off</option>
          <option value={10}>low</option>
          <option value={20}>medium</option>
          <option value={40}>high</option>
        </select>
      </label>
      <label>
        <input type="checkbox" checked={s.stepBand} onChange={(e) => s.set({ stepBand: e.target.checked })} /> show 10–90 % band
      </label>
      <span className="muted">Curves need ≥ 30 stick snaps per axis to be trusted; the label next to "segments" tells you.</span>
    </div>
  );
}

function StepPreview({ bundle, compare }: { bundle: AnalysisBundle; compare?: AnalysisBundle }) {
  const s = useStore();
  return (
    <>
      <StepControls />
      {AXES.map((a) => (
        <div key={a} data-report={`Step response ${a}`}>
          <StepResponseChart
            step={bundle.steps.find((x) => x.axis === a)!}
            compare={compare?.steps.find((x) => x.axis === a) ?? null}
            height={200}
            smoothMs={s.stepSmoothMs}
            showBand={s.stepBand}
          />
        </div>
      ))}
    </>
  );
}

function AnalysisStep({ snap, which, phase }: { snap: SessionSnapshot; which: Flight; phase: ApplyPhase }) {
  const s = useStore();
  const bundle = s.bundles[which];
  const recs = phase === "filters" ? snap.session.recs_filters : snap.session.recs_pids;
  const [tab, setTab] = useState<"charts" | "spectrogram">("charts");
  if (!bundle) return <div className="panel"><div className="empty">Analysis not available — go back and import the log.</div></div>;
  return (
    <div className="panel">
      <h2>{phase === "filters" ? "Filter analysis" : "PID analysis"}</h2>
      <p className="muted">
        {phase === "filters"
          ? "Solid = filtered gyro (what the PID loop sees). Dashed = unfiltered. Peaks above the floor are labelled; 100–250 Hz is frame resonance, above 250 Hz is motor noise."
          : "Orange = averaged step response; shaded = 10–90 % spread across stick moves. Target: overshoot ≤ 1.10, latency ≈ 10–25 ms, steady state 1.0."}
      </p>
      {phase === "filters" && (
        <nav className="subtabs">
          <button className={tab === "charts" ? "active" : ""} onClick={() => setTab("charts")}>Full spectrum</button>
          <button className={tab === "spectrogram" ? "active" : ""} onClick={() => setTab("spectrogram")}>Throttle spectrogram</button>
        </nav>
      )}
      {phase === "filters" && tab === "charts" && <SpectrumPreview bundle={bundle} />}
      {phase === "filters" && tab === "spectrogram" && bundle.spectrograms.map((sg) => (
        <div className="chart-card" key={sg.axis} data-report={`Throttle spectrogram ${sg.axis}`}><SpectrogramCanvas sg={sg} height={220} /></div>
      ))}
      {phase === "pids" && <StepPreview bundle={bundle} />}
      <h3>Suggested changes</h3>
      <RecsTable phase={phase} recs={recs} editable />
    </div>
  );
}

function PidStrategyToggle({ snap }: { snap: SessionSnapshot }) {
  const s = useStore();
  const cur: PidStrategy = snap.session.pid_strategy ?? "heuristic";
  async function set(v: PidStrategy) {
    const n = await run("Saving…", () => api.pidStrategy(v));
    if (n) s.set({ snap: n });
  }
  return (
    <div className="notice">
      <b>ArduCopter PID path:</b>{" "}
      <label className="check"><input type="radio" checked={cur === "heuristic"} onChange={() => set("heuristic")} /> our step-response heuristics (write ATC_RAT_* below)</label>{" "}
      <label className="check"><input type="radio" checked={cur === "autotune"} onChange={() => set("autotune")} /> pilot flies AUTOTUNE, we verify before/after</label>
      {cur === "autotune" && <div className="muted">Skip the write below; Flight C must be an AUTOTUNE flight saved by disarming with sticks centred. AUTOTUNE_AGGR 0.05–0.10, AUTOTUNE_AXES selects the axes.</div>}
    </div>
  );
}

function ApplyStep({ snap, phase }: { snap: SessionSnapshot; phase: ApplyPhase }) {
  const s = useStore();
  const recs = phase === "filters" ? snap.session.recs_filters : snap.session.recs_pids;
  const applied = [...snap.session.applies].reverse().find((a) => a.phase === phase);
  const firmware = snap.session.firmware ?? s.fc?.firmware ?? null;
  const ap = isArduPilot(firmware);
  const cli = paramText(recs, firmware);
  const [copied, setCopied] = useState(false);
  const [saved, setSaved] = useState<string | null>(null);
  async function copy() {
    await navigator.clipboard.writeText(cli);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  }
  async function saveParam() {
    const p = await saveTextAs(`pidtuner_${phase}.${ap ? "param" : "txt"}`, cli);
    if (p) setSaved(p);
  }
  async function confirm() {
    const n = await run("Recording…", () => api.applyConfirm(phase, snap.session.mode === "online" ? (s.fc?.kind ?? "msp") : ap ? "param-manual" : "cli-manual"));
    if (n) s.set({ snap: n });
  }
  return (
    <div className="panel">
      <h2>{phase === "filters" ? "Apply filter settings" : "Apply PID settings"}</h2>
      {ap && phase === "pids" && <PidStrategyToggle snap={snap} />}
      <RecsTable phase={phase} recs={recs} editable={!applied} />
      {snap.session.mode === "online" && cli && !applied && <FcApplyButton phase={phase} />}
      {cli ? (
        <>
          <div className="row">
            <button onClick={copy}>{copied ? "Copied ✓" : ap ? "Copy parameters" : "Copy CLI"}</button>
            <button onClick={saveParam}>{ap ? "Save .param file…" : "Save CLI text…"}</button>
            {saved && <span className="muted">Saved: {saved}</span>}
            <span className="muted">
              {ap
                ? "Load in Mission Planner (Config → Full Parameter List → Load from file → Write Params) or QGC. INS_HNTCH_ENABLE needs a reboot before the other INS_HNTCH_* values can be written."
                : <>Paste into Betaflight Configurator → CLI. The final <code>save</code> reboots the FC.</>}
            </span>
          </div>
          <pre className="cli">{cli}</pre>
        </>
      ) : (
        <div className="notice">Nothing accepted — the step will be recorded as an explicit skip.</div>
      )}
      {applied ? (
        <div className="notice ok">Applied {new Date(applied.at).toLocaleString()} via {applied.method}{applied.verified ? " · verified" : ""}.</div>
      ) : (
        cli && snap.session.mode === "offline" && (
          <label className="check">
            <input type="checkbox" onChange={confirm} /> I have applied these settings and saved (the FC rebooted)
          </label>
        )
      )}
    </div>
  );
}

function CompareStep() {
  const s = useStore();
  const before = s.bundles.b;
  const after = s.bundles.c;
  const a = s.bundles.a;
  if (!before || !after) return <div className="panel"><div className="empty">Logs B and C are needed for the comparison.</div></div>;
  return (
    <div className="panel">
      <h2>Before / after</h2>
      <p className="muted">Blue = before (Flight B), orange = after (Flight C).</p>
      <StepPreview bundle={after} compare={before} />
      <table className="recs">
        <thead><tr><th>Axis</th><th>Overshoot before → after</th><th>Latency before → after</th><th>Steady state before → after</th></tr></thead>
        <tbody>
          {AXES.map((ax) => {
            const b = before.steps.find((x) => x.axis === ax)!;
            const c = after.steps.find((x) => x.axis === ax)!;
            return (
              <tr key={ax}>
                <td>{ax}</td>
                <td>{b.overshoot.toFixed(2)} → <b>{c.overshoot.toFixed(2)}</b></td>
                <td>{b.latency_ms.toFixed(1)} → <b>{c.latency_ms.toFixed(1)} ms</b></td>
                <td>{b.steady_state.toFixed(2)} → <b>{c.steady_state.toFixed(2)}</b></td>
              </tr>
            );
          })}
        </tbody>
      </table>
      {a && <SpectrumPreview bundle={a} />}
    </div>
  );
}

function ReportStep({ snap }: { snap: SessionSnapshot }) {
  const s = useStore();
  const [notes, setNotes] = useState(snap.session.notes);
  const [path, setPath] = useState<string | null>(snap.session.report_file);

  async function exportReport() {
    await api.sessionNotes(notes);
    // Capture every chart rendered on this page.
    const images: { title: string; data_url: string }[] = [];
    document.querySelectorAll<HTMLElement>("[data-report]").forEach((el) => {
      const canvases = Array.from(el.querySelectorAll("canvas"));
      const cv = canvases.find((c) => c.width > 0) ?? null;
      if (cv) images.push({ title: el.dataset.report!, data_url: cv.toDataURL("image/png") });
    });
    const p = await run("Writing report…", () => api.reportExport(images));
    if (p) {
      setPath(p);
      const n = await api.sessionSnapshot();
      s.set({ snap: n });
    }
  }

  const b = s.bundles.b, c = s.bundles.c, a = s.bundles.a;
  return (
    <div className="panel">
      <h2>Report</h2>
      <label>
        Notes for the pilot
        <textarea value={notes} onChange={(e) => setNotes(e.target.value)} rows={4} placeholder="What changed, what to watch for, next steps…" />
      </label>
      <div className="row">
        <button className="primary" onClick={exportReport} disabled={!!s.busy}>Export HTML report</button>
        {path && <span className="muted">Saved: {path}</span>}
      </div>
      <div className="report-charts">
        {c && <StepPreview bundle={c} compare={b} />}
        {!c && b && <StepPreview bundle={b} />}
        {a && <SpectrumPreview bundle={a} />}
      </div>
    </div>
  );
}
