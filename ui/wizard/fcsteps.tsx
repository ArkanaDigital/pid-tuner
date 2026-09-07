import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api } from "../lib/api";
import { run, useStore } from "../lib/store";
import { firmwareText, type ApplyOutcome, type ApplyPhase, type Flight, type PortInfo } from "../lib/types";

/** Refresh the wizard snapshot (guards depend on FC status). */
async function resnap() {
  const snap = await api.sessionSnapshot();
  useStore.getState().set({ snap });
}

export function FcBadge() {
  const fc = useStore((s) => s.fc);
  if (!fc || !fc.connected) return <span className="fc-badge off">FC not connected</span>;
  return (
    <span className={`fc-badge ${fc.armed ? "armed" : "ok"}`}>
      {fc.firmware ? firmwareText(fc.firmware) : "FC"} · {fc.port} · {fc.armed ? "ARMED" : "disarmed"}
    </span>
  );
}

export function ConnectStep() {
  const s = useStore();
  const [ports, setPorts] = useState<PortInfo[]>([]);
  const [port, setPort] = useState("");
  const fc = s.fc;

  const scan = async () => {
    const p = await run("Scanning ports…", api.fcPorts);
    if (p) {
      setPorts(p);
      if (!port && p[0]) setPort(p[0].path);
    }
  };
  useEffect(() => {
    scan();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function connect() {
    const st = await run("Connecting, reading tune, saving backup…", () => api.fcConnect(port));
    if (st) {
      s.set({ fc: st });
      await resnap();
    }
  }
  async function disconnect() {
    const st = await run("…", api.fcDisconnect);
    if (st) {
      s.set({ fc: st });
      await resnap();
    }
  }
  async function backupCli() {
    const st = await run("Running `diff all` (the FC reboots afterwards)…", api.fcBackupCli);
    if (st) {
      s.set({ fc: st });
      await resnap();
    }
  }

  return (
    <div className="panel">
      <h2>Connect the flight controller</h2>
      <p>Plug the quad in over USB, <b>props off</b>. The app identifies the firmware, reads the current tune and stores an MSP snapshot as backup before anything is changed.</p>
      {!fc?.connected ? (
        <div className="row">
          <select value={port} onChange={(e) => setPort(e.target.value)}>
            {ports.map((p) => (
              <option key={p.path} value={p.path}>
                {p.path} {p.product ? `· ${p.product}` : ""} {p.likely_fc ? "★" : ""}
              </option>
            ))}
          </select>
          <button onClick={scan}>Rescan</button>
          <button className="primary" onClick={connect} disabled={!port || !!s.busy}>Connect</button>
        </div>
      ) : (
        <>
          <table className="kv">
            <tbody>
              <tr><td>Firmware</td><td>{fc.firmware ? firmwareText(fc.firmware) : "—"}</td></tr>
              <tr><td>Port</td><td>{fc.port}</td></tr>
              <tr><td>State</td><td className={fc.armed ? "warn" : ""}>{fc.armed ? "ARMED — disarm now" : "disarmed"}</td></tr>
              <tr><td>Blackbox rate</td><td>{fc.log_rate_hz ? `${fc.log_rate_hz.toFixed(0)} Hz` : "—"}</td></tr>
              <tr><td>Backup</td><td>{fc.snapshot_taken ? "MSP snapshot saved" : "not yet"}</td></tr>
            </tbody>
          </table>
          <div className="row">
            <button onClick={backupCli} disabled={!!s.busy}>Also save CLI `diff all` (reboots FC)</button>
            <button onClick={disconnect} disabled={!!s.busy}>Disconnect</button>
          </div>
        </>
      )}
    </div>
  );
}

export function PreflightStep() {
  const s = useStore();
  const fc = s.fc;
  async function fix() {
    const st = await run("Writing logging settings…", api.fcPreflightFix);
    if (st) {
      s.set({ fc: st });
      await resnap();
    }
  }
  async function refresh() {
    const st = await run("Reading…", api.fcRefresh);
    if (st) {
      s.set({ fc: st });
      await resnap();
    }
  }
  return (
    <div className="panel">
      <h2>Preflight logging setup</h2>
      <p>For a useful analysis the flight controller must log at ≥ 2 kHz with the unfiltered gyro included, and have room for the flight.</p>
      <table className="kv">
        <tbody>
          <tr><td>Blackbox rate</td><td>{fc?.log_rate_hz ? `${fc.log_rate_hz.toFixed(0)} Hz` : "—"}</td></tr>
          <tr><td>Unfiltered gyro</td><td>{fc?.raw_gyro_logging_enabled == null ? "—" : fc.raw_gyro_logging_enabled ? "yes (gyroUnfilt / GYRO_SCALED)" : "no"}</td></tr>
          <tr><td>Storage free</td><td>{fc?.storage_free_bytes != null ? `${(fc.storage_free_bytes / 1e6).toFixed(1)} MB` : "—"}</td></tr>
          <tr><td>Debug mode</td><td>{fc?.debug_mode ?? "—"}</td></tr>
        </tbody>
      </table>
      <div className="row">
        <button className="primary" onClick={fix} disabled={!fc?.connected || !!s.busy}>Fix logging settings</button>
        <button onClick={refresh} disabled={!fc?.connected || !!s.busy}>Re-read</button>
        <span className="muted">Sets blackbox_sample_rate for ≥ 2 kHz and, on Betaflight ≤ 4.3, debug_mode = GYRO_SCALED. Saves to EEPROM.</span>
      </div>
    </div>
  );
}

export function DownloadFromFlash({ which }: { which: Flight }) {
  const s = useStore();
  const [prog, setProg] = useState<{ done: number; total: number } | null>(null);
  useEffect(() => {
    const un = listen<{ done: number; total: number }>("fc://progress", (e) => setProg(e.payload));
    return () => {
      un.then((f) => f());
    };
  }, []);
  async function download() {
    setProg({ done: 0, total: 1 });
    const r = await run("Downloading blackbox flash…", () => api.fcDownloadImport(which));
    setProg(null);
    if (r) {
      s.setBundle(which, r.bundle);
      s.set({ snap: r.snapshot });
    }
  }
  if (!s.fc?.connected) return null;
  return (
    <div className="row">
      <button className="primary" onClick={download} disabled={!!s.busy}>Download from flash & import</button>
      {prog && prog.total > 0 && <span className="muted">{((100 * prog.done) / prog.total).toFixed(0)} % ({(prog.done / 1e6).toFixed(1)} / {(prog.total / 1e6).toFixed(1)} MB)</span>}
    </div>
  );
}

export function FcApplyButton({ phase }: { phase: ApplyPhase }) {
  const s = useStore();
  const [outcomes, setOutcomes] = useState<ApplyOutcome[] | null>(null);
  async function write() {
    const r = await run("Writing to the flight controller and verifying…", () => api.fcApply(phase));
    if (r) {
      setOutcomes(r.result.outcomes);
      s.set({ snap: r.snapshot });
    }
  }
  if (!s.fc?.connected) return <div className="notice">Connect the flight controller (step 1) to write settings directly, or switch to the CLI text below.</div>;
  return (
    <>
      <div className="row">
        <button className="primary" onClick={write} disabled={!!s.busy || s.fc.armed}>Write to flight controller</button>
        <span className="muted">Read-modify-write over MSP, EEPROM save, then every value is read back and compared.</span>
      </div>
      {outcomes && (
        <table className="recs">
          <thead><tr><th>Parameter</th><th>Written</th><th>Read back</th><th>Via</th><th /></tr></thead>
          <tbody>
            {outcomes.map((o) => (
              <tr key={o.param} className={o.ok ? "" : "off"}>
                <td><code>{o.param}</code></td><td>{o.wanted}</td><td>{o.read_back ?? "—"}</td><td>{o.via}</td>
                <td className={o.ok ? "conf high" : "conf low"}>{o.ok ? "verified" : "MISMATCH"}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </>
  );
}
