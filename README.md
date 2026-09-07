# PID Tuner

Guided blackbox tuning for FPV drones. Reads Betaflight blackbox logs and
ArduPilot Copter DataFlash `.bin` logs, produces PIDtoolbox-style step responses,
gyro spectra and throttle spectrograms, proposes filter/PID changes with evidence,
and walks the tuner through a 14-step wizard with hard guards — optionally writing
the settings to the flight controller (MSP for Betaflight, MAVLink for ArduPilot)
and verifying them by read-back.

## Layout

```
crates/domain      firmware-agnostic types (FlightLog, Tune, Recommendation, …)
crates/dsp         FFT, Welch PSD, Wiener deconvolution, filter models
crates/bbl-ingest  Betaflight .BBL/.BFL → FlightLog (vendored blackbox-log parser)
crates/ap-ingest   ArduPilot DataFlash .bin → FlightLog (FMT-driven, golden-tested vs pymavlink)
crates/log-ingest  format detection + dispatch
crates/analysis    step response, spectrum, spectrogram, peaks, log quality, predicted AP filter chain
crates/recommend   rule engine (Betaflight `bf.rs`, ArduPilot `ap.rs` + generated `ap_param_meta.rs`)
crates/session     wizard state machine, firmware-aware guards, overrides, persistence
crates/fc-msp      MSPv2 codec, serial transport, Betaflight layouts, apply+verify
crates/fc-mavlink  MAVLink 2 (ardupilotmega): params with typed read-back, log download, reboot, mock FC
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
cargo test --workspace                          # Rust tests (dsp, analysis, session, ap-ingest goldens, fc-msp / fc-mavlink mock FCs)
cargo run --release -p pidtool -- analyze fixtures/bf/bf_4.3.0_matekf405_LOG00001.BFL
cargo run --release -p pidtool -- analyze fixtures/ap/copter_4.6.3_quad_pid_msgs.bin
scripts/gen_ap_golden.py fixtures/ap/x.bin      # regenerate a golden JSON with pymavlink (venv with pymavlink)
scripts/gen_ap_param_meta.py                    # regenerate crates/recommend/src/ap_param_meta.rs from apm.pdef.xml
```

Sessions are stored under `~/Library/Application Support/com.arkana.pidtuner/sessions/<id>/`
(logs, analysis cache, snapshots, report).

## Flight protocol

* **Flight A** — hover 30 s, then 20 s of gentle wobble → noise/filter analysis.
* **Flight B** — sharp isolated stick steps: roll ×5, pitch ×5, yaw ×3 → step response.
* **Flight C** — repeat B with the new PIDs → before/after + report.

Logging: ≥ 2 kHz blackbox rate; unfiltered gyro (native on BF ≥ 4.4, `debug_mode = GYRO_SCALED` on 4.3).

## ArduPilot Copter

Requirements (Preflight writes and verifies them, `INS_LOG_BAT_MASK` reboots the FC):

* `LOG_BITMASK` bits 0 (ATTITUDE_FAST) + 12 (PID) → `RATE`/`PIDx` at `SCHED_LOOP_RATE`; without bit 0 they log at 10 Hz and no step response can be estimated (the guard says so).
* bit 19 (IMU_RAW) + `INS_LOG_BAT_MASK=1`, `INS_LOG_BAT_OPT=4` (pre + post filter), `INS_LOG_BAT_CNT=1024`, `INS_LOG_BAT_LGIN=20` → `ISBH/ISBD` gyro batches for the spectra (F4/F7 boards; ≥ 20 batches required).
* Flight A in AltHold (30 s hover + 20 s rocking), Flight B/C in Stabilize with ≥ 30/30/10 stick snaps.
* PID path: our heuristics (`ATC_RAT_*`, Quicktune oscillation criterion `PIDx.SRate > QUIK_OSC_SMAX`) or the pilot flies AUTOTUNE and Import C checks the gains really changed (`D > AUTOTUNE_MIN_D`).
* Offline: export `NAME,VALUE` `.param` text for Mission Planner / QGC. Online: MAVLink `PARAM_SET` with typed read-back; `INS_HNTCH_ENABLE` is written first, then reboot, then the other `INS_HNTCH_*` values.

Every ArduPilot-derived number is cited to its source file/symbol in `crates/domain/src/ap_consts.rs`,
`crates/session/src/guards.rs` and `crates/fc-mavlink`; parser behaviour is pinned by golden tests
generated with pymavlink (`fixtures/ap/*.golden.json`). Not yet validated on real hardware — a loop-rate
log from an F4/F7 quad with the settings above is the next fixture (`fixtures/ap/copter_hw_loop_rate.bin`).

## License

GPL-3.0-or-later. See `NOTICES.md` for the projects this builds on.
