import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
const { api } = vi.hoisted(() => ({
  api: {
    aiTranscript: vi.fn(async () => ({ turns: [] as unknown[] })),
    aiBudget: vi.fn(async () => ({ provider: "anthropic", model: "claude-sonnet-5", key_set: true, used_tokens: 1500, limit_tokens: 3000, cost_usd: 0.012 })),
    aiChat: vi.fn<(scope: string, text: string) => Promise<unknown>>(),
    aiTranscriptClear: vi.fn(async () => {}),
  },
}));
vi.mock("../lib/api", () => ({ api }));

import AiPanel, { renderLite } from "./AiPanel";
import { useStore } from "../lib/store";
import type { TurnMeta } from "../lib/types";

const turn: TurnMeta = {
  at: "2026-09-08T00:00:00Z",
  step: "import_a",
  provider: "anthropic",
  model: "claude-sonnet-5",
  user_text: "Jelaskan kualitas log ini",
  reply: "Ringkasan:\n1. Rate **4000 Hz**, durasi `62 s`\n2. Hover 40 s\n\n- saturasi 0.1 %\n- anomali: tidak ada",
  usage: { input: 1200, output: 300, cached: 0, reasoning: 0 },
  cost_usd: 0.0081,
  tool_calls: [
    { name: "get_log_quality", args: { flight: "a" }, ok: true, preview: "{...}" },
    { name: "get_anomalies", args: { flight: "a" }, ok: false, preview: "flight a has not been imported yet" },
  ],
  stopped_by: null,
};

beforeEach(() => {
  useStore.setState({ ai: { open: true, turns: [], busy: false, progress: null, budget: null }, settings: null, error: null, recs: [] });
  api.aiChat.mockReset();
});
afterEach(cleanup);

describe("renderLite", () => {
  it("renders paragraphs, numbered and bulleted lists, inline code and bold", () => {
    render(<div>{renderLite(turn.reply)}</div>);
    expect(screen.getAllByRole("list")).toHaveLength(2);
    expect(screen.getAllByRole("listitem")).toHaveLength(4);
    expect(screen.getByText("4000 Hz").tagName).toBe("B");
    expect(screen.getByText("62 s").tagName).toBe("CODE");
  });
  it("does not throw on malformed markdown", () => {
    for (const s of ["", "**unclosed", "`", "1.", "- ", "***", "`a`b`c", "\n\n\n", "1) x\n2) y\n- z"]) {
      expect(() => render(<div>{renderLite(s)}</div>)).not.toThrow();
    }
  });
});

describe("AiPanel", () => {
  it("shows turns with tool badges, usage line and the budget bar", async () => {
    // the panel loads the persisted transcript on mount
    api.aiTranscript.mockResolvedValueOnce({ turns: [turn] });
    await act(async () => {
      render(<AiPanel scope="session" step="import_a" />);
    });
    expect(screen.getByText("Jelaskan kualitas log ini", { selector: ".ai-msg.user" })).toBeTruthy();
    expect(screen.getByText(/get_log_quality/)).toBeTruthy();
    expect(screen.getByText(/get_anomalies/).className).toContain("bad");
    expect(screen.getByText(/in 1.2k · out 300/)).toBeTruthy();
    expect(screen.getByText(/\$0\.0081/)).toBeTruthy();
    // budget from api.aiBudget
    expect(screen.getByText(/1.5k \/ 3.0k/)).toBeTruthy();
    expect(screen.getByRole("progressbar")).toBeTruthy();
    // shortcuts for the step
    expect(screen.getByText("Kenapa guard gagal?")).toBeTruthy();
  });

  it("disables Send while busy and when the budget is used up", async () => {
    useStore.setState({ ai: { open: true, turns: [], busy: true, progress: "AI berpikir…", budget: null } });
    await act(async () => {
      render(<AiPanel scope="quick" />);
    });
    expect((screen.getByText("Kirim") as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByText("AI berpikir…")).toBeTruthy();
    api.aiBudget.mockResolvedValueOnce({ provider: "deepseek", model: "deepseek-v4-flash", key_set: true, used_tokens: 3000, limit_tokens: 3000, cost_usd: 0 });
    useStore.setState({ ai: { open: true, turns: [], busy: false, progress: null, budget: null } });
    await act(async () => {
      render(<AiPanel scope="quick" />);
    });
    const boxes = screen.getAllByRole("textbox");
    const ta = boxes[boxes.length - 1] as HTMLTextAreaElement;
    expect(ta.disabled).toBe(true);
    expect(ta.placeholder).toMatch(/Budget token/);
  });

  it("sends a shortcut prompt and appends the reply", async () => {
    api.aiChat.mockResolvedValueOnce({
      reply: "Log **bagus**.",
      usage: { input: 10, output: 5, cached: 0, reasoning: 0 },
      cost_usd: 0.0001,
      session_usage: { input: 10, output: 5, cached: 0, reasoning: 0 },
      session_cost_usd: 0.0001,
      tool_calls: [],
      stopped_by: null,
      provider: "anthropic",
      model: "claude-sonnet-5",
      snapshot: null,
      quick_recs: [{ id: "r1" }],
    });
    await act(async () => {
      render(<AiPanel scope="quick" />);
    });
    await act(async () => {
      fireEvent.click(screen.getByText("Jelaskan kualitas log ini"));
    });
    expect(api.aiChat).toHaveBeenCalledWith("quick", expect.stringContaining("kualitas"));
    expect(useStore.getState().ai.turns).toHaveLength(1);
    expect(screen.getByText("bagus").tagName).toBe("B");
    expect(useStore.getState().recs).toEqual([{ id: "r1" }]);
    expect(useStore.getState().ai.busy).toBe(false);
  });
});
