import { useMemo } from "react";
import uPlot from "uplot";
import UPlotChart from "./UPlotChart";
import { AXIS_LABEL, type Axis, type NoisePeak, type Spectrum } from "../lib/types";

const RED = "#d42a2a";
const GREY = "#666";

interface Props {
  axis: Axis;
  filt: Spectrum | null;
  raw: Spectrum | null;
  dterm?: Spectrum | null;
  peaks?: NoisePeak[];
  predicted?: Spectrum | null;
  maxHz?: number;
  height?: number;
}

/** PIDtoolbox "Full Spectrum": gyro (solid) vs gyro prefilt (dashed), PSD in dB. */
export default function SpectrumChart({ axis, filt, raw, dterm, peaks = [], predicted, maxHz = 1000, height = 240 }: Props) {
  const opts = useMemo<Omit<uPlot.Options, "width" | "height">>(
    () => ({
      cursor: { drag: { x: true, y: false } },
      legend: { show: true },
      scales: { x: { time: false, range: [0, maxHz] }, y: { range: [-50, 20] } },
      axes: [
        { label: "Frequency (Hz)", stroke: "#222", grid: { stroke: "#ddd", width: 1 } },
        { label: `${AXIS_LABEL[axis]} | Pwr. Spec. Density (dB)`, stroke: "#222", grid: { stroke: "#ddd", width: 1 }, size: 60 },
      ],
      series: [
        {},
        { label: "Gyro", stroke: RED, width: 1.5, points: { show: false } },
        { label: "Gyro prefilt", stroke: RED, width: 1.5, dash: [4, 4], points: { show: false } },
        { label: "D-term", stroke: GREY, width: 1, points: { show: false }, show: !!dterm },
        { label: "Predicted", stroke: "#1a9c4c", width: 1.5, points: { show: false }, show: !!predicted },
      ],
      hooks: {
        draw: [
          (u) => {
            const ctx = u.ctx;
            ctx.save();
            ctx.font = `${11 * devicePixelRatio}px sans-serif`;
            ctx.fillStyle = "#333";
            for (const p of peaks) {
              if (p.axis !== axis) continue;
              const x = u.valToPos(p.f_hz, "x", true);
              const y = u.valToPos(p.psd_db, "y", true);
              ctx.beginPath();
              ctx.arc(x, y, 3 * devicePixelRatio, 0, Math.PI * 2);
              ctx.strokeStyle = p.kind === "gyro_filt" ? RED : "#999";
              ctx.stroke();
              ctx.fillText(`${p.f_hz.toFixed(0)} Hz`, x + 5 * devicePixelRatio, y - 5 * devicePixelRatio);
            }
            ctx.restore();
          },
        ],
      },
    }),
    [axis, maxHz, peaks, !!dterm, !!predicted],
  );

  const data = useMemo<uPlot.AlignedData>(() => {
    const base = filt ?? raw;
    if (!base) return [[], [], [], [], []];
    const f = base.f_hz;
    const align = (s: Spectrum | null | undefined) => (s ? alignTo(s, f) : new Array(f.length).fill(null));
    return [f, align(filt), align(raw), align(dterm), align(predicted)];
  }, [filt, raw, dterm, predicted]);

  return <UPlotChart opts={opts} data={data} height={height} className="chart-plot" />;
}

function alignTo(s: Spectrum, f: number[]): (number | null)[] {
  if (s.f_hz.length === f.length) return s.psd_db;
  const df = s.f_hz[1] - s.f_hz[0];
  return f.map((x) => {
    const i = Math.round(x / df);
    return i < s.psd_db.length ? s.psd_db[i] : null;
  });
}
