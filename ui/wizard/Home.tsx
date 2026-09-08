import { useEffect, useState } from "react";
import { api } from "../lib/api";
import { run, useStore } from "../lib/store";
import { firmwareText, type Mode } from "../lib/types";

export default function Home() {
  const s = useStore();
  const [name, setName] = useState("");
  const [mode, setMode] = useState<Mode>("offline");

  const refresh = async () => {
    const list = await run("Loading sessions…", api.sessionList);
    if (list) s.set({ list });
  };
  useEffect(() => {
    refresh();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function create() {
    const n = name.trim() || `Tune ${new Date().toLocaleString()}`;
    const snap = await run("Creating session…", () => api.sessionCreate(n, mode));
    if (snap) s.set({ snap, bundles: {}, view: "wizard" });
  }
  async function open(id: string) {
    const snap = await run("Opening session…", () => api.sessionOpen(id));
    if (!snap) return;
    const bundles: typeof s.bundles = {};
    for (const f of ["a", "b", "c"] as const) {
      if (snap.session.flights[f]) {
        const b = await api.flightBundle(f).catch(() => null);
        if (b) bundles[f] = b;
      }
    }
    s.set({ snap, bundles, view: "wizard" });
  }
  async function del(id: string) {
    if (!confirm("Delete this session and its logs?")) return;
    await run("Deleting…", () => api.sessionDelete(id));
    refresh();
  }

  return (
    <div className="home">
      <header className="topbar">
        <h1>PID Tuner</h1>
        <span className="spacer" />
        <button onClick={() => s.set({ view: "quick" })}>Quick look at a log</button>
        <button onClick={() => s.set({ prevView: "home", view: "settings" })}>⚙ Settings</button>
        {s.busy && <span className="busy">{s.busy}</span>}
        {s.error && <span className="error">{s.error}</span>}
      </header>
      <div className="home-body">
        <section className="card">
          <h2>New tuning session</h2>
          <label>
            Name / craft
            <input value={name} onChange={(e) => setName(e.target.value)} placeholder="e.g. Agus 5-inch" />
          </label>
          <div className="mode-pick">
            <label className={mode === "offline" ? "sel" : ""}>
              <input type="radio" checked={mode === "offline"} onChange={() => setMode("offline")} />
              <b>Offline</b>
              <span>Logs arrive as files (WhatsApp, SD card). Settings are handed over as CLI text and the pilot applies them.</span>
            </label>
            <label className={mode === "online" ? "sel" : ""}>
              <input type="radio" checked={mode === "online"} onChange={() => setMode("online")} />
              <b>Online</b>
              <span>Flight controller connected over USB. The app reads the tune, pulls logs and writes settings itself.</span>
            </label>
          </div>
          <button className="primary" onClick={create} disabled={!!s.busy}>Start wizard</button>
        </section>
        <section className="card">
          <h2>Sessions</h2>
          {s.list.length === 0 && <div className="empty">No sessions yet.</div>}
          <table className="sessions">
            <tbody>
              {s.list.map((x) => (
                <tr key={x.id}>
                  <td><b>{x.name}</b><br /><span className="muted">{x.craft_name ?? ""} {x.firmware ? firmwareText(x.firmware) : ""}</span></td>
                  <td>{x.mode}</td>
                  <td>{x.current.replace("_", " ")}</td>
                  <td className="muted">{new Date(x.updated_at).toLocaleString()}</td>
                  <td>
                    <button onClick={() => open(x.id)}>Open</button>{" "}
                    <button onClick={() => del(x.id)}>Delete</button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </section>
      </div>
    </div>
  );
}
