//! Named offline Phase 4 pack gate through Phase 3 output and snapshot validators.

use autoresearch_config::ValidatedManifest;
use autoresearch_core::{Complexity, Measurement, MetricDirection, NumericMetricKind};
use autoresearch_evaluator::{
    Artifact, EvaluationContext, EvaluationContextSpec, Observation, build_snapshot,
    validate_output,
};
use autoresearch_product_web::browser::{browser_payload, inspect_declared_route};
use autoresearch_product_web::geo::inspect_local_geo_route;
use autoresearch_product_web::lighthouse::import_reports;
use autoresearch_product_web::seo::inspect_local_route;
use serde_json::json;
use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::{self, JoinHandle};

static NEXT: AtomicU64 = AtomicU64::new(0);
const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
const HOME: &str = include_str!("../../../examples/product-web/index.html");
const METADATA: &str = include_str!("../../../examples/product-web/metadata.html");
const MISSING: &str = include_str!("../../../examples/product-web/missing.html");
const ROBOTS: &str = include_str!("../../../examples/product-web/robots.txt");
const SITEMAP: &str = include_str!("../../../examples/product-web/sitemap.xml");

struct OfflinePack {
    root: PathBuf,
    context: EvaluationContext,
    manifest: ValidatedManifest,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl OfflinePack {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("offline fixture listener");
        let port = listener.local_addr().expect("fixture address").port();
        listener.set_nonblocking(true).expect("nonblocking fixture");
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let worker = thread::spawn(move || {
            while !worker_stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream
                            .set_nonblocking(false)
                            .expect("blocking accepted fixture stream");
                        stream
                            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                            .expect("bounded fixture request read");
                        let mut request = [0_u8; 4096];
                        let count = match stream.read(&mut request) {
                            Ok(0) => continue,
                            Ok(count) => count,
                            Err(error)
                                if matches!(
                                    error.kind(),
                                    std::io::ErrorKind::WouldBlock
                                        | std::io::ErrorKind::TimedOut
                                        | std::io::ErrorKind::ConnectionReset
                                ) =>
                            {
                                continue;
                            }
                            Err(error) => panic!("fixture request: {error}"),
                        };
                        let text = String::from_utf8_lossy(&request[..count]);
                        let path = text.split_whitespace().nth(1).unwrap_or("/");
                        let (status, media, source) = match path {
                            "/" => ("200 OK", "text/html", HOME),
                            "/metadata" => ("200 OK", "text/html", METADATA),
                            "/robots.txt" => ("200 OK", "text/plain", ROBOTS),
                            "/sitemap.xml" => ("200 OK", "application/xml", SITEMAP),
                            _ => ("404 Not Found", "text/html", MISSING),
                        };
                        let body = source.replace("127.0.0.1:4419", &format!("127.0.0.1:{port}"));
                        let response = format!(
                            "HTTP/1.1 {status}\r\nContent-Type: {media}; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        );
                        stream.write_all(response.as_bytes()).expect("response");
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(std::time::Duration::from_millis(10));
                    }
                    Err(error) => panic!("offline fixture listener: {error}"),
                }
            }
        });
        let root = fs::canonicalize(std::env::temp_dir())
            .expect("temp root")
            .join(format!(
                "autoresearch-web-pack-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        let candidate = root.join(".autoresearch/worktrees/run-1/candidate-000001");
        let artifacts = root.join(".autoresearch/runs/run-1/artifacts");
        fs::create_dir_all(&candidate).expect("candidate");
        fs::create_dir_all(&artifacts).expect("artifacts");
        let source = include_str!("../../../examples/product-web/autoresearch.toml")
            .replace("127.0.0.1:4419", &format!("127.0.0.1:{port}"));
        let manifest = ValidatedManifest::parse(&source).expect("frozen pack manifest");
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
        .expect("pack context");
        Self {
            root,
            context,
            manifest,
            stop,
            worker: Some(worker),
        }
    }
}

impl Drop for OfflinePack {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let outcome = worker.join();
            if !thread::panicking() {
                outcome.expect("fixture worker");
            }
        }
        let cleanup = fs::remove_dir_all(&self.root);
        if !thread::panicking() {
            cleanup.expect("owned pack cleanup");
        }
    }
}

fn chromium() -> PathBuf {
    std::env::var_os("CHROMIUM_PATH")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::split_paths(&std::env::var_os("PATH")?)
                .map(|path| path.join("chromium"))
                .find(|path| path.is_file())
        })
        .expect("Phase 4 gate requires CHROMIUM_PATH or chromium on PATH")
}

fn write_fixture_lighthouse_reports(pack: &OfflinePack, route_url: &str) -> Vec<String> {
    let policy = pack
        .manifest
        .web()
        .expect("web")
        .lighthouse()
        .expect("lighthouse policy");
    fs::create_dir(pack.context.artifact_directory().join("lighthouse")).expect("report directory");
    (0..usize::from(policy.warmup_samples() + policy.measured_samples()))
        .map(|index| {
            let path = format!("lighthouse/fixture-{index}.json");
            let report = json!({
                "environment_fingerprint": policy.environment_fingerprint(),
                "report": {
                    "lighthouseVersion": policy.version(), "finalDisplayedUrl": route_url,
                    "categories": {
                    "performance": { "score": ([0.80, 0.81, 0.82, 0.83][index]) },
                    "accessibility": { "score": 0.9 },
                    "best-practices": { "score": 0.9 },
                    "seo": { "score": 0.95 }
                },
                "audits": {
                    "first-contentful-paint": { "numericValue": 900.0 },
                    "largest-contentful-paint": { "numericValue": 1900.0 },
                    "cumulative-layout-shift": { "numericValue": 0.02 },
                    "total-blocking-time": { "numericValue": 40.0 }
                    }
                }
            });
            fs::write(
                pack.context.artifact_directory().join(&path),
                serde_json::to_vec(&report).expect("JSON"),
            )
            .expect("frozen fixture report");
            path
        })
        .collect()
}

#[test]
fn offline_product_web_pack_runs_all_adapters_through_phase3_validator() {
    let pack = OfflinePack::start();
    let browser = inspect_declared_route(&pack.context, &pack.manifest, "home", &chromium())
        .expect("browser, responsive, accessibility");
    assert_eq!(
        browser
            .viewports
            .iter()
            .map(|viewport| viewport.requested_width)
            .collect::<Vec<_>>(),
        [320, 390, 768, 1280]
    );
    assert!(browser.final_url.ends_with('/'));
    for viewport in &browser.viewports {
        assert_eq!(viewport.http_status, Some(200));
        assert_eq!(viewport.observed_width, viewport.requested_width);
        assert!(viewport.document_width <= viewport.observed_width);
        assert!(viewport.overflow_elements.is_empty());
        assert!(viewport.reduced_motion_requested && viewport.reduced_motion_observed);
        assert_eq!(viewport.accessibility.keyboard_status(), "pass");
    }
    let seo = inspect_local_route(&pack.context, &pack.manifest, "home").expect("SEO route");
    assert!(seo.issues.is_empty(), "{seo:?}");
    let geo = inspect_local_geo_route(&pack.context, &pack.manifest, "home").expect("GEO route");
    assert!(geo.issues.is_empty(), "{geo:?}");
    let reports = write_fixture_lighthouse_reports(&pack, &seo.source_url);
    let lighthouse = import_reports(
        &pack.context,
        pack.manifest
            .web()
            .expect("web")
            .lighthouse()
            .expect("policy"),
        &seo.source_url,
        &reports,
    )
    .expect("frozen Lighthouse import");
    let performance = lighthouse
        .fields
        .iter()
        .find(|field| field.name == "performance")
        .expect("performance field")
        .median;
    let mut output = browser_payload(&pack.context, "product_web", &browser);
    assert!(output.measurements.iter().all(|measurement| {
        matches!(measurement, Measurement::HardGate { outcome, .. } if outcome.passed())
    }));
    output.measurements.push(
        Measurement::numeric(
            "fixture_score",
            NumericMetricKind::Objective,
            MetricDirection::Maximize,
            performance,
        )
        .expect("finite lab score"),
    );
    for (name, relative_path) in [
        ("seo_source", &seo.artifact_relative_path),
        ("geo_source", &geo.artifact_relative_path),
    ] {
        output.artifacts.push(Artifact {
            name: name.into(),
            relative_path: relative_path.clone(),
            media_type: "application/json".into(),
        });
    }
    for (index, path) in reports.iter().enumerate() {
        output.artifacts.push(Artifact {
            name: format!("lighthouse_fixture_{index}"),
            relative_path: path.clone(),
            media_type: "application/json".into(),
        });
    }
    output.observations.push(Observation {
        code: "seo_technical_issues".into(),
        detail: seo.issues.len().to_string(),
    });
    output.observations.push(Observation {
        code: "geo_diagnostic_issues".into(),
        detail: geo.issues.len().to_string(),
    });
    output.observations.push(Observation {
        code: "lighthouse_fixture_only".into(),
        detail: "synthetic offline reports validate import contract, not live performance".into(),
    });
    let validated = validate_output(&pack.context, "product_web", output)
        .expect("Phase 3 structural validator");
    let snapshot = build_snapshot(
        &pack.context,
        &pack.manifest,
        vec![validated],
        Complexity::default(),
    )
    .expect("Phase 3 frozen manifest and artifact validator");
    assert_eq!(snapshot.measurements.len(), 7);
    assert!((performance - 82.0).abs() < 0.000_001);
}
