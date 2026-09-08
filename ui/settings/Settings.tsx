import { useEffect, useState } from "react";
import { api } from "../lib/api";
import { run, useStore } from "../lib/store";
import { PROVIDERS, PROVIDER_TITLE, type ModelPrice, type Provider, type SettingsPatch, type SettingsView, type TestResult } from "../lib/types";

export default function Settings() {
  const s = useStore();
  const [view, setView] = useState<SettingsView | null>(s.settings);
  const [keys, setKeys] = useState<Partial<Record<Provider, string>>>({});
  const [models, setModels] = useState<Partial<Record<Provider, string>>>({});
  const [baseUrls, setBaseUrls] = useState<Partial<Record<Provider, string>>>({});
  const [tests, setTests] = useState<Partial<Record<Provider, TestResult>>>({});
  const [budget, setBudget] = useState<number | null>(null);
  const [language, setLanguage] = useState<string | null>(null);
  const [provider, setProvider] = useState<Provider | null>(null);
  const [prices, setPrices] = useState<Record<string, ModelPrice> | null>(null);
  const [saved, setSaved] = useState(false);

  const load = async () => {
    const v = await run("Loading settings…", api.settingsGet);
    if (v) {
      setView(v);
      s.set({ settings: v });
    }
  };
  useEffect(() => {
    load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  if (!view) return <div className="app"><header className="topbar"><button onClick={() => s.set({ view: s.prevView })}>← Back</button><h1>Settings</h1></header></div>;

  const cur = { provider: provider ?? view.provider, language: language ?? view.language, budget: budget ?? view.token_budget_per_session, prices: prices ?? view.prices };

  async function save() {
    const patch: SettingsPatch = {};
    if (provider != null && provider !== view!.provider) patch.provider = provider;
    if (language != null && language !== view!.language) patch.language = language;
    if (budget != null && budget !== view!.token_budget_per_session) patch.token_budget_per_session = Math.max(0, Math.round(budget));
    const provs: SettingsPatch["providers"] = {};
    for (const p of PROVIDERS) {
      const m = models[p];
      const u = baseUrls[p];
      if (m != null || u != null) provs[p] = { model: m ?? view!.providers[p].model, base_url: u != null ? (u.trim() || null) : view!.providers[p].base_url };
    }
    if (Object.keys(provs).length) patch.providers = provs;
    const k: SettingsPatch["keys"] = {};
    for (const p of PROVIDERS) if (keys[p] != null) k[p] = keys[p]!;
    if (Object.keys(k).length) patch.keys = k;
    if (prices) patch.prices = prices;
    const v = await run("Saving settings…", () => api.settingsSet(patch));
    if (v) {
      setView(v);
      s.set({ settings: v });
      setKeys({});
      setModels({});
      setBaseUrls({});
      setPrices(null);
      setSaved(true);
      setTimeout(() => setSaved(false), 1500);
    }
  }

  async function test(p: Provider) {
    if (keys[p] != null) await save();
    const r = await run(`Testing ${PROVIDER_TITLE[p]}…`, () => api.settingsTest(p));
    if (r) setTests((t) => ({ ...t, [p]: r }));
  }

  const dirty = provider != null || language != null || budget != null || prices != null || Object.keys(keys).length > 0 || Object.keys(models).length > 0 || Object.keys(baseUrls).length > 0;

  return (
    <div className="app">
      <header className="topbar">
        <button onClick={() => s.set({ view: s.prevView })}>← Back</button>
        <h1>Settings</h1>
        <span className="spacer" />
        {s.busy && <span className="busy">{s.busy}</span>}
        {s.error && <span className="error">{s.error}</span>}
        {saved && <span className="muted">Saved ✓</span>}
        <button className="primary" onClick={save} disabled={!dirty || !!s.busy}>Save</button>
      </header>
      <main className="content settings">
        {view.load_warnings.map((w, i) => <div className="notice bad" key={i}>{w}</div>)}
        <section className="card">
          <h2>AI helper</h2>
          <p className="muted">Kunci API disimpan di <code>settings.json</code> di folder data aplikasi, dienkripsi ringan dengan kunci per mesin (siapa pun yang punya akses folder dan id mesin bisa membukanya). Kunci hanya dipakai untuk memanggil provider yang kamu pilih; AI hanya mengusulkan, tidak pernah menulis ke FC.</p>
          <div className="settings-grid">
            <label>
              Provider aktif
              <select value={cur.provider} onChange={(e) => setProvider(e.target.value as Provider)}>
                {PROVIDERS.map((p) => <option key={p} value={p}>{PROVIDER_TITLE[p]}{view.keys[p].set ? "" : " (belum ada key)"}</option>)}
              </select>
            </label>
            <label>
              Bahasa jawaban
              <select value={cur.language} onChange={(e) => setLanguage(e.target.value)}>
                <option value="id">Bahasa Indonesia</option>
                <option value="en">English</option>
              </select>
            </label>
            <label>
              Budget token per sesi (0 = tanpa batas)
              <input type="number" min={0} step={10000} value={cur.budget} onChange={(e) => setBudget(Number(e.target.value))} />
            </label>
          </div>
        </section>
        {PROVIDERS.map((p) => (
          <section className="card provider" key={p}>
            <h3>{PROVIDER_TITLE[p]} {cur.provider === p && <span className="rel good">aktif</span>}</h3>
            <div className="settings-grid">
              <label>
                API key {view.keys[p].set && keys[p] == null && <span className="muted">tersimpan: {view.keys[p].masked}</span>}
                <div className="row">
                  <input type="password" placeholder={view.keys[p].set ? "•••••••• (ganti)" : "masukkan API key"} value={keys[p] ?? ""} onChange={(e) => setKeys((k) => ({ ...k, [p]: e.target.value }))} autoComplete="off" />
                  {view.keys[p].set && <button className="small" onClick={() => setKeys((k) => ({ ...k, [p]: "" }))}>Hapus</button>}
                </div>
              </label>
              <label>
                Model
                <input list={`models-${p}`} value={models[p] ?? view.providers[p].model} onChange={(e) => setModels((m) => ({ ...m, [p]: e.target.value }))} />
                <datalist id={`models-${p}`}>{view.known_models[p].map((m) => <option key={m} value={m} />)}</datalist>
              </label>
              <label>
                Base URL (opsional, proxy https)
                <input placeholder="default" value={baseUrls[p] ?? view.providers[p].base_url ?? ""} onChange={(e) => setBaseUrls((b) => ({ ...b, [p]: e.target.value }))} />
              </label>
            </div>
            <div className="row">
              <button onClick={() => test(p)} disabled={!!s.busy || (!view.keys[p].set && !keys[p])}>Test koneksi</button>
              {tests[p] && (
                <span className={tests[p]!.ok ? "muted" : "error"}>
                  {tests[p]!.ok ? `OK · ${tests[p]!.model} · ${tests[p]!.latency_ms} ms · ${tests[p]!.usage.input + tests[p]!.usage.output} token` : `Gagal: ${tests[p]!.message}`}
                </span>
              )}
            </div>
          </section>
        ))}
        <section className="card">
          <h3>Harga (USD per 1M token, estimasi — bisa diedit)</h3>
          <table className="recs price-table">
            <thead><tr><th>Model</th><th>Input</th><th>Output</th><th>Cached input</th></tr></thead>
            <tbody>
              {Object.entries(cur.prices).sort().map(([m, pr]) => (
                <tr key={m}>
                  <td><code>{m}</code></td>
                  {(["input_per_m", "output_per_m", "cached_input_per_m"] as const).map((f) => (
                    <td key={f}>
                      <input className="num" type="number" step={0.01} min={0} value={pr[f] ?? ""} onChange={(e) => setPrices((cp) => ({ ...(cp ?? view.prices), [m]: { ...pr, [f]: e.target.value === "" ? null : Number(e.target.value) } as ModelPrice }))} />
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
          <p className="muted">Biaya yang ditampilkan di panel AI dihitung dari usage yang dilaporkan provider dikalikan tabel ini. Model yang tidak ada di tabel tampil sebagai "—".</p>
        </section>
      </main>
    </div>
  );
}
