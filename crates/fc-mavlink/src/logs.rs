//! DataFlash log listing and download over MAVLink.
//!
//! ArduPilot facts (libraries/AP_Logger/AP_Logger_MAVLinkLogTransfer.cpp):
//! * `LOG_REQUEST_LIST(start, end)`: start clamped to ≥ 1, end to ≤ num_logs; with no
//!   logs one `LOG_ENTRY` with id 0 / num_logs 0 is sent. One `LOG_ENTRY` per call.
//! * `LOG_REQUEST_DATA(id, ofs, count)`: id must be 1..=num_logs (else the transfer
//!   is silently cancelled); remaining = min(size − ofs, count).
//! * `LOG_DATA` carries `MAVLINK_MSG_LOG_DATA_FIELD_DATA_LEN` = 90 bytes; the transfer
//!   ends when a chunk is shorter than 90 bytes or remaining hits 0. Packets per
//!   scheduler call vary (1…250 by link bandwidth), so drops are normal → bitmap + re-request.

use crate::apm::{MavMessage, LOG_REQUEST_DATA_DATA, LOG_REQUEST_END_DATA, LOG_REQUEST_LIST_DATA};
use crate::client::MavClient;
use domain::ap_consts::MAVLINK_LOG_DATA_CHUNK;
use domain::fc::{FcError, LogEntry, ProgressFn};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const CHUNK: usize = MAVLINK_LOG_DATA_CHUNK;

pub fn list_logs(c: &mut MavClient) -> Result<Vec<LogEntry>, FcError> {
    let (sys, comp) = c.target();
    c.send(&MavMessage::LOG_REQUEST_LIST(LOG_REQUEST_LIST_DATA {
        start: 0,
        end: u16::MAX,
        target_system: sys,
        target_component: comp,
    }))?;
    let mut out = Vec::new();
    let mut idle = Instant::now();
    loop {
        match c.pump(Duration::from_millis(200))? {
            Some(MavMessage::LOG_ENTRY(e)) => {
                idle = Instant::now();
                if e.num_logs == 0 {
                    break;
                }
                out.push(LogEntry {
                    id: e.id as u32,
                    size: e.size as u64,
                    time_utc: (e.time_utc != 0).then_some(e.time_utc as u64),
                });
                if e.id == e.last_log_num {
                    break;
                }
            }
            _ => {
                if idle.elapsed() > Duration::from_secs(3) {
                    if out.is_empty() {
                        return Err(FcError::Timeout(
                            "no LOG_ENTRY reply (is LOG_BACKEND_TYPE File and an SD card present?)"
                                .into(),
                        ));
                    }
                    break;
                }
            }
        }
    }
    out.sort_by_key(|e| e.id);
    Ok(out)
}

/// Download log `id` (size from the listing) with a chunk bitmap so dropped
/// packets are re-requested; byte-identical to the file on the SD card.
pub fn download(
    c: &mut MavClient,
    id: u32,
    size: u64,
    progress: ProgressFn<'_>,
    cancel: &AtomicBool,
) -> Result<Vec<u8>, FcError> {
    let (sys, comp) = c.target();
    let size = size as usize;
    let n_chunks = size.div_ceil(CHUNK).max(1);
    let mut have = vec![false; n_chunks];
    let mut buf = vec![0u8; size];
    let mut got_chunks = 0usize;
    let mut done_short = false;
    let request = |c: &mut MavClient, ofs: u32, count: u32| {
        c.send(&MavMessage::LOG_REQUEST_DATA(LOG_REQUEST_DATA_DATA {
            ofs,
            count,
            id: id as u16,
            target_system: sys,
            target_component: comp,
        }))
    };
    request(c, 0, u32::MAX)?;
    let mut idle = Instant::now();
    let mut last_progress = Instant::now();
    let mut retries = 0u32;
    while got_chunks < n_chunks {
        if cancel.load(Ordering::Relaxed) {
            let _ = c.send(&MavMessage::LOG_REQUEST_END(LOG_REQUEST_END_DATA {
                target_system: sys,
                target_component: comp,
            }));
            return Err(FcError::Other("cancelled".into()));
        }
        match c.pump(Duration::from_millis(100))? {
            Some(MavMessage::LOG_DATA(d)) if d.id as u32 == id => {
                idle = Instant::now();
                let ofs = d.ofs as usize;
                let n = d.count as usize;
                if !ofs.is_multiple_of(CHUNK) || ofs >= size {
                    continue;
                }
                let k = ofs / CHUNK;
                let n = n.min(size - ofs);
                if !have[k] && (n == CHUNK || ofs + n == size) {
                    buf[ofs..ofs + n].copy_from_slice(&d.data[..n]);
                    have[k] = true;
                    got_chunks += 1;
                }
                if n < CHUNK {
                    done_short = true;
                }
                if last_progress.elapsed() > Duration::from_millis(200) {
                    progress((got_chunks * CHUNK).min(size) as u64, size as u64);
                    last_progress = Instant::now();
                }
            }
            _ => {}
        }
        // Stalled (drops, or the FC finished its burst / hit the end): re-request the first hole.
        let stalled = idle.elapsed() > Duration::from_millis(if done_short { 300 } else { 1500 });
        if stalled && got_chunks < n_chunks {
            retries += 1;
            if retries > 200 {
                return Err(FcError::Timeout(format!("log {id}: gave up after {retries} re-requests ({got_chunks}/{n_chunks} chunks)")));
            }
            let first = have.iter().position(|h| !h).unwrap();
            let run_end = have[first..]
                .iter()
                .position(|h| *h)
                .map(|p| first + p)
                .unwrap_or(n_chunks);
            let ofs = (first * CHUNK) as u32;
            let count = ((run_end - first) * CHUNK) as u32;
            request(c, ofs, count)?;
            idle = Instant::now();
            done_short = false;
        }
    }
    progress(size as u64, size as u64);
    c.send(&MavMessage::LOG_REQUEST_END(LOG_REQUEST_END_DATA {
        target_system: sys,
        target_component: comp,
    }))?;
    Ok(buf)
}
