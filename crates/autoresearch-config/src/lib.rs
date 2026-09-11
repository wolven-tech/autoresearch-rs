//! Versioned configuration and frozen input identities for autoresearch.

mod identity;
mod manifest;

pub use identity::{FrozenIdentity, IdentityError, InputDigest};
pub use manifest::{
    AuthorityCeiling, Budget, CommandSpec, Evaluator, Experiment, ExternalCapability,
    ManifestError, MetricDefinition, Objective, RepoPath, Scope, ValidatedManifest,
};
