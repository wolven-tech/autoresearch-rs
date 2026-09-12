//! Domain proof for journal-derived candidate recovery requests.

use autoresearch_core::{
    CandidateDecision, CandidateRecoveryRequest, Complexity, DecisionReason, Disposition,
    EvaluationSnapshot, JournalEntry, JournalEvent, Measurement, MetricDirection,
    NumericMetricKind, RecoveryValueError, ReplayState, replay_journal,
};
use std::path::Path;

const BASE: &str = "1111111111111111111111111111111111111111";
const CANDIDATE: &str = "2222222222222222222222222222222222222222";

#[test]
fn only_candidate_actions_form_recovery_requests() {
    let started = started_entries();
    let ReplayState::Run(started_view) = replay_journal(&started).expect("started journal") else {
        panic!("expected run view");
    };
    assert_eq!(request(&started_view), None);

    let based = baseline_entries();
    let ReplayState::Run(based_view) = replay_journal(&based).expect("baseline journal") else {
        panic!("expected run view");
    };
    assert_eq!(request(&based_view), None);

    let prepared = prepared_entries();
    let ReplayState::Run(prepared_view) = replay_journal(&prepared).expect("prepared journal")
    else {
        panic!("expected run view");
    };
    let Some(CandidateRecoveryRequest::Evaluate { run, candidate }) = request(&prepared_view)
    else {
        panic!("expected evaluation recovery");
    };
    assert_eq!(run.head_commit().as_str(), BASE);
    assert_eq!(candidate.index(), 1);
    assert_eq!(candidate.worktree_id(), "candidate-000001");
    assert_eq!(candidate.parent_commit(), run.head_commit());

    let decided = decided_entries(Disposition::Keep);
    let ReplayState::Run(decided_view) = replay_journal(&decided).expect("decided journal") else {
        panic!("expected run view");
    };
    let Some(CandidateRecoveryRequest::Finalize {
        run,
        candidate,
        candidate_commit,
        decision,
    }) = request(&decided_view)
    else {
        panic!("expected finalization recovery");
    };
    assert_eq!(candidate.parent_commit(), run.head_commit());
    assert_eq!(candidate_commit.as_str(), CANDIDATE);
    assert_eq!(decision.disposition, Disposition::Keep);
}

#[test]
fn journal_worktree_identity_must_match_deterministic_candidate() {
    let mut entries = prepared_entries();
    let JournalEvent::CandidatePrepared { worktree_id, .. } = &mut entries[2].event else {
        panic!("expected candidate record");
    };
    *worktree_id = "candidate-999999".into();
    let ReplayState::Run(view) = replay_journal(&entries).expect("journal shape remains valid")
    else {
        panic!("expected run view");
    };

    assert_eq!(
        CandidateRecoveryRequest::from_run_view(&view, Path::new("/tmp/worktrees")),
        Err(RecoveryValueError::WorktreeIdMismatch {
            expected: "candidate-000001".into(),
            actual: "candidate-999999".into(),
        })
    );
}

#[test]
fn abbreviated_journal_commit_cannot_reach_infrastructure() {
    let mut entries = prepared_entries();
    let JournalEvent::RunStarted { base_commit, .. } = &mut entries[0].event else {
        panic!("expected run start");
    };
    *base_commit = "abc".into();
    let JournalEvent::CandidatePrepared { parent_commit, .. } = &mut entries[2].event else {
        panic!("expected candidate record");
    };
    *parent_commit = "abc".into();
    let ReplayState::Run(view) = replay_journal(&entries).expect("replay accepts transport text")
    else {
        panic!("expected run view");
    };

    assert!(matches!(
        CandidateRecoveryRequest::from_run_view(&view, Path::new("/tmp/worktrees")),
        Err(RecoveryValueError::InvalidCommit(_))
    ));

    let mut entries = decided_entries(Disposition::Keep);
    let JournalEvent::CandidateDecisionRecorded {
        candidate_commit, ..
    } = &mut entries[3].event
    else {
        panic!("expected candidate decision");
    };
    *candidate_commit = "def".into();
    let ReplayState::Run(view) = replay_journal(&entries).expect("replay accepts transport text")
    else {
        panic!("expected run view");
    };
    assert!(matches!(
        CandidateRecoveryRequest::from_run_view(&view, Path::new("/tmp/worktrees")),
        Err(RecoveryValueError::InvalidCommit(_))
    ));
}

fn request(view: &autoresearch_core::RunView) -> Option<CandidateRecoveryRequest> {
    CandidateRecoveryRequest::from_run_view(view, Path::new("/tmp/worktrees"))
        .expect("valid recovery request")
}

fn started_entries() -> Vec<JournalEntry> {
    vec![entry(
        0,
        JournalEvent::RunStarted {
            base_commit: BASE.into(),
            frozen_identity: "frozen".into(),
        },
    )]
}

fn baseline_entries() -> Vec<JournalEntry> {
    let mut entries = started_entries();
    entries.push(entry(
        1,
        JournalEvent::BaselineCaptured {
            snapshot: snapshot(10.0),
        },
    ));
    entries
}

fn prepared_entries() -> Vec<JournalEntry> {
    let mut entries = baseline_entries();
    entries.push(entry(
        2,
        JournalEvent::CandidatePrepared {
            index: 1,
            parent_commit: BASE.into(),
            worktree_id: "candidate-000001".into(),
        },
    ));
    entries
}

fn decided_entries(disposition: Disposition) -> Vec<JournalEntry> {
    let mut entries = prepared_entries();
    entries.push(entry(
        3,
        JournalEvent::CandidateDecisionRecorded {
            index: 1,
            candidate_commit: CANDIDATE.into(),
            snapshot: snapshot(11.0),
            decision: CandidateDecision {
                disposition,
                reason: if disposition == Disposition::Keep {
                    DecisionReason::PrimaryImprovement
                } else {
                    DecisionReason::PrimaryRegression
                },
            },
        },
    ));
    entries
}

fn entry(sequence: u64, event: JournalEvent) -> JournalEntry {
    JournalEntry {
        sequence,
        run_id: "run-1".into(),
        event,
    }
}

fn snapshot(value: f64) -> EvaluationSnapshot {
    EvaluationSnapshot {
        measurements: vec![
            Measurement::hard_gate("tests", true, None).expect("gate"),
            Measurement::numeric(
                "score",
                NumericMetricKind::Objective,
                MetricDirection::Maximize,
                value,
            )
            .expect("objective"),
        ],
        complexity: Complexity::default(),
    }
}
