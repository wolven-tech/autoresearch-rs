//! Exact-commit candidate evaluation and durable keep/discard tests.

use autoresearch_config::{Evaluator, FrozenIdentity, ValidatedManifest};
use autoresearch_core::{
    CandidateCommit, CandidateDecision, CandidateFinalization, CandidateWorkspace, Complexity,
    DecisionReason, Disposition, EvaluationSnapshot, EvaluatorFailure, FailureClass, JournalEntry,
    JournalEvent, Measurement, MetricDirection, NumericMetricKind, ReplayState, RepoPath,
    RepositoryInspector, replay_journal,
};
use autoresearch_evaluator::{
    EvaluationContext, EvaluatorOutput, ValidatedOutput, validate_output,
};
use autoresearch_git::{GitRepository, LockedGitRepository, RunLockGuard};
use autoresearch_runner::{BaselineExecutor, RunnerError, evaluate_committed_candidate};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
const MANIFEST: &str = r#"
schema_version = 1
[experiment]
name = "candidate fixture"
[experiment.objective]
name = "score"
direction = "maximize"
[experiment.budget]
max_candidates = 2
max_failures = 1
wall_clock_seconds = 60
[scope]
mutable_paths = ["tracked.txt"]
[agent]
program = "manual"
timeout_seconds = 10
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
"#;

struct Fixture {
    root: PathBuf,
    run_dir: PathBuf,
    base: String,
    candidate: CandidateWorkspace,
    committed: CandidateCommit,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "autoresearch-candidate-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).expect("root");
        git(&root, &["init", "-q", "--initial-branch=main"]);
        git(&root, &["config", "user.name", "Autoresearch Test"]);
        git(&root, &["config", "user.email", "test@example.invalid"]);
        git(&root, &["config", "commit.gpgsign", "false"]);
        fs::create_dir(root.join("docs")).expect("docs");
        for (path, content) in [
            (".gitignore", ".autoresearch/\n"),
            ("tracked.txt", "original\n"),
            ("autoresearch.toml", MANIFEST),
            ("program.md", "Improve tracked code only\n"),
            ("docs/BET.md", "Original gate stays fixed\n"),
        ] {
            fs::write(root.join(path), content).expect("write fixture");
        }
        git(&root, &["add", "."]);
        git(&root, &["commit", "-qm", "fixture"]);
        let base = git_text(&root, &["rev-parse", "HEAD"]);
        let manifest = ValidatedManifest::parse(MANIFEST).expect("manifest");
        let identity = FrozenIdentity::capture(
            &manifest,
            b"Improve tracked code only\n",
            &BTreeMap::new(),
            Some(b"Original gate stays fixed\n"),
        )
        .expect("identity");
        let run_dir = root.join(".autoresearch/runs/run-1");
        write_frozen(&run_dir, &identity);
        let snapshot = GitRepository.inspect(&root, &base).expect("snapshot");
        let lock = RunLockGuard::acquire(&snapshot, "run-1").expect("lock");
        let git = LockedGitRepository::new(&snapshot, &lock).expect("locked Git");
        let run = git.open_run().expect("run");
        let candidate = git.prepare_candidate(&run, 1).expect("candidate");
        fs::write(candidate.path().join("tracked.txt"), "candidate\n").expect("manual edit");
        let committed = git
            .commit_candidate(&run, &candidate, manifest.scope())
            .expect("contained commit");
        drop(git);
        drop(lock);
        write_started_journal(&run_dir, &base, &identity, &candidate);
        Self {
            root,
            run_dir,
            base,
            candidate,
            committed,
        }
    }

    fn journal(&self) -> Vec<JournalEntry> {
        fs::read_to_string(self.run_dir.join("journal.jsonl"))
            .expect("journal")
            .lines()
            .map(|line| serde_json::from_str(line).expect("entry"))
            .collect()
    }
}

fn write_frozen(run_dir: &Path, identity: &FrozenIdentity) {
    let frozen = run_dir.join("frozen");
    fs::create_dir_all(&frozen).expect("frozen");
    fs::write(frozen.join("autoresearch.toml"), MANIFEST).expect("manifest artifact");
    fs::write(frozen.join("program.md"), "Improve tracked code only\n").expect("program artifact");
    fs::write(
        frozen.join("product-gate.md"),
        "Original gate stays fixed\n",
    )
    .expect("gate artifact");
    fs::write(
        run_dir.join("identity.json"),
        serde_json::to_vec(identity).expect("identity JSON"),
    )
    .expect("identity artifact");
}

fn write_started_journal(
    run_dir: &Path,
    base: &str,
    identity: &FrozenIdentity,
    candidate: &CandidateWorkspace,
) {
    let baseline = EvaluationSnapshot {
        measurements: vec![
            Measurement::hard_gate("tests", true, None).expect("gate"),
            Measurement::numeric(
                "score",
                NumericMetricKind::Objective,
                MetricDirection::Maximize,
                42.0,
            )
            .expect("score"),
        ],
        complexity: Complexity::default(),
    };
    let entries = [
        JournalEntry {
            sequence: 0,
            run_id: "run-1".into(),
            event: JournalEvent::RunStarted {
                base_commit: base.into(),
                frozen_identity: identity.aggregate_sha256.clone(),
            },
        },
        JournalEntry {
            sequence: 1,
            run_id: "run-1".into(),
            event: JournalEvent::BaselineCaptured { snapshot: baseline },
        },
        JournalEntry {
            sequence: 2,
            run_id: "run-1".into(),
            event: JournalEvent::CandidatePrepared {
                index: 1,
                parent_commit: base.into(),
                worktree_id: candidate.worktree_id().into(),
            },
        },
    ];
    let mut journal = Vec::new();
    for entry in entries {
        journal.extend(serde_json::to_vec(&entry).expect("entry JSON"));
        journal.push(b'\n');
    }
    fs::write(run_dir.join("journal.jsonl"), journal).expect("journal");
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).expect("fixture cleanup");
    }
}

struct FakeExecutor {
    score: f64,
    gate_passed: bool,
    fail: bool,
}

impl BaselineExecutor for FakeExecutor {
    fn evaluate(
        &self,
        context: &EvaluationContext,
        evaluator: &Evaluator,
    ) -> Result<ValidatedOutput, EvaluatorFailure> {
        if self.fail {
            return Err(EvaluatorFailure {
                class: FailureClass::Timeout,
                detail: "raw secret diagnostic".into(),
            });
        }
        validate_output(
            context,
            evaluator.id(),
            EvaluatorOutput {
                evaluator_id: evaluator.id().into(),
                run_id: context.run_id().to_string(),
                baseline_commit: context.baseline_commit().to_string(),
                evaluated_commit: context.evaluated_commit().to_string(),
                measurements: vec![
                    Measurement::hard_gate("tests", self.gate_passed, None).expect("gate"),
                    Measurement::numeric(
                        "score",
                        NumericMetricKind::Objective,
                        MetricDirection::Maximize,
                        self.score,
                    )
                    .expect("score"),
                ],
                observations: vec![],
                artifacts: vec![],
                warnings: vec![],
            },
        )
        .map_err(|_| EvaluatorFailure {
            class: FailureClass::Validation,
            detail: "fake output invalid".into(),
        })
    }
}

#[test]
fn improved_candidate_is_journaled_before_retained_ref_advances() {
    let fixture = Fixture::new();
    let result = evaluate_committed_candidate(
        &fixture.root,
        "run-1",
        &fixture.committed,
        &FakeExecutor {
            score: 43.0,
            gate_passed: true,
            fail: false,
        },
        BTreeMap::new(),
    )
    .expect("candidate evaluation");
    assert_eq!(result.decision.disposition, Disposition::Keep);
    assert_eq!(result.decision.reason, DecisionReason::PrimaryImprovement);
    assert!(matches!(
        result.finalization,
        CandidateFinalization::Kept { .. }
    ));
    assert_eq!(result.snapshot.complexity.changed_lines, 2);
    assert_eq!(fixture.journal().len(), 5);
    let ReplayState::Run(view) = replay_journal(&fixture.journal()).expect("journal replay") else {
        panic!("run expected")
    };
    assert_eq!(
        view.current_commit(),
        fixture.committed.commit_id().as_str()
    );
    assert_eq!(
        git_text(
            &fixture.root,
            &["rev-parse", "refs/heads/autoresearch/run-1"]
        ),
        fixture.committed.commit_id().as_str()
    );
    assert_eq!(
        git_text(&fixture.root, &["rev-parse", "HEAD"]),
        fixture.base
    );
    assert_eq!(
        fs::read_to_string(fixture.root.join("docs/BET.md")).expect("gate"),
        "Original gate stays fixed\n"
    );
}

#[test]
fn failed_hard_gate_discards_even_with_higher_score() {
    let fixture = Fixture::new();
    let result = evaluate_committed_candidate(
        &fixture.root,
        "run-1",
        &fixture.committed,
        &FakeExecutor {
            score: 99.0,
            gate_passed: false,
            fail: false,
        },
        BTreeMap::new(),
    )
    .expect("candidate evaluation");
    assert_eq!(result.decision.disposition, Disposition::Discard);
    assert_eq!(
        result.decision.reason,
        DecisionReason::FailedHardGates {
            names: vec!["tests".into()]
        }
    );
    assert_eq!(result.finalization, CandidateFinalization::Discarded);
    assert_eq!(
        git_text(
            &fixture.root,
            &["rev-parse", "refs/heads/autoresearch/run-1"]
        ),
        fixture.base
    );
}

#[test]
fn equal_score_uses_measured_changed_lines_and_discards() {
    let fixture = Fixture::new();
    let result = evaluate_committed_candidate(
        &fixture.root,
        "run-1",
        &fixture.committed,
        &FakeExecutor {
            score: 42.0,
            gate_passed: true,
            fail: false,
        },
        BTreeMap::new(),
    )
    .expect("candidate evaluation");
    assert_eq!(result.decision.disposition, Disposition::Discard);
    assert!(matches!(
        result.decision.reason,
        DecisionReason::TieBreaker { .. }
    ));
    assert_eq!(result.snapshot.complexity.changed_lines, 2);
}

#[test]
fn mismatched_commit_and_evaluator_failure_never_advance_ref() {
    let fixture = Fixture::new();
    let forged = CandidateCommit::new(
        autoresearch_core::CommitId::new(fixture.base.clone()).expect("base commit"),
        vec![RepoPath::new("tracked.txt").expect("path")],
        &ValidatedManifest::parse(MANIFEST)
            .expect("manifest")
            .scope()
            .clone(),
    )
    .expect("forged evidence shape");
    assert!(matches!(
        evaluate_committed_candidate(
            &fixture.root,
            "run-1",
            &forged,
            &FakeExecutor {
                score: 99.0,
                gate_passed: true,
                fail: false
            },
            BTreeMap::new(),
        ),
        Err(RunnerError::Git(_))
    ));
    assert_eq!(fixture.journal().len(), 3);
    assert!(matches!(
        evaluate_committed_candidate(
            &fixture.root,
            "run-1",
            &fixture.committed,
            &FakeExecutor {
                score: 99.0,
                gate_passed: true,
                fail: true
            },
            BTreeMap::new(),
        ),
        Err(RunnerError::CandidateEvaluator { .. })
    ));
    assert_eq!(fixture.journal().len(), 3);
    assert_eq!(
        git_text(
            &fixture.root,
            &["rev-parse", "refs/heads/autoresearch/run-1"]
        ),
        fixture.base
    );
    assert!(fixture.candidate.path().exists());
}

#[test]
fn recorded_keep_for_commit_other_than_candidate_head_cannot_advance_ref() {
    use std::io::Write as _;

    let fixture = Fixture::new();
    let decision = JournalEntry {
        sequence: 3,
        run_id: "run-1".into(),
        event: JournalEvent::CandidateDecisionRecorded {
            index: 1,
            candidate_commit: fixture.base.clone(),
            snapshot: EvaluationSnapshot {
                measurements: vec![
                    Measurement::hard_gate("tests", true, None).expect("gate"),
                    Measurement::numeric(
                        "score",
                        NumericMetricKind::Objective,
                        MetricDirection::Maximize,
                        44.0,
                    )
                    .expect("score"),
                ],
                complexity: Complexity::default(),
            },
            decision: CandidateDecision {
                disposition: Disposition::Keep,
                reason: DecisionReason::PrimaryImprovement,
            },
        },
    };
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(fixture.run_dir.join("journal.jsonl"))
        .expect("journal append");
    file.write_all(&serde_json::to_vec(&decision).expect("decision JSON"))
        .expect("write decision");
    file.write_all(b"\n").expect("newline");
    let ReplayState::Run(view) = replay_journal(&fixture.journal()).expect("replay") else {
        panic!("run expected")
    };
    let snapshot = GitRepository
        .inspect(&fixture.root, &fixture.base)
        .expect("snapshot");
    let lock = RunLockGuard::acquire(&snapshot, "run-1").expect("lock");
    let git = LockedGitRepository::new(&snapshot, &lock).expect("locked Git");
    assert!(git.recover_candidate(&view).is_err());
    assert_eq!(
        git_text(
            &fixture.root,
            &["rev-parse", "refs/heads/autoresearch/run-1"]
        ),
        fixture.base
    );
    assert_eq!(fixture.journal().len(), 4);
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

fn git_text(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .expect("git");
    assert!(output.status.success());
    String::from_utf8(output.stdout)
        .expect("Git UTF-8")
        .trim()
        .into()
}
