//! Byte transport + request/response link with timeout and retry.

use crate::codec::{encode, Frame, MspError, Parser};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::time::{Duration, Instant};

pub trait Transport: Send {
    fn write_all(&mut self, data: &[u8]) -> std::io::Result<()>;
    /// Read whatever is available (may return 0 on timeout).
    fn read_some(&mut self, buf: &mut [u8]) -> std::io::Result<usize>;
    fn name(&self) -> String;
}

pub struct SerialTransport {
    port: Box<dyn serialport::SerialPort>,
    name: String,
}

impl SerialTransport {
    pub fn open(path: &str, baud: u32) -> std::io::Result<Self> {
        let port = serialport::new(path, baud)
            .timeout(Duration::from_millis(50))
            .open()
            .map_err(std::io::Error::other)?;
        Ok(Self {
            port,
            name: path.to_string(),
        })
    }
}

impl Transport for SerialTransport {
    fn write_all(&mut self, data: &[u8]) -> std::io::Result<()> {
        self.port.write_all(data)?;
        self.port.flush()
    }
    fn read_some(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self.port.read(buf) {
            Ok(n) => Ok(n),
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => Ok(0),
            Err(e) => Err(e),
        }
    }
    fn name(&self) -> String {
        self.name.clone()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortInfo {
    pub path: String,
    pub vid: Option<u16>,
    pub pid: Option<u16>,
    pub product: Option<String>,
    pub manufacturer: Option<String>,
    /// Heuristic: looks like a flight controller (STM32 VCP, AT32, etc.).
    pub likely_fc: bool,
}

pub fn list_ports() -> Vec<PortInfo> {
    let mut out = Vec::new();
    if let Ok(ports) = serialport::available_ports() {
        for p in ports {
            let (vid, pid, product, manufacturer) = match &p.port_type {
                serialport::SerialPortType::UsbPort(u) => (
                    Some(u.vid),
                    Some(u.pid),
                    u.product.clone(),
                    u.manufacturer.clone(),
                ),
                _ => (None, None, None, None),
            };
            // STM32 VCP 0483:5740, AT32 2E3C:5740, CP210x/CH340 bridges are also common.
            let likely_fc = matches!(vid, Some(0x0483) | Some(0x2E3C))
                || product
                    .as_deref()
                    .map(|s| s.to_ascii_lowercase().contains("betaflight") || s.contains("STM32"))
                    .unwrap_or(false);
            // macOS lists both /dev/cu.* and /dev/tty.*; keep cu.* (non-blocking open).
            if p.port_name.starts_with("/dev/tty.") {
                continue;
            }
            out.push(PortInfo {
                path: p.port_name,
                vid,
                pid,
                product,
                manufacturer,
                likely_fc,
            });
        }
    }
    out.sort_by(|a, b| b.likely_fc.cmp(&a.likely_fc).then(a.path.cmp(&b.path)));
    out
}

/// Request/response over a transport. One request in flight at a time.
pub struct MspLink {
    pub transport: Box<dyn Transport>,
    parser: Parser,
    pub timeout: Duration,
    pub retries: u32,
    buf: Vec<u8>,
}

impl MspLink {
    pub fn new(transport: Box<dyn Transport>) -> Self {
        Self {
            transport,
            parser: Parser::default(),
            timeout: Duration::from_millis(600),
            retries: 3,
            buf: vec![0; 4096],
        }
    }

    /// Send `cmd` and wait for the matching reply payload.
    pub fn request(&mut self, cmd: u16, payload: &[u8]) -> Result<Vec<u8>, MspError> {
        let mut last = MspError::Timeout(cmd);
        for attempt in 0..=self.retries {
            match self.request_once(cmd, payload) {
                Ok(p) => return Ok(p),
                Err(MspError::Unsupported(c)) => return Err(MspError::Unsupported(c)),
                Err(e) => {
                    tracing::debug!("MSP {cmd} attempt {attempt} failed: {e}");
                    last = e;
                }
            }
        }
        Err(last)
    }

    fn request_once(&mut self, cmd: u16, payload: &[u8]) -> Result<Vec<u8>, MspError> {
        self.transport.write_all(&encode(cmd, payload))?;
        let deadline = Instant::now() + self.timeout;
        loop {
            match self.parser.next() {
                Ok(Some(f)) if f.cmd == cmd => {
                    return if f.error {
                        Err(MspError::Unsupported(cmd))
                    } else {
                        Ok(f.payload)
                    };
                }
                Ok(Some(other)) => {
                    tracing::trace!("ignoring stray {other}");
                }
                Ok(None) => {}
                Err(MspError::Checksum) => return Err(MspError::Checksum),
                Err(e) => return Err(e),
            }
            if Instant::now() > deadline {
                return Err(MspError::Timeout(cmd));
            }
            let n = self.transport.read_some(&mut self.buf)?;
            if n > 0 {
                self.parser.push(&self.buf[..n]);
            } else {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }

    /// Raw write (CLI mode).
    pub fn write_raw(&mut self, data: &[u8]) -> Result<(), MspError> {
        Ok(self.transport.write_all(data)?)
    }

    /// Read raw bytes until `until` returns true or the timeout passes.
    pub fn read_raw_until(
        &mut self,
        timeout: Duration,
        mut until: impl FnMut(&[u8]) -> bool,
    ) -> Result<Vec<u8>, MspError> {
        let mut out = Vec::new();
        let deadline = Instant::now() + timeout;
        loop {
            let n = self.transport.read_some(&mut self.buf)?;
            if n > 0 {
                out.extend_from_slice(&self.buf[..n]);
                if until(&out) {
                    return Ok(out);
                }
            } else if Instant::now() > deadline {
                return Ok(out);
            } else {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }

    pub fn frame_for_test(cmd: u16, payload: &[u8]) -> Frame {
        Frame {
            cmd,
            payload: payload.to_vec(),
            error: false,
            v2: false,
        }
    }
}
