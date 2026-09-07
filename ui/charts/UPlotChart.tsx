import { useEffect, useRef } from "react";
import uPlot from "uplot";

interface Props {
  opts: Omit<uPlot.Options, "width" | "height">;
  data: uPlot.AlignedData;
  height: number;
  className?: string;
}

/** Thin React wrapper: recreates the plot when opts change, setData when data changes, resizes with container. */
export default function UPlotChart({ opts, data, height, className }: Props) {
  const el = useRef<HTMLDivElement>(null);
  const plot = useRef<uPlot | null>(null);

  useEffect(() => {
    if (!el.current) return;
    const width = el.current.clientWidth || 600;
    plot.current?.destroy();
    plot.current = new uPlot({ ...opts, width, height }, data, el.current);
    const ro = new ResizeObserver(() => {
      if (el.current && plot.current) plot.current.setSize({ width: el.current.clientWidth, height });
    });
    ro.observe(el.current);
    return () => {
      ro.disconnect();
      plot.current?.destroy();
      plot.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [opts, height]);

  useEffect(() => {
    plot.current?.setData(data);
  }, [data]);

  return <div ref={el} className={className} />;
}
