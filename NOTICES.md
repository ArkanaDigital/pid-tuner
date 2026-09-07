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
