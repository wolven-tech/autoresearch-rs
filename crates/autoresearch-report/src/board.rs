//! Self-contained, script-free review surface for a validated run report.

use crate::{ArtifactReference, CandidateEvidence, RunReportV1};
use autoresearch_core::{DecisionReason, Disposition, Measurement, MetricDirection, RepoPath};
use std::fmt::Write as _;
use std::fs;
use std::path::{Component, Path};
use thiserror::Error;

const STYLE: &str = r#"
:root{color-scheme:light;--paper:#f6f3ec;--surface:#fffcf5;--ink:#18332f;--quiet:#536962;--line:#a9bbb0;--accent:#9c3d21;--accent-soft:#f7e8dc;--focus:#d65d2c;font-family:ui-sans-serif,-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif;font-synthesis:none}
*{box-sizing:border-box}html{scroll-behavior:smooth}body{margin:0;background:var(--paper);color:var(--ink);font-size:1rem;line-height:1.55}a{color:inherit;text-underline-offset:.2em}a:hover{text-decoration-thickness:.16em}a:focus-visible,summary:focus-visible{outline:3px solid var(--focus);outline-offset:4px;border-radius:2px}.skip{position:absolute;top:-10rem;left:1rem;background:var(--surface);padding:.75rem 1rem;z-index:2}.skip:focus{top:1rem}.wrap{width:min(100% - 2rem,72rem);margin-inline:auto}.masthead{background:var(--ink);color:var(--surface);border-bottom:6px solid #c98451}.masthead .wrap{padding-block:1.5rem 2.5rem}.eyebrow{margin:0;text-transform:uppercase;letter-spacing:.14em;font-size:.75rem;font-weight:750}.masthead h1{font-family:ui-serif,Georgia,serif;font-weight:500;line-height:1.05;font-size:clamp(2.4rem,5vw,4.6rem);letter-spacing:-.035em;margin:.55rem 0 1rem}.masthead p{max-width:62ch}.topline{display:flex;flex-wrap:wrap;align-items:center;justify-content:space-between;gap:.5rem 2rem;border-bottom:1px solid #719088;padding-bottom:1rem}.runid{overflow-wrap:anywhere;font-variant-numeric:tabular-nums}.nav{display:flex;flex-wrap:wrap;gap:.5rem 1.25rem}.nav a{display:inline-flex;min-height:2.75rem;align-items:center}.boundary{font-size:.9rem;color:#e5e9dc;margin-bottom:0}.main{padding-block:2.5rem 5rem}.section{border-top:1px solid var(--line);padding-block:2.25rem}.section h2{font-family:ui-serif,Georgia,serif;font-weight:500;font-size:clamp(1.9rem,3vw,2.8rem);line-height:1.12;margin:0 0 1.5rem}.lede{max-width:64ch;color:var(--quiet);margin:-.75rem 0 1.5rem}.summary{display:grid;grid-template-columns:1fr;gap:1px;background:var(--line);border:1px solid var(--line)}.summary>div{background:var(--surface);padding:1.25rem;min-width:0}.summary dt{font-size:.78rem;text-transform:uppercase;letter-spacing:.1em;font-weight:750;color:var(--quiet)}.summary dd{margin:.35rem 0 0;font-family:ui-serif,Georgia,serif;font-size:1.45rem;line-height:1.2;overflow-wrap:anywhere}.summary small{display:block;color:var(--quiet);font:400 .85rem/1.5 ui-sans-serif,-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif;margin-top:.45rem}.state{font-size:.74rem;text-transform:uppercase;letter-spacing:.08em;font-weight:800;border:1px solid currentColor;padding:.2rem .55rem;white-space:nowrap}.state.keep{color:#205c42}.state.discard{color:#9c3d21}.state.pending{color:#4b6170}.empty{border:1px dashed var(--line);padding:1.25rem;background:var(--surface)}.candidate{border-top:1px solid var(--line);padding:1.5rem 0 2rem;min-width:0}.candidate:first-of-type{border-top:0}.candidate-head{display:flex;align-items:baseline;flex-wrap:wrap;justify-content:space-between;gap:.75rem}.candidate h3{font-family:ui-serif,Georgia,serif;font-size:1.7rem;font-weight:500;line-height:1.2;margin:0}.candidate .number{font:750 .78rem/1 ui-sans-serif,-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif;letter-spacing:.08em;color:var(--accent);margin-right:.75rem}.decision{font-size:1.05rem;margin:.6rem 0 0;max-width:64ch}.candidate details{margin-top:1.25rem}.candidate summary{cursor:pointer;display:inline-flex;align-items:center;min-height:2.75rem;font-weight:700;text-decoration:underline;text-underline-offset:.22em}.detail-grid{display:grid;grid-template-columns:1fr;gap:1.5rem;margin-top:1rem}.detail-grid section{min-width:0}.detail-grid h4{font-size:.78rem;letter-spacing:.1em;text-transform:uppercase;margin:0 0 .6rem;color:var(--quiet)}.detail-grid p{margin:.35rem 0}.fine{font-size:.88rem;color:var(--quiet)}code{font-size:.9em;overflow-wrap:anywhere;font-variant-ligatures:none}.measure{font-variant-numeric:tabular-nums;font-weight:750}.gate-list,.paths,.receipts,.artifacts{list-style:none;padding:0;margin:.25rem 0;display:grid;gap:.45rem}.gate-list li,.receipts li{display:flex;flex-wrap:wrap;gap:.35rem .65rem;align-items:baseline;border-bottom:1px solid var(--line);padding:.35rem 0}.gate-label{font-size:.74rem;text-transform:uppercase;letter-spacing:.07em;font-weight:800}.gate-pass{color:#205c42}.gate-fail{color:#9c3d21}.paths li{padding-left:.8rem;border-left:2px solid var(--line)}.artifacts li{min-width:0}.artifacts a{display:inline-flex;min-height:2.75rem;align-items:center}.artifacts img{display:block;width:min(100%,24rem);height:auto;max-height:18rem;object-fit:contain;border:1px solid var(--line);background:var(--surface);margin-top:.4rem}.receipt-type{font-weight:750}.footer{background:var(--ink);color:var(--surface);padding:1.5rem 0}.footer p{margin:0;font-size:.85rem}.mono{font-variant-numeric:tabular-nums;overflow-wrap:anywhere}@media(min-width:44rem){.summary{grid-template-columns:repeat(3,minmax(0,1fr))}.detail-grid{grid-template-columns:repeat(2,minmax(0,1fr))}}@media(min-width:64rem){.detail-grid{grid-template-columns:repeat(3,minmax(0,1fr))}}@media(prefers-reduced-motion:reduce){html{scroll-behavior:auto}}@media print{body{background:white}.masthead,.footer{print-color-adjust:exact}.candidate details{display:block}.candidate details>*{display:block}}
"#;

/// Board refuses unknown schema, unsafe artifact paths, or escaped directories.
#[derive(Debug, Error)]
pub enum BoardError {
    /// Only known report schema may render.
    #[error("unsupported report schema version")]
    Schema,
    /// Artifact reference cannot remain under run-owned directory.
    #[error("unsafe artifact reference")]
    Artifact,
    /// Artifact directory cannot be inspected.
    #[error("artifact inspection failed: {0}")]
    Io(#[from] std::io::Error),
}

/// Renders one report as UTF-8 HTML with inline CSS and no scripts or remote
/// dependencies. Untrusted text is escaped; artifact hrefs are local and
/// validated at rendering time. Missing artifacts become plain text.
///
/// # Errors
///
/// Rejects unknown schema or escaped artifact paths.
pub fn render_board(report: &RunReportV1, run_directory: &Path) -> Result<Vec<u8>, BoardError> {
    if report.schema_version != 1 {
        return Err(BoardError::Schema);
    }
    let mut html = String::with_capacity(16 * 1024);
    html.push_str("<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1,viewport-fit=cover\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; img-src 'self' data:; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'; object-src 'none'\"><title>Run evidence · Autoresearch</title><style>");
    html.push_str(STYLE);
    html.push_str("</style></head><body><a class=\"skip\" href=\"#main\">Skip to evidence</a><header class=\"masthead\"><div class=\"wrap\"><div class=\"topline\"><p class=\"eyebrow\">Autoresearch / Evidence ledger</p><nav class=\"nav\" aria-label=\"Report sections\"><a href=\"#summary\">Summary</a><a href=\"#candidates\">Candidates</a><a href=\"#market\">Market evidence</a><a href=\"#provenance\">Provenance</a></nav></div><h1>What changed. What held.</h1><p class=\"runid\">Run <code>");
    html.push_str(&escape(&report.run_id));
    html.push_str("</code></p><p class=\"boundary\">Internal evaluation supports code selection. It does not validate customer demand or product promotion.</p></div></header><main id=\"main\" class=\"wrap main\">");
    render_summary(&mut html, report);
    render_candidates(&mut html, report, run_directory)?;
    render_market(&mut html, report);
    render_provenance(&mut html, report);
    html.push_str("</main><footer class=\"footer\"><div class=\"wrap\"><p>Rebuilt from frozen inputs and append-only journal. Verify original records before external use.</p></div></footer></body></html>\n");
    Ok(html.into_bytes())
}

fn render_summary(html: &mut String, report: &RunReportV1) {
    html.push_str("<section id=\"summary\" class=\"section\" aria-labelledby=\"summary-title\"><h2 id=\"summary-title\">Run at a glance</h2><dl class=\"summary\"><div><dt>Baseline</dt><dd>");
    metric_value(
        html,
        report.baseline.as_ref().map(|item| &item.snapshot),
        &report.objective.name,
    );
    html.push_str("</dd><small>Commit <code>");
    html.push_str(&escape(&short_commit(&report.base_commit)));
    html.push_str("</code></small></div><div><dt>Current best</dt><dd>");
    metric_value(
        html,
        report.current_best_snapshot.as_ref(),
        &report.objective.name,
    );
    html.push_str("</dd><small>Commit <code>");
    html.push_str(&escape(&short_commit(&report.current_best_commit)));
    html.push_str("</code></small></div><div><dt>Primary objective</dt><dd>");
    html.push_str(&escape(&report.objective.name));
    html.push_str("</dd><small>");
    html.push_str(match report.objective.direction {
        MetricDirection::Maximize => "Higher is better",
        MetricDirection::Minimize => "Lower is better",
    });
    html.push_str("</small></div></dl>");
    if let Some(failure) = &report.baseline_failure {
        html.push_str("<p role=\"status\">Baseline evaluator <strong>");
        html.push_str(&escape(&failure.evaluator_id));
        html.push_str("</strong> failed: ");
        html.push_str(&escape(&format!("{:?}", failure.failure.class)));
        html.push_str(". No comparable baseline score was recorded.</p>");
    }
    html.push_str("</section>");
}

fn render_candidates(
    html: &mut String,
    report: &RunReportV1,
    run_directory: &Path,
) -> Result<(), BoardError> {
    html.push_str("<section id=\"candidates\" class=\"section\" aria-labelledby=\"candidates-title\"><h2 id=\"candidates-title\">Candidate trail</h2>");
    if report.candidates.is_empty() {
        if report.baseline.is_some() {
            html.push_str(
                "<p class=\"empty\">No candidates evaluated. Baseline remains current best.</p>",
            );
        } else {
            html.push_str(
                "<p class=\"empty\">No candidate evidence. Baseline failed or is incomplete.</p>",
            );
        }
    }
    for candidate in &report.candidates {
        render_candidate(html, candidate, run_directory)?;
    }
    html.push_str("</section>");
    Ok(())
}

fn render_candidate(
    html: &mut String,
    candidate: &CandidateEvidence,
    run_directory: &Path,
) -> Result<(), BoardError> {
    let state_class = match candidate.state.as_str() {
        "kept" => "keep",
        "discarded" => "discard",
        _ => "pending",
    };
    let _ = write!(
        html,
        "<article class=\"candidate\" id=\"candidate-{}\"><div class=\"candidate-head\"><h3><span class=\"number\">{:02}</span>Candidate {}</h3><span class=\"state {}\">{}</span></div>",
        candidate.index,
        candidate.index,
        candidate.index,
        state_class,
        escape(&candidate.state)
    );
    html.push_str("<p class=\"decision\">");
    match &candidate.decision {
        Some(decision) => {
            html.push_str(&escape(&decision_label(&decision.reason)));
            if decision.disposition == Disposition::Keep {
                html.push_str(" Candidate retained.");
            } else {
                html.push_str(" Candidate not retained.");
            }
        }
        None => html.push_str("Evaluation incomplete; no keep/discard decision recorded."),
    }
    html.push_str("</p><details><summary>Inspect candidate evidence</summary><div class=\"detail-grid\"><section><h4>Comparison</h4>");
    if let Some(delta) = candidate.objective_delta {
        let _ = write!(
            html,
            "<p>Objective change: <span class=\"measure\">{delta:+}</span></p>"
        );
    } else {
        html.push_str("<p>Objective change: unavailable</p>");
    }
    if let Some(runtime_ms) = candidate.runtime_ms {
        let _ = write!(
            html,
            "<p>Evaluation runtime: <span class=\"measure\">{runtime_ms} ms</span></p>"
        );
    }
    html.push_str("<ul class=\"gate-list\" aria-label=\"Hard gates\">");
    for gate in &candidate.hard_gates {
        let (class, label) = match gate.candidate_passed {
            Some(true) => ("gate-pass", "Passed"),
            Some(false) => ("gate-fail", "Failed"),
            None => ("", "Not measured"),
        };
        let _ = write!(
            html,
            "<li><span class=\"gate-label {}\">{}</span><span>{}</span></li>",
            class,
            label,
            escape(&gate.name)
        );
    }
    html.push_str("</ul></section><section><h4>Exact change</h4><p class=\"fine\">Parent <code>");
    html.push_str(&escape(&candidate.parent_commit));
    html.push_str("</code></p><p class=\"fine\">Candidate <code>");
    html.push_str(
        &candidate
            .candidate_commit
            .as_deref()
            .map_or_else(|| "Not recorded".into(), escape),
    );
    html.push_str("</code></p><ul class=\"paths\" aria-label=\"Changed paths\">");
    for path in &candidate.changed_paths {
        let _ = write!(html, "<li><code>{}</code></li>", escape(path));
    }
    html.push_str("</ul><p class=\"fine\">Path evidence: ");
    html.push_str(&escape(&candidate.changed_paths_status));
    html.push_str("</p></section><section><h4>Artifacts and failures</h4>");
    render_artifacts(html, &candidate.artifacts, run_directory)?;
    for failure in &candidate.failures {
        let _ = write!(
            html,
            "<p class=\"gate-fail\">{}: {:?}. No score recorded for this attempt.</p>",
            escape(&failure.evaluator_id),
            failure.failure.class
        );
    }
    if candidate.artifacts.is_empty() && candidate.failures.is_empty() {
        html.push_str("<p class=\"fine\">No artifact or crash record.</p>");
    }
    html.push_str("</section></div></details></article>");
    Ok(())
}

fn render_artifacts(
    html: &mut String,
    artifacts: &[ArtifactReference],
    run_directory: &Path,
) -> Result<(), BoardError> {
    html.push_str("<ul class=\"artifacts\" aria-label=\"Candidate artifacts\">");
    for artifact in artifacts {
        let url = checked_artifact_url(run_directory, &artifact.relative_path)?;
        html.push_str("<li>");
        if let Some(url) = url {
            let _ = write!(
                html,
                "<a href=\"{}\">Open {} ({})</a>",
                escape(&url),
                escape(&artifact.name),
                escape(&artifact.media_type)
            );
            if matches!(
                artifact.media_type.as_str(),
                "image/png" | "image/jpeg" | "image/webp"
            ) {
                let _ = write!(
                    html,
                    "<img src=\"{}\" alt=\"Screenshot artifact: {}\" loading=\"lazy\">",
                    escape(&url),
                    escape(&artifact.name)
                );
            }
        } else {
            let _ = write!(
                html,
                "<span>{} — file unavailable</span>",
                escape(&artifact.name)
            );
        }
        html.push_str("</li>");
    }
    html.push_str("</ul>");
    Ok(())
}

fn checked_artifact_url(
    run_directory: &Path,
    relative: &str,
) -> Result<Option<String>, BoardError> {
    let path = RepoPath::new(relative).map_err(|_| BoardError::Artifact)?;
    let root = run_directory.join("artifacts");
    let mut current = root.clone();
    match fs::symlink_metadata(&root) {
        Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Ok(_) => return Err(BoardError::Artifact),
        Err(error) => return Err(BoardError::Io(error)),
    }
    for component in Path::new(path.as_str()).components() {
        let Component::Normal(part) = component else {
            return Err(BoardError::Artifact);
        };
        current.push(part);
        match fs::symlink_metadata(&current) {
            Ok(meta) if meta.file_type().is_symlink() => return Err(BoardError::Artifact),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(BoardError::Io(error)),
        }
    }
    if !fs::symlink_metadata(&current)?.is_file() {
        return Err(BoardError::Artifact);
    }
    Ok(Some(format!(
        "artifacts/{}",
        encode_url_path(path.as_str())
    )))
}

fn encode_url_path(path: &str) -> String {
    use std::fmt::Write as _;

    let mut encoded = String::with_capacity(path.len());
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~' | b'/') {
            encoded.push(char::from(byte));
        } else {
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

fn render_market(html: &mut String, report: &RunReportV1) {
    html.push_str("<section id=\"market\" class=\"section\" aria-labelledby=\"market-title\"><h2 id=\"market-title\">Market evidence</h2><p class=\"lede\">Receipts are provenance pointers, not evaluator scores. A verified file digest does not prove a customer claim.</p>");
    if report.market_evidence.receipts.is_empty() {
        html.push_str("<p class=\"empty\">No commercial receipts imported. Product gate remains unassessed.</p>");
    } else {
        html.push_str("<ul class=\"receipts\">");
        for receipt in &report.market_evidence.receipts {
            let _ = write!(
                html,
                "<li><span class=\"receipt-type\">{:?}</span><span>Event timestamp: {}</span><span>Digest status: {:?}</span><code>{}</code></li>",
                receipt.declaration.receipt_type,
                receipt.declaration.occurred_at_unix_ms,
                receipt.digest_status,
                escape(&receipt.receipt_path.display().to_string())
            );
        }
        html.push_str("</ul>");
    }
    html.push_str("</section>");
}

fn render_provenance(html: &mut String, report: &RunReportV1) {
    html.push_str("<section id=\"provenance\" class=\"section\" aria-labelledby=\"provenance-title\"><h2 id=\"provenance-title\">Provenance</h2><div class=\"detail-grid\"><section><h3>Frozen contract</h3><p class=\"mono\"><code>");
    html.push_str(&escape(&report.frozen_identity_sha256));
    html.push_str("</code></p></section><section><h3>Environment</h3><p>");
    html.push_str(&escape(&report.environment.status));
    html.push_str("</p><p class=\"mono\"><code>");
    html.push_str(
        &report
            .environment
            .fingerprint_sha256
            .as_deref()
            .map_or_else(|| "Unavailable".into(), escape),
    );
    html.push_str("</code></p></section><section><h3>Replay state</h3><p>");
    html.push_str(&escape(&report.recovery_action));
    html.push_str("</p></section></div></section>");
}

fn metric_value(
    html: &mut String,
    snapshot: Option<&autoresearch_core::EvaluationSnapshot>,
    name: &str,
) {
    let value = snapshot.and_then(|snapshot| {
        snapshot
            .measurements
            .iter()
            .find_map(|measurement| match measurement {
                Measurement::Numeric {
                    name: actual,
                    value,
                    ..
                } if actual == name => Some(value.get()),
                _ => None,
            })
    });
    if let Some(value) = value {
        let _ = write!(html, "<span class=\"measure\">{value}</span>");
    } else {
        html.push_str("Unavailable");
    }
}

fn decision_label(reason: &DecisionReason) -> String {
    match reason {
        DecisionReason::FailedHardGates { names } => {
            format!("Failed hard gates: {}.", names.join(", "))
        }
        DecisionReason::PrimaryImprovement => "Primary objective improved.".into(),
        DecisionReason::PrimaryRegression => "Primary objective regressed.".into(),
        DecisionReason::TieBreaker { field } => format!("Equal objective; tie-breaker: {field:?}."),
        DecisionReason::NoImprovement => "No measured improvement.".into(),
    }
}

fn short_commit(commit: &str) -> String {
    commit.chars().take(12).collect()
}

fn escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            value if value.is_control() && !matches!(value, '\n' | '\t') => escaped.push(' '),
            value => escaped.push(value),
        }
    }
    escaped
}
