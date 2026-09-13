//! Local rendered-browser probe against disposable candidate-file fixture.

use autoresearch_config::ValidatedManifest;
use autoresearch_core::Measurement;
use autoresearch_evaluator::{EvaluationContext, EvaluationContextSpec};
use autoresearch_product_web::browser::inspect_declared_route;
use autoresearch_product_web::browser::{BrowserError, browser_output, inspect_local_page};
use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread::{self, JoinHandle};
use url::Url;

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);
const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

struct Fixture {
    root: PathBuf,
    context: EvaluationContext,
}

impl Fixture {
    fn new() -> Self {
        let base = fs::canonicalize(std::env::temp_dir()).expect("temp root");
        let root = base.join(format!(
            "autoresearch-web-test-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        let candidate = root.join(".autoresearch/worktrees/run-1/candidate-000001");
        let artifacts = root.join(".autoresearch/runs/run-1/artifacts");
        fs::create_dir_all(&candidate).expect("candidate");
        fs::create_dir_all(&artifacts).expect("artifacts");
        fs::write(candidate.join("index.html"), "<!doctype html><html lang=\"en\"><head><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>Fixture page</title><meta name=\"description\" content=\"Local product fixture\"></head><body><main><h1>Fixture page</h1><button aria-label=\"Run check\">Run</button><img src=\"https://example.invalid/pixel.png\" alt=\"\"></main></body></html>").expect("fixture HTML");
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
        Self { root, context }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).expect("remove owned fixture");
    }
}

fn chromium() -> Option<PathBuf> {
    let configured = std::env::var_os("CHROMIUM_PATH").map(PathBuf::from);
    configured.or_else(|| {
        std::env::split_paths(&std::env::var_os("PATH")?)
            .map(|path| path.join("chromium"))
            .find(|path| path.is_file())
    })
}

struct LocalServer {
    port: u16,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl LocalServer {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("loopback fixture listener");
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
                            .expect("blocking fixture stream");
                        stream
                            .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                            .expect("bounded fixture read");
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
                        let request = String::from_utf8_lossy(&request[..count]);
                        let missing = request.starts_with("GET /missing ");
                        let status = if missing { "404 Not Found" } else { "200 OK" };
                        let body = if missing {
                            "<!doctype html><html lang=\"en\"><title>Missing</title><h1>Missing</h1></html>"
                        } else {
                            "<!doctype html><html lang=\"en\"><head><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>Live fixture</title></head><body><h1>Live fixture</h1><script>console.error('fixture error')</script></body></html>"
                        };
                        let response = format!(
                            "HTTP/1.1 {status}\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        );
                        stream.write_all(response.as_bytes()).expect("response");
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(std::time::Duration::from_millis(10));
                    }
                    Err(error) => panic!("fixture listener: {error}"),
                }
            }
        });
        Self {
            port,
            stop,
            worker: Some(worker),
        }
    }

    fn manifest(&self) -> ValidatedManifest {
        let source = include_str!("../../../examples/product-web/autoresearch.toml")
            .replace("127.0.0.1:4419", &format!("127.0.0.1:{}", self.port));
        ValidatedManifest::parse(&source).expect("local web manifest")
    }
}

impl Drop for LocalServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            worker.join().expect("fixture worker");
        }
    }
}

#[test]
fn rendered_local_page_records_real_viewport_and_screenshot() {
    let Some(chromium) = chromium() else {
        return;
    };
    let fixture = Fixture::new();
    let target = Url::from_file_path(fixture.context.candidate_worktree().join("index.html"))
        .expect("file URL");
    let evidence = inspect_local_page(
        &fixture.context,
        target.as_str(),
        &chromium,
        &[320, 390, 768, 1280],
    )
    .expect("rendered fixture");
    assert_eq!(evidence.title, "Fixture page");
    assert_eq!(evidence.language, "en");
    assert_eq!(evidence.h1_count, 1);
    assert_eq!(evidence.viewports.len(), 4);
    let output =
        browser_output(&fixture.context, "browser", &evidence).expect("valid browser output");
    assert_eq!(output.measurements().len(), 5);
    assert_eq!(output.artifacts().len(), 4);
    assert!(output.measurements().iter().any(|measurement| {
        matches!(measurement, Measurement::HardGate { name, outcome }
            if name == "browser_local_only" && !outcome.passed())
    }));
    for (viewport, expected_width) in evidence.viewports.iter().zip([320, 390, 768, 1280]) {
        assert_eq!(viewport.requested_width, expected_width);
        assert_eq!(viewport.observed_width, expected_width);
        assert!(viewport.document_width <= viewport.observed_width);
        assert!(viewport.reduced_motion_requested);
        assert!(viewport.reduced_motion_observed);
        assert!(viewport.unnamed_controls.is_empty());
        assert!(viewport.blocked_external_network_requests >= 1);
        assert_eq!(
            viewport.screenshot_relative_path,
            format!("browser/viewport-{expected_width}.png")
        );
        let screenshot = fixture
            .context
            .artifact_directory()
            .join(&viewport.screenshot_relative_path);
        assert!(fs::metadata(screenshot).expect("screenshot").len() > 0);
    }
    assert_eq!(
        output
            .artifacts()
            .iter()
            .map(|artifact| artifact.name.as_str())
            .collect::<Vec<_>>(),
        [
            "viewport_320",
            "viewport_390",
            "viewport_768",
            "viewport_1280"
        ]
    );
}

#[test]
fn browser_rejects_remote_targets_before_launch() {
    let fixture = Fixture::new();
    assert!(matches!(
        inspect_local_page(
            &fixture.context,
            "https://example.com",
            "/nonexistent".as_ref(),
            &[390]
        ),
        Err(BrowserError::NonLocalTarget)
    ));
}

#[test]
fn rendered_probe_detects_overflow_and_unnamed_button() {
    let Some(chromium) = chromium() else {
        return;
    };
    let fixture = Fixture::new();
    fs::write(
        fixture.context.candidate_worktree().join("index.html"),
        "<!doctype html><html lang=\"en\"><head><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>Overflow</title></head><body><div style=\"width:700px;height:40px\">Wide</div><button id=\"unnamed\"><svg width=\"16\" height=\"16\"></svg></button></body></html>",
    )
    .expect("overflow fixture");
    let target = Url::from_file_path(fixture.context.candidate_worktree().join("index.html"))
        .expect("file URL");
    let evidence = inspect_local_page(&fixture.context, target.as_str(), &chromium, &[320])
        .expect("rendered overflow fixture");
    let viewport = &evidence.viewports[0];
    assert!(
        viewport.document_width > viewport.requested_width,
        "{viewport:?}"
    );
    assert!(!viewport.overflow_elements.is_empty());
    assert!(viewport.unnamed_controls.contains(&"button#unnamed".into()));
    let output =
        browser_output(&fixture.context, "browser", &evidence).expect("valid browser output");
    assert!(output.measurements().iter().any(|measurement| {
        matches!(measurement, Measurement::HardGate { name, outcome }
            if name == "browser_no_overflow" && !outcome.passed())
    }));
    assert!(output.measurements().iter().any(|measurement| {
        matches!(measurement, Measurement::HardGate { name, outcome }
            if name == "browser_controls_named" && !outcome.passed())
    }));
}

#[test]
fn declared_route_records_http_status_console_errors_and_frozen_name() {
    let Some(chromium) = chromium() else {
        return;
    };
    let fixture = Fixture::new();
    let server = LocalServer::start();
    let manifest = server.manifest();
    let evidence = inspect_declared_route(&fixture.context, &manifest, "home", &chromium)
        .expect("declared local route");
    assert_eq!(evidence.route_name.as_deref(), Some("home"));
    assert_eq!(evidence.expected_http_status, Some(200));
    assert_eq!(evidence.viewports.len(), 4);
    for viewport in &evidence.viewports {
        assert_eq!(viewport.http_status, Some(200), "{viewport:?}");
        assert!(viewport.console_errors >= 1, "{viewport:?}");
    }
    let output = browser_output(&fixture.context, "product_web", &evidence).expect("typed output");
    assert_eq!(output.measurements().len(), 6);
    assert!(output.measurements().iter().any(|measurement| {
        matches!(measurement, Measurement::HardGate { name, outcome }
            if name == "browser_route_status" && outcome.passed())
    }));
    assert!(matches!(
        inspect_declared_route(&fixture.context, &manifest, "undeclared", &chromium),
        Err(BrowserError::UnknownRoute)
    ));
    assert!(matches!(
        inspect_declared_route(&fixture.context, &manifest, "home", "/nonexistent".as_ref()),
        Err(BrowserError::ChromiumUnavailable)
    ));
}

#[test]
fn unreachable_declared_route_cannot_pass_http_gate() {
    let Some(chromium) = chromium() else {
        return;
    };
    let closed = TcpListener::bind("127.0.0.1:0").expect("temporary listener");
    let port = closed.local_addr().expect("address").port();
    drop(closed);
    let source = include_str!("../../../examples/product-web/autoresearch.toml")
        .replace("127.0.0.1:4419", &format!("127.0.0.1:{port}"));
    let manifest = ValidatedManifest::parse(&source).expect("manifest");
    let fixture = Fixture::new();
    match inspect_declared_route(&fixture.context, &manifest, "home", &chromium) {
        Err(BrowserError::Navigation | BrowserError::Inspection) => {}
        Ok(evidence) => {
            let output = browser_output(&fixture.context, "browser", &evidence)
                .expect("typed failed status gate");
            assert!(output.measurements().iter().any(|measurement| {
                matches!(measurement, Measurement::HardGate { name, outcome }
                    if name == "browser_route_status" && !outcome.passed())
            }));
        }
        Err(error) => panic!("unexpected navigation classification: {error}"),
    }
}

#[test]
fn accessibility_probe_separates_visible_focus_and_known_contrast_failure() {
    let Some(chromium) = chromium() else {
        return;
    };
    let fixture = Fixture::new();
    fs::write(
        fixture.context.candidate_worktree().join("index.html"),
        "<!doctype html><html lang=\"en\"><head><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>Accessibility fixture</title><style>body{background:#fff;color:#111}button:focus-visible{outline:3px solid #003399}#low{color:#aaa;background:#fff}</style></head><body><h1>Accessibility fixture</h1><button id=\"primary\">Run check</button><p id=\"low\">Low contrast sample</p></body></html>",
    )
    .expect("accessibility fixture");
    let target = Url::from_file_path(fixture.context.candidate_worktree().join("index.html"))
        .expect("file URL");
    let evidence = inspect_local_page(&fixture.context, target.as_str(), &chromium, &[390])
        .expect("accessibility probe");
    let accessibility = &evidence.viewports[0].accessibility;
    assert_eq!(accessibility.focusable_count, 1);
    assert_eq!(accessibility.tab_targets, ["button#primary"]);
    assert_eq!(accessibility.visible_focus_count, 1);
    assert_eq!(accessibility.keyboard_status(), "pass");
    assert_eq!(accessibility.contrast_status(), "fail");
    assert!(
        accessibility
            .contrast_failures
            .iter()
            .any(|item| item.starts_with("p#low ratio="))
    );
    let output = browser_output(&fixture.context, "browser", &evidence).expect("diagnostic output");
    assert!(output.observations().iter().any(|observation| {
        observation.code == "a11y_contrast_390"
            && observation.detail.contains("status=fail")
            && observation
                .detail
                .contains("artifact=browser/viewport-390.png")
    }));
}

#[test]
fn accessibility_probe_marks_missing_focus_indicator_as_failure() {
    let Some(chromium) = chromium() else {
        return;
    };
    let fixture = Fixture::new();
    fs::write(
        fixture.context.candidate_worktree().join("index.html"),
        "<!doctype html><html lang=\"en\"><head><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>Focus fixture</title><style>button:focus,button:focus-visible{outline:none;box-shadow:none}</style></head><body><h1>Focus fixture</h1><button id=\"missing-focus\">Continue</button></body></html>",
    )
    .expect("focus fixture");
    let target = Url::from_file_path(fixture.context.candidate_worktree().join("index.html"))
        .expect("file URL");
    let evidence = inspect_local_page(&fixture.context, target.as_str(), &chromium, &[390])
        .expect("focus probe");
    assert_eq!(
        evidence.viewports[0].accessibility.keyboard_status(),
        "fail"
    );
    assert_eq!(evidence.viewports[0].accessibility.visible_focus_count, 0);
}
