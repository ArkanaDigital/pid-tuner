//! Flight protocol text shown to the pilot (and handed to the AI helper), one
//! source for both firmwares. Mirrors the previous `ui/wizard/steps.tsx` tables.

use crate::model::Flight;
use domain::Firmware;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Protocol {
    pub title: String,
    pub steps: Vec<String>,
    pub note: String,
    /// Optional alternative procedure (e.g. Betaflight CHIRP for Flight B).
    pub alternative: Option<ProtocolAlt>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtocolAlt {
    pub title: String,
    pub steps: Vec<String>,
    pub note: String,
}

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

pub fn text(firmware: Option<&Firmware>, flight: Flight) -> Protocol {
    let ap = matches!(firmware, Some(Firmware::ArduCopter { .. }));
    match (ap, flight) {
        (false, Flight::A) => Protocol {
            title: "Flight A — noise / filter data".into(),
            steps: s(&[
                "Props on, battery fresh, arm in a safe open area (or over a bed indoors for a tiny whoop).",
                "Hover steadily for 30 seconds at normal hover throttle. No stick input.",
                "Then hover with gentle wobbles for 20 seconds (small roll/pitch rocking, throttle 30–70 %).",
                "Land, disarm. Do not power off before the log is saved.",
            ]),
            note: "This flight only needs to be smooth. The spectrum analysis needs ≥ 20 s of clean hover and a spread of throttle values.".into(),
            alternative: None,
        },
        (false, Flight::B) => Protocol {
            title: "Flight B — step response data".into(),
            steps: s(&[
                "Take off and hover for 5 seconds (ACRO mode).",
                "Roll: sharp stick snap left, hold ½ s, centre, pause 1 s. Repeat right. Do 15 pairs.",
                "Pitch: same pattern forward / back, 15 pairs.",
                "Yaw: same pattern, 5 pairs.",
                "Keep axes separate — one axis moving at a time. Land and disarm.",
            ]),
            note: "Sharp, isolated stick moves ≥ 200 °/s on each axis give the deconvolution what it needs; ≥ 30 segments per axis for a trustworthy curve.".into(),
            alternative: Some(ProtocolAlt {
                title: "Alternative: CHIRP sweep (Betaflight 2025.12+, custom build with USE_CHIRP)".into(),
                steps: s(&[
                    "Flash a build with the CHIRP define, assign the CHIRP mode to an aux switch, set debug_mode = CHIRP and blackbox_high_resolution = ON.",
                    "Keep chirp_* at defaults (0.2 → 600 Hz, 20 s, amplitude 230/230/180 °/s) unless the quad is very small or very large.",
                    "Hover in ACRO (not ANGLE/HORIZON: the attitude loop would be inside the measurement) with sticks centred.",
                    "Switch CHIRP on and hold the hover for the full 20 s sweep, then switch it off; the next activation moves to the next axis (roll → pitch → yaw).",
                    "Do at least two full cycles over all three axes. Land and disarm.",
                ]),
                note: "The app builds the closed-loop frequency response (bandwidth, phase margin, sensitivity peak) and needs ≥ 8 Welch windows and mean coherence ≥ 0.6 per axis.".into(),
            }),
        },
        (false, Flight::C) => Protocol {
            title: "Flight C — verification".into(),
            steps: s(&["Fly the same protocol as Flight B with the new PIDs.", "Optionally add a few flips/rolls and throttle punches.", "Land and disarm."]),
            note: "This log becomes the 'after' in the before/after comparison and the report.".into(),
            alternative: None,
        },
        (true, Flight::A) => Protocol {
            title: "Flight A — noise / filter data (ArduCopter)".into(),
            steps: s(&[
                "Props on, battery fresh, GPS not required. Arm in AltHold (or Loiter) in a safe open area.",
                "Hover steadily for 30 seconds at hover throttle (stick centred). No pitch/roll input.",
                "Then 20 seconds of gentle rocking (small roll/pitch, throttle 30–70 %) so the spectrogram covers a throttle range.",
                "Land, disarm. Wait ~5 s before power-off so the .bin log is closed.",
            ]),
            note: "Needs LOG_BITMASK bits 0+12+19 and the IMU batch sampler (INS_LOG_BAT_MASK=1, INS_LOG_BAT_OPT=4) — Preflight sets them. The gyro spectrum comes from ISBH/ISBD batches; ≥ 20 batches are required.".into(),
            alternative: None,
        },
        (true, Flight::B) => Protocol {
            title: "Flight B — step response data (ArduCopter)".into(),
            steps: s(&[
                "Take off in Stabilize (rate response is what we measure; AltHold/Loiter add position loops). Hover 5 s.",
                "Roll: sharp stick snap left, hold ½ s, centre, pause 1 s. Repeat right. Do 15 pairs.",
                "Pitch: same pattern forward / back, 15 pairs.",
                "Yaw: same pattern, 5 pairs.",
                "One axis at a time. Land and disarm.",
            ]),
            note: "ATC_INPUT_TC shapes the pilot input, so the target seen by the rate loop (PIDx.Tar) is already filtered — snaps ≥ 60 °/s on roll/pitch, ≥ 40 °/s on yaw are still needed. ≥ 30 segments per axis for a trustworthy curve.".into(),
            alternative: None,
        },
        (true, Flight::C) => Protocol {
            title: "Flight C — verification (ArduCopter)".into(),
            steps: s(&[
                "Heuristic path: fly the same Stabilize protocol as Flight B with the new gains.",
                "AUTOTUNE path: fly AUTOTUNE (AUTOTUNE_AXES / AUTOTUNE_AGGR as set), let it finish, land and disarm WITHOUT touching the sticks so the gains are saved — then fly the Flight B protocol once more.",
                "Land and disarm.",
            ]),
            note: "The Import C guard checks ATC_RAT_*_P/D actually changed and D did not end at AUTOTUNE_MIN_D (a failed autotune).".into(),
            alternative: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_flight_has_text_for_both_firmwares() {
        for fw in [
            None,
            Some(Firmware::ArduCopter {
                version: "4.5".into(),
            }),
            Some(Firmware::Betaflight {
                version: "2026.6.1".into(),
                api: (1, 47),
            }),
        ] {
            for f in [Flight::A, Flight::B, Flight::C] {
                let p = text(fw.as_ref(), f);
                assert!(!p.title.is_empty() && p.steps.len() >= 3 && !p.note.is_empty());
            }
        }
        assert!(text(None, Flight::B)
            .alternative
            .as_ref()
            .unwrap()
            .title
            .contains("CHIRP"));
        assert!(text(
            Some(&Firmware::ArduCopter {
                version: "4.5".into()
            }),
            Flight::B
        )
        .alternative
        .is_none());
    }
}
