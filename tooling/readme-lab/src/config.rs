//! Frozen lab configuration: rule files, required commands, limit statements, use-case lanes.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Rule file the proposer may see scores for, relative to this config file.
    pub rules_dev: PathBuf,
    /// Rule file the proposer never sees, relative to this config file.
    pub rules_holdout: PathBuf,
    /// Subcommands one page must demonstrate with a parseable invocation.
    #[serde(default)]
    pub required_subcommands: Vec<String>,
    /// Page that owes `required_subcommands`, relative to the repository root. Absent asks every
    /// scored page for them, which is right only while one page documents the whole CLI.
    pub required_subcommands_page: Option<PathBuf>,
    /// Other `autoresearch-*` binaries a README may invoke.
    #[serde(default)]
    pub known_binaries: BTreeMap<String, KnownBinary>,
    #[serde(default)]
    pub limits: Vec<LimitGroup>,
    /// Page that owes the stated limits, relative to the repository root. Absent asks the scored
    /// page for them. Overclaims are still refused on every page.
    pub limits_page: Option<PathBuf>,
    #[serde(default)]
    pub use_cases: Vec<UseCase>,
    /// Page that owes the use-case lanes, relative to the repository root. Absent asks the scored
    /// page for them.
    pub use_cases_page: Option<PathBuf>,
    /// Absent fails the slop contract's substance gate closed.
    pub substance: Option<Substance>,
    /// Holdout slop points the slop contract may not exceed; absent fails that gate closed.
    pub holdout_ceiling: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KnownBinary {
    /// Source file of the binary, relative to the repository root.
    pub source: PathBuf,
    /// Accepted argument shapes, one per entry: literal tokens, with `<name>` matching any one
    /// token. An empty string accepts only a bare invocation.
    pub argv: Vec<String>,
}

/// A stated limit that must survive: some sentence names a term and negates it.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LimitGroup {
    pub id: String,
    pub terms_any: Vec<String>,
    /// Regex patterns; a clause matching one with no negation in it overclaims the limit.
    #[serde(default)]
    pub claims_any: Vec<String>,
}

/// A showcased lane: a heading naming it whose section carries verified evidence.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UseCase {
    pub id: String,
    /// Case-insensitive substrings, any of which the section heading must contain.
    pub heading_any: Vec<String>,
    /// Exact byte strings copied from the source ledger; the section must contain one.
    pub evidence_any: Vec<String>,
    /// Section must also contain a code block or table.
    #[serde(default = "default_true")]
    pub require_artifact: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Substance {
    pub min_prose_words: usize,
    pub min_code_blocks: usize,
}

const fn default_true() -> bool {
    true
}

impl Config {
    /// Loads config and returns it with the directory its relative paths resolve against.
    pub fn load(path: &Path) -> Result<(Self, PathBuf), String> {
        let source = std::fs::read_to_string(path)
            .map_err(|error| format!("read config {}: {error}", path.display()))?;
        let config = toml::from_str(&source)
            .map_err(|error| format!("parse config {}: {error}", path.display()))?;
        let directory = path
            .parent()
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
        Ok((config, directory))
    }
}
