//! Betaflight CLI over the same serial port: used for `diff all` backups and
//! as a fallback for settings that have no MSP getter/setter.

use crate::codec::MspError;
use crate::transport::MspLink;
use std::time::Duration;

pub struct Cli<'a> {
    link: &'a mut MspLink,
}

impl<'a> Cli<'a> {
    /// Enter CLI mode (`#`). The FC answers with a banner ending in `# `.
    pub fn enter(link: &'a mut MspLink) -> Result<Self, MspError> {
        link.write_raw(b"#")?;
        let banner = link.read_raw_until(Duration::from_secs(2), |b| {
            b.ends_with(b"# ") || b.ends_with(b"#\r\n")
        })?;
        if !banner.windows(1).any(|w| w == b"#") {
            return Err(MspError::Protocol("no CLI prompt".into()));
        }
        Ok(Self { link })
    }

    /// Send a command and collect output until the next prompt.
    pub fn exec(&mut self, cmd: &str, timeout: Duration) -> Result<String, MspError> {
        self.link.write_raw(format!("{cmd}\n").as_bytes())?;
        let out = self
            .link
            .read_raw_until(timeout, |b| b.ends_with(b"\n# ") || b.ends_with(b"\r\n# "))?;
        let s = String::from_utf8_lossy(&out).into_owned();
        // strip echo + trailing prompt
        let s = s
            .strip_prefix(cmd)
            .unwrap_or(&s)
            .trim_start_matches(['\r', '\n'])
            .to_string();
        Ok(s.trim_end_matches("# ").trim_end().to_string())
    }

    pub fn diff_all(&mut self) -> Result<String, MspError> {
        self.exec("diff all", Duration::from_secs(8))
    }

    pub fn get(&mut self, name: &str) -> Result<Option<String>, MspError> {
        let out = self.exec(&format!("get {name}"), Duration::from_secs(2))?;
        Ok(out.lines().find_map(|l| {
            let l = l.trim();
            let (k, v) = l.split_once(" = ")?;
            (k.trim() == name).then(|| v.trim().to_string())
        }))
    }

    pub fn set(&mut self, name: &str, value: &str) -> Result<(), MspError> {
        let out = self.exec(&format!("set {name} = {value}"), Duration::from_secs(2))?;
        if out.to_ascii_lowercase().contains("invalid") || out.contains("###ERROR") {
            return Err(MspError::Protocol(format!("set {name}: {out}")));
        }
        Ok(())
    }

    /// `save` reboots the FC; the link is unusable afterwards.
    pub fn save(self) -> Result<(), MspError> {
        self.link.write_raw(b"save\n")?;
        let _ = self
            .link
            .read_raw_until(Duration::from_millis(800), |_| false);
        Ok(())
    }

    /// Leave CLI without saving (`exit` also reboots on Betaflight; we use it
    /// only when nothing was changed).
    pub fn exit(self) -> Result<(), MspError> {
        self.link.write_raw(b"exit\n")?;
        let _ = self
            .link
            .read_raw_until(Duration::from_millis(500), |_| false);
        Ok(())
    }
}
