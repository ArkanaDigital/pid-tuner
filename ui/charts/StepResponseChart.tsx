import { useMemo } from "react";
import uPlot from "uplot";
import UPlotChart from "./UPlotChart";
import { AXIS_LABEL, type StepResponse } from "../lib/types";

const ORANGE = "#e8891d";
const ORANGE_FILL = "rgba(232,137,29,0.18)";
const BLUE = "#3a7bd5";

interface Props {
  step: StepResponse;
  /** Optional second response drawn in blue (e.g. "before"). */
  compare?: StepResponse | null;
  yMax?: number;
  height?: number;
  /** Moving-average width in ms applied for display (PIDtoolbox smoothFactor). */
  smoothMs?: number;
  /** Show the 10–90 % spread band. */
  showBand?: boolean;
}

function smooth(y: number[], fsHz: number, ms: number): number[] {
  const w = Math.max(1, Math.round((ms / 1000) * fsHz));
  if (w <= 1) return y;
  const half = Math.floor(w / 2);
  const out = new Array(y.length);
  for (let i = 0; i < y.length; i++) {
    const r = Math.min(half, i, y.length - 1 - i);
    let acc = 0;
    for (let j = i - r; j <= i + r; j++) acc += y[j];
    out[i] = acc / (2 * r + 1);
  }
  return out;
}

export function reliability(n: number): { label: string; cls: string } {
  if (n === 0) return { label: "no data", cls: "bad" };
  if (n < 20) return { label: "unreliable", cls: "bad" };
  if (n < 60) return { label: "fair", cls: "warn" };
  return { label: "good", cls: "good" };
}

export default function StepResponseChart({ step, compare, yMax = 1.5, height = 220, smoothMs = 0, showBand = true }: Props) {
  const opts = useMemo<Omit<uPlot.Options, "width" | "height">>(
    () => ({
      title: undefined,
      cursor: { drag: { x: false, y: false } },
      legend: { show: false },
      scales: { x: { time: false, range: [0, 500] }, y: { range: [0, yMax] } },
      axes: [
        { label: "Time (ms)", stroke: "#222", grid: { stroke: "#ddd", width: 1 }, ticks: { stroke: "#bbb" } },
        {
          label: `${AXIS_LABEL[step.axis]} Response`,
          stroke: "#222",
          grid: { stroke: "#ddd", width: 1 },
          ticks: { stroke: "#bbb" },
          values: (_u, vals) => vals.map((v) => v.toFixed(2)),
          size: 60,
        },
      ],
      series: [
        {},
        { label: "1.0", stroke: "#333", width: 1, dash: [8, 6], points: { show: false } },
        { label: "p10", stroke: "transparent", points: { show: false } },
        { label: "p90", stroke: "transparent", points: { show: false } },
        { label: "mean", stroke: ORANGE, width: 2.5, points: { show: false } },
        { label: "compare", stroke: BLUE, width: 2, points: { show: false } },
      ],
      bands: showBand ? [{ series: [3, 2], fill: ORANGE_FILL }] : [],
    }),
    [step.axis, yMax, showBand],
  );

  const data = useMemo<uPlot.AlignedData>(() => {
    const n = step.t_ms.length;
    const fs = n > 1 ? 1000 / (step.t_ms[1] - step.t_ms[0]) : 1000;
    const ones = new Array(n).fill(1);
    const sm = (y: number[]) => (smoothMs > 0 ? smooth(y, fs, smoothMs) : y);
    const cmp = compare && compare.n_segments > 0 ? resample(compare, step.t_ms).map((v) => v ?? 0) : null;
    return [
      step.t_ms,
      ones,
      showBand ? sm(step.p10) : new Array(n).fill(null),
      showBand ? sm(step.p90) : new Array(n).fill(null),
      step.n_segments > 0 ? sm(step.mean) : new Array(n).fill(null),
      cmp ? sm(cmp) : new Array(n).fill(null),
    ];
  }, [step, compare, smoothMs, showBand]);
  const rel = reliability(step.n_segments);

  return (
    <div className="chart-card">
      <div className="chart-side">
        <div className="chart-side-title">{AXIS_LABEL[step.axis]}</div>
        {step.n_segments === 0 ? (
          <div className="chart-side-warn">No usable stick input</div>
        ) : (
          <>
            <Metric label="segments" value={`${step.n_segments}`} warn={step.n_segments < 20} />
            <div className={`rel ${rel.cls}`}>{rel.label}</div>
            <Metric label="overshoot" value={step.overshoot.toFixed(2)} warn={step.overshoot > 1.15} />
            <Metric label="latency" value={`${step.latency_ms.toFixed(1)} ms`} warn={step.latency_ms > 30} />
            <Metric label="steady" value={step.steady_state.toFixed(2)} warn={Math.abs(step.steady_state - 1) > 0.08} />
          </>
        )}
      </div>
      <UPlotChart opts={opts} data={data} height={height} className="chart-plot" />
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

function resample(src: StepResponse, t: number[]): (number | null)[] {
  return t.map((tm) => {
    const i = Math.round((tm / 1000) * (src.t_ms.length / (src.t_ms[src.t_ms.length - 1] / 1000 || 0.5)));
    return i >= 0 && i < src.mean.length ? src.mean[i] : null;
  });
}
