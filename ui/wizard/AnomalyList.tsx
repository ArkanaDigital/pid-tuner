import { ANOMALY_TITLE, type Anomaly } from "../lib/types";

function fmtT(a: Anomaly): string {
  const d = a.t_end_s - a.t_start_s;
  return d < 0.05 ? `${a.t_start_s.toFixed(2)} s` : `${a.t_start_s.toFixed(1)} – ${a.t_end_s.toFixed(1)} s`;
}

function where(a: Anomaly): string {
  if (a.motor != null) return `motor ${a.motor + 1}`;
  if (a.axis) return a.axis;
  return "";
}

/** Compact summary line: "2 critical · 3 warnings" (empty string when clean). */
export function anomalySummary(list: Anomaly[] | undefined): string {
  if (!list || list.length === 0) return "";
  const c = list.filter((a) => a.severity === "critical").length;
  const w = list.filter((a) => a.severity === "warning").length;
  const parts = [];
  if (c) parts.push(`${c} critical`);
  if (w) parts.push(`${w} warning${w > 1 ? "s" : ""}`);
  return parts.join(" · ");
}

export default function AnomalyList({ list, compact }: { list: Anomaly[] | undefined; compact?: boolean }) {
  if (!list || list.length === 0) {
    return <div className="notice ok">No anomalies detected — no desync, clipping, un-commanded oscillation, yaw spin, reversed control, RPM dropout, motor imbalance, heavy vibration or log gaps.</div>;
  }
  const shown = compact ? list.slice(0, 5) : list;
  return (
    <div className="anomalies">
      {list.some((a) => a.severity === "critical") && (
        <div className="notice bad"><b>Critical anomalies</b> — this log points at a hardware problem; fix it before tuning on this data.</div>
      )}
      <table className="recs">
        <thead><tr><th>Severity</th><th>Type</th><th>Where</th><th>When</th><th>Detail</th></tr></thead>
        <tbody>
          {shown.map((a, i) => (
            <tr key={i}>
              <td className={`sev ${a.severity}`}>{a.severity}</td>
              <td><b>{ANOMALY_TITLE[a.kind]}</b></td>
              <td>{where(a)}</td>
              <td>{fmtT(a)}</td>
              <td>{a.detail}</td>
            </tr>
          ))}
        </tbody>
      </table>
      {compact && list.length > shown.length && <div className="muted">+{list.length - shown.length} more in the Anomalies tab.</div>}
    </div>
  );
}
