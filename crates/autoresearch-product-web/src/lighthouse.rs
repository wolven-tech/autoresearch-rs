//! Strict, read-only import of local Lighthouse report artifacts.

use autoresearch_config::{LighthouseSettings, RepoPath};
use autoresearch_evaluator::EvaluationContext;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use thiserror::Error;

const MAX_REPORT_BYTES: u64 = 10 * 1024 * 1024;

/// Lighthouse import failure; never substituted with a pass or score.
#[derive(Debug, Error)]
pub enum LighthouseError {
    /// Frozen warm-up and measured count was not met.
    #[error("report count does not match frozen sampling policy")]
    ReportCount,
    /// Report path is not a run-owned regular file.
    #[error("unsafe or missing Lighthouse report artifact")]
    Artifact,
    /// Report exceeded bounded import size.
    #[error("Lighthouse report exceeds size limit")]
    ReportTooLarge,
    /// Report is not valid JSON envelope.
    #[error("invalid Lighthouse report JSON")]
    InvalidJson,
    /// Version changed between frozen policy and report.
    #[error("Lighthouse version differs from frozen policy")]
    VersionMismatch,
    /// Environment fingerprint changed.
    #[error("Lighthouse environment fingerprint differs from frozen policy")]
    FingerprintMismatch,
    /// Report targets an unexpected route.
    #[error("Lighthouse report URL differs from declared route")]
    RouteMismatch,
    /// Declared metric was absent or invalid.
    #[error("invalid or missing declared Lighthouse field `{0}`")]
    Field(String),
}

/// One frozen field with measured raw values and median objective input.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LighthouseField {
    /// Allowlisted field name.
    pub name: String,
    /// Measured values in capture order, excluding warm-up.
    pub samples: Vec<f64>,
    /// Median measured value.
    pub median: f64,
}

/// Validated lab evidence; no market or WCAG-conformance semantics.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LighthouseEvidence {
    /// Exact frozen Lighthouse version.
    pub version: String,
    /// Exact frozen environment fingerprint.
    pub environment_fingerprint: String,
    /// Route URL verified in every report.
    pub route_url: String,
    /// Number of checked warm-up reports excluded from metrics.
    pub warmup_samples: u8,
    /// Relative report artifacts in supplied order.
    pub report_paths: Vec<String>,
    /// Imported fields only, in manifest declaration order.
    pub fields: Vec<LighthouseField>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReportEnvelope {
    environment_fingerprint: String,
    report: Value,
}

/// Imports exact frozen warm-up and measured report sequence from run artifacts.
///
/// Category scores become 0–100; timing metrics remain milliseconds and CLS
/// remains unitless. INP is not imported: Lighthouse lab runs do not provide a
/// field-usage interaction measurement. Reports are never market evidence.
///
/// # Errors
///
/// Rejects changed version, fingerprint, route, count, missing/invalid field,
/// duplicate/unsafe path, symlink escape, and oversized artifact.
pub fn import_reports(
    context: &EvaluationContext,
    settings: &LighthouseSettings,
    route_url: &str,
    report_paths: &[String],
) -> Result<LighthouseEvidence, LighthouseError> {
    let expected_count =
        usize::from(settings.warmup_samples()) + usize::from(settings.measured_samples());
    if report_paths.len() != expected_count {
        return Err(LighthouseError::ReportCount);
    }
    let mut seen = HashSet::new();
    let mut samples = settings
        .fields()
        .iter()
        .map(|name| LighthouseField {
            name: name.clone(),
            samples: Vec::with_capacity(usize::from(settings.measured_samples())),
            median: 0.0,
        })
        .collect::<Vec<_>>();
    for (index, path) in report_paths.iter().enumerate() {
        if !seen.insert(path) {
            return Err(LighthouseError::Artifact);
        }
        let bytes = read_artifact(context.artifact_directory(), path)?;
        let envelope: ReportEnvelope =
            serde_json::from_slice(&bytes).map_err(|_| LighthouseError::InvalidJson)?;
        if envelope.environment_fingerprint != settings.environment_fingerprint() {
            return Err(LighthouseError::FingerprintMismatch);
        }
        let report = &envelope.report;
        if report.get("lighthouseVersion").and_then(Value::as_str) != Some(settings.version()) {
            return Err(LighthouseError::VersionMismatch);
        }
        if report.get("finalDisplayedUrl").and_then(Value::as_str) != Some(route_url) {
            return Err(LighthouseError::RouteMismatch);
        }
        for field in &mut samples {
            let value = read_field(report, &field.name)?;
            if index >= usize::from(settings.warmup_samples()) {
                field.samples.push(value);
            }
        }
    }
    for field in &mut samples {
        field.median = median(&field.samples);
    }
    Ok(LighthouseEvidence {
        version: settings.version().into(),
        environment_fingerprint: settings.environment_fingerprint().into(),
        route_url: route_url.into(),
        warmup_samples: settings.warmup_samples(),
        report_paths: report_paths.to_vec(),
        fields: samples,
    })
}

fn read_artifact(root: &Path, relative: &str) -> Result<Vec<u8>, LighthouseError> {
    let safe = RepoPath::new(relative).map_err(|_| LighthouseError::Artifact)?;
    let mut path = root.to_path_buf();
    for segment in safe.as_str().split('/') {
        path.push(segment);
        let metadata = fs::symlink_metadata(&path).map_err(|_| LighthouseError::Artifact)?;
        if metadata.file_type().is_symlink() {
            return Err(LighthouseError::Artifact);
        }
    }
    let canonical = fs::canonicalize(&path).map_err(|_| LighthouseError::Artifact)?;
    if !canonical.starts_with(root) || !canonical.is_file() {
        return Err(LighthouseError::Artifact);
    }
    let metadata = fs::metadata(&canonical).map_err(|_| LighthouseError::Artifact)?;
    if metadata.len() > MAX_REPORT_BYTES {
        return Err(LighthouseError::ReportTooLarge);
    }
    fs::read(canonical).map_err(|_| LighthouseError::Artifact)
}

fn read_field(report: &Value, name: &str) -> Result<f64, LighthouseError> {
    let pointer = match name {
        "performance" => "/categories/performance/score",
        "accessibility" => "/categories/accessibility/score",
        "best_practices" => "/categories/best-practices/score",
        "seo" => "/categories/seo/score",
        "fcp_ms" => "/audits/first-contentful-paint/numericValue",
        "lcp_ms" => "/audits/largest-contentful-paint/numericValue",
        "cls" => "/audits/cumulative-layout-shift/numericValue",
        "tbt_ms" => "/audits/total-blocking-time/numericValue",
        _ => return Err(LighthouseError::Field(name.into())),
    };
    let value = report
        .pointer(pointer)
        .and_then(Value::as_f64)
        .ok_or_else(|| LighthouseError::Field(name.into()))?;
    if !value.is_finite() || value < 0.0 {
        return Err(LighthouseError::Field(name.into()));
    }
    if matches!(
        name,
        "performance" | "accessibility" | "best_practices" | "seo"
    ) {
        if value > 1.0 {
            return Err(LighthouseError::Field(name.into()));
        }
        return Ok(value * 100.0);
    }
    Ok(value)
}

fn median(values: &[f64]) -> f64 {
    let mut ordered = values.to_vec();
    ordered.sort_by(f64::total_cmp);
    let middle = ordered.len() / 2;
    if ordered.len().is_multiple_of(2) {
        ordered[middle - 1] + (ordered[middle] - ordered[middle - 1]) / 2.0
    } else {
        ordered[middle]
    }
}
