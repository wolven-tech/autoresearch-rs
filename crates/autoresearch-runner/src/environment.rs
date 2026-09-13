//! Redaction-safe runtime facts bound to journal before first evaluator.

use autoresearch_config::ValidatedManifest;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Host and frozen command-contract fingerprint source. This is not a claim
/// that every installed dependency or remote service was fingerprinted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentRecord {
    /// Record schema version.
    pub schema_version: u8,
    /// Build host operating system family.
    pub os: String,
    /// Build host CPU architecture.
    pub arch: String,
    /// Runner package version.
    pub runner_version: String,
    /// Frozen agent and evaluator program declarations, not credentials.
    pub declared_programs: Vec<String>,
}

impl EnvironmentRecord {
    /// Captures host identifiers and command names without reading ambient
    /// environment variables or provider credentials.
    #[must_use]
    pub fn capture(manifest: &ValidatedManifest) -> Self {
        let mut declared_programs = vec![manifest.agent().program().to_owned()];
        declared_programs.extend(
            manifest
                .evaluators()
                .iter()
                .map(|evaluator| evaluator.command().program().to_owned()),
        );
        Self {
            schema_version: 1,
            os: std::env::consts::OS.into(),
            arch: std::env::consts::ARCH.into(),
            runner_version: env!("CARGO_PKG_VERSION").into(),
            declared_programs,
        }
    }

    /// Checks record still names programs from frozen manifest.
    #[must_use]
    pub fn matches_manifest(&self, manifest: &ValidatedManifest) -> bool {
        self.schema_version == 1
            && !self.os.is_empty()
            && !self.arch.is_empty()
            && !self.runner_version.is_empty()
            && self.declared_programs == Self::capture(manifest).declared_programs
    }

    /// Encodes one canonical newline-terminated JSON record and digest.
    ///
    /// # Errors
    ///
    /// Returns JSON serialization failure.
    pub fn encoded_fingerprint(&self) -> Result<(Vec<u8>, String), serde_json::Error> {
        let mut bytes = serde_json::to_vec(self)?;
        bytes.push(b'\n');
        let fingerprint = sha256(&bytes);
        Ok((bytes, fingerprint))
    }
}

pub(crate) fn sha256(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    Sha256::digest(bytes)
        .iter()
        .fold(String::with_capacity(64), |mut output, byte| {
            let _ = write!(output, "{byte:02x}");
            output
        })
}
