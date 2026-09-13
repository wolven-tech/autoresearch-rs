//! Compile and exercise both documented evaluator examples.

#[allow(dead_code)]
#[path = "../examples/native_evaluator.rs"]
mod native_example;
#[allow(dead_code)]
#[path = "../examples/jsonl_evaluator.rs"]
mod subprocess_example;

use autoresearch_evaluator::{EvaluationContext, EvaluationContextSpec};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);
const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
const REQUEST: &[u8] = include_bytes!("fixtures/request-v1.jsonl");
const RESPONSE: &[u8] = include_bytes!("fixtures/response-success-v1.jsonl");

#[test]
fn native_example_compiles_and_uses_shared_validation() {
    let base = fs::canonicalize(std::env::temp_dir()).expect("temp root");
    let root = base.join(format!(
        "autoresearch-native-example-test-{}-{}",
        std::process::id(),
        NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
    ));
    let candidate = root.join(".autoresearch/worktrees/run-1/candidate-000001");
    let artifacts = root.join(".autoresearch/runs/run-1/artifacts");
    fs::create_dir_all(candidate.join("src")).expect("source dir");
    fs::create_dir_all(&artifacts).expect("artifact dir");
    fs::write(candidate.join("src/lib.rs"), "pub fn ready() {}\n").expect("source");
    let context = EvaluationContext::new(EvaluationContextSpec {
        run_id: "run-1".into(),
        baseline_commit: COMMIT.into(),
        evaluated_commit: COMMIT.into(),
        candidate_worktree: PathBuf::from(&candidate),
        changed_paths: vec!["src/lib.rs".into()],
        declared_environment: BTreeMap::new(),
        artifact_directory: artifacts,
        cancellation_id: "cancel-1".into(),
    })
    .expect("context");
    let output = native_example::evaluate_demo(&context).expect("native example validates");
    assert_eq!(output.evaluator_id(), "source-present");
    assert_eq!(output.measurements().len(), 1);
    assert_eq!(output.measurements()[0].name(), "source_present");
    fs::remove_dir_all(root).expect("remove owned fixture");
}

#[test]
fn subprocess_example_exchanges_protocol_v1_golden_messages() {
    let response = subprocess_example::handle(REQUEST).expect("valid request produces response");
    assert_eq!(response, RESPONSE);
}
