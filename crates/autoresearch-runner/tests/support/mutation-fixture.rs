//! Rust fixture executable for bounded mutation-process integration tests.

use std::fs;
use std::io::{Read, Write};
use std::time::Duration;

fn main() {
    let mut request = String::new();
    std::io::stdin()
        .read_to_string(&mut request)
        .expect("read mutation JSON");
    let payload: serde_json::Value = serde_json::from_str(&request).expect("request JSON");
    assert!(payload.get("candidate_worktree").is_some());
    let mode = std::env::args().nth(1).expect("mode");
    match mode.as_str() {
        "success" => fs::write("tracked.txt", "changed by command\n").expect("write candidate"),
        "exit" => std::process::exit(7),
        "sleep" => std::thread::sleep(Duration::from_secs(5)),
        "env" => {
            if std::env::var_os("HOME").is_some() {
                std::process::exit(8);
            }
            fs::write("tracked.txt", "no ambient home\n").expect("write candidate");
        }
        "protected" => fs::write("program.md", "changed prompt\n").expect("write protected"),
        "overflow" => {
            let bytes = vec![b'x'; 65_536];
            std::io::stdout()
                .write_all(&bytes)
                .expect("write excess output");
        }
        other => panic!("unknown mode: {other}"),
    }
}
