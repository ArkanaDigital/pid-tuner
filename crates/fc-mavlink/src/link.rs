//! Transport: a serial port or TCP socket (SITL) carrying MAVLink 2 frames.
//! A reader thread owns the read half and forwards decoded frames through a
//! channel so callers get `recv(timeout)` semantics; writes are synchronous.

use crate::apm::MavMessage;
use domain::fc::FcError;
use mavlink::error::MessageReadError;
use mavlink::peek_reader::PeekReader;
use mavlink::{read_v2_msg, write_versioned_msg, MavHeader, MavlinkVersion};
use std::io::{BufReader, Read, Write};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError, SyncSender};
use std::sync::Arc;
use std::time::Duration;

pub type Frame = (MavHeader, MavMessage);

pub trait Link: Send {
    fn send(&mut self, msg: &MavMessage) -> Result<(), FcError>;
    /// `Ok(None)` on timeout; `Err` when the link is gone.
    fn recv(&mut self, timeout: Duration) -> Result<Option<Frame>, FcError>;
    fn name(&self) -> &str;
}

struct IoLink<W: Write + Send> {
    name: String,
    writer: W,
    rx: Receiver<Frame>,
    seq: Arc<AtomicU8>,
}

fn spawn_reader<R: Read + Send + 'static>(reader: R, tx: SyncSender<Frame>) {
    std::thread::Builder::new()
        .name("mavlink-rx".into())
        .spawn(move || {
            let mut pr = PeekReader::new(BufReader::with_capacity(4096, reader));
            loop {
                match read_v2_msg::<MavMessage, _>(&mut pr) {
                    Ok(frame) => {
                        if tx.send(frame).is_err() {
                            return;
                        }
                    }
                    // Timeouts (serial read timeout) and parse errors (unknown id / bad crc): keep reading.
                    Err(MessageReadError::Io(e))
                        if matches!(
                            e.kind(),
                            std::io::ErrorKind::TimedOut
                                | std::io::ErrorKind::WouldBlock
                                | std::io::ErrorKind::Interrupted
                        ) =>
                    {
                        continue
                    }
                    Err(MessageReadError::Parse(_)) => continue,
                    Err(MessageReadError::Io(_)) => return, // port gone
                }
            }
        })
        .expect("spawn mavlink reader");
}

impl<W: Write + Send> Link for IoLink<W> {
    fn send(&mut self, msg: &MavMessage) -> Result<(), FcError> {
        let header = MavHeader {
            system_id: crate::GCS_SYSTEM_ID,
            component_id: crate::GCS_COMPONENT_ID,
            sequence: self.seq.fetch_add(1, Ordering::Relaxed),
        };
        write_versioned_msg(&mut self.writer, MavlinkVersion::V2, header, msg)
            .map_err(|e| FcError::Other(format!("mavlink write: {e:?}")))?;
        self.writer
            .flush()
            .map_err(|e| FcError::Other(format!("mavlink flush: {e}")))?;
        Ok(())
    }

    fn recv(&mut self, timeout: Duration) -> Result<Option<Frame>, FcError> {
        match self.rx.recv_timeout(timeout) {
            Ok(f) => Ok(Some(f)),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => Err(FcError::NotConnected),
        }
    }

    fn name(&self) -> &str {
        &self.name
    }
}

pub struct SerialLink(IoLink<Box<dyn serialport::SerialPort>>);

impl SerialLink {
    /// USB CDC ignores the baud rate; SERIAL0_BAUD default 115200 is used for real UARTs.
    pub fn open(port: &str, baud: u32) -> Result<Self, FcError> {
        let rd = serialport::new(port, baud)
            .timeout(Duration::from_millis(100))
            .open()
            .map_err(|e| FcError::Other(format!("open {port}: {e}")))?;
        let wr = rd
            .try_clone()
            .map_err(|e| FcError::Other(format!("clone {port}: {e}")))?;
        let (tx, rx) = sync_channel(8192);
        spawn_reader(rd, tx);
        Ok(Self(IoLink {
            name: port.to_string(),
            writer: wr,
            rx,
            seq: Arc::new(AtomicU8::new(0)),
        }))
    }
}

impl Link for SerialLink {
    fn send(&mut self, msg: &MavMessage) -> Result<(), FcError> {
        self.0.send(msg)
    }
    fn recv(&mut self, timeout: Duration) -> Result<Option<Frame>, FcError> {
        self.0.recv(timeout)
    }
    fn name(&self) -> &str {
        self.0.name()
    }
}

/// TCP client, e.g. SITL `--out tcp:127.0.0.1:5760` / `sim_vehicle.py` port 5760.
pub struct TcpLink(IoLink<std::net::TcpStream>);

impl TcpLink {
    pub fn connect(addr: &str) -> Result<Self, FcError> {
        let s = std::net::TcpStream::connect(addr)
            .map_err(|e| FcError::Other(format!("connect {addr}: {e}")))?;
        s.set_read_timeout(Some(Duration::from_millis(100))).ok();
        s.set_nodelay(true).ok();
        let rd = s.try_clone().map_err(|e| FcError::Other(e.to_string()))?;
        let (tx, rx) = sync_channel(8192);
        spawn_reader(rd, tx);
        Ok(Self(IoLink {
            name: addr.to_string(),
            writer: s,
            rx,
            seq: Arc::new(AtomicU8::new(0)),
        }))
    }
}

impl Link for TcpLink {
    fn send(&mut self, msg: &MavMessage) -> Result<(), FcError> {
        self.0.send(msg)
    }
    fn recv(&mut self, timeout: Duration) -> Result<Option<Frame>, FcError> {
        self.0.recv(timeout)
    }
    fn name(&self) -> &str {
        self.0.name()
    }
}

/// Parse `"serial:/dev/tty.usbmodem1:115200"`, `"tcp:127.0.0.1:5760"` or a bare port path.
pub fn open(address: &str) -> Result<Box<dyn Link>, FcError> {
    if let Some(rest) = address.strip_prefix("tcp:") {
        return Ok(Box::new(TcpLink::connect(rest)?));
    }
    let (port, baud) = match address.strip_prefix("serial:") {
        Some(rest) => match rest.rsplit_once(':') {
            Some((p, b)) if b.parse::<u32>().is_ok() => (p.to_string(), b.parse().unwrap()),
            _ => (rest.to_string(), 115_200),
        },
        None => (address.to_string(), 115_200),
    };
    Ok(Box::new(SerialLink::open(&port, baud)?))
}
