//! Parameter store + MAVLink param helpers.
//!
//! ArduPilot facts (libraries/GCS_MAVLink/GCS_Param.cpp, libraries/AP_Param/AP_Param.cpp):
//! * values travel *by value* as float (`vp->set_float(packet.param_value, var_type)`,
//!   `cast_to_float`), never byte-cast — so an INT32 6 is sent as 6.0.
//! * a successful `PARAM_SET` is acknowledged by `AP_Param::save_sync` →
//!   `GCS_SEND_PARAM` → `PARAM_VALUE` with `param_index = -1` (65535); an unknown
//!   name gets `PARAM_ERROR`/silence, a read-only one echoes the *old* value.
//! * `PARAM_REQUEST_READ` by name uses `param_index = -1`; the reply carries the
//!   requested index back, NaN value when the name does not exist.
//! * `PARAM_REQUEST_LIST` streams at most 5 params per tick without flow control,
//!   so a full list of ~1000 params takes seconds and may drop entries.

use crate::apm::{MavParamType, PARAM_VALUE_DATA};
use domain::tune::{ApTune, MavParamType as DomainType};
use mavlink::types::CharArray;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub name: String,
    pub value: f32,
    pub ptype: MavParamType,
    pub index: u16,
}

#[derive(Debug, Default, Clone)]
pub struct ParamStore {
    pub params: BTreeMap<String, Param>,
    /// `param_count` reported by the FC (total, including ones we may not have yet).
    pub count: Option<u16>,
}

pub fn param_id(name: &str) -> CharArray<16> {
    let mut a = [0u8; 16];
    for (i, b) in name.bytes().take(16).enumerate() {
        a[i] = b;
    }
    CharArray::new(a)
}

/// NUL-terminated only if shorter than 16 (common.xml PARAM_VALUE.param_id).
pub fn name_of(id: &CharArray<16>) -> String {
    let raw: [u8; 16] = (*id).into();
    let end = raw.iter().position(|&b| b == 0).unwrap_or(16);
    String::from_utf8_lossy(&raw[..end]).to_string()
}

pub fn is_integer_type(t: MavParamType) -> bool {
    !matches!(
        t,
        MavParamType::MAV_PARAM_TYPE_REAL32 | MavParamType::MAV_PARAM_TYPE_REAL64
    )
}

/// Read-back comparison: integers exact, floats within 1e-6 relative
/// (values pass through `float` on both ends, so anything tighter is noise).
pub fn values_match(t: MavParamType, wanted: f32, got: f32) -> bool {
    if is_integer_type(t) {
        wanted.round() == got.round()
    } else {
        (wanted - got).abs() <= 1e-6 * wanted.abs().max(1.0)
    }
}

pub fn to_domain_type(t: MavParamType) -> DomainType {
    match t {
        MavParamType::MAV_PARAM_TYPE_UINT8 | MavParamType::MAV_PARAM_TYPE_INT8 => DomainType::Int8,
        MavParamType::MAV_PARAM_TYPE_UINT16 | MavParamType::MAV_PARAM_TYPE_INT16 => {
            DomainType::Int16
        }
        MavParamType::MAV_PARAM_TYPE_REAL32 | MavParamType::MAV_PARAM_TYPE_REAL64 => {
            DomainType::Real32
        }
        _ => DomainType::Int32,
    }
}

impl ParamStore {
    pub fn insert(&mut self, v: &PARAM_VALUE_DATA) -> Param {
        let p = Param {
            name: name_of(&v.param_id),
            value: v.param_value,
            ptype: v.param_type,
            index: v.param_index,
        };
        if v.param_count > 0 {
            self.count = Some(v.param_count);
        }
        // index 65535 (-1) replies do not tell us the position; keep the old index.
        let mut stored = p.clone();
        if p.index == u16::MAX {
            if let Some(old) = self.params.get(&p.name) {
                stored.index = old.index;
            }
        }
        self.params.insert(p.name.clone(), stored);
        p
    }

    pub fn get(&self, name: &str) -> Option<&Param> {
        self.params.get(name)
    }

    pub fn value(&self, name: &str) -> Option<f32> {
        self.params.get(name).map(|p| p.value)
    }

    /// Indices 0..count not yet received (for `PARAM_REQUEST_READ` retries).
    pub fn missing_indices(&self) -> Vec<u16> {
        let Some(n) = self.count else {
            return Vec::new();
        };
        let mut have = vec![false; n as usize];
        for p in self.params.values() {
            if (p.index as usize) < have.len() {
                have[p.index as usize] = true;
            }
        }
        have.iter()
            .enumerate()
            .filter(|(_, h)| !**h)
            .map(|(i, _)| i as u16)
            .collect()
    }

    pub fn is_complete(&self) -> bool {
        self.count
            .map(|n| self.params.len() >= n as usize)
            .unwrap_or(false)
    }

    pub fn tune(&self) -> ApTune {
        let mut t = ApTune::default();
        for p in self.params.values() {
            t.params.insert(p.name.clone(), p.value);
            t.types.insert(p.name.clone(), to_domain_type(p.ptype));
        }
        t
    }

    /// Mission Planner / QGC `.param` text: `NAME,VALUE` per line, `#` comments.
    pub fn to_param_text(&self, header: &str) -> String {
        let mut s = String::new();
        for line in header.lines() {
            s.push_str("# ");
            s.push_str(line);
            s.push('\n');
        }
        for p in self.params.values() {
            s.push_str(&p.name);
            s.push(',');
            s.push_str(&fmt_value(p.ptype, p.value));
            s.push('\n');
        }
        s
    }
}

pub fn fmt_value(t: MavParamType, v: f32) -> String {
    if is_integer_type(t) {
        format!("{}", v.round() as i64)
    } else {
        let s = format!("{v}");
        if s.contains('.') || s.contains('e') {
            s
        } else {
            format!("{s}.0")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn param_id_roundtrip_and_16_char_names() {
        assert_eq!(name_of(&param_id("ATC_RAT_RLL_P")), "ATC_RAT_RLL_P");
        // exactly 16 chars: no NUL terminator on the wire
        let n = "INS_HNTCH_FM_RAT"; // 16 chars
        assert_eq!(n.len(), 16);
        assert_eq!(name_of(&param_id(n)), n);
    }

    #[test]
    fn integer_types_compare_exact_and_floats_relative() {
        assert!(values_match(
            MavParamType::MAV_PARAM_TYPE_INT32,
            180222.0,
            180222.0
        ));
        assert!(!values_match(
            MavParamType::MAV_PARAM_TYPE_INT32,
            180222.0,
            180223.0
        ));
        assert!(values_match(
            MavParamType::MAV_PARAM_TYPE_REAL32,
            0.135,
            0.135
        ));
        assert!(!values_match(
            MavParamType::MAV_PARAM_TYPE_REAL32,
            0.135,
            0.136
        ));
    }

    #[test]
    fn missing_indices_after_partial_list() {
        let mut s = ParamStore::default();
        for (i, n) in ["A", "B", "D"].iter().enumerate() {
            let idx = if *n == "D" { 3 } else { i as u16 };
            s.insert(&PARAM_VALUE_DATA {
                param_value: 1.0,
                param_count: 5,
                param_index: idx,
                param_id: param_id(n),
                param_type: MavParamType::MAV_PARAM_TYPE_REAL32,
            });
        }
        assert_eq!(s.missing_indices(), vec![2, 4]);
        assert!(!s.is_complete());
    }

    #[test]
    fn param_text_is_mission_planner_format() {
        let mut s = ParamStore::default();
        s.insert(&PARAM_VALUE_DATA {
            param_value: 180222.0,
            param_count: 2,
            param_index: 0,
            param_id: param_id("LOG_BITMASK"),
            param_type: MavParamType::MAV_PARAM_TYPE_INT32,
        });
        s.insert(&PARAM_VALUE_DATA {
            param_value: 0.135,
            param_count: 2,
            param_index: 1,
            param_id: param_id("ATC_RAT_RLL_P"),
            param_type: MavParamType::MAV_PARAM_TYPE_REAL32,
        });
        let t = s.to_param_text("PID Tuner backup");
        assert_eq!(
            t,
            "# PID Tuner backup\nATC_RAT_RLL_P,0.135\nLOG_BITMASK,180222\n"
        );
    }
}
