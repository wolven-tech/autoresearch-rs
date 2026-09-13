//! Exact committed-tree diff and dependency evidence fixtures.

use autoresearch_core::{CommitId, MutationBoundary, RepoPath};
use autoresearch_evaluator::{
    DependencyEvidence, DependencyManifest, DiffError, EvaluationContext, EvaluationContextSpec,
    evaluate_commit_diff,
};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
    candidate: PathBuf,
    artifacts: PathBuf,
    parent: CommitId,
}

impl Fixture {
    fn new() -> Self {
        let base = fs::canonicalize(std::env::temp_dir()).expect("canonical temp root");
        let root = base.join(format!(
            "autoresearch-diff-test-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        let candidate = root.join(".autoresearch/worktrees/run-1/candidate-000001");
        let artifacts = root.join(".autoresearch/runs/run-1/artifacts");
        fs::create_dir_all(candidate.join("src")).expect("candidate source directory");
        fs::create_dir_all(candidate.join("assets")).expect("candidate asset directory");
        fs::create_dir_all(&artifacts).expect("artifact directory");
        git(&candidate, &["init", "-q", "-b", "main"]);
        git(&candidate, &["config", "user.name", "Fixture"]);
        git(
            &candidate,
            &["config", "user.email", "fixture@example.invalid"],
        );
        git(&candidate, &["config", "commit.gpgsign", "false"]);
        fs::write(
            candidate.join("Cargo.toml"),
            "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\n[dependencies]\nserde = \"1\"\n",
        )
        .expect("manifest");
        fs::write(candidate.join("src/lib.rs"), "first\nsecond\n").expect("source");
        fs::write(candidate.join("src/old.rs"), "unchanged rename content\n")
            .expect("rename source");
        fs::write(candidate.join("src/unchanged.rs"), "leave me\n").expect("unchanged source");
        fs::write(candidate.join("assets/binary.dat"), [0, 1, 2]).expect("binary");
        fs::write(candidate.join(".gitignore"), "ignored.log\n").expect("ignored rule");
        git(&candidate, &["add", "-A"]);
        git(&candidate, &["commit", "-q", "-m", "baseline"]);
        let parent =
            CommitId::new(git(&candidate, &["rev-parse", "HEAD"]).trim()).expect("parent commit");
        Self {
            root,
            candidate,
            artifacts,
            parent,
        }
    }

    fn change_and_commit(&self) -> CommitId {
        fs::write(self.candidate.join("Cargo.toml"), "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\n[dependencies]\nserde = \"1\"\ntoml = \"1\"\n").expect("new manifest");
        fs::write(self.candidate.join("src/lib.rs"), "first\nthird\nextra\n").expect("new source");
        fs::rename(
            self.candidate.join("src/old.rs"),
            self.candidate.join("src/new.rs"),
        )
        .expect("rename");
        fs::write(self.candidate.join("assets/binary.dat"), [0, 3, 4]).expect("new binary");
        git(&self.candidate, &["add", "-A"]);
        git(&self.candidate, &["commit", "-q", "-m", "candidate"]);
        CommitId::new(git(&self.candidate, &["rev-parse", "HEAD"]).trim())
            .expect("candidate commit")
    }

    fn context(&self, candidate_commit: &CommitId) -> EvaluationContext {
        EvaluationContext::new(EvaluationContextSpec {
            run_id: "run-1".into(),
            baseline_commit: self.parent.to_string(),
            evaluated_commit: candidate_commit.to_string(),
            candidate_worktree: self.candidate.clone(),
            changed_paths: vec![
                "Cargo.toml".into(),
                "assets/binary.dat".into(),
                "src/lib.rs".into(),
                "src/new.rs".into(),
                "src/old.rs".into(),
            ],
            declared_environment: BTreeMap::new(),
            artifact_directory: self.artifacts.clone(),
            cancellation_id: "cancel-1".into(),
        })
        .expect("valid context")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).expect("remove owned fixture");
    }
}

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .expect("git starts");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("Git UTF-8")
}

fn path(name: &str) -> RepoPath {
    RepoPath::new(name).expect("valid path")
}

fn boundary() -> MutationBoundary {
    MutationBoundary::new(
        vec![path("Cargo.toml"), path("src"), path("assets")],
        vec![],
    )
    .expect("boundary")
}

#[test]
fn exact_diff_accounts_for_lines_renames_binary_and_dependencies() {
    let fixture = Fixture::new();
    let candidate = fixture.change_and_commit();
    let context = fixture.context(&candidate);
    let evidence = evaluate_commit_diff(
        &context,
        &fixture.parent,
        &boundary(),
        DependencyManifest::CargoToml(path("Cargo.toml")),
    )
    .expect("exact committed diff");
    assert_eq!(
        evidence
            .changed_paths()
            .iter()
            .map(RepoPath::as_str)
            .collect::<Vec<_>>(),
        [
            "Cargo.toml",
            "assets/binary.dat",
            "src/lib.rs",
            "src/new.rs",
            "src/old.rs"
        ]
    );
    assert_eq!(
        evidence
            .binary_paths()
            .iter()
            .map(RepoPath::as_str)
            .collect::<Vec<_>>(),
        ["assets/binary.dat"]
    );
    assert_eq!(evidence.changed_lines(), 4);
    assert_eq!(
        evidence.dependency_evidence(),
        &DependencyEvidence::Known {
            added: vec!["toml".into()]
        }
    );
    assert_eq!(
        evidence
            .complexity(25)
            .expect("known complexity")
            .dependency_delta,
        1
    );
    assert_eq!(
        evidence
            .complexity(25)
            .expect("known complexity")
            .changed_lines,
        4
    );
    assert_eq!(
        evidence
            .complexity(25)
            .expect("known complexity")
            .runtime_ms,
        25
    );
    assert_eq!(
        fs::read(fixture.candidate.join("src/unchanged.rs")).expect("unchanged"),
        b"leave me\n"
    );
}

#[test]
fn rejects_dirty_or_wrong_parent_or_mismatched_declared_paths() {
    let fixture = Fixture::new();
    let candidate = fixture.change_and_commit();
    let context = fixture.context(&candidate);
    let wrong_parent =
        CommitId::new("0123456789abcdef0123456789abcdef01234567").expect("object id");
    assert!(matches!(
        evaluate_commit_diff(
            &context,
            &wrong_parent,
            &boundary(),
            DependencyManifest::CargoToml(path("Cargo.toml"))
        ),
        Err(DiffError::Topology(_))
    ));

    let mut spec = EvaluationContextSpec {
        run_id: "run-1".into(),
        baseline_commit: fixture.parent.to_string(),
        evaluated_commit: candidate.to_string(),
        candidate_worktree: fixture.candidate.clone(),
        changed_paths: vec!["src/lib.rs".into()],
        declared_environment: BTreeMap::new(),
        artifact_directory: fixture.artifacts.clone(),
        cancellation_id: "cancel-1".into(),
    };
    let mismatch = EvaluationContext::new(spec.clone()).expect("context");
    assert!(matches!(
        evaluate_commit_diff(
            &mismatch,
            &fixture.parent,
            &boundary(),
            DependencyManifest::CargoToml(path("Cargo.toml"))
        ),
        Err(DiffError::ChangedPathsMismatch)
    ));
    spec.changed_paths = context
        .changed_paths()
        .iter()
        .map(|path| path.as_str().into())
        .collect();
    fs::write(fixture.candidate.join("src/lib.rs"), "dirty\n").expect("dirty source");
    let dirty = EvaluationContext::new(spec).expect("context");
    assert!(matches!(
        evaluate_commit_diff(
            &dirty,
            &fixture.parent,
            &boundary(),
            DependencyManifest::CargoToml(path("Cargo.toml"))
        ),
        Err(DiffError::DirtyWorktree)
    ));

    let ignored_fixture = Fixture::new();
    let ignored_candidate = ignored_fixture.change_and_commit();
    fs::write(
        ignored_fixture.candidate.join("ignored.log"),
        "evaluator-visible state\n",
    )
    .expect("ignored file");
    assert!(matches!(
        evaluate_commit_diff(
            &ignored_fixture.context(&ignored_candidate),
            &ignored_fixture.parent,
            &boundary(),
            DependencyManifest::CargoToml(path("Cargo.toml")),
        ),
        Err(DiffError::DirtyWorktree)
    ));
}

#[test]
fn protected_paths_stay_protected_and_unsupported_manifest_is_unavailable() {
    let fixture = Fixture::new();
    let candidate = fixture.change_and_commit();
    let context = fixture.context(&candidate);
    let protected = MutationBoundary::new(
        vec![path("Cargo.toml"), path("src"), path("assets")],
        vec![path("src/new.rs")],
    )
    .expect("protected boundary");
    assert!(matches!(
        evaluate_commit_diff(
            &context,
            &fixture.parent,
            &protected,
            DependencyManifest::CargoToml(path("Cargo.toml"))
        ),
        Err(DiffError::Containment(_))
    ));
    let unsupported = evaluate_commit_diff(
        &context,
        &fixture.parent,
        &boundary(),
        DependencyManifest::Unsupported("package.json".into()),
    )
    .expect("diff with unavailable dependency format");
    assert!(matches!(
        unsupported.dependency_evidence(),
        DependencyEvidence::Unavailable { .. }
    ));
    assert!(matches!(
        unsupported.complexity(25),
        Err(DiffError::DependencyUnavailable)
    ));
}
