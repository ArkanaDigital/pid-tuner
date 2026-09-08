import { describe, expect, it } from "vitest";
import { estimateCost, formatTokens, formatUsd } from "./cost";

describe("estimateCost (parity with appconfig::estimate_cost)", () => {
  it("matches the Rust reference numbers", () => {
    const p = { input_per_m: 3, output_per_m: 15, cached_input_per_m: 0.3 };
    // 800×3 + 200×0.3 + 500×15 = 9960 per 1e6
    expect(estimateCost({ input: 1000, output: 500, cached: 200, reasoning: 0 }, p)!).toBeCloseTo(0.00996, 9);
    const p2 = { input_per_m: 3, output_per_m: 15, cached_input_per_m: null };
    // cached capped at input, cached rate falls back to input rate
    expect(estimateCost({ input: 1000, output: 0, cached: 5000, reasoning: 0 }, p2)!).toBeCloseTo(0.003, 12);
  });
  it("returns null without a price", () => {
    expect(estimateCost({ input: 1, output: 1, cached: 0, reasoning: 0 }, null)).toBeNull();
    expect(estimateCost({ input: 1, output: 1, cached: 0, reasoning: 0 }, undefined)).toBeNull();
  });
});

describe("formatters", () => {
  it("formatTokens", () => {
    expect(formatTokens(0)).toBe("0");
    expect(formatTokens(999)).toBe("999");
    expect(formatTokens(1500)).toBe("1.5k");
    expect(formatTokens(12_345)).toBe("12k");
    expect(formatTokens(2_500_000)).toBe("2.50M");
    expect(formatTokens(NaN)).toBe("—");
  });
  it("formatUsd", () => {
    expect(formatUsd(null)).toBe("—");
    expect(formatUsd(0.00123)).toBe("$0.0012");
    expect(formatUsd(0.5)).toBe("$0.500");
    expect(formatUsd(Infinity)).toBe("—");
  });
});
