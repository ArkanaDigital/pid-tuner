//! Headless driver: `pidtool sessions <log>`, `pidtool analyze <log> [--session N] [--json out]`.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::time::Instant;

#[derive(Parser)]
#[command(name = "pidtool", version, about = "Blackbox tuning analysis CLI")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// List sessions inside a blackbox file.
    Sessions { file: PathBuf },
    /// Ingest a session and print a summary (or the full FlightLog as JSON).
    Ingest {
        file: PathBuf,
        #[arg(long, default_value_t = 0)]
        session: usize,
        #[arg(long)]
        json: Option<PathBuf>,
    },
    /// Run the full analysis and print metrics (or the AnalysisBundle as JSON).
    Analyze {
        file: PathBuf,
        #[arg(long, default_value_t = 0)]
        session: usize,
        #[arg(long)]
        json: Option<PathBuf>,
        /// Use the PID-Analyzer step variant instead of PIDtoolbox.
        #[arg(long)]
        pid_analyzer: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Sessions { file } => {
            let bytes = std::fs::read(&file).with_context(|| format!("read {}", file.display()))?;
            for s in bbl_ingest::list_sessions(&bytes) {
                println!(
                    "#{:<3} {:<40} {:<20} {}",
                    s.index,
                    s.firmware_revision,
                    s.craft_name.unwrap_or_default(),
                    s.error.unwrap_or_default()
                );
            }
        }
        Cmd::Ingest { file, session, json } => {
            let bytes = std::fs::read(&file)?;
            let t0 = Instant::now();
            let log = bbl_ingest::ingest(&bytes, session, &Default::default())?;
            eprintln!("ingested in {:.2?}", t0.elapsed());
            print_log_summary(&log);
            if let Some(p) = json {
                std::fs::write(&p, serde_json::to_vec(&log)?)?;
                eprintln!("wrote {}", p.display());
            }
        }
        Cmd::Analyze { file, session, json, pid_analyzer } => {
            let bytes = std::fs::read(&file)?;
            let t0 = Instant::now();
            let log = bbl_ingest::ingest(&bytes, session, &Default::default())?;
            let t1 = Instant::now();
            let mut opts = analysis::AnalysisOpts::default();
            if pid_analyzer {
                opts.step = analysis::StepOpts::pid_analyzer();
            }
            let bundle = analysis::analyze(&log, &opts, |_| {});
            let t2 = Instant::now();
            print_log_summary(&log);
            eprintln!("ingest {:.2?}, analyze {:.2?}", t1 - t0, t2 - t1);
            println!();
            println!("Step response ({:?}):", opts.step.variant);
            for s in &bundle.steps {
                println!(
                    "  {:<5} segs={:<4} rej={:<3} overshoot={:.3} latency={:.1} ms settle={} ss={:.3}",
                    s.axis.name(),
                    s.n_segments,
                    s.rejected,
                    s.overshoot,
                    s.latency_ms,
                    s.settle_ms.map(|v| format!("{v:.0} ms")).unwrap_or("-".into()),
                    s.steady_state
                );
            }
            println!("Noise peaks:");
            for p in &bundle.peaks {
                println!(
                    "  {:<5} {:<9} {:>7.1} Hz {:>6.1} dB (+{:.1}) {:?}",
                    p.axis.name(),
                    format!("{:?}", p.kind),
                    p.f_hz,
                    p.psd_db,
                    p.prominence_db,
                    p.band
                );
            }
            let q = &bundle.quality;
            println!(
                "Quality: fs={:.0} Hz dur={:.1} s raw_gyro={} pid_terms={} hover={:.1} s @{:.0}% airborne={:?} sat={:.2}% gaps={:.2} s max_sp={:?}",
                q.fs_hz, q.duration_s, q.has_gyro_raw, q.has_pid_terms, q.hover_seconds, q.hover_throttle_pct, q.airborne_range_s,
                q.motor_saturation_pct, q.gap_seconds, q.max_setpoint_per_axis
            );
            if let Some(p) = json {
                std::fs::write(&p, serde_json::to_vec(&bundle)?)?;
                eprintln!("wrote {}", p.display());
            }
        }
    }
    Ok(())
}

fn print_log_summary(log: &domain::FlightLog) {
    println!(
        "{:?} craft={:?} fs={:.0} Hz (src {:.0} Hz, loop {:?}) samples={} dur={:.1} s session {}/{} debug={:?} gaps={} warnings={:?}",
        log.firmware,
        log.meta.craft_name,
        log.fs_hz,
        log.meta.source_rate_hz,
        log.meta.loop_hz,
        log.len(),
        log.duration_s(),
        log.meta.session_index + 1,
        log.meta.session_count,
        log.meta.debug_mode,
        log.gaps.len(),
        log.meta.warnings
    );
    if let domain::Tune::Bf(t) = &log.tune_at_log {
        for (k, name) in ["roll", "pitch", "yaw"].iter().enumerate() {
            let p = t.pids[k];
            print!("  {name}: P{} I{} D{} Dmax{} FF{}", p.p, p.i, p.d, p.d_max, p.ff);
        }
        println!();
        let f = &t.filters;
        println!(
            "  gyro lpf1 {} {:?} dyn {}-{} | lpf2 {} | dterm lpf1 {} dyn {}-{} lpf2 {} | dyn_notch n{} q{} {}-{} Hz | rpm h{} q{} min{} | simplified pids_mode={}",
            f.gyro_lpf1_static_hz, f.gyro_lpf1_type, f.gyro_lpf1_dyn_min_hz, f.gyro_lpf1_dyn_max_hz,
            f.gyro_lpf2_static_hz, f.dterm_lpf1_static_hz, f.dterm_lpf1_dyn_min_hz, f.dterm_lpf1_dyn_max_hz,
            f.dterm_lpf2_static_hz, f.dyn_notch_count, f.dyn_notch_q, f.dyn_notch_min_hz, f.dyn_notch_max_hz,
            f.rpm_filter_harmonics, f.rpm_filter_q, f.rpm_filter_min_hz, t.simplified.pids_mode
        );
    }
}
