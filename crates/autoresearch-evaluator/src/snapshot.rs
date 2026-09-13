//! Frozen-manifest output matching and comparable snapshot construction.

use crate::{EvaluationContext, ValidatedOutput};
use autoresearch_config::{Evaluator, MetricDefinition, ValidatedManifest};
use autoresearch_core::{Complexity, EvaluationSnapshot, Measurement};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Declared-output or artifact-provenance validation failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ValidationError {
    /// Frozen evaluator did not produce output.
    #[error("missing evaluator `{0}`")]
    MissingEvaluator(String),
    /// Result came from undeclared evaluator.
    #[error("undeclared evaluator `{0}`")]
    ExtraEvaluator(String),
    /// Evaluator returned more than one result envelope.
    #[error("duplicate evaluator `{0}`")]
    DuplicateEvaluator(String),
    /// Frozen measurement was missing.
    #[error("missing measurement `{0}`")]
    MissingMeasurement(String),
    /// Output name was not declared for evaluator.
    #[error("undeclared measurement `{0}`")]
    ExtraMeasurement(String),
    /// Hard-gate/numeric role differed from frozen definition.
    #[error("wrong measurement kind for `{0}`")]
    WrongKind(String),
    /// Numeric improvement direction differed from frozen definition.
    #[error("wrong metric direction for `{0}`")]
    WrongDirection(String),
    /// Worktree and artifacts do not share run-owned `.autoresearch` state.
    #[error("artifact root is not run-owned: {0}")]
    ArtifactRoot(String),
    /// Declared artifact does not exist.
    #[error("artifact missing: {}", .0.display())]
    ArtifactMissing(PathBuf),
    /// Artifact resolved through a symlink or outside run-owned root.
    #[error("artifact escapes run-owned directory: {}", .0.display())]
    ArtifactEscape(PathBuf),
    /// Artifact is not a regular file.
    #[error("artifact is not a regular file: {}", .0.display())]
    ArtifactNotFile(PathBuf),
    /// Filesystem inspection failed.
    #[error("cannot inspect artifact {}: {reason}", path.display())]
    ArtifactIo {
        /// Artifact path that failed inspection.
        path: PathBuf,
        /// Filesystem error text.
        reason: String,
    },
}

/// Builds a complete comparable snapshot from structurally validated outputs.
///
/// Every declared evaluator must appear once. Every gate and metric must match
/// frozen names, kind, and direction. Results are reordered into manifest
/// evaluator order, then each evaluator's declared gate/metric order.
///
/// # Errors
///
/// Rejects missing, extra, duplicate, or mismatched outputs and artifacts that
/// are absent, symlinked, or outside the run-owned artifact directory.
pub fn build_snapshot(
    context: &EvaluationContext,
    manifest: &ValidatedManifest,
    outputs: Vec<ValidatedOutput>,
    complexity: Complexity,
) -> Result<EvaluationSnapshot, ValidationError> {
    validate_artifact_root(context)?;
    let declared_ids = manifest
        .evaluators()
        .iter()
        .map(Evaluator::id)
        .collect::<HashSet<_>>();
    let mut by_id = BTreeMap::new();
    for output in outputs {
        let id = output.evaluator_id().to_owned();
        if !declared_ids.contains(id.as_str()) {
            return Err(ValidationError::ExtraEvaluator(id));
        }
        if by_id.insert(id.clone(), output).is_some() {
            return Err(ValidationError::DuplicateEvaluator(id));
        }
    }

    let mut ordered = Vec::new();
    for evaluator in manifest.evaluators() {
        let output = by_id
            .remove(evaluator.id())
            .ok_or_else(|| ValidationError::MissingEvaluator(evaluator.id().to_owned()))?;
        let names = evaluator
            .hard_gates()
            .iter()
            .map(String::as_str)
            .chain(evaluator.metrics().iter().map(MetricDefinition::name))
            .collect::<HashSet<_>>();
        let measurements = output
            .measurements()
            .iter()
            .map(|measurement| (measurement.name(), measurement))
            .collect::<BTreeMap<_, _>>();
        if let Some(name) = measurements.keys().find(|name| !names.contains(**name)) {
            return Err(ValidationError::ExtraMeasurement((*name).to_owned()));
        }

        for gate in evaluator.hard_gates() {
            let measurement = measurements
                .get(gate.as_str())
                .ok_or_else(|| ValidationError::MissingMeasurement(gate.clone()))?;
            if !matches!(measurement, Measurement::HardGate { .. }) {
                return Err(ValidationError::WrongKind(gate.clone()));
            }
            ordered.push((*measurement).clone());
        }
        for metric in evaluator.metrics() {
            let measurement = measurements
                .get(metric.name())
                .ok_or_else(|| ValidationError::MissingMeasurement(metric.name().to_owned()))?;
            match measurement {
                Measurement::Numeric {
                    metric_kind,
                    direction,
                    ..
                } if *metric_kind == metric.kind() && *direction == metric.direction() => {
                    ordered.push((*measurement).clone());
                }
                Measurement::Numeric { metric_kind, .. } if *metric_kind == metric.kind() => {
                    return Err(ValidationError::WrongDirection(metric.name().to_owned()));
                }
                _ => return Err(ValidationError::WrongKind(metric.name().to_owned())),
            }
        }

        for artifact in output.artifacts() {
            validate_artifact(context.artifact_directory(), &artifact.relative_path)?;
        }
    }

    Ok(EvaluationSnapshot {
        measurements: ordered,
        complexity,
    })
}

fn validate_artifact_root(context: &EvaluationContext) -> Result<(), ValidationError> {
    let candidate = context.candidate_worktree();
    let Some(run_worktrees) = candidate.parent() else {
        return Err(ValidationError::ArtifactRoot(
            "worktree lacks run parent".into(),
        ));
    };
    let Some(worktrees) = run_worktrees.parent() else {
        return Err(ValidationError::ArtifactRoot(
            "worktree lacks worktrees parent".into(),
        ));
    };
    let Some(state) = worktrees.parent() else {
        return Err(ValidationError::ArtifactRoot(
            "worktree lacks state parent".into(),
        ));
    };
    if run_worktrees.file_name().and_then(|value| value.to_str()) != Some(context.run_id().as_str())
        || worktrees.file_name().and_then(|value| value.to_str()) != Some("worktrees")
        || state.file_name().and_then(|value| value.to_str()) != Some(".autoresearch")
    {
        return Err(ValidationError::ArtifactRoot(
            "worktree is not under .autoresearch/worktrees/<run-id>".into(),
        ));
    }
    let expected = state
        .join("runs")
        .join(context.run_id().as_str())
        .join("artifacts");
    if !context.artifact_directory().starts_with(&expected) {
        return Err(ValidationError::ArtifactRoot(format!(
            "expected artifact directory under {}",
            expected.display()
        )));
    }
    Ok(())
}

fn validate_artifact(root: &Path, relative_path: &str) -> Result<(), ValidationError> {
    let mut current = root.to_path_buf();
    for segment in relative_path.split('/') {
        current.push(segment);
        let metadata = match fs::symlink_metadata(&current) {
            Ok(metadata) => metadata,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                return Err(ValidationError::ArtifactMissing(current));
            }
            Err(source) => {
                return Err(ValidationError::ArtifactIo {
                    path: current,
                    reason: source.to_string(),
                });
            }
        };
        if metadata.file_type().is_symlink() {
            return Err(ValidationError::ArtifactEscape(current));
        }
    }
    let canonical = fs::canonicalize(&current).map_err(|source| ValidationError::ArtifactIo {
        path: current.clone(),
        reason: source.to_string(),
    })?;
    if !canonical.starts_with(root) {
        return Err(ValidationError::ArtifactEscape(current));
    }
    let metadata = fs::metadata(&canonical).map_err(|source| ValidationError::ArtifactIo {
        path: current.clone(),
        reason: source.to_string(),
    })?;
    if !metadata.is_file() {
        return Err(ValidationError::ArtifactNotFile(current));
    }
    Ok(())
}
