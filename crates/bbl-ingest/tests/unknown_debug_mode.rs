//! Newer firmware adds `debug_mode` ids the vendored parser table does not know
//! (e.g. CHIRP). The header must still parse (vendor patch 5).
use std::path::PathBuf;

fn fixture(rel: &str) -> Option<Vec<u8>> {
    std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/bf").join(rel)).ok()
}

#[test]
fn unknown_debug_mode_id_still_ingests() {
    let Some(bytes) = fixture("bf_2025.12.2_speedybeef7v3_steadyhover.BFL") else { return };
    let needle = b"H debug_mode:16";
    let pos = bytes.windows(needle.len()).position(|w| w == needle).expect("fixture has debug_mode 16");
    let mut mutated = bytes.clone();
    mutated[pos..pos + needle.len()].copy_from_slice(b"H debug_mode:97");
    let a = bbl_ingest::ingest(&bytes, 0, &Default::default()).unwrap();
    let b = bbl_ingest::ingest(&mutated, 0, &Default::default()).unwrap();
    assert_eq!(a.axes[0].gyro_filt.len(), b.axes[0].gyro_filt.len());
    assert_eq!(b.meta.headers.get("bf.debug_mode_raw").map(String::as_str), Some("97"));
    // 2025.12 numbering: 97 = CHIRP, so the name resolves even though the vendored table stops at 4.5
    assert_eq!(b.meta.debug_mode.as_deref(), Some("CHIRP"));
    // the debug channels of a hover log do not look like a sweep → no segments
    assert!(b.chirp.as_ref().map(|c| c.segments.is_empty()).unwrap_or(true));
}

#[test]
fn prerelease_firmware_suffix_is_accepted() {
    let Some(bytes) = fixture("bf_2025.12.2_speedybeef7v3_steadyhover.BFL") else { return };
    let needle = b"H Firmware revision:Betaflight 2025.12.2";
    let pos = bytes.windows(needle.len()).position(|w| w == needle).expect("revision header");
    let mut m = bytes.clone();
    // same length replacement: "2025.12.2" -> "2026.6.0-" (+ next char becomes part of the suffix)
    m[pos..pos + needle.len()].copy_from_slice(b"H Firmware revision:Betaflight 2026.6.0-");
    let log = bbl_ingest::ingest(&m, 0, &Default::default()).unwrap();
    assert!(matches!(&log.firmware, domain::Firmware::Betaflight { version, .. } if version.starts_with("2026.6.0")));
}
