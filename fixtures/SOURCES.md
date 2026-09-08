# Fixture sources

Sample flight logs used as test fixtures. Downloaded 2026-09-07. All files were verified after download:
- Betaflight: file starts with `H Product:Blackbox flight data recorder` (`head -c 64`)
- ArduPilot: file starts with bytes `A3 95` (`xxd | head -1`); parsed with pymavlink `DFReader_binary` to confirm firmware and message types

Filenames were renamed to `<fw>_<version>_<board/desc>.<ext>`; content is byte-identical to the source.

## Betaflight blackbox logs (`bf/`)

| File | Firmware | Board / craft | Size | looptime / pid_process_denom / debug_mode | Logs in file | Source URL | License / repo |
|---|---|---|---|---|---|---|---|
| `bf_4.3.0_matekf405_LOG00001.BFL` | Betaflight 4.3.0 (fc8318f54) STM32F405 | MTKS MATEKF405CTR / - | 6,894,889 B (6.9 MB) | 125 / 1 / 6 | 1 | https://raw.githubusercontent.com/gimbal-ghost/gimbal-ghost/main/test/LOG00001.BFL | gimbal-ghost/gimbal-ghost (GPL-3.0) - test fixture `test/LOG00001.BFL`; also a submodule of blackbox-log/blackbox-log |
| `bf_4.4.3_furyf4osd_btfl_002.bbl` | Betaflight 4.4.3 (738127e7e) STM32F405 | DIAT FURYF4OSD / pisun | 204,800 B (0.2 MB) | 125 / 2 / 0 | 15 | https://raw.githubusercontent.com/vlad-avr/BBLHelper/main/data/btfl_002.bbl | vlad-avr/BBLHelper (no license file) - sample data `data/btfl_002.bbl` |
| `bf_4.5.1_furyf4osd_btfl_004.bbl` | Betaflight 4.5.1 (77d01ba3b) STM32F405 | DIAT FURYF4OSD / - | 1,024,000 B (1.0 MB) | 125 / 2 / 0 | 2 | https://raw.githubusercontent.com/vlad-avr/BBLHelper/main/data/btfl_004.bbl | vlad-avr/BBLHelper (no license file) - sample data `data/btfl_004.bbl` |
| `bf_4.5.2_speedybeef405mini_vx3.5.bbl` | Betaflight 4.5.2 (024f8e13d) STM32F405 | SPBE SPEEDYBEEF405MINI / VX3.5 | 4,663,296 B (4.7 MB) | 125 / 2 / 6 | 1 | https://raw.githubusercontent.com/eddycek/fpvpidlab/main/test-fixtures/bbl/blackbox_2026-03-29T16-17-37-126Z.bbl | eddycek/fpvpidlab (custom "Other" license, (C) 2025-2026 Eduard Cekel) - test fixture `test-fixtures/bbl/` |
| `bf_2025.12.1_speedybeef405aio_btfl_001.bbl` | Betaflight 2025.12.1 (85d201376) STM32F405 | SPBE SPEEDYBEEF405AIO / - | 1,382,400 B (1.4 MB) | 125 / 2 / 0 | 1 | https://raw.githubusercontent.com/Samma3ll/fpv-manager/main/btfl_001_clean.bbl | Samma3ll/fpv-manager (no license file) - sample `btfl_001_clean.bbl` |
| `bf_2025.12.2_speedybeef7v3_steadyhover.BFL` | Betaflight 2025.12.2 (79065c96b) STM32F7X2 | SPBE SPEEDYBEEF7V3 / Mario 5 | 5,720,820 B (5.7 MB) | 312 / 1 / 16 | 1 | https://raw.githubusercontent.com/PedroS235/blackbird/main/tests/fixtures/new202612_BF_steadyhover.BFL | PedroS235/blackbird (MIT) - test fixture `tests/fixtures/new202612_BF_steadyhover.BFL` |

| `bf_2025.12_chirp_gyro_angle.TXT` | Betaflight 2026.6.0-alpha (8c8523411) STM32F7X2 | SPBE / OvershootExpress | 27,475,498 B (26.2 MB) | 125 / 2 / 97 (`CHIRP` in 2025.12 numbering) | 1 | https://raw.githubusercontent.com/pichim/bf_controller_tuning/main/logs/example_logs/Gyro_Angle.TXT | pichim/bf_controller_tuning (GPL-3.0) - `logs/example_logs/Gyro_Angle.TXT`. CHIRP sweeps 0.2→600 Hz, 20 s, high-resolution logging, `P interval 2` (2 kHz). Whole flight in ANGLE mode: roll/pitch sweeps include the attitude loop, yaw is a pure rate loop. |

Notes:
- `bf_2025.12_chirp_gyro_angle.TXT`: the only CHIRP log found; used by `crates/analysis` golden-style tests for segment detection and yaw metrics.
- `bf_2025.12.2_speedybeef7v3_steadyhover.BFL` is described by its source repo as a steady-hover flight.
- `bf_4.3.0_matekf405_LOG00001.BFL` is the only 4.3.x log found; 6.9 MB.
- `bf_4.4.3_furyf4osd_btfl_002.bbl` is small (200 KB) - it was the only Betaflight 4.4.x log found on GitHub. A sibling `btfl_001.bbl` (150 KB, same firmware) exists in the same repo.
- Other candidates seen but not kept: `ilya-epifanov/fc-blackbox` `src/test-data/*` (BF 4.2.0-4.2.11, 0.5-2.4 MB, blackbox-log submodule), `gimbal-ghost` `test/btfl_00{1,2}.bbl` (BF 4.2.9), `jrwrodgers/bbl_decoder/Run2.bbl` (BF 4.5.2, 3.4 MB), `pichim/bf_controller_tuning` (BF 2025-era, 22-30 MB each - too large), `blackbox-log/blackbox-log tests/logs/error-recovery.bbl` (BF 4.2.11, 3.8 KB).

## ArduPilot Copter DataFlash logs (`ap/`)

| File | Firmware | Size | Contents (pymavlink) | Source URL | License / origin |
|---|---|---|---|---|---|
| `copter_4.5.5_tarot_x4_batch_imu.bin` | ArduCopter V4.5.5 (142aece2) | 5,197,824 B (5.2 MB) | 142 s; ISBH=221 ISBD=7080 (batch IMU logging present), IMU=10641, RATE/ATT=1420 (10 Hz), RCIN/RCOU=1420. No PIDR/PIDP/PIDY messages. | https://raw.githubusercontent.com/ArduPilot/MethodicConfigurator/master/ardupilot_methodic_configurator/vehicle_templates/ArduCopter/Tarot_X4/2024-08-05%2018-48-54.bin | ArduPilot/MethodicConfigurator (GPL-3.0) - vehicle template `ArduCopter/Tarot_X4/2024-08-05 18-48-54.bin` |
| `copter_4.6.3_quad_pid_msgs.bin` | ArduCopter V4.6.3 (c10adf90) | 1,224,704 B (1.2 MB) | 49 s; PIDR/PIDP/PIDY=488 each (10 Hz, not loop rate), IMU=2433, RATE/ATT=488, RCIN/RCOU=487. No batch IMU. | https://discuss.ardupilot.org/uploads/short-url/2iLQ52h7XuebnxNXl56s9PXlYu.zip (contains 00000001.BIN) | ArduPilot Discourse, topic 143260 "Quadcopter altitude problem", post 6 by user ljj24680 (2026-04-11) - user-uploaded log, no explicit license |
| `copter_4.4.2_auto_jitter.bin` | ArduCopter V4.4.2 (656c12a5) | 685,658 B (0.7 MB) | 47 s; IMU=1788, RATE/ATT=358 (~7.6 Hz), RCIN/RCOU=358. No PIDR/PIDP/PIDY, no batch IMU. (Correction: the three corrupt regions pymavlink reports, 54/60/52 bytes at offsets 462847/688123/1052664, are in `copter_4.6.3_quad_pid_msgs.bin`, not this file.) | https://discuss.ardupilot.org/uploads/short-url/7WaMhWHgBrhAlEFFR58cI54dkbL.bin | ArduPilot Discourse, topic 108349 "Jitter in auto modes, help with log review", post 1 by user sherry142 (2023-10-26) - user-uploaded log, no explicit license |

Notes:
- None of the ArduPilot logs found has PIDR/PIDP/PIDY at loop rate (that needs `LOG_BITMASK` fast-attitude/PID bits set by the pilot). `copter_4.6.3_quad_pid_msgs.bin` has PID messages at 10 Hz; `copter_4.5.5_tarot_x4_batch_imu.bin` has batch IMU (ISBH/ISBD) suitable for FilterReview-style FFT.
- Discourse uploads are public user posts; treat them as test-only data (no explicit license).
- Other candidates seen but not kept (too old): `dronekit/dronekit-la-testdata/log171.bin` (Copter 3.3-dev), `uladziislau/ardupilot-dataflash-log-parser Tests/Logs/dataflash-sample-{1..5}.bin` (Copter 3.6.8), `ArduPilot/pymavlink tests/test.BIN` (Plane 3.8.2-dev, 64 KB, has PIDR/PIDP/PIDY).
- Checked with no usable logs: `ArduPilot/WebTools`, `ArduPilot/UAVLogViewer` (only `vtol.tlog`), `Williangalvani/JsDataflashParser`, `ArduPilot/ardupilot Tools/Replay`, `AveryanAlex/ardupilot-binlog` (LFS pointer), `betaflight/blackbox-log-viewer`, `betaflight/blackbox-tools`, `dzikus/PIDscope`, `nerdCopter/bbl_parser`, `hajekt2/betaflight-cli` (LFS pointers, objects not fetchable).
