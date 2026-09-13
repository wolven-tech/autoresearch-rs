//! Stable report fixture and malformed-journal refusal tests.

use autoresearch_config::{FrozenIdentity, InputDigest, ValidatedManifest};
use autoresearch_core::{
    CandidateDecision, CandidateFinalization, Complexity, DecisionReason, Disposition,
    EvaluationSnapshot, JournalEntry, JournalEvent, Measurement, MetricDirection,
    NumericMetricKind,
};
use autoresearch_market::MarketEvidenceLedger;
use autoresearch_report::{build_report, to_json_bytes};
use autoresearch_runner::ReportSource;
use std::path::PathBuf;

const MANIFEST: &str = r#"
schema_version = 1
[experiment]
name = "report fixture"
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
program = "manual"
timeout_seconds = 10
[[evaluators]]
id = "check"
hard_gates = ["tests"]
[evaluators.command]
program = "fake"
timeout_seconds = 10
[[evaluators.metrics]]
name = "score"
kind = "objective"
direction = "maximize"
"#;

fn snapshot(score: f64, runtime_ms: u64) -> EvaluationSnapshot {
    EvaluationSnapshot {
        measurements: vec![
            Measurement::hard_gate("tests", true, None).expect("gate"),
            Measurement::numeric(
                "score",
                NumericMetricKind::Objective,
                MetricDirection::Maximize,
                score,
            )
            .expect("score"),
        ],
        complexity: Complexity {
            changed_lines: 1,
            dependency_delta: 0,
            runtime_ms,
        },
    }
}

fn source() -> ReportSource {
    let digest = InputDigest {
        name: "fixture".into(),
        sha256: "0".repeat(64),
    };
    let identity = FrozenIdentity {
        aggregate_sha256: "a".repeat(64),
        manifest: digest.clone(),
        program: digest,
        product_gate: None,
        fixtures: Vec::new(),
    };
    let events = [
        JournalEvent::RunStarted {
            base_commit: "base".into(),
            frozen_identity: identity.aggregate_sha256.clone(),
        },
        JournalEvent::BaselineCaptured {
            snapshot: snapshot(1.0, 11),
        },
        JournalEvent::CandidatePrepared {
            index: 1,
            parent_commit: "base".into(),
            worktree_id: "candidate-1".into(),
        },
        JournalEvent::CandidateDecisionRecorded {
            index: 1,
            candidate_commit: "improved".into(),
            changed_paths: vec!["tracked.txt".into()],
            snapshot: snapshot(2.0, 22),
            decision: CandidateDecision {
                disposition: Disposition::Keep,
                reason: DecisionReason::PrimaryImprovement,
            },
        },
        JournalEvent::CandidateFinalized {
            index: 1,
            outcome: CandidateFinalization::Kept {
                commit: "improved".into(),
            },
        },
    ];
    ReportSource {
        run_id: "report-fixture".into(),
        repository: PathBuf::from("/unused"),
        run_directory: PathBuf::from("/unused/.autoresearch"),
        base_commit: "base".into(),
        current_commit: "improved".into(),
        manifest: ValidatedManifest::parse(MANIFEST).expect("manifest"),
        entries: events
            .into_iter()
            .enumerate()
            .map(|(sequence, event)| JournalEntry {
                sequence: sequence.try_into().expect("small sequence"),
                run_id: "report-fixture".into(),
                event,
            })
            .collect(),
        identity,
        environment: None,
        environment_fingerprint: None,
    }
}

#[test]
fn report_v1_matches_golden_fixture() {
    let report = build_report(
        &source(),
        MarketEvidenceLedger {
            receipts: Vec::new(),
        },
    )
    .expect("report");
    let bytes = to_json_bytes(&report).expect("json");
    assert_eq!(
        String::from_utf8(bytes).expect("utf8"),
        include_str!("fixtures/report-v1.json")
    );
    assert_eq!(report.current_best_commit, "improved");
    assert_eq!(report.candidates[0].objective_delta, Some(1.0));
    assert_eq!(report.candidates[0].changed_paths, ["tracked.txt"]);
}

#[test]
fn malformed_journal_fails_closed() {
    let mut source = source();
    source.entries[3].sequence = 99;
    assert!(
        build_report(
            &source,
            MarketEvidenceLedger {
                receipts: Vec::new()
            }
        )
        .is_err()
    );
}
