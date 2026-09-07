import { useState } from "react";
import { api } from "../lib/api";
import { run, useStore } from "../lib/store";
import type { GuardResult } from "../lib/types";

export default function GuardList({ guards }: { guards: GuardResult[] }) {
  const s = useStore();
  const [overriding, setOverriding] = useState<string | null>(null);
  const [reason, setReason] = useState("");

  async function doOverride(id: string) {
    const snap = await run("Saving override…", () => api.override(id, reason));
    if (snap) {
      s.set({ snap });
      setOverriding(null);
      setReason("");
    }
  }

  return (
    <div className="guards">
      {guards.map((g) => {
        const ok = g.outcome.kind === "pass";
        const cls = ok ? "pass" : g.overridden ? "overridden" : g.outcome.kind === "fail" ? "fail" : "action";
        return (
          <div key={g.id} className={`guard ${cls}`}>
            <span className="guard-icon">{ok ? "✓" : g.overridden ? "⚠" : g.outcome.kind === "fail" ? "✕" : "○"}</span>
            <div className="guard-body">
              <div className="guard-title">{g.title}{g.overridden && <em> — overridden</em>}</div>
              {g.outcome.kind !== "pass" && <div className="guard-msg">{g.outcome.message}</div>}
              {g.outcome.kind === "fail" && g.outcome.fix_hint && <div className="guard-hint">{g.outcome.fix_hint}</div>}
              {overriding === g.id && (
                <div className="override-box">
                  <input autoFocus value={reason} onChange={(e) => setReason(e.target.value)} placeholder="Why is it acceptable to continue? (recorded in the report)" />
                  <button className="primary" disabled={reason.trim().length < 5} onClick={() => doOverride(g.id)}>Override</button>
                  <button onClick={() => setOverriding(null)}>Cancel</button>
                </div>
              )}
            </div>
            {!ok && !g.overridden && g.can_override && overriding !== g.id && (
              <button className="small" onClick={() => setOverriding(g.id)}>Override…</button>
            )}
          </div>
        );
      })}
    </div>
  );
}
