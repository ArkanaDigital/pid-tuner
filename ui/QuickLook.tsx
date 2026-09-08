import { useEffect, useState } from "react";
import { api, pickLogFile } from "./lib/api";
import { useStore } from "./lib/store";
import { AXES, firmwareText, paramValueText, type Axis } from "./lib/types";
import StepResponseChart from "./charts/StepResponseChart";
import { StepControls } from "./wizard/steps";
import SpectrumChart from "./charts/SpectrumChart";
import SpectrogramCanvas from "./charts/SpectrogramCanvas";
import AnomalyList, { anomalySummary } from "./wizard/AnomalyList";
import BodeChart from "./charts/BodeChart";
import FrMetricsTable from "./charts/FrMetricsTable";
import AiPanel from "./ai/AiPanel";

type Tab = "step" | "freq" | "spectrum" | "spectrogram" | "recs" | "anomalies";

export default function QuickLook() {
  const s = useStore();
  const [tab, setTab] = useState<Tab>("step");
  const [session, setSession] = useState(0);

  useEffect(() => {
    api.devAutoloadPath().then((p) => { if (p && !s.log) openPath(p); }).catch(() => {});
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function openFile() {
    const path = await pickLogFile();
    if (!path) return;
    await openPath(path);
  }

  async function openPath(path: string) {
    s.set({ busy: "Reading sessions…", error: null, path, bundle: null, recs: [], log: null });
    try {
      const sessions = await api.sessions(path);
      s.set({ sessions });
      const first = sessions.find((x) => !x.error)?.index ?? 0;
      setSession(first);
      await loadSession(path, first);
    } catch (e) {
      s.set({ error: String(e), busy: null });
    }
  }

  async function loadSession(path: string, idx: number) {
    s.set({ busy: "Decoding log…", error: null });
    try {
      const log = await api.open(path, idx);
      s.set({ log, busy: "Analyzing…" });
      const bundle = await api.analyze(log.id);
      const recsF = await api.recommend(log.id, bundle, "filters");
      const recsP = await api.recommend(log.id, bundle, "pids");
      s.set({ bundle, recs: [...recsF, ...recsP], busy: null });
    } catch (e) {
      s.set({ error: String(e), busy: null });
    }
  }

  const b = s.bundle;
  const log = s.log;

  return (
    <div className="app">
      <header className="topbar">
        <button onClick={() => s.set({ view: "home" })}>← Sessions</button>
        <button className="primary" onClick={openFile} disabled={!!s.busy}>Open blackbox…</button>
        {s.sessions.length > 1 && (
          <select
            value={session}
            onChange={(e) => {
              const v = Number(e.target.value);
              setSession(v);
              if (s.path) loadSession(s.path, v);
            }}
          >
            {s.sessions.map((x) => (
              <option key={x.index} value={x.index} disabled={!!x.error}>
                #{x.index + 1} {x.firmware_revision} {x.craft_name ?? ""} {x.error ? "(error)" : ""}
              </option>
            ))}
          </select>
        )}
        {log && (
          <span className="loginfo">
            {firmwareText(log.firmware)} · {log.craft_name ?? "—"} · {log.fs_hz.toFixed(0)} Hz · {log.duration_s.toFixed(1)} s
            {log.debug_mode ? ` · debug ${log.debug_mode}` : ""}
            {!log.has_gyro_raw && <span className="warn"> · no unfiltered gyro</span>}
          </span>
        )}
        <span className="spacer" />
        {s.busy && <span className="busy">{s.busy}</span>}
        {s.error && <span className="error">{s.error}</span>}
        <button className="small" onClick={() => s.setAi({ open: !s.ai.open })}>{s.ai.open ? "Sembunyikan AI" : "AI helper"}</button>
        <button className="small" onClick={() => s.set({ prevView: "quick", view: "settings" })}>⚙</button>
      </header>

      <nav className="tabs">
        {(["step", "freq", "spectrum", "spectrogram", "recs", "anomalies"] as Tab[]).map((t) => (
          <button key={t} className={`${tab === t ? "active" : ""} ${t === "anomalies" && b?.anomalies.some((a) => a.severity === "critical") ? "warn" : ""}`} onClick={() => setTab(t)} disabled={!b || (t === "freq" && !(b.freq_resp?.length))}>
            {t === "step" ? "Step Response" : t === "freq" ? `Frequency response${b?.freq_resp?.length ? "" : " (no CHIRP)"}` : t === "spectrum" ? "Full Spectrum" : t === "spectrogram" ? "Throttle Spectrogram" : t === "recs" ? `Recommendations (${s.recs.length})` : `Anomalies${b && b.anomalies.length ? ` (${anomalySummary(b.anomalies)})` : ""}`}
          </button>
        ))}
      </nav>

      <div className={`quick-body ${s.ai.open ? "with-ai" : ""}`}>
      <main className="content">
        {!b && !s.busy && <div className="empty">Open a Betaflight blackbox (.BBL / .BFL) or ArduPilot DataFlash (.BIN) log to analyze it.</div>}
        {b && b.anomalies.some((a) => a.severity === "critical") && tab !== "anomalies" && (
          <div className="notice bad">Critical anomalies found ({anomalySummary(b.anomalies)}) — see the Anomalies tab.</div>
        )}
        {b && tab === "anomalies" && <AnomalyList list={b.anomalies} />}
        {b && tab === "freq" && (
          <>
            <p className="muted">Closed-loop response setpoint → gyro from the CHIRP sweeps. Orange solid = |H|, dashed = open loop |L|, grey = sensitivity. The dashed markers are the −3 dB bandwidth (orange) and the gain crossover (green). Coherence below 0.5 means the gyro did not follow the sweep there.</p>
            <FrMetricsTable list={b.freq_resp} />
            {b.freq_resp.map((fr) => (
              <div key={fr.axis} data-report={`Frequency response ${fr.axis}`}><BodeChart fr={fr} /></div>
            ))}
          </>
        )}
        {b && tab === "step" && (
          <section>
            <h2>Step Response Functions</h2>
            <StepControls />
            {AXES.map((a) => {
              const st = b.steps.find((x) => x.axis === a)!;
              return <StepResponseChart key={a} step={st} smoothMs={s.stepSmoothMs} showBand={s.stepBand} />;
            })}
          </section>
        )}
        {b && tab === "spectrum" && (
          <section>
            <h2>Full Spectrum</h2>
            {AXES.map((a: Axis) => (
              <div className="chart-card" key={a}>
                <SpectrumChart
                  axis={a}
                  filt={b.spectra.find((x) => x.axis === a && x.kind === "gyro_filt") ?? null}
                  raw={b.spectra.find((x) => x.axis === a && x.kind === "gyro_raw") ?? null}
                  dterm={b.spectra.find((x) => x.axis === a && x.kind === "d_term") ?? null}
                  peaks={b.peaks}
                  maxHz={Math.min(1000, Math.floor(b.quality.fs_hz / 2))}
                />
              </div>
            ))}
          </section>
        )}
        {b && tab === "spectrogram" && (
          <section>
            <h2>Throttle vs Frequency</h2>
            {b.spectrograms.map((sg) => (
              <div className="chart-card" key={sg.axis}>
                <SpectrogramCanvas sg={sg} />
              </div>
            ))}
          </section>
        )}
        {b && tab === "recs" && (
          <section>
            <h2>Recommendations</h2>
            <div className="quality">
              fs {b.quality.fs_hz.toFixed(0)} Hz · {b.quality.duration_s.toFixed(1)} s · hover {b.quality.hover_seconds.toFixed(1)} s @ {b.quality.hover_throttle_pct.toFixed(0)} % ·
              saturation {b.quality.motor_saturation_pct.toFixed(2)} % · raw gyro {b.quality.has_gyro_raw ? "yes" : "no"} ·
              steps R/P/Y {b.quality.step_segments_per_axis.join("/")}
            </div>
            {s.recs.length === 0 && <div className="empty">No changes suggested from this log.</div>}
            <table className="recs">
              <thead>
                <tr><th>Parameter</th><th>Current</th><th>Proposed</th><th>Why</th><th>Confidence</th></tr>
              </thead>
              <tbody>
                {s.recs.map((r) => (
                  <tr key={r.id}>
                    <td><code>{r.param.name}</code></td>
                    <td>{paramValueText(r.old)}</td>
                    <td><b>{paramValueText(r.new)}</b></td>
                    <td>{r.reason}</td>
                    <td className={`conf ${r.confidence}`}>{r.confidence}</td>
                  </tr>
                ))}
              </tbody>
            </table>
            {s.recs.length > 0 && (
              <pre className="cli">
                {"# Betaflight CLI\n" + s.recs.map((r) => `set ${r.param.name} = ${paramValueText(r.new)}`).join("\n") + "\nsave"}
              </pre>
            )}
          </section>
        )}
      </main>
      {s.ai.open && b && <aside className="ai-drawer"><AiPanel scope="quick" /></aside>}
      </div>
    </div>
  );
}
