# Local patches to blackbox-log 0.4.3

Upstream: https://github.com/blackbox-log/blackbox-log (MIT OR Apache-2.0).

1. `src/lib.rs`: `BETAFLIGHT_SUPPORT` upper bound raised from 4.6.0 to 2100.0.0 so
   Betaflight 4.6 and calendar-versioned releases (2025.12+) are not rejected.
2. `src/headers.rs`: `FirmwareVersion::major` widened to `u16`; versions ≥ 4.6 map
   to `InternalFirmware::Betaflight4_5` field semantics.
3. `Cargo.toml`: test/bench targets and dev-dependencies removed (fixtures not vendored).

4. `src/filter.rs`: `Filter::apply` compared full field names against base names, so
   `OnlyFields(["gyroADC"])` never matched `gyroADC[0]`. Now compares base names.

5. `src/generated/{debug_mode,pwm_protocol}.rs`, `src/headers.rs`: unknown `debug_mode` /
   `motor_pwm_protocol` ids (Betaflight 2025.12+/2026.6 add e.g. `CHIRP` = 96/97 depending on
   the tag) no longer fail header parsing; they map to `DebugMode::Unknown(raw)` /
   `PwmProtocol::Unknown(raw)` and `Headers::debug_mode_raw()` exposes the id. `FirmwareVersion::parse` ignores a
   pre-release suffix (`2026.6.0-alpha`).

Re-verify these when bumping the upstream version.
