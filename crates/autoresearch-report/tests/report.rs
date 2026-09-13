//! Stable report fixture and malformed-journal refusal tests.

use autoresearch_config::{FrozenIdentity, InputDigest, ValidatedManifest};
use autoresearch_core::{
    CandidateDecision, CandidateFinalization, Complexity, DecisionReason, Disposition,
    EvaluationSnapshot, EvaluatorFailure, FailureClass, JournalEntry, JournalEvent, Measurement,
    MetricDirection, NumericMetricKind,
};
use autoresearch_market::MarketEvidenceLedger;
use autoresearch_report::{ArtifactReference, build_report, render_board, to_json_bytes};
use autoresearch_runner::ReportSource;
use headless_chrome::protocol::cdp::Emulation;
use headless_chrome::protocol::cdp::Page;
use headless_chrome::{Browser, LaunchOptions};
use std::fs;
use std::path::PathBuf;
use std::time::Duration;
use url::Url;

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

#[test]
fn failed_attempt_is_reported_without_a_candidate_score() {
    let mut source = source();
    source.entries.truncate(3);
    source.entries.push(JournalEntry {
        sequence: 3,
        run_id: source.run_id.clone(),
        event: JournalEvent::CandidateEvaluatorFailed {
            index: 1,
            evaluator_id: "check".into(),
            evaluated_commit: "attempted".into(),
            failure: EvaluatorFailure {
                class: FailureClass::NonZeroExit,
                detail: "declared evaluator failed; raw process output withheld".into(),
            },
        },
    });
    source.current_commit = "base".into();
    let report = build_report(
        &source,
        MarketEvidenceLedger {
            receipts: Vec::new(),
        },
    )
    .expect("report failure");
    assert_eq!(report.candidates[0].state, "prepared");
    assert!(report.candidates[0].snapshot.is_none());
    assert_eq!(report.candidates[0].failures.len(), 1);
}

#[test]
fn forged_selection_reason_fails_closed() {
    let mut source = source();
    if let JournalEvent::CandidateDecisionRecorded { decision, .. } = &mut source.entries[3].event {
        decision.reason = DecisionReason::NoImprovement;
    }
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

#[test]
fn board_escapes_evaluator_text_and_links_only_local_artifacts() {
    let root =
        std::env::temp_dir().join(format!("autoresearch-report-board-{}", std::process::id()));
    let artifacts = root.join("artifacts");
    fs::create_dir_all(&artifacts).expect("artifact directory");
    fs::write(artifacts.join("view #1.png"), b"fixture").expect("artifact");
    let mut report = build_report(
        &source(),
        MarketEvidenceLedger {
            receipts: Vec::new(),
        },
    )
    .expect("report");
    report.run_id = "<script>alert(1)</script>".into();
    report.candidates[0].artifacts.push(ArtifactReference {
        evaluator_id: "check".into(),
        name: "<img src=x onerror=alert(1)>".into(),
        relative_path: "view #1.png".into(),
        media_type: "image/png".into(),
    });
    let html = String::from_utf8(render_board(&report, &root).expect("board")).expect("UTF-8");
    assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
    assert!(html.contains("href=\"artifacts/view%20%231.png\""));
    assert!(!html.contains("<script>"));
    assert!(!html.contains("<img src=x onerror"));
    assert!(!html.contains("<script src="));
    fs::remove_dir_all(root).expect("remove owned test root");
}

#[cfg(unix)]
#[test]
fn board_rejects_symlinked_artifact() {
    let root =
        std::env::temp_dir().join(format!("autoresearch-report-link-{}", std::process::id()));
    let artifacts = root.join("artifacts");
    fs::create_dir_all(&artifacts).expect("artifact directory");
    std::os::unix::fs::symlink("/etc/passwd", artifacts.join("view.png")).expect("symlink");
    let mut report = build_report(
        &source(),
        MarketEvidenceLedger {
            receipts: Vec::new(),
        },
    )
    .expect("report");
    report.candidates[0].artifacts.push(ArtifactReference {
        evaluator_id: "check".into(),
        name: "View".into(),
        relative_path: "view.png".into(),
        media_type: "image/png".into(),
    });
    assert!(render_board(&report, &root).is_err());
    fs::remove_dir_all(root).expect("remove owned test root");
}

#[test]
fn board_browser_layout_keyboard_and_reduced_motion() {
    let Some(chromium) = std::env::var_os("CHROMIUM_PATH")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::split_paths(&std::env::var_os("PATH")?)
                .map(|path| path.join("chromium"))
                .find(|path| path.is_file())
        })
    else {
        return;
    };
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/report-board-proof");
    fs::create_dir_all(&root).expect("proof directory");
    let report = build_report(
        &source(),
        MarketEvidenceLedger {
            receipts: Vec::new(),
        },
    )
    .expect("report");
    let html_path = root.join("report.html");
    fs::write(&html_path, render_board(&report, &root).expect("board"))
        .expect("write browser fixture");
    let url = Url::from_file_path(&html_path).expect("file URL");
    let browser = Browser::new(
        LaunchOptions::default_builder()
            .path(Some(chromium))
            .window_size(Some((1280, 900)))
            .build()
            .expect("launch options"),
    )
    .expect("Chromium");
    let tab = browser.new_tab().expect("tab");
    tab.set_default_timeout(Duration::from_secs(15));
    for width in [320, 390, 768, 1280] {
        configure_board_viewport(&tab, width);
        tab.navigate_to(url.as_str())
            .and_then(|tab| tab.wait_until_navigated())
            .expect("local board navigation");
        let result = tab
            .evaluate(
                "JSON.stringify({width:innerWidth,scroll:document.documentElement.scrollWidth,landmarks:document.querySelectorAll('main').length,heading:document.querySelectorAll('h1').length,reduced:matchMedia('(prefers-reduced-motion: reduce)').matches,script:document.querySelectorAll('script').length})",
                false,
            )
            .expect("layout script");
        let serialized = result
            .value
            .expect("value")
            .as_str()
            .expect("JSON string")
            .to_owned();
        let state: serde_json::Value = serde_json::from_str(&serialized).expect("layout JSON");
        assert_eq!(state["width"], width);
        assert!(state["scroll"].as_u64().expect("scroll width") <= u64::from(width));
        assert_eq!(state["landmarks"], 1);
        assert_eq!(state["heading"], 1);
        assert_eq!(state["reduced"], true);
        assert_eq!(state["script"], 0);
        tab.press_key("Tab").expect("keyboard Tab");
        let focus = tab
            .evaluate(
                "JSON.stringify({tag:document.activeElement.tagName,outline:getComputedStyle(document.activeElement).outlineStyle,thickness:parseFloat(getComputedStyle(document.activeElement).outlineWidth)})",
                false,
            )
            .expect("focus script");
        let serialized = focus
            .value
            .expect("focus value")
            .as_str()
            .expect("JSON string")
            .to_owned();
        let focus: serde_json::Value = serde_json::from_str(&serialized).expect("focus JSON");
        assert_eq!(focus["tag"], "A");
        assert_ne!(focus["outline"], "none");
        assert!(focus["thickness"].as_f64().expect("outline width") >= 2.0);
        if matches!(width, 320 | 1280) {
            let screenshot = tab
                .capture_screenshot(Page::CaptureScreenshotFormatOption::Png, None, None, true)
                .expect("board screenshot");
            fs::write(root.join(format!("board-{width}.png")), screenshot)
                .expect("save proof screenshot");
        }
    }
    tab.evaluate("document.querySelector('summary').focus()", false)
        .expect("summary focus");
    tab.press_key("Enter").expect("open evidence details");
    let open = tab
        .evaluate("document.querySelector('details').open", false)
        .expect("details state");
    assert_eq!(open.value, Some(serde_json::Value::Bool(true)));
}

fn configure_board_viewport(tab: &headless_chrome::Tab, width: u32) {
    tab.call_method(Emulation::SetDeviceMetricsOverride {
        width,
        height: 900,
        device_scale_factor: 1.0,
        mobile: width <= 390,
        scale: None,
        screen_width: Some(width),
        screen_height: Some(900),
        position_x: None,
        position_y: None,
        dont_set_visible_size: None,
        screen_orientation: None,
        viewport: None,
        display_feature: None,
        device_posture: None,
    })
    .expect("viewport");
    tab.call_method(Emulation::SetEmulatedMedia {
        media: None,
        features: Some(vec![Emulation::MediaFeature {
            name: "prefers-reduced-motion".into(),
            value: "reduce".into(),
        }]),
    })
    .expect("reduced motion");
}
