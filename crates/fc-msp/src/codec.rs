//! MSP v1 / v2 framing.
//!
//! v1: `$M<` dir size:u8 cmd:u8 payload xor-checksum(size,cmd,payload)
//! v2: `$X<` flag:u8 fn:u16le size:u16le payload crc8_dvb_s2(flag..payload)

use std::fmt;

#[derive(Debug, thiserror::Error)]
pub enum MspError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("timeout waiting for MSP {0}")]
    Timeout(u16),
    #[error("MSP {0} not supported by the flight controller")]
    Unsupported(u16),
    #[error("checksum error")]
    Checksum,
    #[error("payload too short for {what} (got {got} bytes)")]
    Short { what: &'static str, got: usize },
    #[error("unexpected reply: expected {expected}, got {got}")]
    Mismatch { expected: u16, got: u16 },
    #[error("{0}")]
    Protocol(String),
    #[error("verification failed: {0}")]
    Verify(String),
    #[error("refusing to write: {0}")]
    Refused(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub cmd: u16,
    pub payload: Vec<u8>,
    /// Flag byte (v2) — bit 0 set on an error reply (`!` direction on v1).
    pub error: bool,
    pub v2: bool,
}

impl fmt::Display for Frame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "MSP{} cmd={} len={}{}",
            if self.v2 { "v2" } else { "v1" },
            self.cmd,
            self.payload.len(),
            if self.error { " ERR" } else { "" }
        )
    }
}

pub fn crc8_dvb_s2(mut crc: u8, b: u8) -> u8 {
    crc ^= b;
    for _ in 0..8 {
        if crc & 0x80 != 0 {
            crc = (crc << 1) ^ 0xD5;
        } else {
            crc <<= 1;
        }
    }
    crc
}

/// Encode a request. Always MSPv2: Betaflight ≥ 4.0 answers in the same
/// version, and v2 avoids v1 jumbo frames for large replies (dataflash reads).
pub fn encode(cmd: u16, payload: &[u8]) -> Vec<u8> {
    encode_v2(cmd, payload)
}

pub fn encode_v1(cmd: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(6 + payload.len());
    out.extend_from_slice(b"$M<");
    out.push(payload.len() as u8);
    out.push(cmd);
    out.extend_from_slice(payload);
    let mut ck = payload.len() as u8 ^ cmd;
    for b in payload {
        ck ^= b;
    }
    out.push(ck);
    out
}

pub fn encode_v2(cmd: u16, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(9 + payload.len());
    out.extend_from_slice(b"$X<");
    let body = [
        &[0u8][..],
        &cmd.to_le_bytes(),
        &(payload.len() as u16).to_le_bytes(),
        payload,
    ]
    .concat();
    out.extend_from_slice(&body);
    let crc = body.iter().fold(0u8, |c, b| crc8_dvb_s2(c, *b));
    out.push(crc);
    out
}

/// Incremental frame parser (handles both versions, jumbo v1 not needed for replies we use).
#[derive(Default)]
pub struct Parser {
    buf: Vec<u8>,
}

enum Scan {
    Need,
    Skip(usize),
    Frame(Frame, usize),
    Bad(usize),
}

impl Parser {
    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
        if self.buf.len() > 1 << 20 {
            self.buf.drain(..self.buf.len() - (1 << 16));
        }
    }

    /// Pull the next complete frame, discarding garbage. `Err(Checksum)` is
    /// returned once per corrupt frame so the caller can retry.
    #[allow(clippy::should_implement_trait)] // fallible, not an Iterator
    pub fn next(&mut self) -> Result<Option<Frame>, MspError> {
        loop {
            match self.scan() {
                Scan::Need => return Ok(None),
                Scan::Skip(n) => {
                    self.buf.drain(..n);
                }
                Scan::Frame(f, n) => {
                    self.buf.drain(..n);
                    return Ok(Some(f));
                }
                Scan::Bad(n) => {
                    self.buf.drain(..n);
                    return Err(MspError::Checksum);
                }
            }
        }
    }

    fn scan(&self) -> Scan {
        let b = &self.buf;
        if b.is_empty() {
            return Scan::Need;
        }
        let Some(start) = b.iter().position(|c| *c == b'$') else {
            return Scan::Skip(b.len());
        };
        if start > 0 {
            return Scan::Skip(start);
        }
        if b.len() < 3 {
            return Scan::Need;
        }
        match b[1] {
            b'M' => {
                if b.len() < 5 {
                    return Scan::Need;
                }
                let dir = b[2];
                let size = b[3] as usize;
                let total = 6 + size;
                if b.len() < total {
                    return Scan::Need;
                }
                let cmd = b[4];
                let payload = &b[5..5 + size];
                let mut ck = size as u8 ^ cmd;
                for x in payload {
                    ck ^= x;
                }
                if ck != b[5 + size] {
                    return Scan::Bad(1);
                }
                Scan::Frame(
                    Frame {
                        cmd: cmd as u16,
                        payload: payload.to_vec(),
                        error: dir == b'!',
                        v2: false,
                    },
                    total,
                )
            }
            b'X' => {
                if b.len() < 9 {
                    return Scan::Need;
                }
                let dir = b[2];
                let flag = b[3];
                let cmd = u16::from_le_bytes([b[4], b[5]]);
                let size = u16::from_le_bytes([b[6], b[7]]) as usize;
                let total = 9 + size;
                if b.len() < total {
                    return Scan::Need;
                }
                let crc = b[3..8 + size].iter().fold(0u8, |c, x| crc8_dvb_s2(c, *x));
                if crc != b[8 + size] {
                    return Scan::Bad(1);
                }
                Scan::Frame(
                    Frame {
                        cmd,
                        payload: b[8..8 + size].to_vec(),
                        error: dir == b'!' || flag & 1 != 0,
                        v2: true,
                    },
                    total,
                )
            }
            _ => Scan::Skip(1),
        }
    }
}

/// Little-endian payload reader mirroring the configurator's DataView helpers.
pub struct Reader<'a> {
    pub data: &'a [u8],
    pub pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }
    pub fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }
    pub fn u8(&mut self) -> Option<u8> {
        let v = *self.data.get(self.pos)?;
        self.pos += 1;
        Some(v)
    }
    pub fn i8(&mut self) -> Option<i8> {
        self.u8().map(|v| v as i8)
    }
    pub fn u16(&mut self) -> Option<u16> {
        let s = self.data.get(self.pos..self.pos + 2)?;
        self.pos += 2;
        Some(u16::from_le_bytes([s[0], s[1]]))
    }
    pub fn u32(&mut self) -> Option<u32> {
        let s = self.data.get(self.pos..self.pos + 4)?;
        self.pos += 4;
        Some(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }
    /// Length-prefixed string (`getText` in the configurator).
    pub fn text(&mut self) -> Option<String> {
        let n = self.u8()? as usize;
        let s = self.data.get(self.pos..self.pos + n)?;
        self.pos += n;
        Some(String::from_utf8_lossy(s).into_owned())
    }
    pub fn bytes(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.data.get(self.pos..self.pos + n)?;
        self.pos += n;
        Some(s)
    }
}

pub struct Writer(pub Vec<u8>);

impl Writer {
    pub fn new() -> Self {
        Self(Vec::new())
    }
    pub fn u8(&mut self, v: u8) -> &mut Self {
        self.0.push(v);
        self
    }
    pub fn i8(&mut self, v: i8) -> &mut Self {
        self.0.push(v as u8);
        self
    }
    pub fn u16(&mut self, v: u16) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub fn u32(&mut self, v: u32) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
}

impl Default for Writer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v1_roundtrip() {
        let req = encode_v1(112, &[]);
        assert_eq!(req, vec![b'$', b'M', b'<', 0, 112, 112]);
        let mut p = Parser::default();
        let reply = [b"$M>".to_vec(), vec![3, 112, 1, 2, 3, 3 ^ 112 ^ 1 ^ 2 ^ 3]].concat();
        p.push(&reply[..4]);
        assert!(p.next().unwrap().is_none());
        p.push(&reply[4..]);
        let f = p.next().unwrap().unwrap();
        assert_eq!(f.cmd, 112);
        assert_eq!(f.payload, vec![1, 2, 3]);
        assert!(!f.v2);
    }

    #[test]
    fn v2_roundtrip_and_garbage() {
        let req = encode_v2(0x3006, &[2]);
        let mut p = Parser::default();
        p.push(b"noise$junk");
        p.push(&req);
        let f = p.next().unwrap().unwrap();
        assert_eq!(f.cmd, 0x3006);
        assert_eq!(f.payload, vec![2]);
        assert!(f.v2);
    }

    #[test]
    fn bad_checksum_reported_once() {
        let mut req = encode_v1(1, &[9, 9]);
        let n = req.len();
        req[n - 1] ^= 0xFF;
        let mut p = Parser::default();
        p.push(&req);
        assert!(matches!(p.next(), Err(MspError::Checksum)));
        assert!(p.next().unwrap().is_none());
    }

    #[test]
    fn crc8_known_vector() {
        // CRC8/DVB-S2 of "123456789" is 0xBC
        let crc = b"123456789".iter().fold(0u8, |c, b| crc8_dvb_s2(c, *b));
        assert_eq!(crc, 0xBC);
    }
}
