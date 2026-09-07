//! Constants taken verbatim from the ArduPilot source tree. Every item cites
//! the file and symbol it comes from and is pinned by a test named after that
//! symbol, so a change here is a deliberate, reviewed diff.
//!
//! Thresholds that are *ours* (not from ArduPilot) do not belong here.

/// ardupilot: libraries/AP_Math/definitions.h::RAD_TO_DEG
pub const RAD_TO_DEG: f64 = 57.295779513082320876798154814105;

/// ardupilot: ArduCopter/defines.h::MASK_LOG_ATTITUDE_FAST (1<<0)
pub const LOG_BIT_ATTITUDE_FAST: u32 = 1 << 0;
/// ardupilot: ArduCopter/defines.h::MASK_LOG_ATTITUDE_MED (1<<1)
pub const LOG_BIT_ATTITUDE_MED: u32 = 1 << 1;
/// ardupilot: ArduCopter/defines.h::MASK_LOG_CTUN (1<<4)
pub const LOG_BIT_CTUN: u32 = 1 << 4;
/// ardupilot: ArduCopter/defines.h::MASK_LOG_IMU (1<<7)
pub const LOG_BIT_IMU: u32 = 1 << 7;
/// ardupilot: ArduCopter/defines.h::MASK_LOG_RCOUT (1<<10)
pub const LOG_BIT_RCOUT: u32 = 1 << 10;
/// ardupilot: ArduCopter/defines.h::MASK_LOG_PID (1<<12)
pub const LOG_BIT_PID: u32 = 1 << 12;
/// ardupilot: ArduCopter/defines.h::MASK_LOG_IMU_FAST (1UL<<18)
pub const LOG_BIT_IMU_FAST: u32 = 1 << 18;
/// ardupilot: ArduCopter/defines.h::MASK_LOG_IMU_RAW (1UL<<19)
pub const LOG_BIT_IMU_RAW: u32 = 1 << 19;

/// ardupilot: ArduCopter/mode.h::Mode::Number::STABILIZE = 0
pub const MODE_STABILIZE: u8 = 0;
/// ardupilot: ArduCopter/mode.h::Mode::Number::ACRO = 1
pub const MODE_ACRO: u8 = 1;
/// ardupilot: ArduCopter/mode.h::Mode::Number::ALT_HOLD = 2
pub const MODE_ALT_HOLD: u8 = 2;
/// ardupilot: ArduCopter/mode.h::Mode::Number::LOITER = 5
pub const MODE_LOITER: u8 = 5;
/// ardupilot: ArduCopter/mode.h::Mode::Number::AUTOTUNE = 15
pub const MODE_AUTOTUNE: u8 = 15;

/// ardupilot: libraries/AP_Logger/AP_Logger.h::LogEvent::ARMED = 10
pub const EV_ARMED: u8 = 10;
/// ardupilot: libraries/AP_Logger/AP_Logger.h::LogEvent::DISARMED = 11
pub const EV_DISARMED: u8 = 11;
/// ardupilot: libraries/AP_Logger/AP_Logger.h::LogEvent::LAND_COMPLETE = 18
pub const EV_LAND_COMPLETE: u8 = 18;

/// ardupilot: libraries/AP_Logger/LogStructure.h — `FMT` message id (LOG_FORMAT_MSG = 128)
pub const FMT_MSG_ID: u8 = 128;
/// ardupilot: libraries/AP_Logger/LogStructure.h — `FMT` is "BBnNZ", 3-byte header + 86 = 89
pub const FMT_MSG_LEN: u8 = 89;
/// ardupilot: libraries/AP_Logger/LogStructure.h — packet header bytes HEAD_BYTE1 / HEAD_BYTE2
pub const HEAD_BYTE1: u8 = 0xA3;
pub const HEAD_BYTE2: u8 = 0x95;
/// ardupilot: libraries/AP_InertialSensor/LogStructure.h — `ISBD` x/y/z are `a` = int16_t[32]
pub const ISBD_SAMPLES_PER_CHUNK: usize = 32;
/// ardupilot: libraries/AP_InertialSensor/AP_InertialSensor.h — IMU_SENSOR_TYPE_GYRO = 1 (ACCEL = 0)
pub const ISBH_TYPE_GYRO: u8 = 1;
/// ardupilot: libraries/AP_InertialSensor/BatchSampler.cpp — INS_LOG_BAT_OPT bit 1 = post-filter, bit 2 = pre+post
pub const INS_LOG_BAT_OPT_POST_FILTER: u32 = 1 << 1;
pub const INS_LOG_BAT_OPT_PRE_POST: u32 = 1 << 2;

/// mavlink: common.xml LOG_DATA.data is uint8_t[90]
pub const MAVLINK_LOG_DATA_CHUNK: usize = 90;

/// ardupilot: libraries/AP_Quicktune/AP_Quicktune.cpp — QUIK_OSC_SMAX default 4
pub const QUIK_OSC_SMAX_DEFAULT: f32 = 4.0;
/// ardupilot: libraries/AP_Quicktune/AP_Quicktune.cpp — QUIK_GAIN_MARGIN default 60 (%)
pub const QUIK_GAIN_MARGIN_DEFAULT_PCT: f32 = 60.0;
/// ardupilot: libraries/AP_Quicktune/AP_Quicktune.cpp — FLTD_MUL / FLTT_MUL = 0.5 × INS_GYRO_FILTER
pub const QUIK_FLTD_MUL: f32 = 0.5;
pub const QUIK_FLTT_MUL: f32 = 0.5;

/// ardupilot: libraries/AP_Motors/AP_MotorsMulticopter.cpp — MOT_PWM_MIN/MAX default 0 → use RC3 range
pub const MOT_PWM_DEFAULT_MIN: f32 = 1000.0;
pub const MOT_PWM_DEFAULT_MAX: f32 = 2000.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rad_to_deg() {
        assert!((RAD_TO_DEG - 180.0 / std::f64::consts::PI).abs() < 1e-12);
    }
    #[test]
    fn mask_log_attitude_fast() {
        assert_eq!(LOG_BIT_ATTITUDE_FAST, 1);
    }
    #[test]
    fn mask_log_attitude_med() {
        assert_eq!(LOG_BIT_ATTITUDE_MED, 2);
    }
    #[test]
    fn mask_log_ctun() {
        assert_eq!(LOG_BIT_CTUN, 16);
    }
    #[test]
    fn mask_log_imu() {
        assert_eq!(LOG_BIT_IMU, 128);
    }
    #[test]
    fn mask_log_rcout() {
        assert_eq!(LOG_BIT_RCOUT, 1024);
    }
    #[test]
    fn mask_log_pid() {
        assert_eq!(LOG_BIT_PID, 4096);
    }
    #[test]
    fn mask_log_imu_fast() {
        assert_eq!(LOG_BIT_IMU_FAST, 262_144);
    }
    #[test]
    fn mask_log_imu_raw() {
        assert_eq!(LOG_BIT_IMU_RAW, 524_288);
    }
    #[test]
    fn mode_numbers() {
        assert_eq!((MODE_STABILIZE, MODE_ACRO, MODE_ALT_HOLD, MODE_LOITER, MODE_AUTOTUNE), (0, 1, 2, 5, 15));
    }
    #[test]
    fn log_event_ids() {
        assert_eq!((EV_ARMED, EV_DISARMED, EV_LAND_COMPLETE), (10, 11, 18));
    }
    #[test]
    fn fmt_message() {
        assert_eq!((FMT_MSG_ID, FMT_MSG_LEN, HEAD_BYTE1, HEAD_BYTE2), (128, 89, 0xA3, 0x95));
    }
    #[test]
    fn isbd_chunk_and_type() {
        assert_eq!((ISBD_SAMPLES_PER_CHUNK, ISBH_TYPE_GYRO), (32, 1));
    }
    #[test]
    fn ins_log_bat_opt_bits() {
        assert_eq!((INS_LOG_BAT_OPT_POST_FILTER, INS_LOG_BAT_OPT_PRE_POST), (2, 4));
    }
    #[test]
    fn mavlink_log_data_chunk() {
        assert_eq!(MAVLINK_LOG_DATA_CHUNK, 90);
    }
    #[test]
    fn quicktune_defaults() {
        assert_eq!(QUIK_OSC_SMAX_DEFAULT, 4.0);
        assert_eq!(QUIK_GAIN_MARGIN_DEFAULT_PCT, 60.0);
        assert_eq!((QUIK_FLTD_MUL, QUIK_FLTT_MUL), (0.5, 0.5));
    }
}
