//! Format-character table and `FMT` definitions.
//! Sizes/scaling per `libraries/AP_Logger/LogStructure.h` and pymavlink
//! `FORMAT_TO_STRUCT` (the reference decoder).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FieldType {
    /// int16_t[32]
    A,
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    F32,
    F64,
    /// char[4]
    N4,
    /// char[16]
    N16,
    /// char[64]
    Z64,
    /// int16 × 0.01
    C16,
    /// uint16 × 0.01
    CU16,
    /// int32 × 0.01
    E32,
    /// uint32 × 0.01
    EU32,
    /// int32 latitude/longitude × 1e-7
    L,
    /// uint8 flight mode
    M,
    I64,
    U64,
    /// IEEE half-precision float (pymavlink `g`)
    F16,
}

impl FieldType {
    pub fn from_char(c: u8) -> Option<Self> {
        Some(match c {
            b'a' => Self::A,
            b'b' => Self::I8,
            b'B' => Self::U8,
            b'h' => Self::I16,
            b'H' => Self::U16,
            b'i' => Self::I32,
            b'I' => Self::U32,
            b'f' => Self::F32,
            b'd' => Self::F64,
            b'n' => Self::N4,
            b'N' => Self::N16,
            b'Z' => Self::Z64,
            b'c' => Self::C16,
            b'C' => Self::CU16,
            b'e' => Self::E32,
            b'E' => Self::EU32,
            b'L' => Self::L,
            b'M' => Self::M,
            b'q' => Self::I64,
            b'Q' => Self::U64,
            b'g' => Self::F16,
            _ => return None,
        })
    }

    pub fn size(self) -> usize {
        match self {
            Self::A => 64,
            Self::I8 | Self::U8 | Self::M => 1,
            Self::I16 | Self::U16 | Self::C16 | Self::CU16 | Self::F16 => 2,
            Self::I32 | Self::U32 | Self::F32 | Self::E32 | Self::EU32 | Self::L | Self::N4 => 4,
            Self::F64 | Self::I64 | Self::U64 => 8,
            Self::N16 => 16,
            Self::Z64 => 64,
        }
    }

    pub fn is_numeric(self) -> bool {
        !matches!(self, Self::A | Self::N4 | Self::N16 | Self::Z64)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FmtDef {
    pub id: u8,
    /// Total message length including the 3-byte header.
    pub len: u8,
    pub name: String,
    pub format: String,
    pub columns: Vec<String>,
    pub types: Vec<FieldType>,
    /// Byte offset of each field inside the payload.
    pub offsets: Vec<usize>,
    /// From `FMTU.UnitIds` (one char per field), if seen.
    pub units: Option<Vec<char>>,
    /// From `FMTU.MultIds` (one char per field), if seen. Metadata only.
    pub mults: Option<Vec<char>>,
    /// Field index carrying the sensor instance (`#` unit), if any.
    pub instance_col: Option<usize>,
    /// Column names were padded/truncated because the FMT text was inconsistent.
    pub columns_padded: bool,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum FmtError {
    #[error("unknown format char '{0}'")]
    UnknownChar(char),
    #[error("declared length {declared} does not match field sizes {computed} (+3 header)")]
    LengthMismatch { declared: u8, computed: usize },
}

impl FmtDef {
    pub fn new(id: u8, len: u8, name: &str, format: &str, columns: &str) -> Result<Self, FmtError> {
        let mut types = Vec::with_capacity(format.len());
        for c in format.bytes() {
            types.push(FieldType::from_char(c).ok_or(FmtError::UnknownChar(c as char))?);
        }
        let computed: usize = types.iter().map(|t| t.size()).sum();
        if (len as usize) < 3 || len as usize - 3 != computed {
            return Err(FmtError::LengthMismatch {
                declared: len,
                computed,
            });
        }
        // pymavlink keeps a definition whose column list disagrees with the
        // format (corrupt FMT text is common); the byte layout is what matters
        // for resynchronisation, so pad / truncate the names instead of rejecting.
        let mut columns: Vec<String> = if columns.is_empty() {
            Vec::new()
        } else {
            columns.split(',').map(|s| s.to_string()).collect()
        };
        let mut columns_padded = false;
        if columns.len() != types.len() {
            columns_padded = true;
            columns.truncate(types.len());
            while columns.len() < types.len() {
                columns.push(format!("col{}", columns.len()));
            }
        }
        let mut offsets = Vec::with_capacity(types.len());
        let mut o = 0;
        for t in &types {
            offsets.push(o);
            o += t.size();
        }
        Ok(Self {
            id,
            len,
            name: name.to_string(),
            format: format.to_string(),
            columns,
            types,
            offsets,
            units: None,
            mults: None,
            instance_col: None,
            columns_padded,
        })
    }

    pub fn payload_len(&self) -> usize {
        self.len as usize - 3
    }

    pub fn col(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| c == name)
    }

    pub fn apply_units(&mut self, unit_ids: &str, mult_ids: &str) {
        let u: Vec<char> = unit_ids.chars().collect();
        let m: Vec<char> = mult_ids.chars().collect();
        if u.len() == self.types.len() {
            self.instance_col = u.iter().position(|c| *c == '#');
            self.units = Some(u);
        }
        if m.len() == self.types.len() {
            self.mults = Some(m);
        }
    }
}

/// Decode NUL-terminated ASCII (lossy for stray bytes).
pub fn cstr(b: &[u8]) -> String {
    let end = b.iter().position(|c| *c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_match_logstructure_h() {
        let expect = [
            (b'a', 64),
            (b'b', 1),
            (b'B', 1),
            (b'h', 2),
            (b'H', 2),
            (b'i', 4),
            (b'I', 4),
            (b'f', 4),
            (b'd', 8),
            (b'n', 4),
            (b'N', 16),
            (b'Z', 64),
            (b'c', 2),
            (b'C', 2),
            (b'e', 4),
            (b'E', 4),
            (b'L', 4),
            (b'M', 1),
            (b'q', 8),
            (b'Q', 8),
        ];
        for (c, n) in expect {
            assert_eq!(FieldType::from_char(c).unwrap().size(), n, "{}", c as char);
        }
        assert!(FieldType::from_char(b'x').is_none());
    }

    #[test]
    fn fmt_of_fmt_is_89_bytes() {
        let f = FmtDef::new(128, 89, "FMT", "BBnNZ", "Type,Length,Name,Format,Columns").unwrap();
        assert_eq!(f.payload_len(), 86);
        assert_eq!(f.offsets, vec![0, 1, 2, 6, 22]);
    }

    #[test]
    fn length_mismatch_is_rejected() {
        assert_eq!(
            FmtDef::new(1, 10, "X", "Qf", "a,b").unwrap_err(),
            FmtError::LengthMismatch {
                declared: 10,
                computed: 12
            }
        );
        assert_eq!(
            FmtDef::new(1, 2, "X", "", "").unwrap_err(),
            FmtError::LengthMismatch {
                declared: 2,
                computed: 0
            }
        );
    }

    #[test]
    fn column_mismatch_is_padded_like_pymavlink() {
        let f = FmtDef::new(
            172,
            37,
            "SA",
            "QBffffffB",
            "TimeUS,State,DVelX,DVelY,garbage,Back",
        )
        .unwrap();
        assert!(f.columns_padded);
        assert_eq!(f.columns.len(), 9);
        assert_eq!(f.columns[8], "col8");
        assert_eq!(FieldType::from_char(b'g').unwrap().size(), 2);
    }

    #[test]
    fn instance_column_from_units() {
        let mut f =
            FmtDef::new(5, 3 + 8 + 1 + 12, "IMU", "QBfff", "TimeUS,I,GyrX,GyrY,GyrZ").unwrap();
        f.apply_units("s#EEE", "F-000");
        assert_eq!(f.instance_col, Some(1));
    }
}
