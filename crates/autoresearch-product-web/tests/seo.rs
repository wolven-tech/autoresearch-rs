//! Disposable loopback route fixtures for technical SEO checks.

use autoresearch_config::ValidatedManifest;
use autoresearch_evaluator::{EvaluationContext, EvaluationContextSpec};
use autoresearch_product_web::seo::{SeoError, inspect_local_route};
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
    CanonicalMismatch,
    MissingCanonical,
    Noindex,
    MissingSitemapUrl,
    MalformedJsonLd,
    Redirect,
    BadStatus,
    OffOriginRedirect,
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
                    Ok((mut stream, _)) => serve(&mut stream, port, scenario),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(std::time::Duration::from_millis(10));
                    }
                    Err(error) => panic!("SEO fixture server: {error}"),
                }
            }
        });
        let root = fs::canonicalize(std::env::temp_dir())
            .expect("temp root")
            .join(format!(
                "autoresearch-seo-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        let candidate = root.join(".autoresearch/worktrees/run-1/candidate-000001");
        let artifacts = root.join(".autoresearch/runs/run-1/artifacts");
        fs::create_dir_all(&candidate).expect("candidate");
        fs::create_dir_all(&artifacts).expect("artifacts");
        let mut source = include_str!("../../../examples/product-web/autoresearch.toml")
            .replace("127.0.0.1:4419", &format!("127.0.0.1:{port}"));
        if matches!(scenario, Scenario::Redirect | Scenario::OffOriginRedirect) {
            source = source.replace(
                "[web.thresholds]",
                "[[web.routes]]\nname = \"redirect\"\npath = \"/redirect\"\nexpected_status = 302\nindexable = true\n\n[web.thresholds]",
            );
        }
        let manifest = ValidatedManifest::parse(&source).expect("SEO manifest");
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

fn serve(stream: &mut std::net::TcpStream, port: u16, scenario: Scenario) {
    let mut request = [0_u8; 4096];
    let count = stream.read(&mut request).expect("request");
    let text = String::from_utf8_lossy(&request[..count]);
    let path = text.split_whitespace().nth(1).unwrap_or("/");
    let origin = format!("http://127.0.0.1:{port}");
    let (status, location, body) = match path {
        "/robots.txt" => ("200 OK", "", format!("User-agent: *\nAllow: /\nSitemap: {origin}/sitemap.xml\n")),
        "/sitemap.xml" => {
            let home = if matches!(scenario, Scenario::MissingSitemapUrl) { String::new() } else { format!("<url><loc>{origin}/</loc></url>") };
            ("200 OK", "", format!("<urlset>{home}<url><loc>{origin}/metadata</loc></url></urlset>"))
        }
        "/redirect" if matches!(scenario, Scenario::OffOriginRedirect) => ("302 Found", "Location: https://example.invalid/\r\n", String::new()),
        "/redirect" => ("302 Found", "Location: /metadata\r\n", String::new()),
        "/missing" => ("404 Not Found", "", "<html><head><meta name=\"robots\" content=\"noindex\"></head><body>Missing</body></html>".into()),
        "/" | "/metadata" => {
            let status = if matches!(scenario, Scenario::BadStatus) { "503 Service Unavailable" } else { "200 OK" };
            let canonical = if matches!(scenario, Scenario::CanonicalMismatch) { format!("{origin}/wrong") } else { format!("{origin}{path}") };
            let canonical_tag = if matches!(scenario, Scenario::MissingCanonical) { String::new() } else { format!("<link rel=\"canonical\" href=\"{canonical}\">") };
            let robots = if matches!(scenario, Scenario::Noindex) { "noindex" } else { "index, follow" };
            let json_ld = if matches!(scenario, Scenario::MalformedJsonLd) { "{invalid" } else { "{\"@type\":\"WebPage\"}" };
            (status, "", format!("<!doctype html><html lang=\"en\"><head><title>SEO fixture</title>{canonical_tag}<meta name=\"robots\" content=\"{robots}\"><script type=\"application/ld+json\">{json_ld}</script></head><body><h1>SEO fixture</h1></body></html>"))
        }
        _ => ("404 Not Found", "", String::new()),
    };
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\n{location}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).expect("response");
}

#[test]
fn declared_good_route_records_source_artifact_without_ranking_claim() {
    let fixture = Fixture::new(Scenario::Good);
    let evidence =
        inspect_local_route(&fixture.context, &fixture.manifest, "home").expect("good SEO route");
    assert_eq!(evidence.hops.len(), 1);
    assert_eq!(evidence.hops[0].status, 200);
    assert!(evidence.issues.is_empty(), "{evidence:?}");
    assert_eq!(evidence.json_ld_blocks, 1);
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
fn fixture_rules_detect_canonical_noindex_sitemap_jsonld_and_status() {
    for (scenario, rule) in [
        (Scenario::CanonicalMismatch, "canonical_mismatch"),
        (Scenario::MissingCanonical, "canonical_mismatch"),
        (Scenario::Noindex, "unexpected_noindex"),
        (Scenario::MissingSitemapUrl, "sitemap_url_missing"),
        (Scenario::MalformedJsonLd, "malformed_json_ld"),
        (Scenario::BadStatus, "http_status"),
    ] {
        let fixture = Fixture::new(scenario);
        let evidence = inspect_local_route(&fixture.context, &fixture.manifest, "home")
            .expect("technical issue recorded");
        let issue = evidence
            .issues
            .iter()
            .find(|issue| issue.rule_id == rule)
            .expect("expected rule");
        assert_eq!(issue.route, "home");
        assert_eq!(issue.source_url, evidence.source_url);
        assert_eq!(issue.source_artifact, evidence.artifact_relative_path);
    }
}

#[test]
fn declared_redirect_chain_is_recorded_and_off_origin_redirect_is_rejected() {
    let fixture = Fixture::new(Scenario::Redirect);
    let evidence = inspect_local_route(&fixture.context, &fixture.manifest, "redirect")
        .expect("declared local redirect");
    assert_eq!(
        evidence
            .hops
            .iter()
            .map(|hop| hop.status)
            .collect::<Vec<_>>(),
        [302, 200]
    );
    assert!(evidence.final_url.ends_with("/metadata"));
    let unsafe_fixture = Fixture::new(Scenario::OffOriginRedirect);
    assert!(matches!(
        inspect_local_route(
            &unsafe_fixture.context,
            &unsafe_fixture.manifest,
            "redirect"
        ),
        Err(SeoError::UnsafeRedirect)
    ));
}
