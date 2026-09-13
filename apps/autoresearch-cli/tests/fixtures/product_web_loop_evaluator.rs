//! Disposable source-backed product-web loop. Fixture score is paragraph count,
//! not a live performance, accessibility, SEO, or market measurement.

use std::io::Read;

fn field(request: &str, key: &str) -> String {
    let prefix = format!("\"{key}\":\"");
    request
        .split(&prefix)
        .nth(1)
        .expect("request field")
        .split('"')
        .next()
        .expect("field value")
        .into()
}

fn main() {
    let mut request = String::new();
    std::io::stdin()
        .read_to_string(&mut request)
        .expect("read protocol request");
    let source = std::fs::read_to_string(format!(
        "{}/site/index.html",
        field(&request, "candidate_worktree")
    ))
    .expect("fixture product page");
    let cta_present = source.contains("<a href=\"/start\">Start</a>");
    let score = source.matches("<p>").count();
    println!(
        "{{\"protocol_version\":1,\"result\":{{\"status\":\"success\",\"output\":{{\"evaluator_id\":\"{}\",\"run_id\":\"{}\",\"baseline_commit\":\"{}\",\"evaluated_commit\":\"{}\",\"measurements\":[{{\"kind\":\"hard_gate\",\"name\":\"cta_present\",\"outcome\":{{\"passed\":{cta_present},\"detail\":null}}}},{{\"kind\":\"numeric\",\"name\":\"fixture_content_items\",\"metric_kind\":\"objective\",\"direction\":\"maximize\",\"value\":{score}}}],\"observations\":[],\"artifacts\":[],\"warnings\":[]}}}}}}",
        field(&request, "evaluator_id"),
        field(&request, "run_id"),
        field(&request, "baseline_commit"),
        field(&request, "evaluated_commit"),
    );
}
