//! Rebuild `setpoint` from `rcCommand` when the log has the Setpoint field
//! disabled (`blackbox_disable_setpoint = ON`, `fields_disabled_mask` bit
//! `FLIGHT_LOG_FIELD_SELECT_SETPOINT` = 2 in blackbox_fielddefs.h).
//!
//! Port of betaflight `src/main/fc/rc.c`: `processRcCommand` (rcCommandf =
//! rcCommand / (500 − deadband), yaw uses yaw_deadband), `applyActualRates`,
//! `applyBetaflightRates`, clamp to `rate_limit[axis]`. What is *not*
//! reproduced is RC smoothing / feedforward interpolation, so the rebuilt
//! setpoint is the raw stick rate (a few ms ahead of the real one).

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub struct RatesProfile {
    /// lookupTableRatesType: 0 BETAFLIGHT, 1 RACEFLIGHT, 2 KISS, 3 ACTUAL, 4 QUICK
    pub rates_type: u8,
    pub rc_rates: [f32; 3],
    pub rc_expo: [f32; 3],
    pub rates: [f32; 3],
    pub rate_limits: [f32; 3],
    pub deadband: f32,
    pub yaw_deadband: f32,
}

fn triple(h: &BTreeMap<String, String>, key: &str) -> Option<[f32; 3]> {
    let v = h.get(key)?;
    let mut it = v.split(',').map(|x| x.trim().parse::<f32>().ok());
    Some([it.next()??, it.next()??, it.next()??])
}

impl RatesProfile {
    pub fn from_headers(h: &BTreeMap<String, String>) -> Option<Self> {
        Some(Self {
            rates_type: h.get("rates_type").and_then(|v| v.parse().ok()).unwrap_or(0),
            rc_rates: triple(h, "rc_rates")?,
            rc_expo: triple(h, "rc_expo")?,
            rates: triple(h, "rates")?,
            rate_limits: triple(h, "rate_limits").or_else(|| triple(h, "rate_limit")).unwrap_or([1998.0; 3]),
            deadband: h.get("deadband").and_then(|v| v.parse().ok()).unwrap_or(0.0),
            yaw_deadband: h.get("yaw_deadband").and_then(|v| v.parse().ok()).unwrap_or(0.0),
        })
    }

    pub fn type_name(&self) -> &'static str {
        match self.rates_type {
            0 => "BETAFLIGHT",
            1 => "RACEFLIGHT",
            2 => "KISS",
            3 => "ACTUAL",
            4 => "QUICK",
            _ => "?",
        }
    }

    pub fn supported(&self) -> bool {
        matches!(self.rates_type, 0 | 3)
    }

    /// `rcCommand[axis]` (−500..500 after deadband, as logged) → angle rate in °/s.
    pub fn setpoint(&self, axis: usize, rc_command: f32) -> f32 {
        let divider = if axis == 2 { 500.0 - self.yaw_deadband } else { 500.0 - self.deadband };
        let rcf = (rc_command / divider).clamp(-1.0, 1.0);
        let abs = rcf.abs();
        let rate = match self.rates_type {
            3 => {
                // applyActualRates
                let expo = self.rc_expo[axis] / 100.0;
                let expof = abs * (rcf.powi(5) * expo + rcf * (1.0 - expo));
                let center = self.rc_rates[axis] * 10.0;
                let stick_movement = (self.rates[axis] * 10.0 - center).max(0.0);
                rcf * center + stick_movement * expof
            }
            _ => {
                // applyBetaflightRates (RC_RATE_INCREMENTAL = 14.54)
                let mut r = rcf;
                if self.rc_expo[axis] != 0.0 {
                    let expof = self.rc_expo[axis] / 100.0;
                    r = r * abs.powi(3) * expof + r * (1.0 - expof);
                }
                let mut rc_rate = self.rc_rates[axis] / 100.0;
                if rc_rate > 2.0 {
                    rc_rate += 14.54 * (rc_rate - 2.0);
                }
                let mut angle = 200.0 * rc_rate * r;
                if self.rates[axis] != 0.0 {
                    let superfactor = 1.0 / (1.0 - abs * (self.rates[axis] / 100.0)).clamp(0.01, 1.0);
                    angle *= superfactor;
                }
                angle
            }
        };
        rate.clamp(-self.rate_limits[axis], self.rate_limits[axis])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hdr(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn actual_rates_match_rc_c() {
        // the user's 2026.6.1 log: rc_rates 12,7,13  rc_expo 30,30,35  rates 67,67,67  ACTUAL
        let p = RatesProfile::from_headers(&hdr(&[("rates_type", "3"), ("rc_rates", "12,7,13"), ("rc_expo", "30,30,35"), ("rates", "67,67,67"), ("rate_limits", "1998,1998,1998"), ("deadband", "0"), ("yaw_deadband", "0")])).unwrap();
        assert!((p.setpoint(0, 500.0) - 670.0).abs() < 1e-3); // full stick = rates×10
        assert!((p.setpoint(0, -500.0) + 670.0).abs() < 1e-3);
        // half stick: expof = 0.5·(0.5⁵·0.3 + 0.5·0.7) = 0.1796875 → 60 + 550·0.1796875
        assert!((p.setpoint(0, 250.0) - 158.828125).abs() < 1e-3);
        // small stick ≈ centre sensitivity slope (rc_rate×10 °/s per full deflection)
        assert!((p.setpoint(1, 5.0) - (0.01 * 70.0 + 600.0 * 0.01 * (0.01f32.powi(5) * 0.3 + 0.01 * 0.7))).abs() < 1e-4);
        assert!((p.setpoint(2, 500.0) - 670.0).abs() < 1e-3);
    }

    #[test]
    fn betaflight_rates_match_rc_c() {
        let p = RatesProfile::from_headers(&hdr(&[("rates_type", "0"), ("rc_rates", "100,100,100"), ("rc_expo", "0,0,0"), ("rates", "70,70,70"), ("rate_limits", "1998,1998,1998")])).unwrap();
        // 200·1.0·1 × 1/(1−0.7)
        assert!((p.setpoint(0, 500.0) - 666.6667).abs() < 1e-2);
        assert!((p.setpoint(0, 0.0)).abs() < 1e-6);
        // rate_limit clamps
        let q = RatesProfile { rate_limits: [400.0; 3], ..p };
        assert_eq!(q.setpoint(0, 500.0), 400.0);
    }

    #[test]
    fn deadband_widens_divider_and_unknown_types_are_flagged() {
        let p = RatesProfile::from_headers(&hdr(&[("rates_type", "3"), ("rc_rates", "10,10,10"), ("rc_expo", "0,0,0"), ("rates", "50,50,50"), ("deadband", "10"), ("yaw_deadband", "20")])).unwrap();
        assert!((p.setpoint(0, 490.0) - 500.0).abs() < 1e-3);
        assert!((p.setpoint(2, 480.0) - 500.0).abs() < 1e-3);
        assert!(!RatesProfile { rates_type: 4, ..p.clone() }.supported());
        assert!(RatesProfile { rates_type: 0, ..p }.supported());
    }
}
