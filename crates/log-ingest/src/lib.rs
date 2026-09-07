//! Format detection and dispatch: Betaflight blackbox vs ArduPilot DataFlash.

use domain::FlightLog;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogFormat {
    Betaflight,
    ArduPilot,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub index: usize,
    pub format: LogFormat,
    pub firmware_revision: String,
    pub craft_name: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum IngestError {
    #[error("unrecognised log format (not a Betaflight blackbox nor an ArduPilot DataFlash .bin)")]
    Unknown,
    #[error(transparent)]
    Betaflight(#[from] bbl_ingest::IngestError),
    #[error(transparent)]
    ArduPilot(#[from] ap_ingest::IngestError),
}

pub fn detect(bytes: &[u8]) -> LogFormat {
    let head = &bytes[..bytes.len().min(64 * 1024)];
    if head.windows(bbl_ingest::headers::MARKER.len()).any(|w| w == bbl_ingest::headers::MARKER) {
        LogFormat::Betaflight
    } else if ap_ingest::looks_like_dataflash(bytes) {
        LogFormat::ArduPilot
    } else {
        LogFormat::Unknown
    }
}

pub fn list_sessions(bytes: &[u8]) -> Vec<SessionInfo> {
    match detect(bytes) {
        LogFormat::Betaflight => bbl_ingest::list_sessions(bytes)
            .into_iter()
            .map(|s| SessionInfo { index: s.index, format: LogFormat::Betaflight, firmware_revision: s.firmware_revision, craft_name: s.craft_name, error: s.error })
            .collect(),
        LogFormat::ArduPilot => ap_ingest::list_sessions(bytes)
            .into_iter()
            .map(|s| SessionInfo { index: s.index, format: LogFormat::ArduPilot, firmware_revision: s.firmware_revision, craft_name: s.craft_name, error: s.error })
            .collect(),
        LogFormat::Unknown => vec![SessionInfo { index: 0, format: LogFormat::Unknown, firmware_revision: String::new(), craft_name: None, error: Some(IngestError::Unknown.to_string()) }],
    }
}

pub fn ingest(bytes: &[u8], session: usize) -> Result<FlightLog, IngestError> {
    match detect(bytes) {
        LogFormat::Betaflight => Ok(bbl_ingest::ingest(bytes, session, &Default::default())?),
        LogFormat::ArduPilot => Ok(ap_ingest::ingest(bytes, session, &Default::default())?),
        LogFormat::Unknown => Err(IngestError::Unknown),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_by_magic() {
        assert_eq!(detect(b"H Product:Blackbox flight data recorder by Nicholas Sherlock\nH Data version:2\n"), LogFormat::Betaflight);
        let mut ap = vec![0xA3u8, 0x95, 128, 128, 89];
        ap.extend_from_slice(b"FMT\0");
        ap.extend_from_slice(&[0u8; 100]);
        assert_eq!(detect(&ap), LogFormat::ArduPilot);
        assert_eq!(detect(b"hello world"), LogFormat::Unknown);
    }
}
