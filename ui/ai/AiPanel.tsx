import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api } from "../lib/api";
import { useStore } from "../lib/store";
import { estimateCost, formatTokens, formatUsd } from "../lib/cost";
import type { AiProgressEvent, AiScope, AiToolCall, Step, TurnMeta } from "../lib/types";
import { QUICK_SHORTCUTS, SHORTCUTS } from "./shortcuts";

/** Very small markdown: paragraphs, numbered/bulleted lists, `code`, **bold**. */
export function renderLite(text: string): React.ReactNode[] {
  const out: React.ReactNode[] = [];
  const lines = text.split(/\r?\n/);
  let list: { ordered: boolean; items: string[] } | null = null;
  const flush = () => {
    if (list) {
      const items = list.items.map((it, i) => <li key={i}>{inline(it)}</li>);
      out.push(list.ordered ? <ol key={out.length}>{items}</ol> : <ul key={out.length}>{items}</ul>);
      list = null;
    }
  };
  for (const raw of lines) {
    const line = raw.trimEnd();
    const m = /^\s*(\d+)[.)]\s+(.*)$/.exec(line);
    const b = /^\s*[-*•]\s+(.*)$/.exec(line);
    if (m) {
      if (!list || !list.ordered) {
        flush();
        list = { ordered: true, items: [] };
      }
      list.items.push(m[2]);
    } else if (b) {
      if (!list || list.ordered) {
        flush();
        list = { ordered: false, items: [] };
      }
      list.items.push(b[1]);
    } else if (line.trim() === "") {
      flush();
    } else {
      flush();
      out.push(<p key={out.length}>{inline(line)}</p>);
    }
  }
  flush();
  return out;
}

function inline(t: string): React.ReactNode[] {
  const parts: React.ReactNode[] = [];
  const re = /(`[^`]+`|\*\*[^*]+\*\*)/g;
  let last = 0;
  let m: RegExpExecArray | null;
  let i = 0;
  while ((m = re.exec(t))) {
    if (m.index > last) parts.push(t.slice(last, m.index));
    const tok = m[0];
    if (tok.startsWith("`")) parts.push(<code key={i++}>{tok.slice(1, -1)}</code>);
    else parts.push(<b key={i++}>{tok.slice(2, -2)}</b>);
    last = m.index + tok.length;
  }
  if (last < t.length) parts.push(t.slice(last));
  return parts;
}

export function ToolBadges({ calls }: { calls: AiToolCall[] }) {
  const [open, setOpen] = useState<number | null>(null);
  if (!calls.length) return null;
  return (
    <div className="ai-tools">
      {calls.map((c, i) => (
        <span key={i} className={`ai-tool ${c.ok ? "" : "bad"}`} onClick={() => setOpen(open === i ? null : i)} title={JSON.stringify(c.args)}>
          {c.ok ? "⚙" : "✕"} {c.name}
          {open === i && <pre className="ai-tool-detail">{JSON.stringify(c.args)}\n→ {c.preview}</pre>}
        </span>
      ))}
    </div>
  );
}

export function UsageLine({ turn, price }: { turn: TurnMeta; price: { input_per_m: number; output_per_m: number; cached_input_per_m: number | null } | null }) {
  const cost = turn.cost_usd > 0 ? turn.cost_usd : estimateCost(turn.usage, price);
  return (
    <div className="ai-usage">
      in {formatTokens(turn.usage.input)} · out {formatTokens(turn.usage.output)}{turn.usage.cached ? ` · cached ${formatTokens(turn.usage.cached)}` : ""} · {formatUsd(cost)} · {turn.model}
      {turn.stopped_by && <span className="warn"> · berhenti: {turn.stopped_by}</span>}
    </div>
  );
}

export default function AiPanel({ scope, step }: { scope: AiScope; step?: Step }) {
  const s = useStore();
  const ai = s.ai;
  const [text, setText] = useState("");
  const listRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    let alive = true;
    (async () => {
      try {
        const [t, b] = await Promise.all([api.aiTranscript(scope), api.aiBudget(scope)]);
        if (alive) s.setAi({ turns: t.turns, budget: b });
      } catch {
        /* settings may be missing */
      }
    })();
    const un = listen<AiProgressEvent>("ai://progress", (e) => {
      const ev = e.payload;
      const msg = ev.kind === "thinking" ? `AI berpikir (ronde ${ev.round})…` : ev.kind === "tool_call" ? `AI membaca ${ev.name}…` : ev.kind === "tool_result" ? `${ev.name} ${ev.ok ? "✓" : "✕"}` : `token ${formatTokens(ev.session.input + ev.session.output)}`;
      useStore.getState().setAi({ progress: msg });
    });
    return () => {
      alive = false;
      un.then((f) => f());
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [scope]);

  useEffect(() => {
    const el = listRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [ai.turns.length, ai.busy]);

  const budget = ai.budget;
  const overBudget = !!budget && budget.limit_tokens > 0 && budget.used_tokens >= budget.limit_tokens;
  const price = s.settings?.prices[budget?.model ?? ""] ?? null;

  async function send(prompt: string) {
    const t = prompt.trim();
    if (!t || ai.busy) return;
    s.setAi({ busy: true, progress: "Mengirim…" });
    s.set({ error: null });
    setText("");
    try {
      const r = await api.aiChat(scope, t);
      const turn: TurnMeta = { at: new Date().toISOString(), step: step ?? "quick_look", provider: r.provider, model: r.model, user_text: t, reply: r.reply, usage: r.usage, cost_usd: r.cost_usd, tool_calls: r.tool_calls, stopped_by: r.stopped_by };
      const b = await api.aiBudget(scope).catch(() => null);
      s.setAi({ turns: [...useStore.getState().ai.turns, turn], budget: b ?? budget });
      if (r.snapshot) s.set({ snap: r.snapshot });
      if (r.quick_recs) s.set({ recs: r.quick_recs });
    } catch (e) {
      s.set({ error: String(e) });
    } finally {
      s.setAi({ busy: false, progress: null });
    }
  }

  async function clear() {
    await api.aiTranscriptClear(scope).catch(() => {});
    const b = await api.aiBudget(scope).catch(() => null);
    s.setAi({ turns: [], budget: b });
  }

  const shortcuts = scope === "quick" ? QUICK_SHORTCUTS : step ? SHORTCUTS[step] : [];

  return (
    <div className="ai-panel">
      <div className="ai-head">
        <b>AI helper</b>
        {budget ? (
          <span className="muted"> {budget.provider} · {budget.model}{budget.key_set ? "" : " · key belum diisi"}</span>
        ) : (
          <span className="muted"> belum dikonfigurasi</span>
        )}
        <span className="spacer" />
        <button className="small" onClick={() => s.set({ prevView: s.view, view: "settings" })}>⚙</button>
        <button className="small" onClick={clear} disabled={ai.busy || !ai.turns.length}>Bersihkan</button>
      </div>
      {budget && (
        <div className="ai-budget" title={`${budget.used_tokens} / ${budget.limit_tokens || "∞"} token`}>
          <progress value={budget.limit_tokens ? Math.min(budget.used_tokens, budget.limit_tokens) : 0} max={budget.limit_tokens || 1} />
          <span>{formatTokens(budget.used_tokens)}{budget.limit_tokens ? ` / ${formatTokens(budget.limit_tokens)}` : ""} · {formatUsd(budget.cost_usd)}</span>
        </div>
      )}
      <div className="ai-messages" ref={listRef}>
        {ai.turns.length === 0 && <div className="muted ai-empty">Tanya apa saja tentang log, guard, atau rekomendasi di langkah ini. AI hanya mengusulkan; kamu yang menulis ke FC.</div>}
        {ai.turns.map((t, i) => (
          <div key={i} className="ai-turn">
            <div className="ai-msg user">{t.user_text}</div>
            <div className="ai-msg assistant">
              {renderLite(t.reply)}
              <ToolBadges calls={t.tool_calls} />
              <UsageLine turn={t} price={price} />
            </div>
          </div>
        ))}
        {ai.busy && <div className="ai-msg assistant busy">{ai.progress ?? "…"}</div>}
      </div>
      {shortcuts.length > 0 && (
        <div className="ai-shortcuts">
          {shortcuts.map((sc) => <button key={sc.id} className="small" disabled={ai.busy || overBudget} onClick={() => send(sc.prompt)}>{sc.label}</button>)}
        </div>
      )}
      <div className="ai-input">
        <textarea rows={2} value={text} placeholder={overBudget ? "Budget token sesi habis — naikkan di Settings" : "Tulis pertanyaan… (Ctrl/Cmd+Enter untuk kirim)"} disabled={ai.busy || overBudget} onChange={(e) => setText(e.target.value)} onKeyDown={(e) => { if ((e.metaKey || e.ctrlKey) && e.key === "Enter") send(text); }} />
        <button className="primary" disabled={ai.busy || overBudget || !text.trim()} onClick={() => send(text)}>Kirim</button>
      </div>
    </div>
  );
}
