# PID Tuner

Guided blackbox tuning for FPV drones. Reads Betaflight blackbox logs (ArduPilot
in progress), produces PIDtoolbox-style step responses, gyro spectra and throttle
spectrograms, proposes filter/PID changes with evidence, and walks the tuner
through a 14-step wizard with hard guards — optionally writing the settings to
the flight controller over MSP and verifying them by read-back.

## Layout

```
crates/domain      firmware-agnostic types (FlightLog, Tune, Recommendation, …)
crates/dsp         FFT, Welch PSD, Wiener deconvolution, filter models
crates/bbl-ingest  Betaflight .BBL/.BFL → FlightLog (vendored blackbox-log parser)
crates/analysis    step response, spectrum, spectrogram, peaks, log quality
crates/recommend   rule engine (Betaflight)
crates/session     wizard state machine, guards, overrides, persistence
crates/fc-msp      MSPv2 codec, serial transport, Betaflight layouts, apply+verify
crates/cli         `pidtool` headless analysis
src-tauri          Tauri 2 shell (commands, report export)
ui                 React + uPlot front-end
vendor/blackbox-log  patched upstream parser (see PATCHES.md)
fixtures           sample logs (see SOURCES.md)
```

## Develop

```
pnpm install
pnpm tauri dev                                  # desktop app
PIDTUNER_OPEN=fixtures/bf/x.bbl pnpm tauri dev  # auto-open a log in "Quick look"
cargo test --workspace                          # Rust tests (dsp, analysis, session, fc-msp mock FC)
cargo run --release -p pidtool -- analyze fixtures/bf/bf_4.3.0_matekf405_LOG00001.BFL
```

Sessions are stored under `~/Library/Application Support/com.arkana.pidtuner/sessions/<id>/`
(logs, analysis cache, snapshots, report).

## Flight protocol

* **Flight A** — hover 30 s, then 20 s of gentle wobble → noise/filter analysis.
* **Flight B** — sharp isolated stick steps: roll ×5, pitch ×5, yaw ×3 → step response.
* **Flight C** — repeat B with the new PIDs → before/after + report.

Logging: ≥ 2 kHz blackbox rate; unfiltered gyro (native on BF ≥ 4.4, `debug_mode = GYRO_SCALED` on 4.3).

## License

GPL-3.0-or-later. See `NOTICES.md` for the projects this builds on.
