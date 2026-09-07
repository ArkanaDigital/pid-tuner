import { useEffect, useRef } from "react";
import { AXIS_LABEL, type Spectrogram } from "../lib/types";

interface Props {
  sg: Spectrogram;
  height?: number;
  dbMin?: number;
  dbMax?: number;
}

/** Throttle (y, %) vs frequency (x, Hz) heat map, PIDtoolbox style. */
export default function SpectrogramCanvas({ sg, height = 260, dbMin = -45, dbMax = 5 }: Props) {
  const ref = useRef<HTMLCanvasElement>(null);
  useEffect(() => {
    const cv = ref.current;
    if (!cv) return;
    const nf = sg.f_hz.length;
    const nb = sg.throttle_bins.length;
    const img = new ImageData(nf, nb);
    for (let b = 0; b < nb; b++) {
      const row = nb - 1 - b; // throttle increases upward
      for (let k = 0; k < nf; k++) {
        const v = sg.counts[b] > 0 ? (sg.db[b * nf + k] - dbMin) / (dbMax - dbMin) : 0;
        const [r, g, bl] = turbo(Math.max(0, Math.min(1, v)));
        const o = (row * nf + k) * 4;
        img.data[o] = r;
        img.data[o + 1] = g;
        img.data[o + 2] = bl;
        img.data[o + 3] = 255;
      }
    }
    const off = document.createElement("canvas");
    off.width = nf;
    off.height = nb;
    off.getContext("2d")!.putImageData(img, 0, 0);
    const W = cv.clientWidth || 600;
    cv.width = W * devicePixelRatio;
    cv.height = height * devicePixelRatio;
    const ctx = cv.getContext("2d")!;
    ctx.scale(devicePixelRatio, devicePixelRatio);
    const L = 48, B = 28, T = 8, R = 10;
    ctx.fillStyle = "#fff";
    ctx.fillRect(0, 0, W, height);
    ctx.imageSmoothingEnabled = false;
    ctx.drawImage(off, L, T, W - L - R, height - T - B);
    ctx.strokeStyle = "#333";
    ctx.strokeRect(L, T, W - L - R, height - T - B);
    ctx.fillStyle = "#222";
    ctx.font = "11px sans-serif";
    ctx.textAlign = "center";
    const fmax = sg.f_hz[nf - 1];
    for (let f = 0; f <= fmax; f += 100) {
      const x = L + ((W - L - R) * f) / fmax;
      ctx.fillText(String(f), x, height - B + 14);
      ctx.beginPath(); ctx.moveTo(x, height - B); ctx.lineTo(x, height - B + 4); ctx.stroke();
    }
    ctx.fillText("Frequency (Hz)", L + (W - L - R) / 2, height - 4);
    ctx.textAlign = "right";
    for (let t = 0; t <= 100; t += 25) {
      const y = T + ((height - T - B) * (100 - t)) / 100;
      ctx.fillText(`${t}%`, L - 6, y + 4);
    }
    ctx.save();
    ctx.translate(12, T + (height - T - B) / 2);
    ctx.rotate(-Math.PI / 2);
    ctx.textAlign = "center";
    ctx.fillText(`${AXIS_LABEL[sg.axis]} throttle`, 0, 0);
    ctx.restore();
  }, [sg, height, dbMin, dbMax]);
  return <canvas ref={ref} style={{ width: "100%", height }} />;
}

/** Google "turbo" colormap polynomial approximation. */
function turbo(x: number): [number, number, number] {
  const r = 34.61 + x * (1172.33 - x * (10793.56 - x * (33300.12 - x * (38394.49 - x * 14825.05))));
  const g = 23.31 + x * (557.33 + x * (1225.33 - x * (3574.96 - x * (1073.77 + x * 707.56))));
  const b = 27.2 + x * (3211.1 - x * (15327.97 - x * (27814 - x * (22569.18 - x * 6838.66))));
  return [clamp(r), clamp(g), clamp(b)];
}
function clamp(v: number) {
  return Math.max(0, Math.min(255, Math.round(v)));
}
