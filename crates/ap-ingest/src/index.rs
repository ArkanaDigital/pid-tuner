//! Phase 1: scan the file, learn FMT definitions, index message offsets.
//! Behaviour matches pymavlink `DFReader_binary._parse_next`: on a bad
//! header or unknown type, advance one byte; a trailing message that does not
//! fit is dropped.

use crate::format::{cstr, FmtDef};
use domain::ap_consts::{FMT_MSG_ID, HEAD_BYTE1, HEAD_BYTE2};
use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct ScanStats {
    pub messages: usize,
    /// (offset, skipped_bytes) of each corrupt region.
    pub corrupt_regions: Vec<(usize, usize)>,
    pub warnings: Vec<String>,
}

pub struct Index<'a> {
    pub bytes: &'a [u8],
    defs: Vec<Option<FmtDef>>,
    by_name: HashMap<String, u8>,
    /// Payload start offsets per message id.
    offsets: HashMap<u8, Vec<usize>>,
    pub stats: ScanStats,
    /// Decoded `PARM` (last value wins) in file order.
    pub params: Vec<(String, f32)>,
    pub messages: Vec<String>,
}

impl<'a> Index<'a> {
    pub fn scan(bytes: &'a [u8]) -> Self {
        let mut ix = Index {
            bytes,
            defs: vec![None; 256],
            by_name: HashMap::new(),
            offsets: HashMap::new(),
            stats: ScanStats::default(),
            params: Vec::new(),
            messages: Vec::new(),
        };
        // Bootstrap FMT so the first FMT message can be decoded.
        let fmt = FmtDef::new(
            FMT_MSG_ID,
            89,
            "FMT",
            "BBnNZ",
            "Type,Length,Name,Format,Columns",
        )
        .unwrap();
        ix.by_name.insert("FMT".into(), FMT_MSG_ID);
        ix.defs[FMT_MSG_ID as usize] = Some(fmt);

        let n = bytes.len();
        let mut pos = 0usize;
        let mut skip_start: Option<usize> = None;
        while pos + 3 <= n {
            let ok_header = bytes[pos] == HEAD_BYTE1 && bytes[pos + 1] == HEAD_BYTE2;
            let id = bytes[pos + 2];
            let def_len = if ok_header {
                ix.defs[id as usize].as_ref().map(|d| d.len as usize)
            } else {
                None
            };
            let Some(len) = def_len else {
                if skip_start.is_none() {
                    skip_start = Some(pos);
                }
                pos += 1;
                continue;
            };
            if pos + len > n {
                // truncated trailing message: drop it
                ix.stats
                    .warnings
                    .push(format!("truncated trailing message id {id} at {pos}"));
                break;
            }
            if let Some(s) = skip_start.take() {
                ix.stats.corrupt_regions.push((s, pos - s));
            }
            let payload = &bytes[pos + 3..pos + len];
            ix.stats.messages += 1;
            match id {
                FMT_MSG_ID => ix.learn_fmt(payload),
                _ => {
                    ix.offsets.entry(id).or_default().push(pos + 3);
                    ix.eager(id, payload);
                }
            }
            pos += len;
        }
        if let Some(s) = skip_start {
            if n > s {
                ix.stats.corrupt_regions.push((s, n - s));
            }
        }
        ix
    }

    fn learn_fmt(&mut self, p: &[u8]) {
        let id = p[0];
        let len = p[1];
        let name = cstr(&p[2..6]);
        let format = cstr(&p[6..22]);
        let columns = cstr(&p[22..86]);
        match FmtDef::new(id, len, &name, &format, &columns) {
            Ok(d) => {
                if d.columns_padded {
                    self.stats.warnings.push(format!(
                        "FMT {name} (id {id}): column names inconsistent with format, padded"
                    ));
                }
                if id != FMT_MSG_ID {
                    self.by_name.insert(d.name.clone(), id);
                    self.defs[id as usize] = Some(d);
                }
            }
            Err(e) => self
                .stats
                .warnings
                .push(format!("FMT {name} (id {id}) ignored: {e}")),
        }
    }

    /// Small messages we always decode during the scan.
    fn eager(&mut self, id: u8, p: &[u8]) {
        let Some(def) = self.defs[id as usize].as_ref() else {
            return;
        };
        match def.name.as_str() {
            "FMTU" => {
                // QBNN: TimeUS, FmtType, UnitIds, MultIds
                if p.len() >= 8 + 1 + 16 + 16 {
                    let t = p[8];
                    let u = cstr(&p[9..25]);
                    let m = cstr(&p[25..41]);
                    if let Some(d) = self.defs[t as usize].as_mut() {
                        d.apply_units(&u, &m);
                    }
                }
            }
            "PARM" => {
                // QNf[f]: TimeUS, Name, Value[, Default]
                if let (Some(ci), Some(vi)) = (def.col("Name"), def.col("Value")) {
                    let name = cstr(&p[def.offsets[ci]..def.offsets[ci] + 16]);
                    let o = def.offsets[vi];
                    let v = f32::from_le_bytes([p[o], p[o + 1], p[o + 2], p[o + 3]]);
                    self.params.push((name, v));
                }
            }
            "MSG" => {
                if let Some(ci) = def.col("Message") {
                    self.messages
                        .push(cstr(&p[def.offsets[ci]..def.offsets[ci] + 64]));
                }
            }
            _ => {}
        }
    }

    pub fn def(&self, name: &str) -> Option<&FmtDef> {
        self.by_name
            .get(name)
            .and_then(|id| self.defs[*id as usize].as_ref())
    }

    pub fn def_by_id(&self, id: u8) -> Option<&FmtDef> {
        self.defs[id as usize].as_ref()
    }

    pub fn defs(&self) -> impl Iterator<Item = &FmtDef> {
        self.defs.iter().filter_map(|d| d.as_ref())
    }

    pub fn count(&self, name: &str) -> usize {
        self.by_name
            .get(name)
            .and_then(|id| self.offsets.get(id))
            .map(|v| v.len())
            .unwrap_or(0)
    }

    pub fn offsets(&self, name: &str) -> &[usize] {
        self.by_name
            .get(name)
            .and_then(|id| self.offsets.get(id))
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    pub fn param(&self, name: &str) -> Option<f32> {
        self.params
            .iter()
            .rev()
            .find(|(n, _)| n == name)
            .map(|(_, v)| *v)
    }
}
