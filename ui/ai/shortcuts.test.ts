import { describe, expect, it } from "vitest";
import { STEPS } from "../lib/types";
import { QUICK_SHORTCUTS, SHORTCUTS } from "./shortcuts";

describe("shortcuts", () => {
  it("every wizard step has at least one shortcut with unique ids", () => {
    for (const step of STEPS) {
      const list = SHORTCUTS[step];
      expect(list, step).toBeDefined();
      expect(list.length, step).toBeGreaterThan(0);
      const ids = new Set(list.map((s) => s.id));
      expect(ids.size, step).toBe(list.length);
      for (const s of list) {
        expect(s.label.trim().length).toBeGreaterThan(0);
        expect(s.prompt.trim().length).toBeGreaterThan(20);
      }
    }
    expect(Object.keys(SHORTCUTS).sort()).toEqual([...STEPS].sort());
  });
  it("quick look has its own shortcuts", () => {
    expect(QUICK_SHORTCUTS.length).toBeGreaterThan(0);
    expect(new Set(QUICK_SHORTCUTS.map((s) => s.id)).size).toBe(QUICK_SHORTCUTS.length);
  });
});
