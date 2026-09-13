//! Real child-process mutation tests. Fixture executable is Rust, not shell.

use autoresearch_config::{FrozenIdentity, ValidatedManifest};
use autoresearch_core::{FailureClass, RepositoryInspector};
use autoresearch_evaluator::CancellationToken;
use autoresearch_git::{GitRepository, LockedGitRepository, RunLockGuard};
use autoresearch_runner::{CommandMutationAdapter, MutationCommandError, MutationRequest};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

static NEXT: AtomicU64 = AtomicU64::new(0);
const PROGRAM: &str = "Improve score within scope\n";

struct Fixture {
    root: PathBuf,
    manifest: ValidatedManifest,
    identity: FrozenIdentity,
}

impl Fixture {
    fn new(mode: &str, timeout_seconds: u64) -> Self {
        let root = std::env::temp_dir().join(format!(
            "autoresearch-command-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).expect("root");
        git(&root, &["init", "-q", "--initial-branch=main"]);
        git(&root, &["config", "user.name", "Autoresearch Test"]);
        git(&root, &["config", "user.email", "test@example.invalid"]);
        git(&root, &["config", "commit.gpgsign", "false"]);
        fs::create_dir(root.join("docs")).expect("docs");
        let binary = env!("CARGO_BIN_EXE_autoresearch-mutation-fixture");
        let manifest_source = format!(
            r#"
schema_version = 1
[experiment]
name = "command fixture"
[experiment.objective]
name = "score"
direction = "maximize"
[experiment.budget]
max_candidates = 1
max_failures = 1
wall_clock_seconds = 30
[scope]
mutable_paths = ["tracked.txt"]
[agent]
program = "{binary}"
args = ["{mode}"]
timeout_seconds = {timeout_seconds}
[[evaluators]]
id = "one"
hard_gates = ["tests"]
[evaluators.command]
program = "fake"
timeout_seconds = 10
[[evaluators.metrics]]
name = "score"
kind = "objective"
direction = "maximize"
"#
        );
        for (path, content) in [
            (".gitignore", ".autoresearch/\n"),
            ("tracked.txt", "initial\n"),
            ("autoresearch.toml", manifest_source.as_str()),
            ("program.md", PROGRAM),
            ("docs/BET.md", "Original gate\n"),
        ] {
            fs::write(root.join(path), content).expect("write fixture");
        }
        git(&root, &["add", "."]);
        git(&root, &["commit", "-qm", "fixture"]);
        let manifest = ValidatedManifest::parse(&manifest_source).expect("manifest");
        let identity = FrozenIdentity::capture(
            &manifest,
            PROGRAM.as_bytes(),
            &BTreeMap::new(),
            Some(b"Original gate\n"),
        )
        .expect("identity");
        Self {
            root,
            manifest,
            identity,
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).expect("fixture cleanup");
    }
}

fn invoke(
    mode: &str,
    timeout: u64,
    cancellation: CancellationToken,
    allowed: bool,
) -> Result<(), MutationCommandError> {
    let fixture = Fixture::new(mode, timeout);
    let snapshot = GitRepository
        .inspect(&fixture.root, "HEAD")
        .expect("snapshot");
    let lock = RunLockGuard::acquire(&snapshot, "run-1").expect("lock");
    let git = LockedGitRepository::new(&snapshot, &lock).expect("locked Git");
    let run = git.open_run().expect("run");
    let candidate = git.prepare_candidate(&run, 1).expect("candidate");
    let request = MutationRequest::new(
        &candidate,
        &fixture.manifest,
        PROGRAM,
        &fixture.identity,
        vec![],
        "Change tracked source to improve score",
        "run-1-command-1",
    )
    .expect("request");
    let allowlist = if allowed {
        vec![PathBuf::from(env!(
            "CARGO_BIN_EXE_autoresearch-mutation-fixture"
        ))]
    } else {
        vec![]
    };
    let adapter =
        CommandMutationAdapter::new(&allowlist, 1024, 1024, cancellation).expect("adapter");
    let result = adapter.execute_and_commit(
        &fixture.root,
        &git,
        &run,
        &candidate,
        &request,
        &fixture.manifest,
    );
    match result {
        Ok(report) => {
            assert_eq!(
                report.candidate_commit.changed_paths()[0].as_str(),
                "tracked.txt"
            );
            assert_eq!(
                fs::read_to_string(fixture.root.join("tracked.txt")).expect("caller"),
                "initial\n"
            );
            assert!(report.stdout_bytes <= 1024);
            assert!(report.stderr_bytes <= 1024);
            Ok(())
        }
        Err(error) => {
            assert_eq!(
                fs::read_to_string(fixture.root.join("program.md")).expect("caller prompt"),
                PROGRAM
            );
            Err(error)
        }
    }
}

#[test]
fn allowed_command_and_scrubbed_environment_work_without_caller_mutation() {
    invoke("success", 5, CancellationToken::default(), true).expect("command mutation");
    invoke("env", 5, CancellationToken::default(), true).expect("HOME absent in child");
}

#[test]
fn command_adapter_denies_unlisted_executable_and_nonzero_exit() {
    assert!(matches!(
        invoke("success", 5, CancellationToken::default(), false),
        Err(MutationCommandError::ExecutableNotAllowed)
    ));
    assert!(matches!(
        invoke("exit", 5, CancellationToken::default(), true),
        Err(MutationCommandError::Process {
            class: FailureClass::NonZeroExit,
            ..
        })
    ));
}

#[test]
fn command_adapter_kills_timeout_cancellation_and_output_overflow() {
    assert!(matches!(
        invoke("sleep", 1, CancellationToken::default(), true),
        Err(MutationCommandError::Process {
            class: FailureClass::Timeout,
            ..
        })
    ));
    let cancellation = CancellationToken::default();
    let signal = cancellation.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        signal.cancel();
    });
    assert!(matches!(
        invoke("sleep", 5, cancellation, true),
        Err(MutationCommandError::Process {
            class: FailureClass::Cancelled,
            ..
        })
    ));
    assert!(matches!(
        invoke("overflow", 5, CancellationToken::default(), true),
        Err(MutationCommandError::Process {
            class: FailureClass::OutputLimit,
            ..
        })
    ));
}

#[test]
fn command_that_changes_frozen_prompt_cannot_report_success() {
    assert!(matches!(
        invoke("protected", 5, CancellationToken::default(), true),
        Err(MutationCommandError::Mutation(_))
    ));
}

#[test]
fn provider_examples_parse_with_default_denied_external_authority() {
    const BEFORE: &str = r#"
schema_version = 1
[experiment]
name = "example"
[experiment.objective]
name = "score"
direction = "maximize"
[experiment.budget]
max_candidates = 1
max_failures = 1
wall_clock_seconds = 900
[scope]
mutable_paths = ["web/src"]
"#;
    const AFTER: &str = r#"
[[evaluators]]
id = "one"
hard_gates = ["tests"]
[evaluators.command]
program = "local-evaluator"
timeout_seconds = 10
[[evaluators.metrics]]
name = "score"
kind = "objective"
direction = "maximize"
"#;
    for fragment in [
        include_str!("../../../examples/agents/codex.agent.toml"),
        include_str!("../../../examples/agents/claude.agent.toml"),
        include_str!("../../../examples/agents/custom.agent.toml"),
    ] {
        let source = format!("{BEFORE}\n{fragment}\n{AFTER}");
        let manifest = ValidatedManifest::parse(&source).expect("example parses");
        assert!(manifest.authority().allowed().is_empty());
        assert!(manifest.agent().program().starts_with("/opt/approved/bin/"));
    }
}

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .expect("git");
    assert!(
        output.status.success(),
        "git {:?}: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}
