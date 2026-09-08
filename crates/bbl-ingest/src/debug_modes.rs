//! Betaflight `debug_mode` id → name for firmware newer than the vendored
//! `blackbox-log` table (which stops at Betaflight 4.5).
//!
//! Sources (verified 2026-09-08): `src/main/build/debug.h` at tags `2025.12.2`
//! and `2026.6.1`. The id of `CHIRP` differs between the two lines because
//! 2026.6 dropped `AUTOPILOT_POSITION`; a 2026.6.0-alpha log seen in the wild
//! still used the 2025.12 numbering, so callers must treat this as a hint and
//! prefer the signal signature check in [`crate::chirp`].

/// `debugType_e` of Betaflight 2026.6.1, index = id.
const BF_2026_6: &[&str] = &[
    "NONE", "CYCLETIME", "BATTERY", "GYRO_FILTERED", "ACCELEROMETER", "PIDLOOP", "RC_INTERPOLATION", "ANGLERATE", "ESC_SENSOR", "SCHEDULER", "STACK", "ESC_SENSOR_RPM",
    "ESC_SENSOR_TMP", "ALTITUDE", "FFT", "FFT_TIME", "FFT_FREQ", "RX_FRSKY_SPI", "RX_SFHSS_SPI", "GYRO_RAW", "MULTI_GYRO_RAW", "MULTI_GYRO_DIFF", "MAX7456_SIGNAL",
    "MAX7456_SPICLOCK", "SBUS", "FPORT", "RANGEFINDER", "RANGEFINDER_QUALITY", "OPTICALFLOW", "LIDAR_TF", "ADC_INTERNAL", "RUNAWAY_TAKEOFF", "SDIO", "CURRENT_SENSOR",
    "USB", "SMARTAUDIO", "RTH", "ITERM_RELAX", "ACRO_TRAINER", "RC_SMOOTHING", "RX_SIGNAL_LOSS", "RC_SMOOTHING_RATE", "ANTI_GRAVITY", "DYN_LPF", "RX_SPEKTRUM_SPI",
    "DSHOT_RPM_TELEMETRY", "RPM_FILTER", "D_MAX", "AC_CORRECTION", "AC_ERROR", "MULTI_GYRO_SCALED", "DSHOT_RPM_ERRORS", "CRSF_LINK_STATISTICS_UPLINK",
    "CRSF_LINK_STATISTICS_PWR", "CRSF_LINK_STATISTICS_DOWN", "BARO", "AUTOPILOT_ALTITUDE", "DYN_IDLE", "FEEDFORWARD_LIMIT", "FEEDFORWARD", "BLACKBOX_OUTPUT",
    "GYRO_SAMPLE", "RX_TIMING", "D_LPF", "VTX_TRAMP", "GHST", "GHST_MSP", "SCHEDULER_DETERMINISM", "TIMING_ACCURACY", "RX_EXPRESSLRS_SPI", "RX_EXPRESSLRS_PHASELOCK",
    "RX_STATE_TIME", "GPS_RESCUE_VELOCITY", "GPS_RESCUE_HEADING", "GPS_RESCUE_TRACKING", "GPS_CONNECTION", "ATTITUDE", "VTX_MSP", "GPS_DOP", "FAILSAFE",
    "GYRO_CALIBRATION", "ANGLE_MODE", "ANGLE_TARGET", "CURRENT_ANGLE", "DSHOT_TELEMETRY_COUNTS", "RPM_LIMIT", "RC_STATS", "MAG_CALIB", "MAG_TASK_RATE", "EZLANDING",
    "TPA", "S_TERM", "SPA", "TASK", "GIMBAL", "WING_SETPOINT", "CHIRP", "FLASH_TEST_PRBS", "MAVLINK_TELEMETRY", "AUTOPILOT_PID", "POSITION_NAV", "AUTOPILOT_STOP",
];

/// 2025.12.x: identical up to `WING_SETPOINT`, then `AUTOPILOT_POSITION` before `CHIRP`.
fn bf_2025_12(raw: u32) -> Option<&'static str> {
    const TAIL: &[&str] = &["AUTOPILOT_POSITION", "CHIRP", "FLASH_TEST_PRBS", "MAVLINK_TELEMETRY"];
    let wing = BF_2026_6.iter().position(|n| *n == "WING_SETPOINT")? as u32;
    if raw <= wing {
        BF_2026_6.get(raw as usize).copied()
    } else {
        TAIL.get((raw - wing - 1) as usize).copied()
    }
}

/// Name for a raw `debug_mode` id on Betaflight `major.minor` (calendar versions only).
pub fn name_for(raw: u32, major_minor: (u32, u32)) -> Option<&'static str> {
    match major_minor {
        (2025, _) => bf_2025_12(raw),
        (y, _) if y >= 2026 => BF_2026_6.get(raw as usize).copied(),
        _ => None,
    }
}

/// Ids that mean `CHIRP` on any known calendar version (2025.12: 97, 2026.6: 96).
pub fn chirp_ids() -> &'static [u32] {
    &[96, 97]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chirp_ids_per_line() {
        assert_eq!(name_for(97, (2025, 12)), Some("CHIRP"));
        assert_eq!(name_for(96, (2025, 12)), Some("AUTOPILOT_POSITION"));
        assert_eq!(name_for(96, (2026, 6)), Some("CHIRP"));
        assert_eq!(name_for(97, (2026, 6)), Some("FLASH_TEST_PRBS"));
        assert_eq!(name_for(89, (2026, 6)), Some("EZLANDING"));
        assert_eq!(name_for(89, (2025, 12)), Some("EZLANDING"));
        assert_eq!(name_for(0, (2026, 6)), Some("NONE"));
        assert_eq!(name_for(500, (2026, 6)), None);
        assert_eq!(name_for(97, (4, 5)), None);
        assert!(chirp_ids().contains(&96) && chirp_ids().contains(&97));
    }
}
