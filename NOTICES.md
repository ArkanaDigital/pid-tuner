# Third-party notices

PID Tuner is licensed under GPL-3.0-or-later (see LICENSE). Algorithms and
reference implementations were ported from:

- **PIDtoolbox** — Brian White (Beer-Ware / GPL-3.0 via the ianrmurphy and
  skoch1s forks): `PTstepcalc.m` (Wiener step response), `PTSpec2d.m`,
  `PTthrSpec.m`, `PTtuningParams.m` metrics. → `crates/analysis`, `crates/dsp`.
- **PID-Analyzer** — Florian Melsheimer (Plasmatree): Gaussian-regularised
  deconvolution variant. → `crates/dsp/src/wiener.rs`.
- **ArduPilot WebTools** — ArduPilot dev team (GPL-3.0): `Libraries/fft.js`
  (Welch PSD scaling), `FilterReview.js` biquad / notch / harmonic-notch
  models, `PIDReview.js` step response. → `crates/dsp/src/{welch,filters}.rs`.
- **blackbox-log** — wetheredge (MIT OR Apache-2.0), vendored with patches in
  `vendor/blackbox-log` (see `PATCHES.md`).
- **Betaflight** — header/field semantics from `src/main/blackbox/blackbox.c`
  and MSP layouts from `betaflight-configurator/src/js/msp/MSPHelper.js` (GPL-3.0).
- Tuning heuristics follow the Betaflight PID Tuning Guide, 4.3 Tuning Notes
  and Oscar Liang's blackbox tuning guides.
- **ArduPilot** — ArduPilot dev team (GPL-3.0): DataFlash wire format and
  message semantics (`libraries/AP_Logger/LogStructure.h`, `AP_Logger.h`,
  `AP_InertialSensor/BatchSampler.cpp`, `AC_AttitudeControl`, `AC_PID`,
  `ArduCopter/defines.h`, `AP_Quicktune.cpp`), MAVLink parameter / log-transfer
  handler behaviour (`GCS_MAVLink/GCS_Param.cpp`, `GCS_Common.cpp`,
  `AP_Logger/AP_Logger_MAVLinkLogTransfer.cpp`, `AP_Param/AP_Param.cpp`) and the
  parameter metadata `apm.pdef.xml` (generated into
  `crates/recommend/src/ap_param_meta.rs`). Constants are cited per symbol in
  `crates/domain/src/ap_consts.rs`. → `crates/ap-ingest`, `crates/fc-mavlink`,
  `crates/recommend/src/ap.rs`.
- **pymavlink** — ArduPilot dev team (LGPL-3.0): `DFReader.py` is the oracle for
  the ArduPilot parser golden tests (`scripts/gen_ap_golden.py`); no code is
  copied.
- **JsDataflashParser** — ArduPilot WebTools (GPL-3.0): two-phase index/decode
  design of the `.bin` parser. → `crates/ap-ingest/src/{index,decode}.rs`.
- **ardupilot-binlog** — MIT OR Apache-2.0: format-character size table used as
  a cross-check for `crates/ap-ingest/src/format.rs` (crate not linked).
- **rust-mavlink** — MIT OR Apache-2.0: MAVLink 2 framing and the
  `ardupilotmega` dialect generated from the official XML. → `crates/fc-mavlink`.
- **Betaflight Configurator** — Betaflight dev team (GPL-3.0-or-later): Autotune tab
  `src/js/blackbox/spectral_analysis.js` and `chirp_bbl_parser.js` (Welch
  cross-spectrum transfer estimate, coherence, open-loop/sensitivity, metrics and
  gain-scale logic) → `crates/dsp/src/cross.rs`, `crates/analysis/src/chirp.rs`,
  `crates/bbl-ingest/src/chirp.rs`, `crates/recommend/src/bf_chirp.rs`.
- **Betaflight Blackbox Explorer** — `src/flightlog_fielddefs.js` (GPL-3.0) as the
  cross-check for the `debug_mode` name tables in `crates/bbl-ingest/src/debug_modes.rs`
  (values verified against `src/main/build/debug.h` at tags 2025.12.2 and 2026.6.1).
- **pichim/bf_controller_tuning** — Peter Michael (GPL-3.0): example chirp log
  `logs/example_logs/Gyro_Angle.TXT` used as a test fixture (see `fixtures/SOURCES.md`).
