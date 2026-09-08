import { AXIS_LABEL, type FrequencyResponse } from "../lib/types";

const f = (v: number | null | undefined, d = 1, unit = "") => (v == null || !Number.isFinite(v) ? "—" : `${v.toFixed(d)}${unit}`);

/** Per-axis metrics of the CHIRP frequency response, incl. the phase-margin targets. */
export default function FrMetricsTable({ list, before }: { list: FrequencyResponse[]; before?: FrequencyResponse[] }) {
  if (!list.length) return null;
  const row = (fr: FrequencyResponse, cmp?: FrequencyResponse) => {
    const m = fr.metrics;
    const b = cmp?.metrics;
    const pair = (a: number | null, bb: number | null | undefined, d: number, unit = "") => (b ? `${f(bb, d)} → ${f(a, d, unit)}` : f(a, d, unit));
    return (
      <tr key={fr.axis}>
        <td>{AXIS_LABEL[fr.axis]}{fr.angle_mode ? " (ANGLE)" : ""}</td>
        <td>{fr.n_sweeps} / {fr.n_windows}</td>
        <td>{pair(m.coherence_mean, b?.coherence_mean, 2)}</td>
        <td>{pair(m.bandwidth_hz, b?.bandwidth_hz, 1, " Hz")}</td>
        <td>{pair(m.crossover_hz, b?.crossover_hz, 1, " Hz")}</td>
        <td>{pair(m.phase_margin_deg, b?.phase_margin_deg, 0, "°")}</td>
        <td>{pair(m.resonant_peak_db, b?.resonant_peak_db, 1, " dB")}</td>
        <td>{pair(m.loop_delay_ms, b?.loop_delay_ms, 2, " ms")}</td>
        <td>{pair(m.sens_peak_db, b?.sens_peak_db, 1, " dB")}</td>
        <td>{m.targets.map((t) => `${t.pm_deg}°: ${f(t.crossover_hz, 0)} Hz ×${f(t.gain_to_target, 2)}`).join(" · ")}</td>
      </tr>
    );
  };
  return (
    <table className="recs fr-metrics">
      <thead>
        <tr><th>Axis</th><th>Sweeps / windows</th><th>Coherence</th><th>Bandwidth</th><th>Crossover</th><th>Phase margin</th><th>Resonant peak</th><th>Loop delay</th><th>Sensitivity peak</th><th>Targets (PM: crossover, gain)</th></tr>
      </thead>
      <tbody>{list.map((fr) => row(fr, before?.find((x) => x.axis === fr.axis)))}</tbody>
    </table>
  );
}
