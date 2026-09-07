//! Betaflight MSP client: framing, transport, message layouts and a
//! high-level [`MspClient`] that reads/writes the tune with verification.
//!
//! Layouts are transcribed from betaflight-configurator `MSPHelper.js`
//! (GPL-3.0) and gated on the MSP API version the FC reports.

pub mod cli;
pub mod client;
pub mod codec;
pub mod fc_impl;
pub mod layouts;
pub mod transport;

pub use client::{ApplyOutcome, ApplyResult, MspClient};
pub use codec::{Frame, MspError};
pub use transport::{list_ports, MspLink, PortInfo, Transport};

pub mod codes {
    pub const MSP_API_VERSION: u16 = 1;
    pub const MSP_FC_VARIANT: u16 = 2;
    pub const MSP_FC_VERSION: u16 = 3;
    pub const MSP_BOARD_INFO: u16 = 4;
    pub const MSP_BUILD_INFO: u16 = 5;
    pub const MSP_SET_REBOOT: u16 = 68;
    pub const MSP_DATAFLASH_SUMMARY: u16 = 70;
    pub const MSP_DATAFLASH_READ: u16 = 71;
    pub const MSP_DATAFLASH_ERASE: u16 = 72;
    pub const MSP_SDCARD_SUMMARY: u16 = 79;
    pub const MSP_BLACKBOX_CONFIG: u16 = 80;
    pub const MSP_SET_BLACKBOX_CONFIG: u16 = 81;
    pub const MSP_ADVANCED_CONFIG: u16 = 90;
    pub const MSP_SET_ADVANCED_CONFIG: u16 = 91;
    pub const MSP_FILTER_CONFIG: u16 = 92;
    pub const MSP_SET_FILTER_CONFIG: u16 = 93;
    pub const MSP_PID_ADVANCED: u16 = 94;
    pub const MSP_SET_PID_ADVANCED: u16 = 95;
    pub const MSP_ARMING_DISABLE: u16 = 99;
    pub const MSP_STATUS: u16 = 101;
    pub const MSP_PID: u16 = 112;
    pub const MSP_SIMPLIFIED_TUNING: u16 = 140;
    pub const MSP_SET_SIMPLIFIED_TUNING: u16 = 141;
    pub const MSP_CALCULATE_SIMPLIFIED_PID: u16 = 142;
    pub const MSP_CALCULATE_SIMPLIFIED_GYRO: u16 = 143;
    pub const MSP_CALCULATE_SIMPLIFIED_DTERM: u16 = 144;
    pub const MSP_VALIDATE_SIMPLIFIED_TUNING: u16 = 145;
    pub const MSP_STATUS_EX: u16 = 150;
    pub const MSP_UID: u16 = 160;
    pub const MSP_SET_PID: u16 = 202;
    pub const MSP_EEPROM_WRITE: u16 = 250;

    pub const REBOOT_FIRMWARE: u8 = 0;
    pub const REBOOT_MSC: u8 = 2;
}
