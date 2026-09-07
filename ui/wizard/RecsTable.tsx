import { api } from "../lib/api";
import { run, useStore } from "../lib/store";
import { paramValueText, type ApplyPhase, type ParamValue, type Recommendation } from "../lib/types";

export function cliText(recs: Recommendation[]): string {
  const lines = recs.filter((r) => r.accepted).map((r) => `set ${r.param.name} = ${paramValueText(r.new)}`);
  return lines.length ? `# PID Tuner\n${lines.join("\n")}\nsave` : "";
}

export default function RecsTable({ phase, recs, editable }: { phase: ApplyPhase; recs: Recommendation[]; editable: boolean }) {
  const s = useStore();

  async function toggle(r: Recommendation) {
    const snap = await run("Saving…", () => api.recsSet(phase, r.id, !r.accepted));
    if (snap) s.set({ snap });
  }
  async function edit(r: Recommendation, text: string) {
    let v: ParamValue;
    if (r.new.type === "bool") v = { type: "bool", value: /^(1|on|true)$/i.test(text) };
    else {
      const n = Number(text);
      if (!Number.isFinite(n)) return;
      v = { ...r.new, value: n } as ParamValue;
    }
    const snap = await run("Saving…", () => api.recsSet(phase, r.id, r.accepted, v));
    if (snap) s.set({ snap });
  }

  if (recs.length === 0) return <div className="empty">No changes suggested — this part of the tune looks fine.</div>;
  return (
    <table className="recs">
      <thead>
        <tr>{editable && <th />}<th>Parameter</th><th>Current</th><th>Proposed</th><th>Why</th><th>Confidence</th></tr>
      </thead>
      <tbody>
        {recs.map((r) => (
          <tr key={r.id} className={r.accepted ? "" : "off"}>
            {editable && <td><input type="checkbox" checked={r.accepted} onChange={() => toggle(r)} /></td>}
            <td><code>{r.param.name}</code></td>
            <td>{paramValueText(r.old)}</td>
            <td>
              {editable ? (
                <input className="num" defaultValue={paramValueText(r.new)} onBlur={(e) => edit(r, e.target.value)} />
              ) : (
                <b>{paramValueText(r.new)}</b>
              )}
            </td>
            <td>
              {r.reason}
              {r.evidence.map((e, i) => (
                <span className="evidence" key={i}>
                  {e.kind === "peak" && ` · ${e.axis} ${e.f_hz.toFixed(0)} Hz ${e.psd_db.toFixed(0)} dB`}
                  {e.kind === "step" && ` · ${e.axis} overshoot ${e.overshoot.toFixed(2)} / ${e.latency_ms.toFixed(0)} ms`}
                </span>
              ))}
            </td>
            <td className={`conf ${r.confidence}`}>{r.confidence}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}
