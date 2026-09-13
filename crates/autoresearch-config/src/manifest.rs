//! TOML manifest parsing, normalization, and cross-field validation.

use autoresearch_core::{
    MetricDirection, MutationBoundary, MutationBoundaryError, NumericMetricKind, RepoPath,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use thiserror::Error;
use url::Url;

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
    /// Product-web target is not a safe local fixture origin or route.
    #[error("invalid web target {field}: {reason}")]
    InvalidWebTarget {
        /// Field being rejected.
        field: String,
        /// Rejection reason.
        reason: &'static str,
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
    web: Option<WebTargets>,
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

    /// Returns optional frozen local product-web targets.
    #[must_use]
    pub const fn web(&self) -> Option<&WebTargets> {
        self.web.as_ref()
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

/// Local route and capture settings included in frozen manifest identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WebTargets {
    origin: String,
    routes: Vec<WebRoute>,
    viewports: Vec<u32>,
    reduced_motion: bool,
    thresholds: WebThresholds,
    lighthouse: Option<LighthouseSettings>,
    seo: Option<SeoSettings>,
    geo: Option<GeoSettings>,
}

impl WebTargets {
    /// Returns loopback HTTP origin without trailing route.
    #[must_use]
    pub fn origin(&self) -> &str {
        &self.origin
    }

    /// Returns stable route definitions.
    #[must_use]
    pub fn routes(&self) -> &[WebRoute] {
        &self.routes
    }

    /// Returns frozen CSS viewport widths.
    #[must_use]
    pub fn viewports(&self) -> &[u32] {
        &self.viewports
    }

    /// Returns whether reduced-motion media emulation is required.
    #[must_use]
    pub const fn reduced_motion(&self) -> bool {
        self.reduced_motion
    }

    /// Returns optional lab-only thresholds.
    #[must_use]
    pub const fn thresholds(&self) -> &WebThresholds {
        &self.thresholds
    }

    /// Returns optional frozen Lighthouse import policy.
    #[must_use]
    pub const fn lighthouse(&self) -> Option<&LighthouseSettings> {
        self.lighthouse.as_ref()
    }

    /// Returns optional frozen technical SEO policy.
    #[must_use]
    pub const fn seo(&self) -> Option<&SeoSettings> {
        self.seo.as_ref()
    }

    /// Returns optional frozen GEO diagnostic policy.
    #[must_use]
    pub const fn geo(&self) -> Option<&GeoSettings> {
        self.geo.as_ref()
    }
}

/// Canonical entity, exact product facts, and passage bound for local GEO checks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GeoSettings {
    entity_name: String,
    facts: BTreeMap<String, String>,
    max_passages: u8,
}

impl GeoSettings {
    /// Returns canonical visible entity name.
    #[must_use]
    pub fn entity_name(&self) -> &str {
        &self.entity_name
    }

    /// Returns exact human-readable fact strings keyed by stable identifiers.
    #[must_use]
    pub fn facts(&self) -> &BTreeMap<String, String> {
        &self.facts
    }

    /// Returns maximum inspected passage count.
    #[must_use]
    pub const fn max_passages(&self) -> u8 {
        self.max_passages
    }
}

/// Frozen local sitemap/robots paths and redirect bound.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SeoSettings {
    sitemap_path: String,
    robots_path: String,
    max_redirects: u8,
}

impl SeoSettings {
    /// Returns declared sitemap path.
    #[must_use]
    pub fn sitemap_path(&self) -> &str {
        &self.sitemap_path
    }

    /// Returns declared robots path.
    #[must_use]
    pub fn robots_path(&self) -> &str {
        &self.robots_path
    }

    /// Returns maximum followed redirect hops.
    #[must_use]
    pub const fn max_redirects(&self) -> u8 {
        self.max_redirects
    }
}

/// Frozen Lighthouse version, environment, fields, and sample counts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LighthouseSettings {
    version: String,
    environment_fingerprint: String,
    warmup_samples: u8,
    measured_samples: u8,
    fields: Vec<String>,
}

impl LighthouseSettings {
    /// Returns exact Lighthouse version required by imported reports.
    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Returns expected environment fingerprint.
    #[must_use]
    pub fn environment_fingerprint(&self) -> &str {
        &self.environment_fingerprint
    }

    /// Returns number of ignored warm-up reports.
    #[must_use]
    pub const fn warmup_samples(&self) -> u8 {
        self.warmup_samples
    }

    /// Returns number of measured reports aggregated by median.
    #[must_use]
    pub const fn measured_samples(&self) -> u8 {
        self.measured_samples
    }

    /// Returns allowlisted metric names in manifest order.
    #[must_use]
    pub fn fields(&self) -> &[String] {
        &self.fields
    }
}

/// One named route in local product-web fixture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WebRoute {
    name: String,
    path: String,
    expected_status: u16,
    indexable: bool,
}

impl WebRoute {
    /// Returns stable route name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns origin-relative route path.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Returns expected HTTP status for route checks.
    #[must_use]
    pub const fn expected_status(&self) -> u16 {
        self.expected_status
    }

    /// Returns whether route is expected to be indexable.
    #[must_use]
    pub const fn indexable(&self) -> bool {
        self.indexable
    }
}

/// Optional lab thresholds; no commercial or product-gate semantics.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WebThresholds {
    /// Maximum Lighthouse LCP in milliseconds.
    pub max_lcp_ms: Option<u32>,
    /// Maximum Lighthouse CLS, multiplied by 1000.
    pub max_cls_milli: Option<u16>,
    /// Minimum Lighthouse accessibility score, 0–100.
    pub min_accessibility_score: Option<u8>,
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
    web: Option<RawWebTargets>,
    #[serde(default)]
    authority: AuthorityCeiling,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawWebTargets {
    origin: String,
    routes: Vec<RawWebRoute>,
    viewports: Vec<u32>,
    #[serde(default = "default_reduced_motion")]
    reduced_motion: bool,
    #[serde(default)]
    thresholds: WebThresholds,
    #[serde(default)]
    lighthouse: Option<RawLighthouseSettings>,
    #[serde(default)]
    seo: Option<RawSeoSettings>,
    #[serde(default)]
    geo: Option<RawGeoSettings>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawGeoSettings {
    entity_name: String,
    facts: BTreeMap<String, String>,
    max_passages: u8,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSeoSettings {
    sitemap_path: String,
    robots_path: String,
    max_redirects: u8,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLighthouseSettings {
    version: String,
    environment_fingerprint: String,
    warmup_samples: u8,
    measured_samples: u8,
    fields: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawWebRoute {
    name: String,
    path: String,
    expected_status: u16,
    #[serde(default)]
    indexable: Option<bool>,
}

const fn default_reduced_motion() -> bool {
    true
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
        let web = self.web.map(validate_web_targets).transpose()?;

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
            web,
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

fn validate_web_targets(raw: RawWebTargets) -> Result<WebTargets, ManifestError> {
    let origin = Url::parse(&raw.origin).map_err(|_| ManifestError::InvalidWebTarget {
        field: "web.origin".into(),
        reason: "must be a loopback HTTP origin",
    })?;
    let loopback = origin.host_str().is_some_and(|host| {
        host == "localhost"
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    });
    if origin.scheme() != "http"
        || !loopback
        || !origin.username().is_empty()
        || origin.password().is_some()
        || origin.path() != "/"
        || origin.query().is_some()
        || origin.fragment().is_some()
    {
        return Err(ManifestError::InvalidWebTarget {
            field: "web.origin".into(),
            reason: "must be a loopback HTTP origin without credentials, path, query, or fragment",
        });
    }
    if raw.viewports != [320, 390, 768, 1280] {
        return Err(ManifestError::InvalidWebTarget {
            field: "web.viewports".into(),
            reason: "must be exactly [320, 390, 768, 1280] in ascending order",
        });
    }
    let routes = validate_web_routes(raw.routes)?;
    if raw.thresholds.max_lcp_ms == Some(0)
        || raw
            .thresholds
            .max_cls_milli
            .is_some_and(|value| value > 1000)
        || raw
            .thresholds
            .min_accessibility_score
            .is_some_and(|value| value > 100)
    {
        return Err(ManifestError::InvalidWebTarget {
            field: "web.thresholds".into(),
            reason: "threshold is outside supported range",
        });
    }
    let lighthouse = raw
        .lighthouse
        .map(validate_lighthouse_settings)
        .transpose()?;
    let seo = raw.seo.map(validate_seo_settings).transpose()?;
    let geo = raw.geo.map(validate_geo_settings).transpose()?;
    Ok(WebTargets {
        origin: origin.origin().ascii_serialization(),
        routes,
        viewports: raw.viewports,
        reduced_motion: raw.reduced_motion,
        thresholds: raw.thresholds,
        lighthouse,
        seo,
        geo,
    })
}

fn validate_geo_settings(raw: RawGeoSettings) -> Result<GeoSettings, ManifestError> {
    let entity_name = nonblank(raw.entity_name, "web.geo.entity_name")?;
    if entity_name.len() > 120
        || raw.facts.is_empty()
        || raw.facts.len() > 16
        || !(1..=32).contains(&raw.max_passages)
        || raw.facts.iter().any(|(key, value)| {
            key.is_empty()
                || key.len() > 40
                || !key
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
                || value.trim().is_empty()
                || value.len() > 200
        })
    {
        return Err(ManifestError::InvalidWebTarget {
            field: "web.geo".into(),
            reason: "requires bounded entity, 1–16 keyed facts, and 1–32 passages",
        });
    }
    Ok(GeoSettings {
        entity_name,
        facts: raw.facts,
        max_passages: raw.max_passages,
    })
}

fn validate_web_routes(raw_routes: Vec<RawWebRoute>) -> Result<Vec<WebRoute>, ManifestError> {
    if raw_routes.is_empty() {
        return Err(ManifestError::InvalidWebTarget {
            field: "web.routes".into(),
            reason: "at least one route is required",
        });
    }
    let mut names = HashSet::new();
    let mut paths = HashSet::new();
    let mut routes = Vec::with_capacity(raw_routes.len());
    for (index, route) in raw_routes.into_iter().enumerate() {
        let name = nonblank(route.name, &format!("web.routes[{index}].name"))?;
        if !names.insert(name.clone()) {
            return Err(ManifestError::Duplicate {
                kind: "web route name",
                name,
            });
        }
        if !valid_web_route_path(&route.path) {
            return Err(ManifestError::InvalidWebTarget {
                field: format!("web.routes[{index}].path"),
                reason: "must be a plain origin-relative path without escapes or query",
            });
        }
        if !paths.insert(route.path.clone()) {
            return Err(ManifestError::Duplicate {
                kind: "web route path",
                name: route.path,
            });
        }
        if !(100..=599).contains(&route.expected_status) {
            return Err(ManifestError::InvalidWebTarget {
                field: format!("web.routes[{index}].expected_status"),
                reason: "must be an HTTP status from 100 through 599",
            });
        }
        routes.push(WebRoute {
            name,
            path: route.path,
            expected_status: route.expected_status,
            indexable: route.indexable.unwrap_or(route.expected_status == 200),
        });
    }
    routes.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(routes)
}

fn validate_seo_settings(raw: RawSeoSettings) -> Result<SeoSettings, ManifestError> {
    if !valid_web_route_path(&raw.sitemap_path)
        || !valid_web_route_path(&raw.robots_path)
        || raw.sitemap_path == raw.robots_path
        || raw.max_redirects > 5
    {
        return Err(ManifestError::InvalidWebTarget {
            field: "web.seo".into(),
            reason: "requires distinct safe paths and at most five redirects",
        });
    }
    Ok(SeoSettings {
        sitemap_path: raw.sitemap_path,
        robots_path: raw.robots_path,
        max_redirects: raw.max_redirects,
    })
}

fn validate_lighthouse_settings(
    raw: RawLighthouseSettings,
) -> Result<LighthouseSettings, ManifestError> {
    let version = nonblank(raw.version, "web.lighthouse.version")?;
    let fingerprint = raw.environment_fingerprint;
    if fingerprint.len() != 64
        || !fingerprint
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(ManifestError::InvalidWebTarget {
            field: "web.lighthouse.environment_fingerprint".into(),
            reason: "must be 64 lowercase hexadecimal characters",
        });
    }
    if raw.warmup_samples > 10 || !(1..=10).contains(&raw.measured_samples) {
        return Err(ManifestError::InvalidWebTarget {
            field: "web.lighthouse.samples".into(),
            reason: "warm-up must be 0..=10 and measured count 1..=10",
        });
    }
    if raw.fields.is_empty() || raw.fields.len() > 8 {
        return Err(ManifestError::InvalidWebTarget {
            field: "web.lighthouse.fields".into(),
            reason: "must declare one to eight supported fields",
        });
    }
    let mut seen = HashSet::new();
    for field in &raw.fields {
        if !matches!(
            field.as_str(),
            "performance"
                | "accessibility"
                | "best_practices"
                | "seo"
                | "fcp_ms"
                | "lcp_ms"
                | "cls"
                | "tbt_ms"
        ) {
            return Err(ManifestError::InvalidWebTarget {
                field: "web.lighthouse.fields".into(),
                reason: "field is not on Lighthouse import allowlist",
            });
        }
        if !seen.insert(field) {
            return Err(ManifestError::Duplicate {
                kind: "Lighthouse field",
                name: field.clone(),
            });
        }
    }
    Ok(LighthouseSettings {
        version,
        environment_fingerprint: fingerprint,
        warmup_samples: raw.warmup_samples,
        measured_samples: raw.measured_samples,
        fields: raw.fields,
    })
}

fn valid_web_route_path(path: &str) -> bool {
    path.starts_with('/')
        && !path.starts_with("//")
        && !path.contains("//")
        && path
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '-' | '_' | '.'))
        && path
            .split('/')
            .all(|segment| segment != "." && segment != "..")
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
    use crate::FrozenIdentity;
    use std::collections::BTreeMap;

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

    const WEB: &str = r#"
[web]
origin = "http://127.0.0.1:4402"
viewports = [320, 390, 768, 1280]
reduced_motion = true

[[web.routes]]
name = "home"
path = "/"
expected_status = 200

[[web.routes]]
name = "missing"
path = "/missing"
expected_status = 404

[[web.routes]]
name = "metadata"
path = "/metadata"
expected_status = 200

[web.thresholds]
max_lcp_ms = 2500
max_cls_milli = 100
min_accessibility_score = 90
"#;

    const LIGHTHOUSE: &str = r#"
[web.lighthouse]
version = "fixture-lighthouse-v1"
environment_fingerprint = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
warmup_samples = 1
measured_samples = 3
fields = ["performance", "accessibility", "best_practices", "seo", "fcp_ms", "lcp_ms", "cls", "tbt_ms"]
"#;

    const SEO: &str = r#"
[web.seo]
robots_path = "/robots.txt"
sitemap_path = "/sitemap.xml"
max_redirects = 2
"#;

    const GEO: &str = r#"
[web.geo]
entity_name = "Fixture Studio"
facts = { price = "£39 once", privacy = "Local-only" }
max_passages = 16
"#;

    #[test]
    fn manifest_parses_and_adds_control_paths() {
        let manifest = ValidatedManifest::parse(VALID).expect("valid manifest");
        assert_eq!(manifest.schema_version(), 1);
        assert_eq!(manifest.experiment().budget().max_candidates, 8);
        assert_eq!(manifest.evaluators().len(), 1);
        assert!(manifest.web().is_none(), "legacy v1 manifest stays valid");
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
    fn web_fixture_targets_are_canonical_and_frozen() {
        let source = format!("{VALID}{WEB}");
        let manifest = ValidatedManifest::parse(&source).expect("web manifest");
        let web = manifest.web().expect("web targets");
        assert_eq!(web.origin(), "http://127.0.0.1:4402");
        assert_eq!(web.viewports(), [320, 390, 768, 1280]);
        assert!(web.reduced_motion());
        assert_eq!(
            web.routes().iter().map(WebRoute::name).collect::<Vec<_>>(),
            ["home", "metadata", "missing"]
        );
        assert_eq!(web.routes()[2].expected_status(), 404);
        let first = FrozenIdentity::capture(&manifest, b"program", &BTreeMap::new(), None)
            .expect("frozen identity");
        for changed in [
            source.replace("max_lcp_ms = 2500", "max_lcp_ms = 2400"),
            source.replace("reduced_motion = true", "reduced_motion = false"),
            source.replace("path = \"/metadata\"", "path = \"/meta\""),
        ] {
            let changed_manifest = ValidatedManifest::parse(&changed).expect("changed web target");
            let identity =
                FrozenIdentity::capture(&changed_manifest, b"program", &BTreeMap::new(), None)
                    .expect("changed identity");
            assert_ne!(first.aggregate_sha256, identity.aggregate_sha256);
        }
    }

    #[test]
    fn web_fixture_rejects_unsafe_and_duplicate_targets() {
        for web in [
            WEB.replace("127.0.0.1", "example.com"),
            WEB.replace("/missing", "../missing"),
            WEB.replace("/missing", "/%2e%2e/missing"),
            WEB.replace("name = \"missing\"", "name = \"home\""),
            WEB.replace("path = \"/missing\"", "path = \"/metadata\""),
            WEB.replace("[320, 390, 768, 1280]", "[320, 390, 768]"),
            WEB.replace("max_lcp_ms = 2500", "max_lcp_ms = 0"),
            WEB.replace("expected_status = 404", "expected_status = 700"),
        ] {
            assert!(
                ValidatedManifest::parse(&format!("{VALID}{web}")).is_err(),
                "{web}"
            );
        }
    }

    #[test]
    fn checked_in_product_web_manifest_parses() {
        let source = include_str!("../../../examples/product-web/autoresearch.toml");
        let manifest = ValidatedManifest::parse(source).expect("checked-in fixture manifest");
        assert_eq!(manifest.web().expect("web").routes().len(), 3);
    }

    #[test]
    fn lighthouse_policy_is_frozen_and_rejects_unsafe_fields() {
        let source = format!("{VALID}{WEB}{LIGHTHOUSE}");
        let manifest = ValidatedManifest::parse(&source).expect("Lighthouse policy");
        let policy = manifest
            .web()
            .expect("web")
            .lighthouse()
            .expect("lighthouse");
        assert_eq!(policy.warmup_samples(), 1);
        assert_eq!(policy.measured_samples(), 3);
        assert_eq!(policy.fields().len(), 8);
        let baseline = FrozenIdentity::capture(&manifest, b"program", &BTreeMap::new(), None)
            .expect("baseline identity");
        let changed = source.replace("measured_samples = 3", "measured_samples = 4");
        let changed_manifest = ValidatedManifest::parse(&changed).expect("changed policy");
        let changed_identity =
            FrozenIdentity::capture(&changed_manifest, b"program", &BTreeMap::new(), None)
                .expect("changed identity");
        assert_ne!(baseline.aggregate_sha256, changed_identity.aggregate_sha256);
        for invalid in [
            source.replace("measured_samples = 3", "measured_samples = 0"),
            source.replace("warmup_samples = 1", "warmup_samples = 11"),
            source.replace("\"tbt_ms\"", "\"inp_ms\""),
            source.replace("\"tbt_ms\"", "\"seo\""),
            source.replace(
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "not-a-hash",
            ),
        ] {
            assert!(ValidatedManifest::parse(&invalid).is_err());
        }
    }

    #[test]
    fn seo_policy_is_frozen_and_rejects_unsafe_paths() {
        let source = format!("{VALID}{WEB}{SEO}");
        let manifest = ValidatedManifest::parse(&source).expect("SEO policy");
        let policy = manifest.web().expect("web").seo().expect("seo");
        assert_eq!(policy.robots_path(), "/robots.txt");
        assert_eq!(policy.sitemap_path(), "/sitemap.xml");
        assert_eq!(policy.max_redirects(), 2);
        assert!(!manifest.web().expect("web").routes()[2].indexable());
        let baseline = FrozenIdentity::capture(&manifest, b"program", &BTreeMap::new(), None)
            .expect("baseline identity");
        for changed in [
            source.replace("max_redirects = 2", "max_redirects = 3"),
            source.replace(
                "path = \"/metadata\"",
                "path = \"/metadata\"\nindexable = false",
            ),
        ] {
            let changed_manifest = ValidatedManifest::parse(&changed).expect("changed SEO policy");
            let changed_identity =
                FrozenIdentity::capture(&changed_manifest, b"program", &BTreeMap::new(), None)
                    .expect("changed identity");
            assert_ne!(baseline.aggregate_sha256, changed_identity.aggregate_sha256);
        }
        for invalid in [
            source.replace("max_redirects = 2", "max_redirects = 6"),
            source.replace("/robots.txt", "../robots.txt"),
            source.replace("/sitemap.xml", "https://example.com/sitemap.xml"),
            source.replace("/sitemap.xml", "/robots.txt"),
        ] {
            assert!(ValidatedManifest::parse(&invalid).is_err());
        }
    }

    #[test]
    fn geo_policy_is_frozen_and_bounded() {
        let source = format!("{VALID}{WEB}{GEO}");
        let manifest = ValidatedManifest::parse(&source).expect("GEO policy");
        let policy = manifest.web().expect("web").geo().expect("geo");
        assert_eq!(policy.entity_name(), "Fixture Studio");
        assert_eq!(
            policy.facts().get("price").map(String::as_str),
            Some("£39 once")
        );
        assert_eq!(policy.max_passages(), 16);
        let baseline = FrozenIdentity::capture(&manifest, b"program", &BTreeMap::new(), None)
            .expect("baseline identity");
        for changed in [
            source.replace("Fixture Studio", "Other Studio"),
            source.replace("£39 once", "£49 once"),
            source.replace("max_passages = 16", "max_passages = 8"),
        ] {
            let changed_manifest = ValidatedManifest::parse(&changed).expect("changed GEO policy");
            let changed_identity =
                FrozenIdentity::capture(&changed_manifest, b"program", &BTreeMap::new(), None)
                    .expect("changed identity");
            assert_ne!(baseline.aggregate_sha256, changed_identity.aggregate_sha256);
        }
        for invalid in [
            source.replace("max_passages = 16", "max_passages = 0"),
            source.replace("max_passages = 16", "max_passages = 33"),
            source.replace("price =", "bad.key ="),
            source.replace("£39 once", ""),
        ] {
            assert!(ValidatedManifest::parse(&invalid).is_err());
        }
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
