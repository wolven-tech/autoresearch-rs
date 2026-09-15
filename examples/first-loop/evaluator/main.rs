//! Simple paragraph counter evaluator for the first loop example.
//! Counts <p> elements in site/index.html and checks the CTA link exists.

use std::fs;
use std::io::{self, Read};

fn field(request: &str, key: &str) -> String {
    let prefix = format!("\"{key}\":\"");
    let value = request.split(&prefix).nth(1).unwrap_or("");
    value.split('"').next().unwrap_or("").to_owned()
}

fn main() {
    let mut request = String::new();
    io::stdin().read_to_string(&mut request).expect("read stdin");

    let run_id = field(&request, "run_id");
    let evaluator_id = field(&request, "evaluator_id");
    let baseline_commit = field(&request, "baseline_commit");
    let evaluated_commit = field(&request, "evaluated_commit");
    let candidate_worktree = field(&request, "candidate_worktree");

    let html_path = format!("{}/site/index.html", candidate_worktree);
    // A missing page is a candidate the gate should reject, not an evaluator failure: a failure
    // leaves the candidate active, and every later call retries it.
    let html_content = fs::read_to_string(&html_path).unwrap_or_default();

    let paragraph_count = html_content.matches("<p>").count() as i32;
    let cta_present = html_content.contains("class=\"cta\"");

    let cta_gate = if cta_present {
        "\"outcome\":{\"passed\":true,\"detail\":null}"
    } else {
        "\"outcome\":{\"passed\":false,\"detail\":\"CTA link missing\"}"
    };

    let response = format!(
        "{{\"protocol_version\":1,\"result\":{{\"status\":\"success\",\"output\":{{\"evaluator_id\":\"{}\",\"run_id\":\"{}\",\"baseline_commit\":\"{}\",\"evaluated_commit\":\"{}\",\"measurements\":[{{\"kind\":\"hard_gate\",\"name\":\"cta_present\",{}}},{{\"kind\":\"numeric\",\"name\":\"paragraph_count\",\"metric_kind\":\"objective\",\"direction\":\"minimize\",\"value\":{}}}],\"observations\":[],\"artifacts\":[],\"warnings\":[]}}}}}}",
        evaluator_id, run_id, baseline_commit, evaluated_commit, cta_gate, paragraph_count
    );
    println!("{response}");
}
