//! Standalone process fixture compiled by process integration tests.

use std::io::{Read, Write};
use std::time::Duration;

fn field(request: &str, key: &str) -> String {
    let prefix = format!("\"{key}\":\"");
    let value = request.split(&prefix).nth(1).expect("request field");
    value.split('"').next().expect("field value").to_owned()
}

fn main() {
    let mut request = String::new();
    std::io::stdin().read_to_string(&mut request).expect("request input");
    let mode = std::env::args().nth(1).expect("fixture mode");
    match mode.as_str() {
        "nonzero" => {
            eprintln!("SECRET_TOKEN=do-not-leak");
            std::process::exit(7);
        }
        "hang" => std::thread::sleep(Duration::from_secs(3)),
        "malformed" => {
            println!("debugging evaluator");
            println!("{{}}");
            return;
        }
        "stdout-overflow" => {
            std::io::stdout().write_all(&vec![b'x'; 32_768]).expect("write stdout");
            return;
        }
        "stderr-overflow" => {
            std::io::stderr().write_all(&vec![b'x'; 32_768]).expect("write stderr");
        }
        "stderr" => eprintln!("safe diagnostic"),
        "env" => {
            if std::env::var("ALLOW_ME").as_deref() != Ok("ok")
                || std::env::var("HOME").is_ok()
                || std::env::var("SECRET_TOKEN").is_ok()
            {
                std::process::exit(9);
            }
        }
        "raw-success" => {
            println!("check passed");
            return;
        }
        "raw-extra" => {
            println!("ordinary tool output");
            println!("second output line");
            return;
        }
        "raw-fail" => {
            eprintln!("check failed: SECRET_TOKEN=do-not-leak");
            std::process::exit(7);
        }
        "raw-env" => {
            if std::env::var("ALLOW_ME").as_deref() != Ok("ok")
                || std::env::var("HOME").is_ok()
            {
                std::process::exit(9);
            }
            println!("environment scoped");
            return;
        }
        "raw-cwd" => {
            if !std::env::current_dir()
                .expect("working directory")
                .ends_with("candidate-000001")
            {
                std::process::exit(10);
            }
            println!("candidate only");
            return;
        }
        "literal" => {
            if std::env::args().nth(2).as_deref() != Some("$(touch /tmp/never-run)") {
                std::process::exit(11);
            }
        }
        "success" | "json-numeric" => {}
        _ => std::process::exit(12),
    }

    let numeric = if mode == "json-numeric" {
        ",{\"kind\":\"numeric\",\"name\":\"score\",\"metric_kind\":\"objective\",\"direction\":\"maximize\",\"value\":1.25}"
    } else {
        ""
    };
    let response = format!(
        "{{\"protocol_version\":1,\"result\":{{\"status\":\"success\",\"output\":{{\"evaluator_id\":\"{}\",\"run_id\":\"{}\",\"baseline_commit\":\"{}\",\"evaluated_commit\":\"{}\",\"measurements\":[{{\"kind\":\"hard_gate\",\"name\":\"tests\",\"outcome\":{{\"passed\":true,\"detail\":null}}}}{numeric}],\"observations\":[],\"artifacts\":[],\"warnings\":[]}}}}}}",
        field(&request, "evaluator_id"),
        field(&request, "run_id"),
        field(&request, "baseline_commit"),
        field(&request, "evaluated_commit"),
    );
    println!("{response}");
}
