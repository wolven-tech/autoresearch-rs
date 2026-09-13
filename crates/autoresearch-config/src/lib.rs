//! Versioned configuration and frozen input identities for autoresearch.

mod identity;
mod manifest;

pub use autoresearch_core::{MutationBoundary as Scope, RepoPath};
pub use identity::{FrozenIdentity, IdentityError, InputDigest};
pub use manifest::{
    AuthorityCeiling, Budget, CommandSpec, Evaluator, Experiment, ExternalCapability, GeoSettings,
    LighthouseSettings, ManifestError, MetricDefinition, Objective, ProductionSettings,
    SeoSettings, ValidatedManifest, WebRoute, WebTargets, WebThresholds,
};
