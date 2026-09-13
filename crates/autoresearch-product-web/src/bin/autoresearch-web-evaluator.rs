//! Strict JSONL subprocess wrapper for one frozen local product-web route.

use autoresearch_config::{RepoPath, ValidatedManifest};
use autoresearch_evaluator::{
    EvaluationContext, EvaluationContextSpec, EvaluatorFailure, FailureClass, PROTOCOL_VERSION,
    ProtocolRequest, ProtocolResponse, ProtocolResult, decode_request, encode_response,
};
use autoresearch_product_web::browser::{browser_payload, inspect_declared_route};
use std::fs;
use std::io::{Read, Write};
use std::path::Path;

const MAX_REQUEST_BYTES: u64 = 1_048_576;

fn main() {
    if run().is_err() {
        eprintln!("product-web evaluator request rejected");
        std::process::exit(2);
    }
}

fn run() -> Result<(), ()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let (route, manifest_path) = match args.as_slice() {
        [flag, route] if flag == "--route" => (route.as_str(), "autoresearch.toml"),
        [route_flag, route, manifest_flag, path]
            if route_flag == "--route" && manifest_flag == "--manifest" =>
        {
            (route.as_str(), path.as_str())
        }
        _ => return Err(()),
    };
    let mut input = Vec::new();
    std::io::stdin()
        .take(MAX_REQUEST_BYTES + 1)
        .read_to_end(&mut input)
        .map_err(|_| ())?;
    if u64::try_from(input.len()).map_err(|_| ())? > MAX_REQUEST_BYTES {
        return Err(());
    }
    let request = decode_request(&input).map_err(|_| ())?;
    let result = evaluate_request(request, route, manifest_path);
    let response = ProtocolResponse {
        protocol_version: PROTOCOL_VERSION,
        result,
    };
    let encoded = encode_response(&response).map_err(|_| ())?;
    std::io::stdout().write_all(&encoded).map_err(|_| ())
}

fn evaluate_request(request: ProtocolRequest, route: &str, manifest_path: &str) -> ProtocolResult {
    match evaluate_route(request, route, manifest_path) {
        Ok(output) => ProtocolResult::Success { output },
        Err(failure) => ProtocolResult::Failure { failure },
    }
}

fn evaluate_route(
    request: ProtocolRequest,
    route: &str,
    manifest_path: &str,
) -> Result<autoresearch_evaluator::EvaluatorOutput, EvaluatorFailure> {
    let context = EvaluationContext::new(EvaluationContextSpec {
        run_id: request.run_id,
        baseline_commit: request.baseline_commit,
        evaluated_commit: request.evaluated_commit,
        candidate_worktree: request.candidate_worktree,
        changed_paths: request.changed_paths,
        declared_environment: request.declared_environment,
        artifact_directory: request.artifact_directory,
        cancellation_id: request.cancellation_id,
    })
    .map_err(|_| failure(FailureClass::Validation, "invalid evaluator context"))?;
    let relative = RepoPath::new(manifest_path)
        .map_err(|_| failure(FailureClass::Validation, "unsafe manifest path"))?;
    let path = context.candidate_worktree().join(relative.as_str());
    let canonical = fs::canonicalize(&path)
        .map_err(|_| failure(FailureClass::Validation, "manifest unavailable"))?;
    if path != canonical || !path.starts_with(context.candidate_worktree()) || !path.is_file() {
        return Err(failure(
            FailureClass::Validation,
            "manifest escaped candidate",
        ));
    }
    let source = fs::read_to_string(path)
        .map_err(|_| failure(FailureClass::Validation, "manifest unreadable"))?;
    let manifest = ValidatedManifest::parse(&source)
        .map_err(|_| failure(FailureClass::Validation, "manifest invalid"))?;
    if !manifest
        .evaluators()
        .iter()
        .any(|evaluator| evaluator.id() == request.evaluator_id)
    {
        return Err(failure(FailureClass::Validation, "evaluator undeclared"));
    }
    let chromium = context
        .declared_environment()
        .get("CHROMIUM_PATH")
        .ok_or_else(|| failure(FailureClass::Reported, "Chromium path unavailable"))?;
    let evidence = inspect_declared_route(&context, &manifest, route, Path::new(chromium))
        .map_err(|_| failure(FailureClass::Reported, "browser route unavailable"))?;
    Ok(browser_payload(&context, &request.evaluator_id, &evidence))
}

fn failure(class: FailureClass, detail: &'static str) -> EvaluatorFailure {
    EvaluatorFailure {
        class,
        detail: detail.into(),
    }
}
