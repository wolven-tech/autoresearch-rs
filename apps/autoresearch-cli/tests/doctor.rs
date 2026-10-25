//! Process-level readiness checks with isolated executable lookup and write traps.

#![cfg(unix)]

use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

struct TestRepository {
    directory: PathBuf,
    root: PathBuf,
    bin: PathBuf,
}

impl TestRepository {
    fn new(agent: &str, evaluator: &str) -> Self {
        let git = std::env::split_paths(&std::env::var_os("PATH").expect("fixture PATH"))
            .map(|path| path.join("git"))
            .find(|path| {
                path.metadata().is_ok_and(|metadata| {
                    metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
                })
            })
            .expect("Git executable in fixture PATH")
            .canonicalize()
            .expect("canonical Git executable");
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("test clock")
            .as_nanos();
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let directory = std::env::temp_dir().join(format!(
            "autoresearch-doctor-test-{}-{nanos}-{id}",
            std::process::id()
        ));
        fs::create_dir(&directory).expect("create fixture directory");
        let directory = directory
            .canonicalize()
            .expect("canonical fixture directory");
        let repository = Self {
            root: directory.join("repository"),
            bin: directory.join("bin"),
            directory,
        };
        fs::create_dir(&repository.root).expect("create fixture repository");
        fs::create_dir(&repository.bin).expect("create isolated PATH");
        symlink(git, repository.bin.join("git")).expect("expose only Git in PATH");
        let mut init = Command::new(repository.bin.join("git"));
        init.env_clear()
            .env("PATH", &repository.bin)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .arg("-C")
            .arg(&repository.root)
            .args(["init", "-q", "--template="]);
        let output = run_process(init);
        assert!(output.status.success(), "{output:?}");
        fs::write(repository.root.join(".gitignore"), ".autoresearch/\n")
            .expect("ignore private run state");
        fs::write(
            repository.root.join("program.md"),
            "Improve declared source.\n",
        )
        .expect("write program");
        fs::create_dir(repository.root.join("src")).expect("create source directory");
        fs::write(
            repository.root.join("src/example.rs"),
            "pub fn example() {}\n",
        )
        .expect("write source");
        fs::create_dir_all(repository.root.join(".autoresearch/existing-run"))
            .expect("create existing private run state");
        fs::write(
            repository
                .root
                .join(".autoresearch/existing-run/journal.jsonl"),
            "existing private evidence\n",
        )
        .expect("write private run state");
        let manifest = format!(
            r#"schema_version = 1

[experiment]
name = "doctor fixture"
[experiment.objective]
name = "score"
direction = "maximize"
[experiment.budget]
max_candidates = 2
max_failures = 1
wall_clock_seconds = 60

[scope]
mutable_paths = ["src"]

[agent]
program = "{agent}"
timeout_seconds = 10

[[evaluators]]
id = "fixture"
hard_gates = ["tests"]
[evaluators.command]
program = "{evaluator}"
timeout_seconds = 10
[[evaluators.metrics]]
name = "score"
kind = "objective"
direction = "maximize"
"#
        );
        fs::write(repository.root.join("autoresearch.toml"), manifest).expect("write manifest");
        repository.write_trap("agent", "agent.executed");
        repository.write_trap("evaluator", "evaluator.executed");
        repository
    }

    fn write_trap(&self, name: &str, marker: &str) {
        let path = self.root.join(name);
        fs::write(
            &path,
            format!("#!/bin/sh\nprintf executed > {marker}\nexit 97\n"),
        )
        .expect("write executable trap");
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("make trap executable");
    }

    fn doctor(&self, args: &[&str]) -> Output {
        let before = snapshot(&self.root);
        let mut command = Command::new(env!("CARGO_BIN_EXE_autoresearch"));
        command
            .env_clear()
            .env("PATH", &self.bin)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .current_dir(&self.root)
            .arg("--repository")
            .arg(&self.root)
            .args(args);
        let output = run_process(command);
        assert_eq!(
            snapshot(&self.root),
            before,
            "doctor must leave repository bytes unchanged"
        );
        assert!(
            !self.root.join("agent.executed").exists(),
            "agent was executed"
        );
        assert!(
            !self.root.join("evaluator.executed").exists(),
            "evaluator was executed"
        );
        output
    }

    fn doctor_json(&self, failed_checks: &[&str]) -> Value {
        let output = self.doctor(&["--json", "doctor"]);
        let ready = failed_checks.is_empty();
        assert_eq!(
            output.status.code(),
            Some(if ready { 0 } else { 4 }),
            "{output:?}"
        );
        assert!(output.stderr.is_empty(), "{output:?}");
        let report: Value = serde_json::from_slice(&output.stdout).expect("doctor JSON report");
        assert_eq!(report.as_object().expect("report object").len(), 4);
        assert_eq!(report["command"], "doctor");
        assert_eq!(
            report["repository"],
            self.root.to_str().expect("UTF-8 fixture path")
        );
        assert_eq!(report["ready"], ready);
        let checks = report["checks"].as_array().expect("checks array");
        for check in checks {
            assert_eq!(check.as_object().expect("check object").len(), 3);
            assert!(check["name"].is_string());
            assert!(check["passed"].is_boolean());
            assert!(check["detail"].is_string());
        }
        let failed: Vec<_> = checks
            .iter()
            .filter(|check| check["passed"] == false)
            .map(|check| check["name"].as_str().expect("check name"))
            .collect();
        assert_eq!(failed, failed_checks, "{report}");
        report
    }
}

impl Drop for TestRepository {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.directory).expect("remove fixture directory");
    }
}

fn run_process(mut command: Command) -> Output {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn fixture process");
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if child.try_wait().expect("poll fixture process").is_some() {
            return child.wait_with_output().expect("collect fixture output");
        }
        if Instant::now() >= deadline {
            child.kill().expect("kill timed out fixture process");
            let output = child
                .wait_with_output()
                .expect("reap timed out fixture process");
            panic!("fixture process exceeded deadline: {command:?}: {output:?}");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn snapshot(root: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    let mut files = BTreeMap::new();
    for entry in fs::read_dir(root).expect("read fixture directory") {
        let entry = entry.expect("fixture directory entry");
        let path = entry.path();
        if entry.file_type().expect("fixture file type").is_dir() {
            files.insert(path.clone(), None);
            files.extend(snapshot(&path));
        } else {
            files.insert(
                path,
                Some(fs::read(entry.path()).expect("read fixture file")),
            );
        }
    }
    files
}

fn check<'a>(report: &'a Value, name: &str) -> &'a Value {
    let mut matches = report["checks"]
        .as_array()
        .expect("checks array")
        .iter()
        .filter(|check| check["name"] == name);
    let check = matches.next().expect("required doctor check");
    assert!(matches.next().is_none(), "duplicate check: {name}");
    check
}

#[test]
fn manual_agent_is_ready_without_executable_and_preserves_json_shape() {
    let repository = TestRepository::new("manual", "./evaluator");
    assert!(!repository.bin.join("manual").exists());
    let report = repository.doctor_json(&[]);
    let agent = check(&report, "agent_executable");
    assert_eq!(agent["passed"], true);
    assert!(
        agent["detail"]
            .as_str()
            .expect("agent detail")
            .contains("manual")
    );
    assert_eq!(
        check(&report, "evaluator_executable:fixture../evaluator")["passed"],
        true
    );
}

#[test]
fn manual_agent_text_output_keeps_agent_check() {
    let repository = TestRepository::new("manual", "./evaluator");
    let output = repository.doctor(&["doctor"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    let text = String::from_utf8(output.stdout).expect("doctor text output");
    assert!(text.contains("ready: true\n"), "{text}");
    assert!(text.contains("pass agent_executable: manual"), "{text}");
    assert!(
        text.contains("pass evaluator_executable:fixture../evaluator:"),
        "{text}"
    );
}

#[test]
fn missing_command_agent_still_fails_readiness() {
    let repository = TestRepository::new("missing-agent", "./evaluator");
    let report = repository.doctor_json(&["agent_executable"]);
    assert_eq!(
        check(&report, "agent_executable")["detail"],
        "`missing-agent` not found or not executable"
    );
}

#[test]
fn missing_evaluator_still_fails_with_manual_agent() {
    let repository = TestRepository::new("manual", "missing-evaluator");
    let report = repository.doctor_json(&["evaluator_executable:fixture.missing-evaluator"]);
    assert_eq!(check(&report, "agent_executable")["passed"], true);
    assert_eq!(
        check(&report, "evaluator_executable:fixture.missing-evaluator")["detail"],
        "`missing-evaluator` not found or not executable"
    );
}

#[test]
fn manual_sentinel_does_not_apply_to_evaluators() {
    let repository = TestRepository::new("manual", "manual");
    let report = repository.doctor_json(&["evaluator_executable:fixture.manual"]);
    assert_eq!(check(&report, "agent_executable")["passed"], true);
    assert_eq!(
        check(&report, "evaluator_executable:fixture.manual")["detail"],
        "`manual` not found or not executable"
    );
}

#[test]
fn manual_agent_sentinel_is_case_sensitive() {
    let repository = TestRepository::new("Manual", "./evaluator");
    let report = repository.doctor_json(&["agent_executable"]);
    assert_eq!(
        check(&report, "agent_executable")["detail"],
        "`Manual` not found or not executable"
    );
}

#[test]
fn command_agent_and_evaluator_are_checked_without_execution() {
    let repository = TestRepository::new("./agent", "./evaluator");
    let report = repository.doctor_json(&[]);
    assert_eq!(
        check(&report, "agent_executable")["detail"],
        repository
            .root
            .join("./agent")
            .to_str()
            .expect("agent path")
    );
    assert_eq!(check(&report, "agent_executable")["passed"], true);
    assert_eq!(
        check(&report, "evaluator_executable:fixture../evaluator")["passed"],
        true
    );
}

#[test]
fn nonexecutable_command_agent_still_fails_readiness() {
    let repository = TestRepository::new("./agent", "./evaluator");
    fs::set_permissions(
        repository.root.join("agent"),
        fs::Permissions::from_mode(0o644),
    )
    .expect("remove agent executable permission");
    let report = repository.doctor_json(&["agent_executable"]);
    assert_eq!(
        check(&report, "agent_executable")["detail"],
        "`./agent` not found or not executable"
    );
}
