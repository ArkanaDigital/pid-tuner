import { api } from "../lib/api";
import { run, useStore } from "../lib/store";
import GuardList from "./GuardList";
import { StepPanel } from "./steps";
import { FcBadge } from "./fcsteps";
import AiPanel from "../ai/AiPanel";
import { useEffect } from "react";
import type { Step } from "../lib/types";

export default function Wizard() {
  const s = useStore();
  const snap = s.snap;
  const online = snap?.session.mode === "online";
  useEffect(() => {
    if (!online) return;
    let stop = false;
    const tick = async () => {
      try {
        const fc = await api.fcPoll();
        if (stop) return;
        const prev = useStore.getState().fc;
        useStore.getState().set({ fc });
        if (prev?.connected !== fc.connected || prev?.armed !== fc.armed) {
          const n = await api.sessionSnapshot();
          if (!stop) useStore.getState().set({ snap: n });
        }
      } catch {
        /* ignore */
      }
    };
    tick();
    const id = setInterval(tick, 1500);
    return () => {
      stop = true;
      clearInterval(id);
    };
  }, [online]);
  if (!snap) return null;
  const cur = snap.steps.find((x) => x.step === snap.session.current)!;

  async function next() {
    const n = await run("Checking…", api.next);
    if (n) s.set({ snap: n });
  }
  async function back() {
    const n = await run("…", api.back);
    if (n) s.set({ snap: n });
  }
  async function goto(step: Step) {
    const n = await run("…", () => api.goto(step));
    if (n) s.set({ snap: n });
  }

  return (
    <div className="wizard">
      <header className="topbar">
        <button onClick={() => s.set({ view: "home", snap: null })}>← Sessions</button>
        <b>{snap.session.name}</b>
        <span className="muted">{snap.session.mode} mode</span>
        {online && <FcBadge />}
        <span className="spacer" />
        {s.busy && <span className="busy">{s.busy}</span>}
        {s.error && <span className="error">{s.error}</span>}
        <button className="small" onClick={() => s.setAi({ open: !s.ai.open })}>{s.ai.open ? "Sembunyikan AI" : "AI helper"}</button>
        <button className="small" onClick={() => s.set({ prevView: "wizard", view: "settings" })}>⚙</button>
      </header>
      <div className={`wizard-body ${s.ai.open ? "with-ai" : ""}`}>
        <aside className="stepper">
          {snap.steps.map((st, i) => (
            <button
              key={st.step}
              className={`step ${st.status} ${st.step === snap.session.current ? "current" : ""}`}
              disabled={st.status === "locked"}
              onClick={() => st.step !== snap.session.current && goto(st.step)}
            >
              <span className="step-no">{st.status === "passed" ? "✓" : st.status === "skipped" ? "–" : i + 1}</span>
              <span className="step-title">{st.title}</span>
            </button>
          ))}
        </aside>
        <main className="wizard-main">
          <StepPanel snap={snap} />
        </main>
        <aside className="guard-pane">
          <h3>Before you continue</h3>
          <GuardList guards={cur.guards} />
          {s.ai.open && <AiPanel scope="session" step={cur.step} />}
          <div className="nav">
            <button onClick={back} disabled={!snap.can_back || !!s.busy}>← Back</button>
            <button className="primary" onClick={next} disabled={!snap.can_next || !!s.busy}>
              {snap.session.current === "report" ? "Finish" : "Next →"}
            </button>
          </div>
        </aside>
      </div>
    </div>
  );
}
