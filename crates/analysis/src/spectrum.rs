//! Gyro / D-term spectra ("Full Spectrum" panel).

use crate::range_idx;
use domain::{Axis, FlightLog, Spectrum, SpectrumKind};
use dsp::welch::{periodogram_pidtoolbox, welch, WelchOpts};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpectrumMode {
    /// Averaged Welch PSD with Hann + energy correction (ArduPilot WebTools scaling).
    Welch,
    /// Single Hann window over the whole range (PIDtoolbox `PTSpec2d`).
    PidToolbox,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpectrumOpts {
    pub mode: SpectrumMode,
    /// Welch FFT size; `0` = auto (~1 Hz resolution, capped at 4096).
    pub nfft: usize,
    pub overlap: f32,
    /// Discard bins above this frequency (0 = keep up to Nyquist).
    pub max_hz: f32,
}

impl Default for SpectrumOpts {
    fn default() -> Self {
        Self { mode: SpectrumMode::Welch, nfft: 0, overlap: 0.5, max_hz: 1000.0 }
    }
}

pub(crate) fn series<'a>(log: &'a FlightLog, axis: Axis, kind: SpectrumKind) -> Option<&'a [f32]> {
    let ax = log.axis(axis);
    match kind {
        SpectrumKind::GyroRaw => ax.gyro_raw.as_deref(),
        SpectrumKind::GyroFilt => Some(&ax.gyro_filt),
        SpectrumKind::DTerm => ax.d.as_deref(),
        SpectrumKind::Setpoint => Some(&ax.setpoint),
        SpectrumKind::PidSum => ax.pid_sum.as_deref(),
        SpectrumKind::Predicted => None,
    }
}

pub fn auto_nfft(fs: f64, len: usize) -> usize {
    let want = (fs.round() as usize).next_power_of_two().clamp(256, 4096);
    let mut n = want;
    while n > 256 && n > len {
        n /= 2;
    }
    n
}

pub fn spectrum(
    log: &FlightLog,
    axis: Axis,
    kind: SpectrumKind,
    opts: &SpectrumOpts,
    range_s: Option<(f32, f32)>,
) -> Option<Spectrum> {
    let s = series(log, axis, kind)?;
    let (i0, i1) = range_idx(log, range_s);
    let x = &s[i0.min(s.len())..i1.min(s.len())];
    if x.len() < 256 {
        return None;
    }
    let psd = match opts.mode {
        SpectrumMode::Welch => {
            let nfft = if opts.nfft == 0 { auto_nfft(log.fs_hz, x.len()) } else { opts.nfft };
            welch(x, log.fs_hz, WelchOpts { nfft, overlap: opts.overlap, ..Default::default() })
        }
        SpectrumMode::PidToolbox => periodogram_pidtoolbox(x, log.fs_hz),
    };
    let keep = if opts.max_hz > 0.0 {
        psd.f_hz.iter().take_while(|f| **f <= opts.max_hz).count()
    } else {
        psd.f_hz.len()
    };
    Some(Spectrum {
        axis,
        kind,
        f_hz: psd.f_hz[..keep].to_vec(),
        psd_db: psd.psd_db[..keep].to_vec(),
        nfft: psd.nfft,
        fs_hz: psd.fs_hz,
    })
}
