//! Minimal self-contained HTML report (charts embedded as PNG data URLs).

use crate::wizard::ReportImage;
use domain::*;
use session::*;

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub fn render(s: &Session, bundles: &[(Flight, AnalysisBundle)], images: &[ReportImage]) -> String {
    let mut h = String::new();
    h.push_str(
        "<!doctype html><html><head><meta charset='utf-8'><title>PID Tuning Report</title><style>",
    );
    h.push_str("body{font-family:-apple-system,Segoe UI,Helvetica,Arial,sans-serif;max-width:1100px;margin:24px auto;color:#1d1d1f;padding:0 16px}");
    h.push_str("h1{font-size:22px}h2{font-size:17px;margin-top:28px;border-bottom:1px solid #ddd;padding-bottom:4px}table{border-collapse:collapse;width:100%}");
    h.push_str("th,td{text-align:left;padding:6px 8px;border-bottom:1px solid #eee;vertical-align:top}th{background:#fafafa}img{max-width:100%;border:1px solid #e0e0e4;border-radius:6px;margin:8px 0}");
    h.push_str("pre{background:#1e1e24;color:#eee;padding:12px;border-radius:8px}.muted{color:#666}</style></head><body>");
    h.push_str(&format!("<h1>PID Tuning Report — {}</h1>", esc(&s.name)));
    h.push_str(&format!(
        "<p class='muted'>Session {} · {:?} mode · created {} · {}</p>",
        s.id,
        s.mode,
        s.created_at.format("%Y-%m-%d %H:%M"),
        s.firmware
            .as_ref()
            .map(|f| format!("{f:?}"))
            .unwrap_or_default()
    ));

    h.push_str("<h2>Flights</h2><table><tr><th>Flight</th><th>Log</th><th>Rate</th><th>Duration</th><th>Raw gyro</th><th>Hover</th><th>Steps R/P/Y</th></tr>");
    for (f, r) in &s.flights {
        let q = &r.quality;
        h.push_str(&format!(
            "<tr><td>{f:?}</td><td>{}</td><td>{:.0} Hz</td><td>{:.1} s</td><td>{}</td><td>{:.1} s</td><td>{}/{}/{}</td></tr>",
            esc(&r.original_path), q.fs_hz, q.duration_s, if q.has_gyro_raw { "yes" } else { "no" }, q.hover_seconds,
            q.step_segments_per_axis[0], q.step_segments_per_axis[1], q.step_segments_per_axis[2]
        ));
    }
    h.push_str("</table>");

    h.push_str("<h2>Step response metrics</h2><table><tr><th>Flight</th><th>Axis</th><th>Segments</th><th>Overshoot</th><th>Latency</th><th>Steady state</th></tr>");
    for (f, b) in bundles {
        for st in &b.steps {
            h.push_str(&format!(
                "<tr><td>{f:?}</td><td>{}</td><td>{}</td><td>{:.2}</td><td>{:.1} ms</td><td>{:.2}</td></tr>",
                st.axis.name(), st.n_segments, st.overshoot, st.latency_ms, st.steady_state
            ));
        }
    }
    h.push_str("</table>");

    if bundles.iter().any(|(_, b)| !b.freq_resp.is_empty()) {
        h.push_str("<h2>Frequency response (CHIRP)</h2><table><tr><th>Flight</th><th>Axis</th><th>Sweeps / windows</th><th>Coherence</th><th>Bandwidth</th><th>Crossover</th><th>Phase margin</th><th>Resonant peak</th><th>Loop delay</th><th>Sensitivity peak</th></tr>");
        for (f, b) in bundles {
            for fr in &b.freq_resp {
                let m = &fr.metrics;
                h.push_str(&format!(
                    "<tr><td>{f:?}</td><td>{}{}</td><td>{} / {}</td><td>{:.2}</td><td>{:.1} Hz</td><td>{:.1} Hz</td><td>{:.0}°</td><td>{:+.1} dB @ {:.0} Hz</td><td>{:.2} ms</td><td>{:+.1} dB</td></tr>",
                    fr.axis.name(), if fr.angle_mode { " (ANGLE mode)" } else { "" }, fr.n_sweeps, fr.n_windows, m.coherence_mean, m.bandwidth_hz, m.crossover_hz, m.phase_margin_deg, m.resonant_peak_db, m.resonant_peak_hz, m.loop_delay_ms, m.sens_peak_db
                ));
            }
        }
        h.push_str("</table>");
    }

    if bundles.iter().any(|(_, b)| !b.anomalies.is_empty()) {
        h.push_str("<h2>Anomalies</h2><table><tr><th>Flight</th><th>Severity</th><th>Type</th><th>Time</th><th>Detail</th></tr>");
        for (f, b) in bundles {
            for a in &b.anomalies {
                h.push_str(&format!(
                    "<tr><td>{f:?}</td><td>{:?}</td><td>{}</td><td>{:.1}–{:.1} s</td><td>{}</td></tr>",
                    a.severity, a.kind.title(), a.t_start_s, a.t_end_s, esc(&a.detail)
                ));
            }
        }
        h.push_str("</table>");
    }

    h.push_str("<h2>Noise peaks</h2><table><tr><th>Flight</th><th>Axis</th><th>Signal</th><th>Frequency</th><th>Level</th><th>Band</th></tr>");
    for (f, b) in bundles {
        for p in &b.peaks {
            h.push_str(&format!(
                "<tr><td>{f:?}</td><td>{}</td><td>{:?}</td><td>{:.0} Hz</td><td>{:.1} dB (+{:.0})</td><td>{:?}</td></tr>",
                p.axis.name(), p.kind, p.f_hz, p.psd_db, p.prominence_db, p.band
            ));
        }
    }
    h.push_str("</table>");

    for (title, recs) in [
        ("Filter changes", &s.recs_filters),
        ("PID changes", &s.recs_pids),
    ] {
        h.push_str(&format!("<h2>{title}</h2>"));
        if recs.is_empty() {
            h.push_str("<p class='muted'>No changes.</p>");
            continue;
        }
        h.push_str("<table><tr><th>Parameter</th><th>Before</th><th>After</th><th>Reason</th><th>Applied</th></tr>");
        for r in recs {
            h.push_str(&format!(
                "<tr><td><code>{}</code></td><td>{}</td><td><b>{}</b></td><td>{}</td><td>{}</td></tr>",
                esc(r.param.name()), r.old, r.new, esc(&r.reason), if r.accepted { "yes" } else { "no" }
            ));
        }
        h.push_str("</table>");
        let text = domain::fc::export_text_for(s.firmware.as_ref(), recs);
        if !text.is_empty() {
            h.push_str(&format!("<pre>{}</pre>", esc(&text)));
        }
    }

    if !s.overrides.is_empty() {
        h.push_str("<h2>Guard overrides</h2><ul>");
        for o in &s.overrides {
            h.push_str(&format!(
                "<li><b>{:?} / {}</b>: {}</li>",
                o.step,
                esc(&o.guard_id),
                esc(&o.reason)
            ));
        }
        h.push_str("</ul>");
    }

    if !images.is_empty() {
        h.push_str("<h2>Charts</h2>");
        for im in images {
            h.push_str(&format!(
                "<h3>{}</h3><img src='{}' alt='{}'>",
                esc(&im.title),
                im.data_url,
                esc(&im.title)
            ));
        }
    }
    if !s.notes.trim().is_empty() {
        h.push_str(&format!(
            "<h2>Notes</h2><p>{}</p>",
            esc(&s.notes).replace('\n', "<br>")
        ));
    }
    h.push_str("</body></html>");
    h
}
