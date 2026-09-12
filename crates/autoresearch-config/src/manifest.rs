//! TOML manifest parsing, normalization, and cross-field validation.

use autoresearch_core::{
    MetricDirection, MutationBoundary, MutationBoundaryError, NumericMetricKind, RepoPath,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashSet};
use thiserror::Error;

const SCHEMA_VERSION: u32 = 1;
const CONTROL_PATHS: [&str; 2] = ["autoresearch.toml", "program.md"];

/// Manifest parsing or validation failure.
#[derive(Debug, Error)]
pub enum ManifestError {
    /// TOML could not be parsed into schema.
    #[error("invalid TOML manifest: {0}")]
    Parse(#[from] toml::de::Error),
    /// Only known schema version can execute.
    #[error("unsupported schema version {actual}; expected {expected}")]
    UnsupportedSchema {
        /// Version accepted by this binary.
        expected: u32,
        /// Version declared by manifest.
        actual: u32,
    },
    /// Required string contains no visible characters.
    #[error("{field} cannot be blank")]
    Blank {
        /// Logical field path.
        field: String,
    },
    /// Bounded execution values must be non-zero.
    #[error("{field} must be greater than zero")]
    Zero {
        /// Logical field path.
        field: String,
    },
    /// Repository-relative path can escape or serialize ambiguously.
    #[error("unsafe repository path `{path}`: {reason}")]
    UnsafePath {
        /// Rejected path.
        path: String,
        /// Human-readable rejection reason.
        reason: &'static str,
    },
    /// Mutable root falls inside protected control surface.
    #[error("mutable path `{mutable}` is inside protected path `{protected}`")]
    MutableProtected {
        /// Rejected mutable root.
        mutable: String,
        /// Protecting root.
        protected: String,
    },
    /// Identifier appears more than once in namespace.
    #[error("duplicate {kind} `{name}`")]
    Duplicate {
        /// Identifier class.
        kind: &'static str,
        /// Duplicated value.
        name: String,
    },
    /// Frozen objective has no matching evaluator output.
    #[error("objective `{0}` is not declared by any evaluator")]
    MissingObjective(String),
    /// Only configured primary may use objective role.
    #[error("unexpected objective metric `{0}`; only primary objective may drive selection")]
    ExtraObjective(String),
    /// Objective output direction must match experiment definition.
    #[error("objective `{name}` direction does not match experiment objective")]
    ObjectiveDirectionMismatch {
        /// Frozen objective name.
        name: String,
    },
}

/// Validated, normalized manifest accepted by runner components.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ValidatedManifest {
    schema_version: u32,
    experiment: Experiment,
    scope: MutationBoundary,
    agent: CommandSpec,
    evaluators: Vec<Evaluator>,
    authority: AuthorityCeiling,
}

impl ValidatedManifest {
    /// Parses TOML and validates all safety and comparison invariants.
    ///
    /// # Errors
    ///
    /// Returns [`ManifestError`] for malformed TOML or invalid cross-field data.
    pub fn parse(source: &str) -> Result<Self, ManifestError> {
        toml::from_str::<RawManifest>(source)?.validate()
    }

    /// Returns supported schema version.
    #[must_use]
    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    /// Returns experiment policy.
    #[must_use]
    pub const fn experiment(&self) -> &Experiment {
        &self.experiment
    }

    /// Returns normalized mutation scope.
    #[must_use]
    pub const fn scope(&self) -> &MutationBoundary {
        &self.scope
    }

    /// Returns mutation-agent command.
    #[must_use]
    pub const fn agent(&self) -> &CommandSpec {
        &self.agent
    }

    /// Returns evaluators in execution order.
    #[must_use]
    pub fn evaluators(&self) -> &[Evaluator] {
        &self.evaluators
    }

    /// Returns maximum declared external authority.
    #[must_use]
    pub const fn authority(&self) -> &AuthorityCeiling {
        &self.authority
    }
}

/// Named bounded experiment and frozen objective.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Experiment {
    name: String,
    objective: Objective,
    budget: Budget,
}

impl Experiment {
    /// Returns display name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns primary objective.
    #[must_use]
    pub const fn objective(&self) -> &Objective {
        &self.objective
    }

    /// Returns hard run bounds.
    #[must_use]
    pub const fn budget(&self) -> Budget {
        self.budget
    }
}

/// Primary metric definition frozen at baseline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Objective {
    name: String,
    direction: MetricDirection,
}

impl Objective {
    /// Returns globally unique metric name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns improvement direction.
    #[must_use]
    pub const fn direction(&self) -> MetricDirection {
        self.direction
    }
}

/// Explicit bounds preventing unbounded experimentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Budget {
    /// Maximum candidates evaluated in run.
    pub max_candidates: u32,
    /// Maximum evaluator failures tolerated before stop.
    pub max_failures: u32,
    /// Maximum wall-clock duration in seconds.
    pub wall_clock_seconds: u64,
}

/// Executable plus literal arguments and timeout; no shell interpolation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommandSpec {
    program: String,
    args: Vec<String>,
    timeout_seconds: u64,
}

impl CommandSpec {
    /// Returns executable name or path.
    #[must_use]
    pub fn program(&self) -> &str {
        &self.program
    }

    /// Returns literal process arguments.
    #[must_use]
    pub fn args(&self) -> &[String] {
        &self.args
    }

    /// Returns process timeout in seconds.
    #[must_use]
    pub const fn timeout_seconds(&self) -> u64 {
        self.timeout_seconds
    }
}

/// One evaluator command and declared output namespace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Evaluator {
    id: String,
    command: CommandSpec,
    hard_gates: Vec<String>,
    metrics: Vec<MetricDefinition>,
}

impl Evaluator {
    /// Returns evaluator identifier.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Returns process command.
    #[must_use]
    pub const fn command(&self) -> &CommandSpec {
        &self.command
    }

    /// Returns hard-gate names emitted by evaluator.
    #[must_use]
    pub fn hard_gates(&self) -> &[String] {
        &self.hard_gates
    }

    /// Returns numeric output definitions.
    #[must_use]
    pub fn metrics(&self) -> &[MetricDefinition] {
        &self.metrics
    }
}

/// Declared numeric evaluator output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MetricDefinition {
    name: String,
    kind: NumericMetricKind,
    direction: MetricDirection,
}

impl MetricDefinition {
    /// Returns globally unique metric name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns selection/reporting role.
    #[must_use]
    pub const fn kind(&self) -> NumericMetricKind {
        self.kind
    }

    /// Returns improvement direction.
    #[must_use]
    pub const fn direction(&self) -> MetricDirection {
        self.direction
    }
}

/// Maximum external capabilities a run may request.
///
/// Runner must still receive explicit per-run authorization. This declaration
/// alone never grants authority.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AuthorityCeiling {
    allow: BTreeSet<ExternalCapability>,
}

impl AuthorityCeiling {
    /// Returns declared capabilities in stable order.
    #[must_use]
    pub const fn allowed(&self) -> &BTreeSet<ExternalCapability> {
        &self.allow
    }

    /// Returns whether manifest permits a per-run request for capability.
    #[must_use]
    pub fn permits(&self, capability: ExternalCapability) -> bool {
        self.allow.contains(&capability)
    }
}

/// External side effect requiring manifest declaration and per-run approval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalCapability {
    /// Network access from agent or evaluator.
    Network,
    /// Deployment to hosting provider.
    Deployment,
    /// Human or automated outreach.
    Outreach,
    /// Purchase or domain registration.
    Purchase,
    /// Payment-system mutation.
    Payment,
    /// Write against production data or service.
    ProductionWrite,
    /// Permission or visibility change.
    PermissionChange,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawManifest {
    schema_version: u32,
    experiment: RawExperiment,
    scope: RawScope,
    agent: RawCommand,
    #[serde(default)]
    evaluators: Vec<RawEvaluator>,
    #[serde(default)]
    authority: AuthorityCeiling,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawExperiment {
    name: String,
    objective: RawObjective,
    budget: Budget,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawObjective {
    name: String,
    direction: MetricDirection,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawScope {
    mutable_paths: Vec<String>,
    #[serde(default)]
    protected_paths: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCommand {
    program: String,
    #[serde(default)]
    args: Vec<String>,
    timeout_seconds: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEvaluator {
    id: String,
    command: RawCommand,
    #[serde(default)]
    hard_gates: Vec<String>,
    #[serde(default)]
    metrics: Vec<RawMetricDefinition>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMetricDefinition {
    name: String,
    kind: NumericMetricKind,
    direction: MetricDirection,
}

impl<'de> Deserialize<'de> for Budget {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct RawBudget {
            max_candidates: u32,
            max_failures: u32,
            wall_clock_seconds: u64,
        }

        let raw = RawBudget::deserialize(deserializer)?;
        Ok(Self {
            max_candidates: raw.max_candidates,
            max_failures: raw.max_failures,
            wall_clock_seconds: raw.wall_clock_seconds,
        })
    }
}

impl RawManifest {
    fn validate(self) -> Result<ValidatedManifest, ManifestError> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(ManifestError::UnsupportedSchema {
                expected: SCHEMA_VERSION,
                actual: self.schema_version,
            });
        }

        let name = nonblank(self.experiment.name, "experiment.name")?;
        let objective_name = nonblank(self.experiment.objective.name, "experiment.objective.name")?;
        nonzero(
            self.experiment.budget.max_candidates,
            "experiment.budget.max_candidates",
        )?;
        nonzero(
            self.experiment.budget.wall_clock_seconds,
            "experiment.budget.wall_clock_seconds",
        )?;

        let scope = validate_scope(self.scope)?;
        let agent = validate_command(self.agent, "agent")?;
        let objective = Objective {
            name: objective_name.clone(),
            direction: self.experiment.objective.direction,
        };
        let evaluators = validate_evaluators(self.evaluators, &objective)?;

        Ok(ValidatedManifest {
            schema_version: self.schema_version,
            experiment: Experiment {
                name,
                objective,
                budget: self.experiment.budget,
            },
            scope,
            agent,
            evaluators,
            authority: self.authority,
        })
    }
}

fn validate_scope(raw: RawScope) -> Result<MutationBoundary, ManifestError> {
    if raw.mutable_paths.is_empty() {
        return Err(ManifestError::Blank {
            field: "scope.mutable_paths".into(),
        });
    }
    let mut mutable_paths = raw
        .mutable_paths
        .into_iter()
        .map(parse_repo_path)
        .collect::<Result<Vec<_>, _>>()?;
    mutable_paths.sort();
    mutable_paths.dedup();

    let mut protected_paths = raw
        .protected_paths
        .into_iter()
        .chain(CONTROL_PATHS.map(str::to_owned))
        .map(parse_repo_path)
        .collect::<Result<Vec<_>, _>>()?;
    protected_paths.sort();
    protected_paths.dedup();

    MutationBoundary::new(mutable_paths, protected_paths).map_err(|error| match error {
        MutationBoundaryError::EmptyMutablePaths => ManifestError::Blank {
            field: "scope.mutable_paths".into(),
        },
        MutationBoundaryError::MutableProtected { mutable, protected } => {
            ManifestError::MutableProtected { mutable, protected }
        }
    })
}

fn parse_repo_path(path: String) -> Result<RepoPath, ManifestError> {
    RepoPath::new(path).map_err(|error| ManifestError::UnsafePath {
        path: error.path().to_owned(),
        reason: error.reason(),
    })
}

fn validate_command(raw: RawCommand, field: &str) -> Result<CommandSpec, ManifestError> {
    let program = nonblank(raw.program, &format!("{field}.program"))?;
    nonzero(raw.timeout_seconds, &format!("{field}.timeout_seconds"))?;
    Ok(CommandSpec {
        program,
        args: raw.args,
        timeout_seconds: raw.timeout_seconds,
    })
}

fn validate_evaluators(
    raw_evaluators: Vec<RawEvaluator>,
    objective: &Objective,
) -> Result<Vec<Evaluator>, ManifestError> {
    let mut evaluator_ids = HashSet::with_capacity(raw_evaluators.len());
    let mut measurement_names = HashSet::new();
    let mut objective_found = false;
    let mut evaluators = Vec::with_capacity(raw_evaluators.len());

    for (index, raw) in raw_evaluators.into_iter().enumerate() {
        let id = nonblank(raw.id, &format!("evaluators[{index}].id"))?;
        insert_unique(&mut evaluator_ids, &id, "evaluator")?;
        let command = validate_command(raw.command, &format!("evaluators[{index}].command"))?;

        let mut hard_gates = Vec::with_capacity(raw.hard_gates.len());
        for (gate_index, gate) in raw.hard_gates.into_iter().enumerate() {
            let gate = nonblank(
                gate,
                &format!("evaluators[{index}].hard_gates[{gate_index}]"),
            )?;
            insert_unique(&mut measurement_names, &gate, "measurement")?;
            hard_gates.push(gate);
        }

        let mut metrics = Vec::with_capacity(raw.metrics.len());
        for (metric_index, raw_metric) in raw.metrics.into_iter().enumerate() {
            let name = nonblank(
                raw_metric.name,
                &format!("evaluators[{index}].metrics[{metric_index}].name"),
            )?;
            insert_unique(&mut measurement_names, &name, "measurement")?;
            if raw_metric.kind == NumericMetricKind::Objective {
                if name != objective.name {
                    return Err(ManifestError::ExtraObjective(name));
                }
                if raw_metric.direction != objective.direction {
                    return Err(ManifestError::ObjectiveDirectionMismatch { name });
                }
                objective_found = true;
            }
            metrics.push(MetricDefinition {
                name,
                kind: raw_metric.kind,
                direction: raw_metric.direction,
            });
        }

        evaluators.push(Evaluator {
            id,
            command,
            hard_gates,
            metrics,
        });
    }

    if !objective_found {
        return Err(ManifestError::MissingObjective(objective.name.clone()));
    }
    Ok(evaluators)
}

fn nonblank(value: String, field: &str) -> Result<String, ManifestError> {
    if value.trim().is_empty() {
        Err(ManifestError::Blank {
            field: field.to_owned(),
        })
    } else {
        Ok(value)
    }
}

fn nonzero(value: impl Into<u64>, field: &str) -> Result<(), ManifestError> {
    if value.into() == 0 {
        Err(ManifestError::Zero {
            field: field.to_owned(),
        })
    } else {
        Ok(())
    }
}

fn insert_unique(
    values: &mut HashSet<String>,
    value: &str,
    kind: &'static str,
) -> Result<(), ManifestError> {
    if values.insert(value.to_owned()) {
        Ok(())
    } else {
        Err(ManifestError::Duplicate {
            kind,
            name: value.to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = r#"
schema_version = 1

[experiment]
name = "landing page accessibility"

[experiment.objective]
name = "accessibility_score"
direction = "maximize"

[experiment.budget]
max_candidates = 8
max_failures = 2
wall_clock_seconds = 1800

[scope]
mutable_paths = ["web/src", "web/public"]
protected_paths = ["docs/BET.md"]

[agent]
program = "codex"
args = ["exec", "--full-auto"]
timeout_seconds = 600

[[evaluators]]
id = "web"
hard_gates = ["tests", "no_overflow"]

[evaluators.command]
program = "autoresearch-web-evaluator"
args = ["--jsonl"]
timeout_seconds = 300

[[evaluators.metrics]]
name = "accessibility_score"
kind = "objective"
direction = "maximize"

[[evaluators.metrics]]
name = "qualified_receipts"
kind = "market_evidence"
direction = "maximize"
"#;

    #[test]
    fn manifest_parses_and_adds_control_paths() {
        let manifest = ValidatedManifest::parse(VALID).expect("valid manifest");
        assert_eq!(manifest.schema_version(), 1);
        assert_eq!(manifest.experiment().budget().max_candidates, 8);
        assert_eq!(manifest.evaluators().len(), 1);
        assert_eq!(
            manifest
                .scope()
                .protected_paths()
                .iter()
                .map(RepoPath::as_str)
                .collect::<Vec<_>>(),
            ["autoresearch.toml", "docs/BET.md", "program.md"]
        );
    }

    #[test]
    fn manifest_rejects_unknown_keys() {
        let source = VALID.replace("schema_version = 1", "schema_version = 1\nsurprise = true");
        assert!(matches!(
            ValidatedManifest::parse(&source),
            Err(ManifestError::Parse(_))
        ));
    }

    #[test]
    fn manifest_rejects_unsafe_paths_and_control_mutation() {
        for source in [
            VALID.replace("web/src", "../outside"),
            VALID.replace("web/src", "/tmp/outside"),
            VALID.replace("web/src", "program.md"),
        ] {
            assert!(ValidatedManifest::parse(&source).is_err());
        }
    }

    #[test]
    fn manifest_rejects_duplicate_names() {
        let source = VALID.replace(
            "hard_gates = [\"tests\", \"no_overflow\"]",
            "hard_gates = [\"tests\", \"tests\"]",
        );
        assert!(matches!(
            ValidatedManifest::parse(&source),
            Err(ManifestError::Duplicate {
                kind: "measurement",
                ..
            })
        ));
    }

    #[test]
    fn manifest_requires_matching_primary_objective() {
        let missing = VALID.replace("kind = \"objective\"", "kind = \"diagnostic\"");
        assert!(matches!(
            ValidatedManifest::parse(&missing),
            Err(ManifestError::MissingObjective(_))
        ));

        let mismatch = VALID.replacen("direction = \"maximize\"", "direction = \"minimize\"", 1);
        assert!(matches!(
            ValidatedManifest::parse(&mismatch),
            Err(ManifestError::ObjectiveDirectionMismatch { .. })
        ));

        let extra = VALID.replace(
            "name = \"qualified_receipts\"\nkind = \"market_evidence\"",
            "name = \"other_score\"\nkind = \"objective\"",
        );
        assert!(matches!(
            ValidatedManifest::parse(&extra),
            Err(ManifestError::ExtraObjective(_))
        ));
    }

    #[test]
    fn manifest_requires_bounded_commands_and_run() {
        for source in [
            VALID.replace("max_candidates = 8", "max_candidates = 0"),
            VALID.replace("program = \"codex\"", "program = \" \""),
            VALID.replacen("timeout_seconds = 600", "timeout_seconds = 0", 1),
        ] {
            assert!(ValidatedManifest::parse(&source).is_err());
        }
    }
}
