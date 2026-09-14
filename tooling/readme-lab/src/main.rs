//! `readme-lab`: README evaluator for autoresearch-rs, plus a local scoring command.
//!
//! `evaluate` answers one JSONL protocol v1 request. `score` prints the same evaluation as a
//! JSON report; holdout rule hits stay hidden unless `--include-holdout` is passed.

mod config;
mod gates;
mod markdown;
mod slop;

use autoresearch_core::{Measurement, MetricDirection, NumericMetricKind};
use autoresearch_evaluator::{
    EvaluatorOutput, PROTOCOL_VERSION, ProtocolResponse, ProtocolResult, decode_request,
    encode_response,
};
use clap::{Parser, Subcommand, ValueEnum};
use config::Config;
use gates::Gate;
use serde::Serialize;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "readme-lab", version, about)]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Answer one autoresearch JSONL request read from stdin.
    Evaluate {
        #[arg(long, value_enum)]
        contract: Contract,
        /// Lab config, relative to the candidate worktree.
        #[arg(long)]
        config: PathBuf,
    },
    /// Score a README directly and print a JSON report.
    Score {
        #[arg(long, value_enum)]
        contract: Contract,
        /// Lab config, relative to `--root`.
        #[arg(long)]
        config: PathBuf,
        /// Repository root that links and binaries resolve against.
        #[arg(long)]
        root: PathBuf,
        /// README to score; defaults to README.md under `--root`.
        #[arg(long)]
        readme: Option<PathBuf>,
        #[arg(long)]
        include_holdout: bool,
    },
    /// Check every rule fires on its example_bad and stays silent on its example_good.
    Selftest {
        #[arg(long)]
        rules: PathBuf,
    },
}

/// Which frozen contract the evaluator reports.
#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Contract {
    /// Objective: uncovered use-case lanes. Slop is diagnostic.
    Coverage,
    /// Objective: dev slop points. Coverage, substance, and holdout become gates.
    Slop,
}

struct Evaluation {
    gates: Vec<Gate>,
    uncovered: Vec<String>,
    dev: slop::Score,
    holdout: slop::Score,
    prose_words: usize,
    code_blocks: usize,
    invocations: usize,
}

fn main() -> ExitCode {
    let args = Args::parse();
    let result = match args.command {
        Command::Evaluate { contract, config } => run_evaluate(contract, &config),
        Command::Score {
            contract,
            config,
            root,
            readme,
            include_holdout,
        } => run_score(contract, &config, &root, readme.as_deref(), include_holdout),
        Command::Selftest { rules } => match run_selftest(&rules) {
            Ok(true) => return ExitCode::SUCCESS,
            Ok(false) => return ExitCode::FAILURE,
            Err(error) => Err(error),
        },
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("readme-lab: {error}");
            ExitCode::from(2)
        }
    }
}

/// Returns whether every rule passed; prints one line per rule.
fn run_selftest(path: &Path) -> Result<bool, String> {
    let rules = slop::load(path)?;
    let mut failures = 0;
    for rule in &rules {
        let spec = &rule.spec;
        let label = format!(
            "{} [{}] {}",
            spec.id,
            spec.category.as_deref().unwrap_or("-"),
            spec.name.as_deref().unwrap_or("")
        );
        let (Some(bad), Some(good)) = (spec.example_bad.as_deref(), spec.example_good.as_deref())
        else {
            failures += 1;
            println!("FAIL {label}: example_bad or example_good missing");
            continue;
        };
        let bad_hits = slop::hits(&markdown::parse(bad), rule);
        let good_hits = slop::hits(&markdown::parse(good), rule);
        let passed = bad_hits > 0 && good_hits == 0;
        if !passed {
            failures += 1;
        }
        println!(
            "{} {label}: bad_hits={bad_hits} good_hits={good_hits}",
            if passed { "ok  " } else { "FAIL" }
        );
    }
    println!("{} rules, {failures} failed", rules.len());
    Ok(failures == 0)
}

fn resolve(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

fn evaluate(
    root: &Path,
    readme: &Path,
    config_path: &Path,
    contract: Contract,
) -> Result<Evaluation, String> {
    let source = std::fs::read_to_string(readme)
        .map_err(|error| format!("read {}: {error}", readme.display()))?;
    let (config, config_dir) = Config::load(config_path)?;
    let document = markdown::parse(&source);
    let dev = slop::score(&document, &slop::load(&config_dir.join(&config.rules_dev))?);
    let holdout = slop::score(
        &document,
        &slop::load(&config_dir.join(&config.rules_holdout))?,
    );

    let cli = gates::cli_invocations_parse(&document, root, &config);
    let mut all = vec![
        gates::links_resolve(&document, root),
        cli.gate,
        gates::code_blocks_parse(&document),
        gates::limits_preserved(&document, &config),
        gates::markdown_well_formed(&document),
    ];
    let uncovered = gates::uncovered_use_cases(&document, &config);
    let prose_words = document.prose_words();
    let code_blocks = document.code_blocks.len();

    if contract == Contract::Slop {
        let missing: Vec<String> = uncovered
            .iter()
            .map(|id| format!("no section covers `{id}`"))
            .collect();
        all.push(Gate::new(
            "use_cases_covered",
            &missing,
            format!("{} lanes covered", config.use_cases.len()),
        ));
        all.push(gates::substance_floor(
            prose_words,
            code_blocks,
            config.substance.as_ref(),
        ));
        let ceiling = config.holdout_ceiling;
        let failures = match ceiling {
            Some(ceiling) if holdout.points as f64 <= ceiling => Vec::new(),
            Some(ceiling) => vec![format!(
                "holdout slop {} exceeds ceiling {ceiling}",
                holdout.points
            )],
            None => vec!["holdout_ceiling missing from config".to_owned()],
        };
        all.push(Gate::new(
            "holdout_not_regressed",
            &failures,
            format!("holdout slop {} within ceiling", holdout.points),
        ));
    }

    Ok(Evaluation {
        gates: all,
        uncovered,
        dev,
        holdout,
        prose_words,
        code_blocks,
        invocations: cli.invocations,
    })
}

fn measurements(evaluation: &Evaluation, contract: Contract) -> Result<Vec<Measurement>, String> {
    let error = |error: autoresearch_core::MetricError| error.to_string();
    let mut out = Vec::new();
    for gate in &evaluation.gates {
        out.push(Measurement::hard_gate(gate.name, gate.passed, Some(gate.detail.clone())).map_err(error)?);
    }
    let dev_points = evaluation.dev.points as f64;
    let uncovered = evaluation.uncovered.len() as f64;
    let (objective_name, objective_value) = match contract {
        Contract::Coverage => ("uncovered_use_cases", uncovered),
        Contract::Slop => ("slop_points", dev_points),
    };
    out.push(
        Measurement::numeric(
            objective_name,
            NumericMetricKind::Objective,
            MetricDirection::Minimize,
            objective_value,
        )
        .map_err(error)?,
    );
    let mut diagnostics = vec![
        ("holdout_slop_points", MetricDirection::Minimize, evaluation.holdout.points as f64),
        ("prose_words", MetricDirection::Maximize, evaluation.prose_words as f64),
        ("code_blocks", MetricDirection::Maximize, evaluation.code_blocks as f64),
    ];
    if contract == Contract::Coverage {
        diagnostics.insert(0, ("slop_points", MetricDirection::Minimize, dev_points));
    }
    for (name, direction, value) in diagnostics {
        out.push(
            Measurement::numeric(name, NumericMetricKind::Diagnostic, direction, value)
                .map_err(error)?,
        );
    }
    Ok(out)
}

fn run_evaluate(contract: Contract, config: &Path) -> Result<(), String> {
    let mut input = Vec::new();
    std::io::stdin()
        .read_to_end(&mut input)
        .map_err(|error| format!("read request: {error}"))?;
    let request = decode_request(&input).map_err(|error| error.to_string())?;
    let root = request.candidate_worktree.clone();
    let evaluation = evaluate(
        &root,
        &root.join("README.md"),
        &resolve(&root, config),
        contract,
    )?;
    let response = ProtocolResponse {
        protocol_version: PROTOCOL_VERSION,
        result: ProtocolResult::Success {
            output: EvaluatorOutput {
                evaluator_id: request.evaluator_id,
                run_id: request.run_id,
                baseline_commit: request.baseline_commit,
                evaluated_commit: request.evaluated_commit,
                measurements: measurements(&evaluation, contract)?,
                observations: vec![],
                artifacts: vec![],
                warnings: vec![],
            },
        },
    };
    let bytes = encode_response(&response).map_err(|error| error.to_string())?;
    std::io::stdout()
        .write_all(&bytes)
        .map_err(|error| format!("write response: {error}"))
}

#[derive(Serialize)]
struct GateView<'a> {
    name: &'a str,
    passed: bool,
    detail: &'a str,
}

#[derive(Serialize)]
struct Report<'a> {
    contract: &'static str,
    all_gates_passed: bool,
    objective: &'static str,
    objective_value: f64,
    gates: Vec<GateView<'a>>,
    slop_points: u64,
    uncovered_use_cases: &'a [String],
    prose_words: usize,
    code_blocks: usize,
    cli_invocations: usize,
    dev_rules: Vec<&'a slop::RuleScore>,
    #[serde(skip_serializing_if = "Option::is_none")]
    holdout_slop_points: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    holdout_rules: Option<Vec<&'a slop::RuleScore>>,
}

fn run_score(
    contract: Contract,
    config: &Path,
    root: &Path,
    readme: Option<&Path>,
    include_holdout: bool,
) -> Result<(), String> {
    let readme = readme.map_or_else(|| root.join("README.md"), Path::to_path_buf);
    let evaluation = evaluate(root, &readme, &resolve(root, config), contract)?;
    let (objective, objective_value) = match contract {
        Contract::Coverage => ("uncovered_use_cases", evaluation.uncovered.len() as f64),
        Contract::Slop => ("slop_points", evaluation.dev.points as f64),
    };
    fn hitting(score: &slop::Score) -> Vec<&slop::RuleScore> {
        score.rules.iter().filter(|rule| rule.hits > 0).collect()
    }
    let report = Report {
        contract: match contract {
            Contract::Coverage => "coverage",
            Contract::Slop => "slop",
        },
        all_gates_passed: evaluation.gates.iter().all(|gate| gate.passed),
        objective,
        objective_value,
        gates: evaluation
            .gates
            .iter()
            .map(|gate| GateView {
                name: gate.name,
                passed: gate.passed,
                detail: &gate.detail,
            })
            .collect(),
        slop_points: evaluation.dev.points,
        uncovered_use_cases: &evaluation.uncovered,
        prose_words: evaluation.prose_words,
        code_blocks: evaluation.code_blocks,
        cli_invocations: evaluation.invocations,
        dev_rules: hitting(&evaluation.dev),
        holdout_slop_points: include_holdout.then_some(evaluation.holdout.points),
        holdout_rules: include_holdout.then(|| hitting(&evaluation.holdout)),
    };
    let json = serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?;
    println!("{json}");
    Ok(())
}
