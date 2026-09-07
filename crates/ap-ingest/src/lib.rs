//! ArduPilot DataFlash (`.bin`) → [`domain::FlightLog`].
//!
//! Two-phase design ported from `JsDataflashParser` (GPL-3.0): phase 1 scans
//! the file once and records the payload offset of every message per type
//! (FMT-driven, resynchronising byte-wise on corruption exactly like
//! pymavlink `DFReader_binary`); phase 2 decodes single columns lazily.
//! Field-size / scaling rules follow pymavlink `FORMAT_TO_STRUCT`:
//! `c/C/e/E` ÷100, `L` ÷1e7, everything else raw. `FMTU` multipliers are
//! metadata only (they never scale values).

pub mod decode;
pub mod format;
pub mod index;
pub mod map;
pub mod rates;

pub use format::{FieldType, FmtDef};
pub use index::{Index, ScanStats};
pub use map::{ingest, list_sessions, IngestError, IngestOpts, SessionInfo};

/// True when the bytes look like an ArduPilot DataFlash log: a `FMT` message
/// describing `FMT` itself appears in the first 64 KiB.
pub fn looks_like_dataflash(bytes: &[u8]) -> bool {
    use domain::ap_consts::{FMT_MSG_ID, FMT_MSG_LEN, HEAD_BYTE1, HEAD_BYTE2};
    let n = bytes.len().min(64 * 1024);
    let b = &bytes[..n];
    let mut i = 0;
    while i + 8 <= b.len() {
        if b[i] == HEAD_BYTE1 && b[i + 1] == HEAD_BYTE2 && b[i + 2] == FMT_MSG_ID && b[i + 3] == FMT_MSG_ID && b[i + 4] == FMT_MSG_LEN && &b[i + 5..i + 8] == b"FMT" {
            return true;
        }
        i += 1;
    }
    false
}
