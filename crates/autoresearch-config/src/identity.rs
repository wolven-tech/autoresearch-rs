//! Canonical identities for frozen experiment inputs.

use crate::ValidatedManifest;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use thiserror::Error;

const DOMAIN: &[u8] = b"autoresearch/frozen-inputs/v1";

/// Identity construction failure.
#[derive(Debug, Error)]
pub enum IdentityError {
    /// Validated manifest failed canonical JSON serialization.
    #[error("failed to serialize validated manifest: {0}")]
    Serialize(#[from] serde_json::Error),
    /// Fixture names become report keys and cannot be blank.
    #[error("frozen fixture name cannot be blank")]
    BlankFixtureName,
}

/// Named SHA-256 digest for one frozen input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputDigest {
    /// Stable input name.
    pub name: String,
    /// Lowercase hexadecimal SHA-256.
    pub sha256: String,
}

/// Aggregate and component identities captured before mutation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrozenIdentity {
    /// Aggregate SHA-256 covering every component and presence marker.
    pub aggregate_sha256: String,
    /// Canonical validated manifest digest.
    pub manifest: InputDigest,
    /// Raw program prompt digest.
    pub program: InputDigest,
    /// Optional original product-gate digest.
    pub product_gate: Option<InputDigest>,
    /// Fixture digests sorted by stable fixture name.
    pub fixtures: Vec<InputDigest>,
}

impl FrozenIdentity {
    /// Captures relocation-independent identity of every frozen run input.
    ///
    /// # Errors
    ///
    /// Returns [`IdentityError`] when canonical serialization fails or a
    /// fixture has a blank name.
    pub fn capture(
        manifest: &ValidatedManifest,
        program: &[u8],
        fixtures: &BTreeMap<String, Vec<u8>>,
        product_gate: Option<&[u8]>,
    ) -> Result<Self, IdentityError> {
        if fixtures.keys().any(|name| name.trim().is_empty()) {
            return Err(IdentityError::BlankFixtureName);
        }

        let manifest_bytes = serde_json::to_vec(manifest)?;
        let manifest_digest = component_digest("manifest", &manifest_bytes);
        let program_digest = component_digest("program", program);
        let product_gate_digest = product_gate.map(|bytes| component_digest("product_gate", bytes));
        let fixture_digests = fixtures
            .iter()
            .map(|(name, bytes)| component_digest(&format!("fixture:{name}"), bytes))
            .collect::<Vec<_>>();

        let mut aggregate = Sha256::new();
        append_segment(&mut aggregate, b"domain", DOMAIN);
        append_segment(&mut aggregate, b"manifest", &manifest_bytes);
        append_segment(&mut aggregate, b"program", program);
        match product_gate {
            Some(bytes) => {
                append_segment(&mut aggregate, b"product_gate_present", b"1");
                append_segment(&mut aggregate, b"product_gate", bytes);
            }
            None => append_segment(&mut aggregate, b"product_gate_present", b"0"),
        }
        for (name, bytes) in fixtures {
            append_segment(&mut aggregate, b"fixture_name", name.as_bytes());
            append_segment(&mut aggregate, b"fixture_bytes", bytes);
        }

        Ok(Self {
            aggregate_sha256: hex_digest(aggregate.finalize()),
            manifest: manifest_digest,
            program: program_digest,
            product_gate: product_gate_digest,
            fixtures: fixture_digests,
        })
    }
}

fn component_digest(name: &str, bytes: &[u8]) -> InputDigest {
    let mut digest = Sha256::new();
    append_segment(&mut digest, b"domain", DOMAIN);
    append_segment(&mut digest, b"name", name.as_bytes());
    append_segment(&mut digest, b"bytes", bytes);
    InputDigest {
        name: name.to_owned(),
        sha256: hex_digest(digest.finalize()),
    }
}

fn append_segment(digest: &mut Sha256, tag: &[u8], bytes: &[u8]) {
    digest.update(u64::try_from(tag.len()).unwrap_or(u64::MAX).to_be_bytes());
    digest.update(tag);
    digest.update(u64::try_from(bytes.len()).unwrap_or(u64::MAX).to_be_bytes());
    digest.update(bytes);
}

fn hex_digest(bytes: impl AsRef<[u8]>) -> String {
    use std::fmt::Write as _;

    bytes.as_ref().iter().fold(
        String::with_capacity(bytes.as_ref().len() * 2),
        |mut output, byte| {
            let _ = write!(output, "{byte:02x}");
            output
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST_A: &str = r#"
schema_version = 1
[experiment]
name = "test"
[experiment.objective]
name = "score"
direction = "maximize"
[experiment.budget]
max_candidates = 2
max_failures = 1
wall_clock_seconds = 60
[scope]
mutable_paths = ["web/src", "web/public"]
[agent]
program = "manual"
timeout_seconds = 10
[[evaluators]]
id = "score"
hard_gates = ["tests"]
[evaluators.command]
program = "score"
timeout_seconds = 10
[[evaluators.metrics]]
name = "score"
kind = "objective"
direction = "maximize"
"#;

    #[test]
    fn identity_ignores_toml_key_and_mutable_path_order() {
        let manifest_a = ValidatedManifest::parse(MANIFEST_A).expect("valid fixture");
        let manifest_b = ValidatedManifest::parse(
            &MANIFEST_A
                .replace(
                    "max_candidates = 2\nmax_failures = 1",
                    "max_failures = 1\nmax_candidates = 2",
                )
                .replace(
                    "mutable_paths = [\"web/src\", \"web/public\"]",
                    "mutable_paths = [\"web/public\", \"web/src\"]",
                ),
        )
        .expect("valid reordered fixture");
        let fixtures = BTreeMap::new();
        let first =
            FrozenIdentity::capture(&manifest_a, b"prompt", &fixtures, None).expect("identity");
        let second =
            FrozenIdentity::capture(&manifest_b, b"prompt", &fixtures, None).expect("identity");
        assert_eq!(first, second);
    }

    #[test]
    fn every_frozen_input_changes_aggregate() {
        let manifest = ValidatedManifest::parse(MANIFEST_A).expect("valid fixture");
        let changed_manifest = ValidatedManifest::parse(
            &MANIFEST_A.replace("max_candidates = 2", "max_candidates = 3"),
        )
        .expect("valid changed fixture");
        let fixtures = BTreeMap::from([("viewport".to_owned(), b"390".to_vec())]);
        let baseline = FrozenIdentity::capture(&manifest, b"prompt", &fixtures, Some(b"gate"))
            .expect("identity");

        let changed_program =
            FrozenIdentity::capture(&manifest, b"other", &fixtures, Some(b"gate"))
                .expect("identity");
        let changed_gate = FrozenIdentity::capture(&manifest, b"prompt", &fixtures, Some(b"other"))
            .expect("identity");
        let changed_fixtures = BTreeMap::from([("viewport".to_owned(), b"768".to_vec())]);
        let changed_fixture =
            FrozenIdentity::capture(&manifest, b"prompt", &changed_fixtures, Some(b"gate"))
                .expect("identity");
        let changed_config =
            FrozenIdentity::capture(&changed_manifest, b"prompt", &fixtures, Some(b"gate"))
                .expect("identity");

        for changed in [
            changed_program,
            changed_gate,
            changed_fixture,
            changed_config,
        ] {
            assert_ne!(baseline.aggregate_sha256, changed.aggregate_sha256);
        }
    }

    #[test]
    fn absence_and_empty_gate_are_distinct() {
        let manifest = ValidatedManifest::parse(MANIFEST_A).expect("valid fixture");
        let fixtures = BTreeMap::new();
        let absent =
            FrozenIdentity::capture(&manifest, b"prompt", &fixtures, None).expect("identity");
        let empty =
            FrozenIdentity::capture(&manifest, b"prompt", &fixtures, Some(b"")).expect("identity");
        assert_ne!(absent.aggregate_sha256, empty.aggregate_sha256);
    }

    #[test]
    fn segment_lengths_prevent_ambiguous_concatenation() {
        let manifest = ValidatedManifest::parse(MANIFEST_A).expect("valid fixture");
        let left = BTreeMap::from([("ab".to_owned(), b"c".to_vec())]);
        let right = BTreeMap::from([("a".to_owned(), b"bc".to_vec())]);
        let left_identity = FrozenIdentity::capture(&manifest, b"", &left, None).expect("identity");
        let right_identity =
            FrozenIdentity::capture(&manifest, b"", &right, None).expect("identity");
        assert_ne!(
            left_identity.aggregate_sha256,
            right_identity.aggregate_sha256
        );
    }
}
