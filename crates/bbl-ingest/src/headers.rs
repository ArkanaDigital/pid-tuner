//! Raw header scanning, independent of the decoder, so every `H key:value`
//! line is available (the decoder consumes some of them).

use std::collections::BTreeMap;

pub const MARKER: &[u8] = b"H Product:Blackbox flight data recorder by Nicholas Sherlock";

/// Byte offsets of every session start marker.
pub fn session_offsets(bytes: &[u8]) -> Vec<usize> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(pos) = find(&bytes[i..], MARKER) {
        out.push(i + pos);
        i += pos + MARKER.len();
    }
    out
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// All `H key:value` header lines of session `index`.
pub fn raw_headers(bytes: &[u8], index: usize) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    let offsets = session_offsets(bytes);
    let Some(&start) = offsets.get(index) else { return map };
    let mut pos = start;
    while pos < bytes.len() {
        if bytes[pos] != b'H' || bytes.get(pos + 1) != Some(&b' ') {
            break;
        }
        let end = bytes[pos..].iter().position(|b| *b == b'\n').map(|e| pos + e).unwrap_or(bytes.len());
        let line = String::from_utf8_lossy(&bytes[pos + 2..end]);
        if let Some((k, v)) = line.split_once(':') {
            map.insert(k.trim().to_string(), v.trim().to_string());
        }
        pos = end + 1;
    }
    map
}

/// `"Betaflight 4.5.1 (77d01ba3b) STM32F405"` → `"4.5.1"`.
pub fn firmware_version(h: &BTreeMap<String, String>) -> Option<String> {
    let rev = h.get("Firmware revision")?;
    rev.split_whitespace().nth(1).map(str::to_string)
}
