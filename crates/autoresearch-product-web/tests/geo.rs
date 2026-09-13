//! Local-only GEO diagnostic fixtures; never live search evidence.

use autoresearch_config::ValidatedManifest;
use autoresearch_evaluator::{EvaluationContext, EvaluationContextSpec};
use autoresearch_product_web::geo::inspect_local_geo_route;
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

#[derive(Clone, Copy)]
enum Scenario {
    Good,
    EntityConflict,
    Uncited,
    MissingReference,
    FactConflict,
}

struct Fixture {
    root: PathBuf,
    context: EvaluationContext,
    manifest: ValidatedManifest,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl Fixture {
    fn new(scenario: Scenario) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("loopback server");
        let port = listener.local_addr().expect("address").port();
        listener.set_nonblocking(true).expect("nonblocking");
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let worker = thread::spawn(move || {
            while !worker_stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => serve(&mut stream, scenario),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(std::time::Duration::from_millis(10));
                    }
                    Err(error) => panic!("GEO fixture server: {error}"),
                }
            }
        });
        let root = fs::canonicalize(std::env::temp_dir())
            .expect("temp root")
            .join(format!(
                "autoresearch-geo-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        let candidate = root.join(".autoresearch/worktrees/run-1/candidate-000001");
        let artifacts = root.join(".autoresearch/runs/run-1/artifacts");
        fs::create_dir_all(&candidate).expect("candidate");
        fs::create_dir_all(&artifacts).expect("artifacts");
        let source = include_str!("../../../examples/product-web/autoresearch.toml")
            .replace("127.0.0.1:4419", &format!("127.0.0.1:{port}"));
        let manifest = ValidatedManifest::parse(&source).expect("GEO manifest");
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
            stop,
            worker: Some(worker),
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            worker.join().expect("server worker");
        }
        fs::remove_dir_all(&self.root).expect("owned fixture cleanup");
    }
}

fn serve(stream: &mut std::net::TcpStream, scenario: Scenario) {
    let mut request = [0_u8; 4096];
    let count = stream.read(&mut request).expect("request");
    let text = String::from_utf8_lossy(&request[..count]);
    let path = text.split_whitespace().nth(1).unwrap_or("/");
    let entity = if matches!(scenario, Scenario::EntityConflict) {
        "Other fixture"
    } else {
        "Product-web fixture"
    };
    let price = if matches!(scenario, Scenario::FactConflict) {
        "Remote checks"
    } else {
        "Local-only checks"
    };
    let source_ref = if matches!(scenario, Scenario::Uncited) {
        ""
    } else {
        "data-geo-source-ref=\"fixture-method\""
    };
    let source = if matches!(scenario, Scenario::MissingReference) {
        ""
    } else {
        "<a id=\"fixture-method\" href=\"/metadata\">Source</a>"
    };
    let body = if path == "/" {
        format!(
            "<!doctype html><html><head><meta property=\"og:site_name\" content=\"{entity}\"></head><body><article data-geo-passage=\"operation\"><p data-geo-fact=\"operation\">{price}</p><p data-geo-claim {source_ref}>Responsive layout at frozen widths.</p>{source}</article><article data-geo-passage=\"evidence\"><p data-geo-fact=\"evidence\">Lab evidence, not market receipts</p></article></body></html>"
        )
    } else {
        "<html><body>Metadata</body></html>".into()
    };
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).expect("response");
}

#[test]
fn source_grounded_local_passages_create_bounded_artifact() {
    let fixture = Fixture::new(Scenario::Good);
    let evidence =
        inspect_local_geo_route(&fixture.context, &fixture.manifest, "home").expect("GEO evidence");
    assert!(evidence.issues.is_empty(), "{evidence:?}");
    assert_eq!(evidence.entity_names, ["Product-web fixture"]);
    assert_eq!(evidence.passages.len(), 2);
    assert!(evidence.passages[0].excerpt.contains("Responsive layout"));
    assert_eq!(evidence.passages[0].source_links, ["/metadata"]);
    assert!(
        fs::metadata(
            fixture
                .context
                .artifact_directory()
                .join(&evidence.artifact_relative_path)
        )
        .expect("artifact")
        .len()
            > 0
    );
}

#[test]
fn entity_claim_source_and_fact_defects_keep_source_passage_and_rule() {
    for (scenario, rule) in [
        (Scenario::EntityConflict, "entity_name_conflict"),
        (Scenario::Uncited, "uncited_claim"),
        (Scenario::MissingReference, "missing_source_reference"),
        (Scenario::FactConflict, "product_fact_conflict"),
    ] {
        let fixture = Fixture::new(scenario);
        let evidence = inspect_local_geo_route(&fixture.context, &fixture.manifest, "home")
            .expect("GEO evidence");
        let issue = evidence
            .issues
            .iter()
            .find(|issue| issue.rule_id == rule)
            .expect("expected rule");
        assert_eq!(issue.route, "home");
        assert_eq!(issue.source_url, evidence.source_url);
        assert!(!issue.source_passage.is_empty());
        assert_eq!(issue.source_artifact, evidence.artifact_relative_path);
    }
}
