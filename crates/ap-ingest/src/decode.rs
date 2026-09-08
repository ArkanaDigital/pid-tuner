//! Phase 2: lazy column decoding.

use crate::format::{cstr, FieldType, FmtDef};
use crate::index::Index;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    F(f64),
    I(i64),
    U(u64),
    Str(String),
    I16x32(Vec<i16>),
}

impl Value {
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::F(v) => Some(*v),
            Value::I(v) => Some(*v as f64),
            Value::U(v) => Some(*v as f64),
            _ => None,
        }
    }
}

/// Decode one field of a payload. Scaling follows pymavlink `FORMAT_TO_STRUCT`.
pub fn decode_field(p: &[u8], off: usize, t: FieldType) -> Value {
    let b = &p[off..];
    match t {
        FieldType::I8 => Value::I(b[0] as i8 as i64),
        FieldType::U8 | FieldType::M => Value::U(b[0] as u64),
        FieldType::I16 => Value::I(i16::from_le_bytes([b[0], b[1]]) as i64),
        FieldType::U16 => Value::U(u16::from_le_bytes([b[0], b[1]]) as u64),
        FieldType::I32 => Value::I(i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as i64),
        FieldType::U32 => Value::U(u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as u64),
        FieldType::F32 => Value::F(f32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64),
        FieldType::F16 => Value::F(half_to_f32(u16::from_le_bytes([b[0], b[1]])) as f64),
        FieldType::F64 => Value::F(f64::from_le_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ])),
        FieldType::I64 => Value::I(i64::from_le_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ])),
        FieldType::U64 => Value::U(u64::from_le_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ])),
        // pymavlink divides by 1/mult for accuracy: 12345 -> 123.45 exactly
        FieldType::C16 => Value::F(i16::from_le_bytes([b[0], b[1]]) as f64 / 100.0),
        FieldType::CU16 => Value::F(u16::from_le_bytes([b[0], b[1]]) as f64 / 100.0),
        FieldType::E32 => Value::F(i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64 / 100.0),
        FieldType::EU32 => Value::F(u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64 / 100.0),
        FieldType::L => Value::F(i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64 / 1e7),
        FieldType::N4 => Value::Str(cstr(&b[..4])),
        FieldType::N16 => Value::Str(cstr(&b[..16])),
        FieldType::Z64 => Value::Str(cstr(&b[..64])),
        FieldType::A => Value::I16x32(
            (0..32)
                .map(|k| i16::from_le_bytes([b[2 * k], b[2 * k + 1]]))
                .collect(),
        ),
    }
}

/// IEEE 754 binary16 → f32.
pub fn half_to_f32(h: u16) -> f32 {
    let sign = ((h >> 15) & 1) as u32;
    let exp = ((h >> 10) & 0x1f) as i32;
    let frac = (h & 0x3ff) as u32;
    let bits = if exp == 0 {
        if frac == 0 {
            sign << 31
        } else {
            // subnormal
            let mut e = -1i32;
            let mut f = frac;
            while f & 0x400 == 0 {
                f <<= 1;
                e -= 1;
            }
            let f = (f & 0x3ff) << 13;
            (sign << 31) | (((127 - 15 + 1 + e) as u32) << 23) | f
        }
    } else if exp == 31 {
        (sign << 31) | 0x7f80_0000 | (frac << 13)
    } else {
        (sign << 31) | (((exp + 127 - 15) as u32) << 23) | (frac << 13)
    };
    f32::from_bits(bits)
}

impl<'a> Index<'a> {
    fn payloads(&self, def: &FmtDef, name: &str) -> impl Iterator<Item = &'a [u8]> + '_ {
        let plen = def.payload_len();
        let bytes = self.bytes;
        self.offsets(name).iter().map(move |&o| &bytes[o..o + plen])
    }

    /// Whole column as f64 (numeric fields only). `None` if the message or column is unknown.
    pub fn column_f64(&self, msg: &str, col: &str) -> Option<Vec<f64>> {
        let def = self.def(msg)?;
        let ci = def.col(col)?;
        let t = def.types[ci];
        if !t.is_numeric() {
            return None;
        }
        let off = def.offsets[ci];
        Some(
            self.payloads(def, msg)
                .map(|p| decode_field(p, off, t).as_f64().unwrap_or(f64::NAN))
                .collect(),
        )
    }

    pub fn column_u64(&self, msg: &str, col: &str) -> Option<Vec<u64>> {
        let def = self.def(msg)?;
        let ci = def.col(col)?;
        let t = def.types[ci];
        let off = def.offsets[ci];
        Some(
            self.payloads(def, msg)
                .map(|p| match decode_field(p, off, t) {
                    Value::U(v) => v,
                    Value::I(v) => v as u64,
                    Value::F(v) => v as u64,
                    _ => 0,
                })
                .collect(),
        )
    }

    pub fn column_i16x32(&self, msg: &str, col: &str) -> Option<Vec<Vec<i16>>> {
        let def = self.def(msg)?;
        let ci = def.col(col)?;
        if def.types[ci] != FieldType::A {
            return None;
        }
        let off = def.offsets[ci];
        Some(
            self.payloads(def, msg)
                .map(|p| match decode_field(p, off, FieldType::A) {
                    Value::I16x32(v) => v,
                    _ => vec![],
                })
                .collect(),
        )
    }

    pub fn column_str(&self, msg: &str, col: &str) -> Option<Vec<String>> {
        let def = self.def(msg)?;
        let ci = def.col(col)?;
        let t = def.types[ci];
        let off = def.offsets[ci];
        Some(
            self.payloads(def, msg)
                .map(|p| match decode_field(p, off, t) {
                    Value::Str(s) => s,
                    v => format!("{v:?}"),
                })
                .collect(),
        )
    }

    /// One decoded row (all columns) by row index.
    pub fn row(&self, msg: &str, i: usize) -> Option<Vec<(String, Value)>> {
        let def = self.def(msg)?;
        let o = *self.offsets(msg).get(i)?;
        let p = &self.bytes[o..o + def.payload_len()];
        Some(
            def.columns
                .iter()
                .enumerate()
                .map(|(k, c)| (c.clone(), decode_field(p, def.offsets[k], def.types[k])))
                .collect(),
        )
    }

    /// Distinct instance ids (from the `#` unit column), in first-seen order.
    pub fn instances(&self, msg: &str) -> Vec<u8> {
        let Some(def) = self.def(msg) else {
            return vec![];
        };
        let Some(ci) = def.instance_col else {
            return vec![];
        };
        let off = def.offsets[ci];
        let t = def.types[ci];
        let mut out: Vec<u8> = Vec::new();
        for p in self.payloads(def, msg) {
            let v = decode_field(p, off, t).as_f64().unwrap_or(0.0) as u8;
            if !out.contains(&v) {
                out.push(v);
            }
        }
        out
    }

    /// Row indices belonging to one instance.
    pub fn instance_rows(&self, msg: &str, inst: u8) -> Vec<usize> {
        let Some(def) = self.def(msg) else {
            return vec![];
        };
        let Some(ci) = def.instance_col else {
            return (0..self.count(msg)).collect();
        };
        let off = def.offsets[ci];
        let t = def.types[ci];
        self.payloads(def, msg)
            .enumerate()
            .filter(|(_, p)| decode_field(p, off, t).as_f64().unwrap_or(0.0) as u8 == inst)
            .map(|(i, _)| i)
            .collect()
    }

    /// Column restricted to one instance.
    pub fn column_f64_inst(&self, msg: &str, inst: u8, col: &str) -> Option<Vec<f64>> {
        let all = self.column_f64(msg, col)?;
        Some(
            self.instance_rows(msg, inst)
                .into_iter()
                .map(|i| all[i])
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaling_rules_match_pymavlink() {
        let p = 12345i16.to_le_bytes();
        assert_eq!(decode_field(&p, 0, FieldType::C16), Value::F(123.45));
        let p = (-12345i16).to_le_bytes();
        assert_eq!(decode_field(&p, 0, FieldType::I16), Value::I(-12345));
        assert_eq!(decode_field(&p, 0, FieldType::C16), Value::F(-123.45));
        let p = 65535u16.to_le_bytes();
        assert_eq!(decode_field(&p, 0, FieldType::CU16), Value::F(655.35));
        let p = (-1073741824i32).to_le_bytes();
        assert_eq!(decode_field(&p, 0, FieldType::L), Value::F(-107.3741824));
        assert_eq!(decode_field(&p, 0, FieldType::E32), Value::F(-10737418.24));
        let p = u64::MAX.to_le_bytes();
        assert_eq!(decode_field(&p, 0, FieldType::U64), Value::U(u64::MAX));
        let mut p = vec![0u8; 64];
        p[0..2].copy_from_slice(&(-7i16).to_le_bytes());
        p[62..64].copy_from_slice(&(31i16).to_le_bytes());
        match decode_field(&p, 0, FieldType::A) {
            Value::I16x32(v) => {
                assert_eq!(v.len(), 32);
                assert_eq!((v[0], v[31]), (-7, 31));
            }
            v => panic!("{v:?}"),
        }
        assert_eq!(half_to_f32(0x3C00), 1.0);
        assert_eq!(half_to_f32(0xC000), -2.0);
        assert!((half_to_f32(0x3555) - 0.333252).abs() < 1e-6);
        let mut s = *b"ATT\0";
        s[3] = 0;
        assert_eq!(decode_field(&s, 0, FieldType::N4), Value::Str("ATT".into()));
    }
}
