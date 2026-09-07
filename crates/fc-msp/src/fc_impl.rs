//! `domain::fc::FlightController` for Betaflight over MSP, so the wizard drives
//! MSP and MAVLink backends through one interface.

use crate::client::{reconnect, MspClient};
use crate::codec::MspError;
use crate::layouts::ApiVersion;
use domain::fc::*;
use domain::*;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

impl From<MspError> for FcError {
    fn from(e: MspError) -> Self {
        match e {
            MspError::Refused(s) => FcError::Refused(s),
            MspError::Verify(s) => FcError::Verify(s),
            MspError::Timeout(id) => FcError::Timeout(format!("MSP {id}")),
            MspError::Unsupported(id) => FcError::Unsupported(format!("MSP {id}")),
            other => FcError::Other(other.to_string()),
        }
    }
}

impl MspClient {
    /// Identity, tune, logging config and storage in one `FcStatus`.
    pub fn fc_status(&mut self) -> Result<(FcStatus, BfTune), MspError> {
        let tune = self.read_tune()?;
        let st = self.status()?;
        let api = self.api();
        let log_rate = self.blackbox_rate_hz()?;
        let debug_mode = tune.get_raw("debug_mode").and_then(|v| v.parse::<u8>().ok());
        // gyroUnfilt is logged natively from BF 4.4 (API 1.45); older builds need debug_mode 6 = GYRO_SCALED.
        let raw_ok = api.at_least(1, 45) || debug_mode == Some(6);
        let storage = match self.dataflash_summary() {
            Ok(d) if d.supported => Some(d.total_size.saturating_sub(d.used_size) as u64),
            _ => self.sdcard_summary()?.filter(|s| s.supported).map(|s| s.free_kb as u64 * 1024),
        };
        Ok((
            FcStatus {
                connected: true,
                port: Some(self.port.clone()),
                kind: Some(FcKind::Msp),
                firmware: Some(self.firmware()),
                armed: st.armed(),
                heartbeat_age_s: 0.0,
                tune: Some(Tune::Bf(tune.clone())),
                log_rate_hz: log_rate,
                debug_mode: debug_mode.map(|d| if d == 6 { "GYRO_SCALED".to_string() } else { d.to_string() }),
                storage_free_bytes: storage,
                pid_logging_enabled: Some(true),
                raw_gyro_logging_enabled: Some(raw_ok),
                snapshot_taken: false,
                ..Default::default()
            },
            tune,
        ))
    }

    /// Blackbox rate ≥ 2 kHz and (BF < 4.4) debug_mode = GYRO_SCALED, then EEPROM save.
    pub fn fix_logging(&mut self) -> Result<ApplyResult, MspError> {
        let api: ApiVersion = self.api();
        let st = self.status()?;
        if st.armed() {
            return Err(MspError::Refused("armed".into()));
        }
        let loop_hz = if st.cycle_time_us > 0 { 1e6 / st.cycle_time_us as f64 } else { 8000.0 };
        let mut outcomes = Vec::new();
        if let Some(mut bb) = self.read_blackbox()? {
            let before = bb.clone();
            if api.at_least(1, 44) {
                // largest divisor that still gives ≥ 2 kHz
                let mut div = 0u8;
                while div < 4 && loop_hz / (1u32 << (div + 1)) as f64 >= 2000.0 {
                    div += 1;
                }
                bb.sample_rate = div;
            } else {
                bb.rate_num = 1;
                bb.rate_denom = ((loop_hz / 2000.0).floor() as u8).max(1);
            }
            if bb.device == 0 {
                bb.device = 1; // flash
            }
            if bb != before {
                self.write_blackbox(&bb)?;
                outcomes.push(ApplyOutcome { param: "blackbox_sample_rate".into(), wanted: format!("{}", bb.sample_rate), read_back: None, ok: true, via: "msp".into() });
            }
        }
        if !api.at_least(1, 45) {
            if let Some(mut ac) = self.read_advanced_config()? {
                if ac.debug_mode != 6 {
                    ac.debug_mode = 6; // GYRO_SCALED
                    self.write_advanced_config(&ac)?;
                    outcomes.push(ApplyOutcome { param: "debug_mode".into(), wanted: "GYRO_SCALED".into(), read_back: None, ok: true, via: "msp".into() });
                }
            }
        }
        if !outcomes.is_empty() {
            self.eeprom_write()?;
            // read back
            let rate = self.blackbox_rate_hz()?.unwrap_or(0.0);
            let dbg = self.read_advanced_config()?.map(|a| a.debug_mode);
            for o in outcomes.iter_mut() {
                match o.param.as_str() {
                    "blackbox_sample_rate" => {
                        o.read_back = Some(format!("{rate:.0} Hz"));
                        o.ok = rate >= 1990.0;
                    }
                    "debug_mode" => {
                        o.read_back = dbg.map(|d| d.to_string());
                        o.ok = dbg == Some(6);
                    }
                    _ => {}
                }
            }
        }
        let verified = outcomes.iter().all(|o| o.ok);
        Ok(ApplyResult { outcomes, verified, rebooted: false })
    }
}

impl FlightController for MspClient {
    fn kind(&self) -> FcKind {
        FcKind::Msp
    }

    fn poll(&mut self) -> Result<FcStatus, FcError> {
        let st = self.status()?;
        Ok(FcStatus { connected: true, port: Some(self.port.clone()), kind: Some(FcKind::Msp), firmware: Some(self.firmware()), armed: st.armed(), heartbeat_age_s: 0.0, ..Default::default() })
    }

    fn full_status(&mut self) -> Result<FcStatus, FcError> {
        Ok(self.fc_status()?.0)
    }

    fn read_tune(&mut self) -> Result<Tune, FcError> {
        Ok(Tune::Bf(MspClient::read_tune(self)?))
    }

    /// `diff all` through the CLI. Leaving the CLI reboots the FC, so we reconnect.
    fn backup(&mut self) -> Result<Backup, FcError> {
        let diff = self.cli_diff_all()?;
        self.reboot_and_reconnect_after_cli()?;
        Ok(Backup { label: "diff-all".into(), ext: "txt".into(), bytes: diff.into_bytes() })
    }

    fn preflight_fix(&mut self, fix: PreflightFix) -> Result<ApplyResult, FcError> {
        let PreflightFix::Logging = fix;
        Ok(self.fix_logging()?)
    }

    fn apply(&mut self, recs: &[Recommendation]) -> Result<ApplyResult, FcError> {
        let r = MspClient::apply(self, recs)?;
        if r.rebooted {
            self.reboot_and_reconnect_after_cli()?;
        }
        Ok(r)
    }

    fn reboot_and_reconnect(&mut self) -> Result<(), FcError> {
        self.link.request(crate::codes::MSP_SET_REBOOT, &[crate::codes::REBOOT_FIRMWARE]).ok();
        self.reboot_and_reconnect_after_cli()
    }

    fn list_logs(&mut self) -> Result<Vec<LogEntry>, FcError> {
        let s = self.dataflash_summary()?;
        if !s.supported || s.used_size == 0 {
            return Ok(Vec::new());
        }
        Ok(vec![LogEntry { id: 1, size: s.used_size as u64, time_utc: None }])
    }

    fn download_log(&mut self, _id: Option<u32>, progress: ProgressFn<'_>, cancel: &AtomicBool) -> Result<Vec<u8>, FcError> {
        let bytes = self.dataflash_download(|d, t| {
            progress(d as u64, t as u64);
        })?;
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(FcError::Other("cancelled".into()));
        }
        Ok(bytes)
    }

    fn export_text(&self, recs: &[Recommendation]) -> String {
        export_text_for(Some(&self.firmware()), recs)
    }
}

impl MspClient {
    /// The FC has just rebooted (CLI `save`/`exit` or MSP_SET_REBOOT): reopen the port.
    fn reboot_and_reconnect_after_cli(&mut self) -> Result<(), FcError> {
        let port = self.port.clone();
        let fresh = reconnect(&port, Duration::from_secs(15))?;
        *self = fresh;
        Ok(())
    }
}
