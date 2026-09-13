//! End-to-end JSONL browser adapter through Phase 3 subprocess policy.

use autoresearch_config::ValidatedManifest;
use autoresearch_core::Measurement;
use autoresearch_evaluator::{
    CancellationToken, EvaluationContext, EvaluationContextSpec, FailureClass, ProcessLimits,
    evaluate_subprocess,
};
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

fn chromium() -> Option<PathBuf> {
    std::env::var_os("CHROMIUM_PATH")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::split_paths(&std::env::var_os("PATH")?)
                .map(|path| path.join("chromium"))
                .find(|path| path.is_file())
        })
}

fn start_server() -> (u16, Arc<AtomicBool>, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("loopback listener");
    let port = listener.local_addr().expect("listener address").port();
    listener
        .set_nonblocking(true)
        .expect("nonblocking listener");
    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = Arc::clone(&stop);
    let worker = thread::spawn(move || {
        while !worker_stop.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let mut request = [0_u8; 4096];
                    let _ = stream.read(&mut request);
                    let body = "<!doctype html><html lang=\"en\"><head><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>Process fixture</title></head><body><h1>Process fixture</h1><script>console.error('fixture')</script></body></html>";
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    stream
                        .write_all(response.as_bytes())
                        .expect("fixture response");
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(error) => panic!("fixture listener: {error}"),
            }
        }
    });
    (port, stop, worker)
}

#[test]
fn declared_browser_subprocess_returns_one_validated_jsonl_response() {
    let Some(chromium) = chromium() else {
        return;
    };
    let (port, stop, worker) = start_server();

    let root = fs::canonicalize(std::env::temp_dir())
        .expect("temp root")
        .join(format!(
            "autoresearch-web-process-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
    let candidate = root.join(".autoresearch/worktrees/run-1/candidate-000001");
    let artifacts = root.join(".autoresearch/runs/run-1/artifacts");
    fs::create_dir_all(candidate.join("examples/product-web")).expect("candidate");
    fs::create_dir_all(&artifacts).expect("artifacts");
    let binary = env!("CARGO_BIN_EXE_autoresearch-web-evaluator");
    let source = include_str!("../../../examples/product-web/autoresearch.toml")
        .replace("127.0.0.1:4419", &format!("127.0.0.1:{port}"))
        .replace(
            "program = \"autoresearch-web-evaluator\"",
            &format!("program = \"{binary}\""),
        );
    fs::write(
        candidate.join("examples/product-web/autoresearch.toml"),
        &source,
    )
    .expect("candidate manifest");
    let manifest = ValidatedManifest::parse(&source).expect("manifest");
    let context = EvaluationContext::new(EvaluationContextSpec {
        run_id: "run-1".into(),
        baseline_commit: COMMIT.into(),
        evaluated_commit: COMMIT.into(),
        candidate_worktree: candidate,
        changed_paths: vec!["examples/product-web/index.html".into()],
        declared_environment: BTreeMap::from([(
            "CHROMIUM_PATH".into(),
            chromium.to_string_lossy().into_owned(),
        )]),
        artifact_directory: artifacts.clone(),
        cancellation_id: "cancel-1".into(),
    })
    .expect("context");
    let limits = ProcessLimits::new(64 * 1024, 8 * 1024).expect("limits");
    let result = evaluate_subprocess(
        &context,
        &manifest.evaluators()[0],
        limits,
        &CancellationToken::default(),
    )
    .expect("browser subprocess");
    assert_eq!(result.output.artifacts().len(), 4);
    assert_eq!(result.output.observations().len(), 4);
    assert!(result.output.observations().iter().all(|observation| {
        observation.detail.contains("http_status=Some(200)")
            && observation.detail.contains("console_errors=1")
    }));
    assert!(result.output.measurements().iter().any(|measurement| {
        matches!(measurement, Measurement::HardGate { name, outcome }
            if name == "browser_route_status" && outcome.passed())
    }));
    for artifact in result.output.artifacts() {
        assert!(
            fs::metadata(artifacts.join(&artifact.relative_path))
                .expect("screenshot")
                .len()
                > 0
        );
    }
    let missing = EvaluationContext::new(EvaluationContextSpec {
        run_id: "run-1".into(),
        baseline_commit: COMMIT.into(),
        evaluated_commit: COMMIT.into(),
        candidate_worktree: context.candidate_worktree().to_path_buf(),
        changed_paths: vec!["examples/product-web/index.html".into()],
        declared_environment: BTreeMap::new(),
        artifact_directory: artifacts,
        cancellation_id: "cancel-2".into(),
    })
    .expect("missing Chromium context");
    let failure = evaluate_subprocess(
        &missing,
        &manifest.evaluators()[0],
        limits,
        &CancellationToken::default(),
    )
    .expect_err("missing Chromium cannot pass");
    assert_eq!(failure.failure.class, FailureClass::Reported);
    assert!(!failure.failure.detail.contains("CHROMIUM_PATH="));
    stop.store(true, Ordering::Relaxed);
    worker.join().expect("server worker");
    fs::remove_dir_all(root).expect("owned fixture cleanup");
}
