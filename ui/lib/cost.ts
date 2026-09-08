import type { ModelPrice, Usage } from "./types";

/** Same formula as `appconfig::estimate_cost` (USD). */
export function estimateCost(u: Usage, price: ModelPrice | null | undefined): number | null {
  if (!price) return null;
  const cached = Math.min(u.cached, u.input);
  const uncached = u.input - cached;
  const cachedRate = price.cached_input_per_m ?? price.input_per_m;
  return (uncached * price.input_per_m + cached * cachedRate + u.output * price.output_per_m) / 1e6;
}

export function formatTokens(n: number): string {
  if (!Number.isFinite(n)) return "—";
  if (n < 1000) return `${n}`;
  if (n < 1_000_000) return `${(n / 1000).toFixed(n < 10_000 ? 1 : 0)}k`;
  return `${(n / 1_000_000).toFixed(2)}M`;
}

export function formatUsd(x: number | null | undefined): string {
  if (x == null || !Number.isFinite(x)) return "—";
  if (x < 0.01) return `$${x.toFixed(4)}`;
  return `$${x.toFixed(3)}`;
}
