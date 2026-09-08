import { useMemo } from "react";
import uPlot from "uplot";
import UPlotChart from "./UPlotChart";
import { AXIS_LABEL, type FrequencyResponse } from "../lib/types";

const ORANGE = "#e8891d";
const BLUE = "#3a7bd5";
const GREY = "#888";
const GREEN = "#1a9c4c";

interface Props {
  fr: FrequencyResponse;
  /** Optional "before" response drawn in blue (closed loop only). */
  compare?: FrequencyResponse | null;
  height?: number;
  maxHz?: number;
}

export function chirpReliability(fr: FrequencyResponse): { label: string; cls: string } {
  const c = fr.metrics.coherence_mean ?? 0;
  if (fr.angle_mode) return { label: "ANGLE mode", cls: "bad" };
  if (fr.n_windows < 8 || c < 0.6) return { label: "unreliable", cls: "bad" };
  if (c < 0.8) return { label: "fair", cls: "warn" };
  return { label: "good", cls: "good" };
}

const fmt = (v: number | null | undefined, d = 1, unit = "") => (v == null || !Number.isFinite(v) ? "—" : `${v.toFixed(d)}${unit}`);

/** Three stacked uPlot panels sharing a log-x axis: magnitude, phase, coherence. */
export default function BodeChart({ fr, compare, height = 170, maxHz = 500 }: Props) {
  const xScale = useMemo<uPlot.Scale>(() => ({ time: false, distr: 3, log: 10, range: [1, maxHz] }), [maxHz]);
  const xAxis = (label: string): uPlot.Axis => ({ label, stroke: "#222", grid: { stroke: "#ddd", width: 1 }, ticks: { stroke: "#bbb" }, values: (_u, vals) => vals.map((v) => (v >= 1 ? `${v}` : "")) });
  const m = fr.metrics;
  const marks = (u: uPlot) => {
    const ctx = u.ctx;
    ctx.save();
    ctx.setLineDash([4, 4]);
    for (const [x, color] of [[m.bandwidth_hz, ORANGE], [m.crossover_hz, GREEN]] as [number | null, string][]) {
      if (x == null || !Number.isFinite(x) || x < 1) continue;
      const px = u.valToPos(x, "x", true);
      ctx.strokeStyle = color;
      ctx.beginPath();
      ctx.moveTo(px, u.bbox.top);
      ctx.lineTo(px, u.bbox.top + u.bbox.height);
      ctx.stroke();
    }
    ctx.restore();
  };

  const magOpts = useMemo<Omit<uPlot.Options, "width" | "height">>(
    () => ({
      cursor: { drag: { x: false, y: false } },
      legend: { show: true },
      scales: { x: xScale, y: { range: [-40, 20] } },
      axes: [xAxis(""), { label: `${AXIS_LABEL[fr.axis]} | Magnitude (dB)`, stroke: "#222", grid: { stroke: "#ddd", width: 1 }, size: 60 }],
      series: [
        {},
        { label: "|H| closed loop", stroke: ORANGE, width: 2, points: { show: false } },
        { label: "|L| open loop", stroke: ORANGE, width: 1.5, dash: [6, 4], points: { show: false } },
        { label: "|S| sensitivity", stroke: GREY, width: 1, points: { show: false } },
        { label: "|H| before", stroke: BLUE, width: 1.5, points: { show: false }, show: !!compare },
      ],
      hooks: { draw: [marks] },
    }),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [fr.axis, xScale, !!compare, m.bandwidth_hz, m.crossover_hz],
  );
  const phaseOpts = useMemo<Omit<uPlot.Options, "width" | "height">>(
    () => ({
      cursor: { drag: { x: false, y: false } },
      legend: { show: true },
      scales: { x: xScale, y: { range: [-360, 90] } },
      axes: [xAxis(""), { label: "Phase (°)", stroke: "#222", grid: { stroke: "#ddd", width: 1 }, size: 60 }],
      series: [
        {},
        { label: "∠H", stroke: ORANGE, width: 2, points: { show: false } },
        { label: "∠L (unwrapped)", stroke: ORANGE, width: 1.5, dash: [6, 4], points: { show: false } },
        { label: "−180°", stroke: "#333", width: 1, dash: [2, 4], points: { show: false } },
      ],
      hooks: { draw: [marks] },
    }),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [xScale, m.bandwidth_hz, m.crossover_hz],
  );
  const cohOpts = useMemo<Omit<uPlot.Options, "width" | "height">>(
    () => ({
      cursor: { drag: { x: false, y: false } },
      legend: { show: true },
      scales: { x: xScale, y: { range: [0, 1] } },
      axes: [xAxis("Frequency (Hz)"), { label: "Coherence γ²", stroke: "#222", grid: { stroke: "#ddd", width: 1 }, size: 60 }],
      series: [
        {},
        { label: "γ²", stroke: GREEN, width: 2, points: { show: false }, fill: "rgba(26,156,76,0.12)" },
        { label: "0.5 gate", stroke: "#c62828", width: 1, dash: [2, 4], points: { show: false } },
      ],
      hooks: { draw: [marks] },
    }),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [xScale, m.bandwidth_hz, m.crossover_hz],
  );

  const { magData, phaseData, cohData } = useMemo(() => {
    const keep = fr.f_hz.map((f) => f >= 1);
    const pick = <T,>(arr: T[]) => arr.filter((_, i) => keep[i]);
    const f = pick(fr.f_hz);
    const nn = (a: (number | null)[]) => a.map((v) => (v == null || !Number.isFinite(v) ? null : v));
    const cmp = compare ? alignTo(compare.f_hz, compare.h_mag_db, f) : new Array(f.length).fill(null);
    return {
      magData: [f, nn(pick(fr.h_mag_db)), nn(pick(fr.l_mag_db)), nn(pick(fr.s_mag_db)), cmp] as uPlot.AlignedData,
      phaseData: [f, nn(pick(fr.h_phase_deg)), nn(pick(fr.l_phase_deg)), new Array(f.length).fill(-180)] as uPlot.AlignedData,
      cohData: [f, nn(pick(fr.coherence)), new Array(f.length).fill(0.5)] as uPlot.AlignedData,
    };
  }, [fr, compare]);

  const rel = chirpReliability(fr);
  return (
    <div className="chart-card bode">
      <div className="chart-side">
        <div className="chart-side-title">{AXIS_LABEL[fr.axis]}</div>
        <Metric label="sweeps / windows" value={`${fr.n_sweeps} / ${fr.n_windows}`} warn={fr.n_windows < 8} />
        <div className={`rel ${rel.cls}`}>{rel.label}</div>
        <Metric label="coherence" value={fmt(m.coherence_mean, 2)} warn={(m.coherence_mean ?? 0) < 0.6} />
        <Metric label="bandwidth" value={fmt(m.bandwidth_hz, 1, " Hz")} />
        <Metric label="crossover" value={fmt(m.crossover_hz, 1, " Hz")} />
        <Metric label="phase margin" value={fmt(m.phase_margin_deg, 0, "°")} warn={m.phase_margin_deg != null && m.phase_margin_deg < 45} />
        <Metric label="resonant peak" value={fmt(m.resonant_peak_db, 1, " dB")} warn={m.resonant_peak_db != null && m.resonant_peak_db > 3} />
        <Metric label="loop delay" value={fmt(m.loop_delay_ms, 2, " ms")} />
        <Metric label="sensitivity peak" value={fmt(m.sens_peak_db, 1, " dB")} warn={m.sens_peak_db != null && m.sens_peak_db > 6} />
        {fr.angle_mode && <div className="chart-side-warn">Flown in ANGLE mode: attitude loop included</div>}
      </div>
      <div className="chart-plot bode-panels">
        <UPlotChart opts={magOpts} data={magData} height={height} />
        <UPlotChart opts={phaseOpts} data={phaseData} height={height * 0.8} />
        <UPlotChart opts={cohOpts} data={cohData} height={height * 0.6} />
      </div>
    </div>
  );
}

function Metric({ label, value, warn }: { label: string; value: string; warn?: boolean }) {
  return (
    <div className={"metric" + (warn ? " warn" : "")}>
      <span>{label}</span>
      <b>{value}</b>
    </div>
  );
}

function alignTo(srcF: number[], srcY: (number | null)[], f: number[]): (number | null)[] {
  if (srcF.length < 2) return new Array(f.length).fill(null);
  const df = srcF[1] - srcF[0];
  return f.map((x) => {
    const i = Math.round(x / df);
    const v = i < srcY.length ? srcY[i] : null;
    return v == null || !Number.isFinite(v) ? null : v;
  });
}
