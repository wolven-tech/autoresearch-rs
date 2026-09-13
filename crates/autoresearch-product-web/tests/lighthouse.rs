//! Synthetic local Lighthouse reports test importer contract, not live scores.

use autoresearch_config::{LighthouseSettings, ValidatedManifest};
use autoresearch_evaluator::{EvaluationContext, EvaluationContextSpec};
use autoresearch_product_web::lighthouse::{LighthouseError, import_reports};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
const FINGERPRINT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const URL: &str = "http://127.0.0.1:4419/";

struct Fixture {
    root: PathBuf,
    context: EvaluationContext,
    manifest: ValidatedManifest,
}

impl Fixture {
    fn new() -> Self {
        let root = fs::canonicalize(std::env::temp_dir())
            .expect("temp root")
            .join(format!(
                "autoresearch-lhr-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        let candidate = root.join(".autoresearch/worktrees/run-1/candidate-000001");
        let artifacts = root.join(".autoresearch/runs/run-1/artifacts");
        fs::create_dir_all(&candidate).expect("candidate");
        fs::create_dir_all(artifacts.join("lighthouse")).expect("artifacts");
        let source = format!(
            "{}\n[web.lighthouse]\nversion = \"fixture-lighthouse-v1\"\nenvironment_fingerprint = \"{FINGERPRINT}\"\nwarmup_samples = 1\nmeasured_samples = 3\nfields = [\"performance\", \"accessibility\", \"best_practices\", \"seo\", \"fcp_ms\", \"lcp_ms\", \"cls\", \"tbt_ms\"]\n",
            include_str!("../../../examples/product-web/autoresearch.toml")
        );
        let manifest = ValidatedManifest::parse(&source).expect("Lighthouse manifest");
        let context = EvaluationContext::new(EvaluationContextSpec {
            run_id: "run-1".into(),
            baseline_commit: COMMIT.into(),
            evaluated_commit: COMMIT.into(),
            candidate_worktree: candidate,
            changed_paths: vec!["index.html".into()],
            declared_environment: BTreeMap::new(),
            artifact_directory: artifacts,
            cancellation_id: "cancel-1".into(),
        })
        .expect("context");
        Self {
            root,
            context,
            manifest,
        }
    }

    fn settings(&self) -> &LighthouseSettings {
        self.manifest
            .web()
            .expect("web")
            .lighthouse()
            .expect("Lighthouse")
    }

    fn write(&self, index: usize, performance: f64, lcp: f64) -> String {
        let relative = format!("lighthouse/sample-{index}.json");
        let value = report(performance, lcp);
        fs::write(
            self.context.artifact_directory().join(&relative),
            serde_json::to_vec(&value).expect("report JSON"),
        )
        .expect("report artifact");
        relative
    }

    fn four_reports(&self) -> Vec<String> {
        [(0.01, 9999.0), (0.6, 2000.0), (0.9, 2400.0), (0.7, 2200.0)]
            .into_iter()
            .enumerate()
            .map(|(index, (score, lcp))| self.write(index, score, lcp))
            .collect()
    }

    fn overwrite(&self, relative: &str, value: &Value) {
        fs::write(
            self.context.artifact_directory().join(relative),
            serde_json::to_vec(value).expect("modified JSON"),
        )
        .expect("modified report");
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).expect("owned fixture cleanup");
    }
}

fn report(performance: f64, lcp: f64) -> Value {
    json!({
        "environment_fingerprint": FINGERPRINT,
        "report": {
            "lighthouseVersion": "fixture-lighthouse-v1",
            "finalDisplayedUrl": URL,
            "categories": {
                "performance": {"score": performance},
                "accessibility": {"score": 0.95},
                "best-practices": {"score": 0.90},
                "seo": {"score": 1.0}
            },
            "audits": {
                "first-contentful-paint": {"numericValue": 900.0},
                "largest-contentful-paint": {"numericValue": lcp},
                "cumulative-layout-shift": {"numericValue": 0.02},
                "total-blocking-time": {"numericValue": 40.0}
            },
            "ignored_extra_field": "not imported"
        }
    })
}

#[test]
fn imports_only_declared_fields_and_median_after_warmup() {
    let fixture = Fixture::new();
    let paths = fixture.four_reports();
    let evidence = import_reports(&fixture.context, fixture.settings(), URL, &paths)
        .expect("validated reports");
    assert_eq!(evidence.warmup_samples, 1);
    assert_eq!(evidence.report_paths, paths);
    assert_eq!(evidence.fields.len(), 8);
    let performance = evidence
        .fields
        .iter()
        .find(|field| field.name == "performance")
        .expect("performance");
    assert_eq!(performance.samples, [60.0, 90.0, 70.0]);
    assert!((performance.median - 70.0).abs() < 1e-9);
    let lcp = evidence
        .fields
        .iter()
        .find(|field| field.name == "lcp_ms")
        .expect("LCP");
    assert_eq!(lcp.samples, [2000.0, 2400.0, 2200.0]);
    assert!((lcp.median - 2200.0).abs() < 1e-9);
}

#[test]
fn rejects_missing_changed_and_nonfinite_report_values() {
    for (mutation, expected) in [
        ("version", "version"),
        ("fingerprint", "fingerprint"),
        ("missing_field", "field"),
        ("nonfinite", "JSON"),
    ] {
        let fixture = Fixture::new();
        let paths = fixture.four_reports();
        let path = fixture.context.artifact_directory().join(&paths[2]);
        let mut value = report(0.9, 2400.0);
        match mutation {
            "version" => value["report"]["lighthouseVersion"] = json!("changed"),
            "fingerprint" => value["environment_fingerprint"] = json!("changed"),
            "missing_field" => value["report"]["audits"]
                .as_object_mut()
                .expect("audits")
                .remove("largest-contentful-paint")
                .map(drop)
                .expect("LCP field"),
            "nonfinite" => {
                fs::write(
                    path,
                    b"{\"report\":{\"categories\":{\"performance\":{\"score\":1e999}}}}",
                )
                .expect("nonfinite report");
                let error = import_reports(&fixture.context, fixture.settings(), URL, &paths)
                    .expect_err("nonfinite denied");
                assert!(matches!(
                    error,
                    LighthouseError::InvalidJson | LighthouseError::Field(_)
                ));
                continue;
            }
            _ => unreachable!(),
        }
        fixture.overwrite(&paths[2], &value);
        let error = import_reports(&fixture.context, fixture.settings(), URL, &paths)
            .expect_err("changed report denied");
        match expected {
            "version" => assert!(matches!(error, LighthouseError::VersionMismatch)),
            "fingerprint" => assert!(matches!(error, LighthouseError::FingerprintMismatch)),
            "field" => assert!(matches!(error, LighthouseError::Field(_))),
            _ => unreachable!(),
        }
    }
}

#[test]
fn rejects_out_of_root_duplicate_and_missing_reports() {
    let fixture = Fixture::new();
    let mut paths = fixture.four_reports();
    paths[2] = "../outside.json".into();
    assert!(matches!(
        import_reports(&fixture.context, fixture.settings(), URL, &paths),
        Err(LighthouseError::Artifact)
    ));
    paths[2] = paths[1].clone();
    assert!(matches!(
        import_reports(&fixture.context, fixture.settings(), URL, &paths),
        Err(LighthouseError::Artifact)
    ));
    assert!(matches!(
        import_reports(&fixture.context, fixture.settings(), URL, &paths[..3]),
        Err(LighthouseError::ReportCount)
    ));
}
