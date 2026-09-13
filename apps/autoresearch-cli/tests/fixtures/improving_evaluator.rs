//! Deterministic JSONL evaluator for kept-commit verification tests.

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
        .expect("read request");
    let baseline = field(&request, "baseline_commit");
    let evaluated = field(&request, "evaluated_commit");
    let score = if baseline == evaluated { 1 } else { 2 };
    println!(
        "{{\"protocol_version\":1,\"result\":{{\"status\":\"success\",\"output\":{{\"evaluator_id\":\"{}\",\"run_id\":\"{}\",\"baseline_commit\":\"{}\",\"evaluated_commit\":\"{}\",\"measurements\":[{{\"kind\":\"hard_gate\",\"name\":\"tests\",\"outcome\":{{\"passed\":true,\"detail\":null}}}},{{\"kind\":\"numeric\",\"name\":\"score\",\"metric_kind\":\"objective\",\"direction\":\"maximize\",\"value\":{score}}}],\"observations\":[],\"artifacts\":[],\"warnings\":[]}}}}}}",
        field(&request, "evaluator_id"),
        field(&request, "run_id"),
        baseline,
        evaluated,
    );
}
