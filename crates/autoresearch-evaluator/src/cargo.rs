//! Fixed Cargo checks bound to distinct, explicitly declared hard gates.

use crate::{
    CancellationToken, EvaluationContext, FailureClass, ProcessEvaluation, ProcessFailure,
    ProcessLimits, evaluate_command_gate,
};
use autoresearch_config::Evaluator;
use std::fs;
use std::io::ErrorKind;
use std::path::{Component, Path};

/// Supported Cargo checks. Custom targets, toolchains, and flags require a
/// separate adapter with its own declared gate; they are not inferred here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CargoCheck {
    /// `cargo fmt --all --check`
    Format,
    /// `cargo clippy --workspace --all-targets --all-features -- -D warnings`
    Clippy,
    /// `cargo test --workspace --all-features`
    Test,
    /// `cargo build --workspace --all-features`
    Build,
}

impl CargoCheck {
    /// Literal command arguments for this check.
    #[must_use]
    pub fn args(self) -> &'static [&'static str] {
        match self {
            Self::Format => &["fmt", "--all", "--check"],
            Self::Clippy => &[
                "clippy",
                "--workspace",
                "--all-targets",
                "--all-features",
                "--",
                "-D",
                "warnings",
            ],
            Self::Test => &["test", "--workspace", "--all-features"],
            Self::Build => &["build", "--workspace", "--all-features"],
        }
    }

    /// Required distinct hard-gate declaration.
    #[must_use]
    pub fn gate_name(self) -> &'static str {
        match self {
            Self::Format => "cargo_format",
            Self::Clippy => "cargo_clippy",
            Self::Test => "cargo_test",
            Self::Build => "cargo_build",
        }
    }

    /// Stable label for manifest identifiers and logs.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Format => "format",
            Self::Clippy => "clippy",
            Self::Test => "test",
            Self::Build => "build",
        }
    }
}

/// Runs one exact Cargo check using shared bounded subprocess policy.
///
/// Caller declares an exact Cargo command and its corresponding hard gate in
/// frozen manifest. SDK verifies both before execution. Cargo is forced
/// offline; target output remains in run-owned artifact directory. SDK does
/// not provide OS-level filesystem or network sandboxing.
///
/// # Errors
///
/// Rejects unsupported command, gate, toolchain/target flags, offline policy,
/// or output directory. Process and nonzero-exit failures propagate unchanged.
pub fn evaluate_cargo_check(
    context: &EvaluationContext,
    evaluator: &Evaluator,
    check: CargoCheck,
    limits: ProcessLimits,
    cancellation: &CancellationToken,
) -> Result<ProcessEvaluation, ProcessFailure> {
    let command = evaluator.command();
    let program = Path::new(command.program());
    let program_valid = if program == Path::new("cargo") {
        true
    } else {
        program.is_absolute()
            && program.is_file()
            && program.file_name().is_some_and(|name| name == "cargo")
    };
    let args_valid = command
        .args()
        .iter()
        .map(String::as_str)
        .eq(check.args().iter().copied());
    let gates_valid =
        evaluator.hard_gates() == [check.gate_name()] && evaluator.metrics().is_empty();
    let environment = context.declared_environment();
    let offline = environment
        .get("CARGO_NET_OFFLINE")
        .is_some_and(|value| value == "true");
    let target_valid = environment.get("CARGO_TARGET_DIR").is_some_and(|value| {
        let target = Path::new(value);
        target.is_absolute()
            && target.starts_with(context.artifact_directory())
            && target
                .components()
                .all(|part| !matches!(part, Component::ParentDir | Component::CurDir))
            && !has_symlink_or_non_directory(context.artifact_directory(), target)
    });
    if !program_valid || !args_valid || !gates_valid || !offline || !target_valid {
        return Err(ProcessFailure::new(
            FailureClass::Validation,
            "Cargo check declaration or offline artifact policy unsupported",
            0,
            0,
        ));
    }
    evaluate_command_gate(context, evaluator, limits, cancellation)
}

fn has_symlink_or_non_directory(root: &Path, target: &Path) -> bool {
    let Ok(relative) = target.strip_prefix(root) else {
        return true;
    };
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => return true,
            Ok(_) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => return false,
            Err(_) => return true,
        }
    }
    false
}
