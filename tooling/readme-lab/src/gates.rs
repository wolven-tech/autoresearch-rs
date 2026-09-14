//! Hard gates. Each names what failed so a discard is reviewable.

use crate::config::{Config, Substance};
use crate::markdown::{
    self, BlockKind, Document, Heading, Line, LineKind, closes_fence, code_spans, indent_width,
    opens_fence, sentences, table_width,
};
use clap::Parser as _;
use clap::error::ErrorKind;
use regex::Regex;
use std::collections::{BTreeSet, HashSet};
use std::path::{Component, Path};
use std::sync::LazyLock;

const DETAIL_LIMIT: usize = 900;
/// Every line of these blocks is a command.
const SHELL_LANGS: &[&str] = &[
    "", "bash", "sh", "shell", "zsh", "ksh", "fish", "nu", "powershell", "pwsh", "ps1", "bat",
    "batch", "cmd",
];
/// Prompted lines are commands and the rest is output; a block with no prompt is all commands.
/// Any other language is scanned only on prompted lines.
const SESSION_LANGS: &[&str] = &[
    "console",
    "shell-session",
    "sh-session",
    "shellsession",
    "bash-session",
    "terminal",
];
const GLOBAL_VALUE_FLAGS: &[&str] = &["--repository"];
const HELP_FLAGS: &[&str] = &["-h", "--help", "-V", "--version"];
const CARGO_GLOBAL_VALUE_FLAGS: &[&str] = &["-Z", "--config", "-C", "--color", "--explain"];
const CARGO_RUN_VALUE_FLAGS: &[&str] = &[
    "-p",
    "--package",
    "--bin",
    "--example",
    "-F",
    "--features",
    "-j",
    "--jobs",
    "--profile",
    "--target",
    "--target-dir",
    "--manifest-path",
    "--message-format",
    "--color",
    "--config",
    "-Z",
    "--lockfile-path",
];
const MANIFEST_KEYS: &[&str] = &[
    "schema_version",
    "experiment",
    "scope",
    "agent",
    "evaluators",
    "web",
    "authority",
];
/// A heading naming more lanes than this is a catch-all and claims none of them.
const LANES_PER_HEADING: usize = 2;
const NEGATION_WINDOW: usize = 15;
const POSTFIX_NEGATION_WINDOW: usize = 6;
/// Most words between a modifier claim and the limit term it affirms.
const MODIFIER_REACH: usize = 2;
const CLAIM_EXCERPT: usize = 120;
const FRAGMENT_OBJECTIVE: &str = "readme_fragment_objective";
const FRAGMENT_BASE: &str = r#"
schema_version = 1

[experiment]
name = "readme fragment"

[experiment.objective]
name = "readme_fragment_objective"
direction = "minimize"

[experiment.budget]
max_candidates = 1
max_failures = 1
wall_clock_seconds = 1

[scope]
mutable_paths = ["src"]

[agent]
program = "manual"
timeout_seconds = 1

[[evaluators]]
id = "readme_fragment"

[evaluators.command]
program = "/bin/true"
timeout_seconds = 1

[[evaluators.metrics]]
name = "readme_fragment_objective"
kind = "objective"
direction = "minimize"
"#;

fn regex(pattern: &str) -> Regex {
    Regex::new(pattern).expect("static regex")
}

static MD_LINK: LazyLock<Regex> = LazyLock::new(|| {
    regex(
        r#"\[(?:[^\[\]]|\[[^\[\]]*\])*\]\(\s*(?:<([^<>\n]*)>|((?:[^()\s]|\([^()\s]*\))*))(?:\s+(?:"[^"]*"|'[^']*'|\([^()]*\)))?\s*\)"#,
    )
});
static HTML_LINK: LazyLock<Regex> =
    LazyLock::new(|| regex(r#"(?i)\b(?:href|src)\s*=\s*(?:"([^"]*)"|'([^']*)')"#));
static HTML_ANCHOR: LazyLock<Regex> =
    LazyLock::new(|| regex(r#"(?i)\b(?:id|name)\s*=\s*"([^"]+)""#));
static REF_DEF: LazyLock<Regex> =
    LazyLock::new(|| regex(r"(?m)^[ \t]{0,3}\[[^\]\n]+\]:[ \t]*(?:<([^<>\n]*)>|(\S+))"));
static LINE_ANCHOR: LazyLock<Regex> =
    LazyLock::new(|| regex(r"^L\d+(?:C\d+)?(?:-L\d+(?:C\d+)?)?$"));
static NEGATION: LazyLock<Regex> = LazyLock::new(|| {
    regex(
        r"(?i)\b(not|no|never|isn't|isn’t|aren't|aren’t|doesn't|doesn’t|don't|don’t|won't|won’t|cannot|can't|can’t|without|nor|neither|nothing|none|unavailable|unsupported|excluded|out of scope)\b",
    )
});
/// Negation that follows the term it governs: "X is out of scope".
static POSTFIX_NEGATION: LazyLock<Regex> = LazyLock::new(|| {
    regex(
        r"(?i)\b(?:(?:is|are|was|were|remains?|stays?)\s+(?:not|never|unavailable|unsupported|excluded)|out of scope|unsupported|unavailable|excluded|not (?:supported|provided|included|claimed|offered))\b",
    )
});
static COLON_LEAD_IN: LazyLock<Regex> = LazyLock::new(|| {
    regex(
        r"(?i)(?:\b(?:limits?|limitations?|non-goals?|out of scope|unsupported|not supported)|\b(?:not|never)\s+(?:include[sd]?|do|does|provides?|offers?|claims?|covers?|supports?))\W*$",
    )
});
static CLAUSE_BREAK: LazyLock<Regex> = LazyLock::new(|| {
    regex(
        r"(?i)[;:—–()]|,\s*(?:and|so|which|but)\b|\b(?:but|while|whereas|although|though|however|yet|instead)\b",
    )
});
/// Affirmative wording that turns a limit term into a claim when no negation governs it.
static CLAIM: LazyLock<Regex> = LazyLock::new(|| {
    regex(
        r"(?i)\b(?:certif\w*|conform(?:s|ant|ance)?|complian(?:t|ce)|guarantee[sd]?|proves?|proven|verifie[sd]|achieves?|equivalent|parity|deploys|deployed to|publishes|published to|(?:to|in) production|autonomously|sandboxed|isolated|ensures?|delivers?)\b",
    )
});
/// Claim wording that modifies whatever sits next to it, so it claims only near the term.
static MODIFIER_CLAIM: LazyLock<Regex> =
    LazyLock::new(|| regex(r"(?i)\b(?:matches|automatic(?:ally)?|fully)\b"));
static LIMITS_HEADING: LazyLock<Regex> = LazyLock::new(|| {
    regex(r"(?i)\b(?:limits?|limitations?|non-goals?|out of scope|unsupported|not supported)\b")
});
static ENV_ASSIGNMENT: LazyLock<Regex> = LazyLock::new(|| regex(r"^[A-Za-z_][A-Za-z0-9_]*="));
static PROMPT: LazyLock<Regex> = LazyLock::new(|| {
    regex(r"^\s*(?:PS [^>\n]*>|[\w.-]+@[\w.-]+(?::[^\s$#%>]*)?[$#%]|[$%])\s+")
});
static REDIRECT: LazyLock<Regex> =
    LazyLock::new(|| regex(r"^\d*(?:>>?|<<?<?|>&|<&|&>>?)(.*)$"));
static PLACEHOLDER: LazyLock<Regex> = LazyLock::new(|| regex(r"^<[^<>\s]+>$"));
static INLINE_COMMENT: LazyLock<Regex> = LazyLock::new(|| regex(r"<!--.*?-->"));
static LINK_TEXT: LazyLock<Regex> = LazyLock::new(|| regex(r"!?\[([^\]]*)\]\([^)]*\)"));
static URL: LazyLock<Regex> = LazyLock::new(|| regex(r"<?https?://[^\s)>\]]+>?"));
static HTML_TAG: LazyLock<Regex> = LazyLock::new(|| regex(r"</?[A-Za-z][^>]*>"));
static H1_TAG: LazyLock<Regex> = LazyLock::new(|| regex(r"(?i)<h1[\s>]"));
static A11Y: LazyLock<Regex> = LazyLock::new(|| regex(r"\ba11y\b"));

pub struct Gate {
    pub name: &'static str,
    pub passed: bool,
    pub detail: String,
}

impl Gate {
    pub fn new(name: &'static str, failures: &[String], passing_detail: String) -> Self {
        let passed = failures.is_empty();
        let mut detail = if passed {
            passing_detail
        } else {
            failures.join("; ")
        };
        if detail.len() > DETAIL_LIMIT {
            let mut cut = DETAIL_LIMIT;
            while !detail.is_char_boundary(cut) {
                cut -= 1;
            }
            detail.truncate(cut);
            detail.push('…');
        }
        Self {
            name,
            passed,
            detail,
        }
    }
}

/// Relative links and images resolve inside the repository with exact letter case; anchors
/// match headings, in this document or in the linked Markdown file.
pub fn links_resolve(document: &Document, root: &Path) -> Gate {
    let anchors = anchor_set(document);
    let mut failures = Vec::new();
    let mut checked = 0;
    for (first_line, text) in link_runs(document) {
        let line_starts: Vec<usize> = std::iter::once(0)
            .chain(text.match_indices('\n').map(|(index, _)| index + 1))
            .collect();
        let matches = MD_LINK
            .captures_iter(&text)
            .chain(HTML_LINK.captures_iter(&text))
            .chain(REF_DEF.captures_iter(&text));
        for captures in matches {
            let Some(target) = captures.get(1).or_else(|| captures.get(2)) else {
                continue;
            };
            let offset = captures.get(0).map_or(0, |whole| whole.start());
            let line = first_line + line_starts.partition_point(|&start| start <= offset) - 1;
            match check_target(target.as_str(), root, &anchors) {
                Ok(true) => checked += 1,
                Ok(false) => {}
                Err(reason) => failures.push(format!("line {line}: {reason}")),
            }
        }
    }
    Gate::new(
        "links_resolve",
        &failures,
        format!("{checked} local links resolve"),
    )
}

/// Rendered text runs a link may wrap inside: consecutive non-blank, non-code lines.
fn link_runs(document: &Document) -> Vec<(usize, String)> {
    let mut runs = Vec::new();
    let mut current: Option<(usize, String)> = None;
    for line in &document.lines {
        match line.kind {
            LineKind::Blank | LineKind::Code | LineKind::Fence | LineKind::IndentedCode => {
                runs.extend(current.take());
            }
            LineKind::Heading => {
                runs.extend(current.take());
                runs.push((line.number, line.nocode.clone()));
            }
            _ => match &mut current {
                Some((_, text)) => {
                    text.push('\n');
                    text.push_str(&line.nocode);
                }
                None => current = Some((line.number, line.nocode.clone())),
            },
        }
    }
    runs.extend(current);
    runs
}

fn anchor_set(document: &Document) -> HashSet<String> {
    let mut anchors: HashSet<String> = document
        .rendered_headings()
        .into_iter()
        .map(|heading| heading.slug)
        .collect();
    for line in &document.lines {
        if line.kind == LineKind::Html {
            for captures in HTML_ANCHOR.captures_iter(&line.raw) {
                anchors.insert(captures[1].to_lowercase());
            }
        }
    }
    anchors
}

/// Returns `Ok(true)` for a resolved local target, `Ok(false)` for an external one.
fn check_target(target: &str, root: &Path, anchors: &HashSet<String>) -> Result<bool, String> {
    let target = target.trim();
    let lower = target.to_lowercase();
    if target.is_empty() {
        return Err("empty link target".into());
    }
    if ["http://", "https://", "mailto:", "ftp://"]
        .iter()
        .any(|scheme| lower.starts_with(scheme))
    {
        return Ok(false);
    }
    if let Some(anchor) = target.strip_prefix('#') {
        return if anchors.contains(&anchor.to_lowercase()) {
            Ok(true)
        } else {
            Err(format!("`#{anchor}` matches no heading"))
        };
    }
    let path_part = target.split(['#', '?']).next().unwrap_or_default();
    let fragment = target.split_once('#').map(|(_, fragment)| fragment);
    let decoded = percent_decode(path_part);
    let mut relative = decoded.trim_start_matches('/');
    while let Some(rest) = relative.strip_prefix("./") {
        relative = rest;
    }
    if Path::new(relative)
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(format!("`{target}` escapes the repository"));
    }
    if !exists_exact(root, relative) {
        return Err(if root.join(relative).exists() {
            format!("`{target}` differs in letter case from the path on disk")
        } else {
            format!("`{target}` does not exist")
        });
    }
    let markdown_file = [".md", ".markdown"]
        .iter()
        .any(|extension| relative.to_ascii_lowercase().ends_with(extension));
    if let Some(fragment) = fragment.filter(|fragment| {
        markdown_file && !fragment.is_empty() && !LINE_ANCHOR.is_match(fragment)
    }) {
        let source = std::fs::read_to_string(root.join(relative))
            .map_err(|error| format!("`{target}` unreadable: {error}"))?;
        if !anchor_set(&markdown::parse(&source)).contains(&fragment.to_lowercase()) {
            return Err(format!("`{target}` matches no heading in that file"));
        }
    }
    Ok(true)
}

/// Existence with byte-exact names, so a case mismatch fails on case-insensitive filesystems too.
fn exists_exact(root: &Path, relative: &str) -> bool {
    let mut current = root.to_path_buf();
    for component in Path::new(relative).components() {
        let Component::Normal(name) = component else {
            continue;
        };
        let Ok(entries) = std::fs::read_dir(&current) else {
            return false;
        };
        if !entries.flatten().any(|entry| entry.file_name() == name) {
            return false;
        }
        current.push(name);
    }
    current.exists()
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let hex = bytes
            .get(index + 1..index + 3)
            .and_then(|pair| std::str::from_utf8(pair).ok())
            .and_then(|pair| u8::from_str_radix(pair, 16).ok());
        match (bytes[index], hex) {
            (b'%', Some(byte)) => {
                out.push(byte);
                index += 3;
            }
            (byte, _) => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub struct CliScan {
    pub gate: Gate,
    pub invocations: usize,
}

struct CliState<'a> {
    root: &'a Path,
    config: &'a Config,
    failures: Vec<String>,
    invocations: usize,
    used: BTreeSet<String>,
}

/// A code block from any container, with 1-based source line numbers.
struct Snippet {
    line: usize,
    lang: String,
    lines: Vec<(usize, String)>,
}

enum Target {
    Cli(Vec<String>),
    Binary(String, Vec<String>),
    Ignored,
}

/// Every shell command invoking an autoresearch binary, in code blocks or inline code, parses
/// with the real CLI definition or the binary's configured argument shapes.
pub fn cli_invocations_parse(document: &Document, root: &Path, config: &Config) -> CliScan {
    let mut state = CliState {
        root,
        config,
        failures: Vec::new(),
        invocations: 0,
        used: BTreeSet::new(),
    };

    for snippet in snippets(document) {
        let lang = snippet.lang.as_str();
        let shell = SHELL_LANGS.contains(&lang);
        let session = SESSION_LANGS.contains(&lang);
        let prompted = snippet.lines.iter().any(|(_, text)| PROMPT.is_match(text));
        for (line_number, command) in logical_lines(&snippet.lines) {
            let prompt = PROMPT.find(&command);
            let include = shell || (session && (prompt.is_some() || !prompted)) || prompt.is_some();
            if !include {
                continue;
            }
            let command = prompt.map_or(command.as_str(), |found| &command[found.end()..]);
            for segment in command_segments(command) {
                check_segment(&segment, false, line_number, &mut state);
            }
        }
    }

    for line in &document.lines {
        if !matches!(
            line.kind,
            LineKind::Paragraph
                | LineKind::ListItem
                | LineKind::Quote
                | LineKind::TableRow
                | LineKind::Heading
        ) {
            continue;
        }
        for span in code_spans(&line.raw) {
            let command = PROMPT.find(&span).map_or(span.as_str(), |found| &span[found.end()..]);
            for segment in command_segments(command) {
                check_segment(&segment, true, line.number, &mut state);
            }
        }
    }

    for required in &config.required_subcommands {
        if !state.used.contains(required) {
            state
                .failures
                .push(format!("no parseable `autoresearch {required}` example"));
        }
    }
    CliScan {
        gate: Gate::new(
            "cli_invocations_parse",
            &state.failures,
            format!(
                "{} invocations parse; subcommands shown: {}",
                state.invocations,
                state.used.iter().cloned().collect::<Vec<_>>().join(", ")
            ),
        ),
        invocations: state.invocations,
    }
}

/// An inline span that only names a program, or names a subcommand without its required
/// arguments, is a reference rather than an invocation.
fn check_segment(tokens: &[String], inline: bool, line: usize, state: &mut CliState<'_>) {
    let tokens = strip_prefix_commands(tokens);
    let Some((program, args)) = tokens.split_first() else {
        return;
    };
    let name = program.rsplit('/').next().unwrap_or(program);
    let target = if name == "autoresearch" {
        Target::Cli(args.to_vec())
    } else if name.starts_with("autoresearch-") {
        Target::Binary(name.to_owned(), args.to_vec())
    } else if name == "cargo" {
        cargo_target(args, line, state)
    } else {
        Target::Ignored
    };
    let label = if inline { "inline " } else { "" };
    match target {
        Target::Cli(args) => {
            if inline && args.is_empty() {
                return;
            }
            state.invocations += 1;
            match parse_cli(&args) {
                Ok(Some(subcommand)) => {
                    state.used.insert(subcommand);
                }
                Ok(None) => {}
                Err((kind, _)) if inline && incomplete(kind) => {}
                Err((_, error)) => state.failures.push(format!(
                    "line {line}: {label}`autoresearch {}` rejected: {error}",
                    args.join(" ")
                )),
            }
        }
        Target::Binary(name, args) => {
            if inline && args.is_empty() {
                return;
            }
            state.invocations += 1;
            if let Err(reason) = check_binary(&name, &args, state) {
                state.failures.push(format!("line {line}: {label}{reason}"));
            }
        }
        Target::Ignored => {}
    }
}

fn check_binary(name: &str, args: &[String], state: &CliState<'_>) -> Result<(), String> {
    let Some(binary) = state.config.known_binaries.get(name) else {
        return Err(format!("`{name}` is not a shipped binary"));
    };
    if !state.root.join(&binary.source).exists() {
        return Err(format!(
            "`{name}` source `{}` is missing",
            binary.source.display()
        ));
    }
    if binary.argv.iter().any(|shape| argv_matches(shape, args)) {
        return Ok(());
    }
    let shapes: Vec<String> = binary.argv.iter().map(|shape| format!("`{shape}`")).collect();
    Err(format!(
        "`{name} {}` matches no accepted argument shape ({})",
        args.join(" "),
        shapes.join(", ")
    ))
}

fn argv_matches(shape: &str, args: &[String]) -> bool {
    let pattern: Vec<&str> = shape.split_whitespace().collect();
    pattern.len() == args.len()
        && pattern
            .iter()
            .zip(args)
            .all(|(expected, actual)| PLACEHOLDER.is_match(expected) || expected == actual)
}

fn cargo_target(args: &[String], line: usize, state: &mut CliState<'_>) -> Target {
    let mut index = 0;
    while let Some(arg) = args.get(index) {
        if arg.starts_with('+') || (arg.starts_with('-') && arg != "--") {
            if CARGO_GLOBAL_VALUE_FLAGS.contains(&arg.as_str()) {
                index += 1;
            }
            index += 1;
        } else {
            break;
        }
    }
    let Some(subcommand) = args.get(index) else {
        return Target::Ignored;
    };
    let rest = &args[index + 1..];
    match subcommand.as_str() {
        "run" | "r" => cargo_run_target(rest, state.config),
        "install" => {
            let path = rest.iter().enumerate().find_map(|(position, arg)| {
                arg.strip_prefix("--path=").map(str::to_owned).or_else(|| {
                    (arg == "--path")
                        .then(|| rest.get(position + 1).cloned())
                        .flatten()
                })
            });
            if let Some(path) = path {
                if !state.root.join(&path).join("Cargo.toml").exists() {
                    state.failures.push(format!(
                        "line {line}: `cargo install --path {path}` has no Cargo.toml"
                    ));
                }
            }
            Target::Ignored
        }
        _ => Target::Ignored,
    }
}

fn cargo_run_target(args: &[String], config: &Config) -> Target {
    let mut package = None;
    let mut bin = None;
    let mut forwarded: &[String] = &[];
    let mut index = 0;
    while let Some(arg) = args.get(index) {
        if arg == "--" {
            forwarded = &args[index + 1..];
            break;
        }
        if !arg.starts_with('-') {
            forwarded = &args[index..];
            break;
        }
        let (flag, attached) = match arg.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => (flag, Some(value.to_owned())),
            _ if arg.len() > 2 && arg.starts_with("-p") => ("-p", Some(arg[2..].to_owned())),
            _ => (arg.as_str(), None),
        };
        let value = if CARGO_RUN_VALUE_FLAGS.contains(&flag) {
            attached.or_else(|| {
                index += 1;
                args.get(index).cloned()
            })
        } else {
            None
        };
        match flag {
            "-p" | "--package" => package = value,
            "--bin" => bin = value,
            _ => {}
        }
        index += 1;
    }
    let forwarded = forwarded.to_vec();
    match (bin.as_deref(), package.as_deref()) {
        (Some("autoresearch"), _) | (None, Some("autoresearch-cli")) => Target::Cli(forwarded),
        (Some(name), _) if name.starts_with("autoresearch-") => {
            Target::Binary(name.to_owned(), forwarded)
        }
        (None, Some(package)) => {
            let owned: Vec<&String> = config
                .known_binaries
                .iter()
                .filter(|(_, binary)| {
                    binary.source.starts_with(Path::new("crates").join(package))
                        || binary.source.starts_with(Path::new("apps").join(package))
                })
                .map(|(name, _)| name)
                .collect();
            match owned.as_slice() {
                [name] => Target::Binary((*name).clone(), forwarded),
                _ => Target::Ignored,
            }
        }
        _ => Target::Ignored,
    }
}

fn incomplete(kind: ErrorKind) -> bool {
    matches!(
        kind,
        ErrorKind::MissingRequiredArgument
            | ErrorKind::MissingSubcommand
            | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
    )
}

fn try_cli(args: &[String]) -> Result<(), (ErrorKind, String)> {
    let argv = std::iter::once("autoresearch".to_owned()).chain(args.iter().cloned());
    autoresearch_cli::Cli::try_parse_from(argv)
        .map(|_| ())
        .map_err(|error| {
            let message = error
                .to_string()
                .lines()
                .next()
                .unwrap_or("parse error")
                .trim_start_matches("error: ")
                .to_owned();
            (error.kind(), message)
        })
}

/// clap stops at the first help or version flag, so the remaining arguments are re-parsed
/// without it; only an incomplete remainder is excused.
fn parse_cli(args: &[String]) -> Result<Option<String>, (ErrorKind, String)> {
    match try_cli(args) {
        Ok(()) => Ok(subcommand(args)),
        Err((kind, _)) if matches!(kind, ErrorKind::DisplayHelp | ErrorKind::DisplayVersion) => {
            let rest: Vec<String> = args
                .iter()
                .filter(|arg| !HELP_FLAGS.contains(&arg.as_str()))
                .cloned()
                .collect();
            if rest.is_empty() || rest.len() == args.len() {
                return Ok(None);
            }
            match try_cli(&rest) {
                Ok(()) => Ok(None),
                Err((kind, _))
                    if incomplete(kind)
                        || matches!(kind, ErrorKind::DisplayHelp | ErrorKind::DisplayVersion) =>
                {
                    Ok(None)
                }
                Err(error) => Err(error),
            }
        }
        Err(error) => Err(error),
    }
}

fn subcommand(args: &[String]) -> Option<String> {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if GLOBAL_VALUE_FLAGS.contains(&arg.as_str()) {
            iter.next();
        } else if !arg.starts_with('-') {
            return Some(arg.clone());
        }
    }
    None
}

fn snippets(document: &Document) -> Vec<Snippet> {
    let mut out: Vec<Snippet> = document
        .code_blocks
        .iter()
        .map(|block| Snippet {
            line: block.line,
            lang: block.lang.clone(),
            lines: block
                .body
                .lines()
                .enumerate()
                .map(|(offset, text)| (block.line + 1 + offset, text.to_owned()))
                .collect(),
        })
        .collect();
    out.extend(container_snippets(document));
    out
}

/// Code the line scanner leaves as prose: indented code blocks, fences inside blockquotes, and
/// fences indented 4+ columns inside list items.
fn container_snippets(document: &Document) -> Vec<Snippet> {
    let lines = &document.lines;
    let mut out = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let line = &lines[index];
        if line.kind == LineKind::IndentedCode {
            let start = line.number;
            let mut body = Vec::new();
            while let Some(member) = lines
                .get(index)
                .filter(|member| matches!(member.kind, LineKind::IndentedCode | LineKind::Blank))
            {
                body.push((member.number, dedent(&member.raw, 4)));
                index += 1;
            }
            out.push(Snippet {
                line: start,
                lang: String::new(),
                lines: body,
            });
            continue;
        }
        let quoted = line.kind == LineKind::Quote;
        let nested = line.kind == LineKind::Paragraph && indent_width(&line.raw) >= 4;
        index += 1;
        if !quoted && !nested {
            continue;
        }
        let opener = if quoted {
            strip_quote(&line.raw)
        } else {
            line.raw.clone()
        };
        let fence_indent = indent_width(&opener);
        let Some((fence_char, fence_len, lang)) = opens_fence(&dedent(&opener, fence_indent)) else {
            continue;
        };
        let mut body = Vec::new();
        while let Some(next) = lines.get(index) {
            let text = if quoted {
                if next.kind != LineKind::Quote {
                    break;
                }
                strip_quote(&next.raw)
            } else {
                if next.kind != LineKind::Blank && indent_width(&next.raw) < fence_indent {
                    break;
                }
                next.raw.clone()
            };
            index += 1;
            let content = dedent(&text, fence_indent);
            if closes_fence(&content, fence_char, fence_len) {
                break;
            }
            body.push((next.number, content));
        }
        out.push(Snippet {
            line: line.number,
            lang,
            lines: body,
        });
    }
    out
}

fn strip_quote(raw: &str) -> String {
    let mut text = raw;
    loop {
        let trimmed = text.trim_start_matches(' ');
        match trimmed.strip_prefix('>') {
            Some(rest) if text.len() - trimmed.len() <= 3 => {
                text = rest.strip_prefix(' ').unwrap_or(rest);
            }
            _ => return text.to_owned(),
        }
    }
}

fn dedent(text: &str, columns: usize) -> String {
    let mut removed = 0;
    let mut rest = text;
    while removed < columns {
        match rest.chars().next() {
            Some(' ') => removed += 1,
            Some('\t') => removed += 4,
            _ => break,
        }
        rest = &rest[1..];
    }
    rest.to_owned()
}

/// Joins backslash continuations; yields (source line number, command).
fn logical_lines(lines: &[(usize, String)]) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut pending: Option<(usize, String)> = None;
    for (number, line) in lines {
        let (start, mut text) = pending.take().unwrap_or((*number, String::new()));
        let trimmed = line.trim_end();
        if let Some(stripped) = trimmed.strip_suffix('\\') {
            text.push_str(stripped);
            text.push(' ');
            pending = Some((start, text));
        } else {
            text.push_str(trimmed);
            out.push((start, text));
        }
    }
    out.extend(pending);
    out
}

/// Every simple command in a shell line: `$( )` and backtick substitutions, then each segment
/// between unquoted `;`, `&`, `&&`, `|`, `||`, with comments and redirections removed.
fn command_segments(command: &str) -> Vec<Vec<String>> {
    let (outer, inner) = extract_substitutions(command);
    let mut segments: Vec<Vec<String>> = inner
        .iter()
        .flat_map(|text| command_segments(text))
        .collect();
    segments.extend(split_segments(&outer));
    segments
}

fn extract_substitutions(command: &str) -> (String, Vec<String>) {
    let chars: Vec<char> = command.chars().collect();
    let mut outer = String::new();
    let mut inner = Vec::new();
    let mut single = false;
    let mut double = false;
    let mut index = 0;
    while index < chars.len() {
        let character = chars[index];
        if character == '\\' && !single {
            outer.push(character);
            if let Some(&next) = chars.get(index + 1) {
                outer.push(next);
            }
            index += 2;
            continue;
        }
        if !single
            && !double
            && character == '#'
            && (index == 0 || chars[index - 1].is_whitespace())
        {
            break;
        }
        match character {
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            '$' if !single
                && chars.get(index + 1) == Some(&'(')
                && chars.get(index + 2) != Some(&'(') =>
            {
                let mut depth = 1;
                let mut end = index + 2;
                while end < chars.len() {
                    match chars[end] {
                        '(' => depth += 1,
                        ')' => depth -= 1,
                        _ => {}
                    }
                    if depth == 0 {
                        break;
                    }
                    end += 1;
                }
                inner.push(chars[index + 2..end.min(chars.len())].iter().collect());
                outer.push_str("SUBST");
                index = end + 1;
                continue;
            }
            '`' if !single => {
                let end = chars[index + 1..]
                    .iter()
                    .position(|&c| c == '`')
                    .map_or(chars.len(), |offset| index + 1 + offset);
                inner.push(chars[index + 1..end].iter().collect());
                outer.push_str("SUBST");
                index = end + 1;
                continue;
            }
            _ => {}
        }
        outer.push(character);
        index += 1;
    }
    (outer, inner)
}

fn split_segments(line: &str) -> Vec<Vec<String>> {
    let chars: Vec<char> = line.chars().collect();
    let mut segments: Vec<Vec<(String, bool)>> = Vec::new();
    let mut tokens: Vec<(String, bool)> = Vec::new();
    let mut current = String::new();
    let mut started = false;
    let mut literal = false;
    let mut quote: Option<char> = None;
    let flush = |tokens: &mut Vec<(String, bool)>,
                 current: &mut String,
                 started: &mut bool,
                 literal: &mut bool| {
        if *started {
            tokens.push((std::mem::take(current), *literal));
        }
        *started = false;
        *literal = false;
    };
    let mut index = 0;
    while index < chars.len() {
        let character = chars[index];
        match quote {
            Some(open) if character == open => quote = None,
            Some('"')
                if character == '\\'
                    && matches!(chars.get(index + 1), Some('"' | '\\' | '$' | '`')) =>
            {
                index += 1;
                current.push(chars[index]);
            }
            Some(_) => current.push(character),
            None => match character {
                '\'' | '"' => {
                    literal |= !started;
                    quote = Some(character);
                    started = true;
                }
                '\\' => {
                    literal |= !started;
                    if let Some(&next) = chars.get(index + 1) {
                        current.push(next);
                        index += 1;
                    }
                    started = true;
                }
                '#' if !started => break,
                ';' | '|' => {
                    flush(&mut tokens, &mut current, &mut started, &mut literal);
                    segments.push(std::mem::take(&mut tokens));
                }
                '&' if started && (current.ends_with('>') || current.ends_with('<')) => {
                    current.push(character);
                }
                '&' if !started && chars.get(index + 1) == Some(&'>') => {
                    current.push(character);
                    started = true;
                }
                '&' => {
                    flush(&mut tokens, &mut current, &mut started, &mut literal);
                    segments.push(std::mem::take(&mut tokens));
                }
                c if c.is_whitespace() => {
                    flush(&mut tokens, &mut current, &mut started, &mut literal);
                }
                _ => {
                    current.push(character);
                    started = true;
                }
            },
        }
        index += 1;
    }
    flush(&mut tokens, &mut current, &mut started, &mut literal);
    segments.push(tokens);
    segments
        .into_iter()
        .map(drop_redirections)
        .filter(|segment| !segment.is_empty())
        .collect()
}

fn drop_redirections(tokens: Vec<(String, bool)>) -> Vec<String> {
    let mut out = Vec::new();
    let mut iter = tokens.into_iter();
    while let Some((text, literal)) = iter.next() {
        if !literal && !PLACEHOLDER.is_match(&text) {
            if let Some(captures) = REDIRECT.captures(&text) {
                if captures[1].is_empty() {
                    iter.next();
                }
                continue;
            }
        }
        out.push(text);
    }
    out
}

/// Skips environment assignments and commands that run another command: `sudo`, `env`, `time`.
fn strip_prefix_commands(mut tokens: &[String]) -> &[String] {
    loop {
        while tokens.first().is_some_and(|token| ENV_ASSIGNMENT.is_match(token)) {
            tokens = &tokens[1..];
        }
        let Some(first) = tokens.first() else {
            return tokens;
        };
        let name = first.rsplit('/').next().unwrap_or(first);
        let (value_flags, positionals): (&[&str], usize) = match name {
            "sudo" | "doas" => (
                &[
                    "-u", "-g", "-h", "-p", "-C", "-D", "-R", "-T", "-U", "--user", "--group",
                ],
                0,
            ),
            "env" => (&["-u", "--unset", "-C", "--chdir", "-S", "--split-string"], 0),
            "time" => (&["-o", "-f", "--output", "--format"], 0),
            "nice" => (&["-n", "--adjustment"], 0),
            "watch" => (&["-n", "--interval"], 0),
            "timeout" | "gtimeout" => (&["-s", "-k", "--signal", "--kill-after"], 1),
            "xargs" => (&["-n", "-I", "-P", "-L", "-s", "-d", "-E", "-a"], 0),
            "nohup" | "exec" | "command" | "builtin" | "noglob" => (&[], 0),
            _ => return tokens,
        };
        tokens = &tokens[1..];
        while let Some(token) = tokens.first() {
            if name == "env" && ENV_ASSIGNMENT.is_match(token) {
                tokens = &tokens[1..];
                continue;
            }
            if token == "--" {
                tokens = &tokens[1..];
                break;
            }
            if !token.starts_with('-') || token == "-" {
                break;
            }
            let takes_value = value_flags.contains(&token.as_str());
            tokens = &tokens[1..];
            if takes_value && !tokens.is_empty() {
                tokens = &tokens[1..];
            }
        }
        tokens = &tokens[positionals.min(tokens.len())..];
    }
}

/// TOML manifests and manifest fragments validate against the real schema; JSON protocol
/// records decode with the real protocol; other TOML, JSON, and JSONL blocks parse.
pub fn code_blocks_parse(document: &Document) -> Gate {
    let mut failures = Vec::new();
    let mut checked = 0;
    let blocks = document
        .code_blocks
        .iter()
        .map(|block| (block.line, block.lang.clone(), block.body.clone()))
        .chain(container_snippets(document).into_iter().map(|snippet| {
            let body: String = snippet
                .lines
                .iter()
                .map(|(_, text)| format!("{text}\n"))
                .collect();
            (snippet.line, snippet.lang, body)
        }));
    for (line, lang, body) in blocks {
        let result = match lang.as_str() {
            "toml" => {
                checked += 1;
                toml::from_str::<toml::Table>(&body)
                    .map_err(|error| format!("TOML invalid: {error}"))
                    .and_then(|table| check_manifest(&table, &body))
            }
            "json" | "jsonc" | "json5" => {
                checked += 1;
                check_json(&body, lang != "json")
            }
            "jsonl" | "ndjson" => {
                checked += 1;
                body.lines()
                    .filter(|record| !record.trim().is_empty())
                    .try_for_each(|record| check_json(record, false))
                    .map_err(|error| format!("JSONL record: {error}"))
            }
            "" | "ini" | "cfg" | "conf" => match toml::from_str::<toml::Table>(&body) {
                Ok(table) if is_manifest(&table) => {
                    checked += 1;
                    check_manifest(&table, &body)
                }
                _ => Ok(()),
            },
            _ => Ok(()),
        };
        if let Err(reason) = result {
            let reason = reason.split_whitespace().collect::<Vec<_>>().join(" ");
            failures.push(format!("block at line {line}: {reason}"));
        }
    }
    Gate::new(
        "code_blocks_parse",
        &failures,
        format!("{checked} structured blocks parse"),
    )
}

fn is_manifest(table: &toml::Table) -> bool {
    MANIFEST_KEYS.iter().any(|key| table.contains_key(*key))
}

fn check_manifest(table: &toml::Table, body: &str) -> Result<(), String> {
    if !is_manifest(table) {
        return Ok(());
    }
    if table.contains_key("schema_version") {
        return autoresearch_config::ValidatedManifest::parse(body)
            .map(|_| ())
            .map_err(|error| format!("manifest invalid: {error}"));
    }
    validate_fragment(table)
}

/// Replaces the fragment's tables in a minimal valid manifest (merging inside `[experiment]` and
/// into the single evaluator)
/// and validates the result, aligning the objective with whichever side the fragment omits.
fn validate_fragment(fragment: &toml::Table) -> Result<(), String> {
    let mut manifest: toml::Table = toml::from_str(FRAGMENT_BASE).expect("static manifest");
    for (key, value) in fragment {
        match (manifest.get_mut(key), value) {
            (Some(toml::Value::Table(base)), toml::Value::Table(overlay)) if key == "experiment" => {
                for (inner_key, inner_value) in overlay {
                    base.insert(inner_key.clone(), inner_value.clone());
                }
            }
            (Some(toml::Value::Array(base)), toml::Value::Table(overlay)) if key == "evaluators" => {
                if let Some(evaluator) = base.first_mut().and_then(toml::Value::as_table_mut) {
                    merge_evaluator(evaluator, overlay);
                }
            }
            _ => {
                manifest.insert(key.clone(), value.clone());
            }
        }
    }
    let fragment_objective = fragment
        .get("experiment")
        .and_then(|experiment| experiment.get("objective"))
        .cloned();
    let fragment_evaluators = fragment.contains_key("evaluators");
    match (fragment_objective, fragment_evaluators) {
        (Some(objective), false) => {
            if let Some(toml::Value::Array(evaluators)) = manifest.get_mut("evaluators") {
                for metric in evaluators
                    .iter_mut()
                    .filter_map(|evaluator| evaluator.get_mut("metrics"))
                    .filter_map(toml::Value::as_array_mut)
                    .flatten()
                    .filter_map(toml::Value::as_table_mut)
                {
                    for key in ["name", "direction"] {
                        if let Some(value) = objective.get(key) {
                            metric.insert(key.to_owned(), value.clone());
                        }
                    }
                }
            }
        }
        (None, true) => {
            let declared = fragment_metrics(fragment)
                .into_iter()
                .find(|metric| is_objective_metric(metric))
                .cloned();
            if let (Some(metric), Some(objective)) = (
                declared,
                manifest
                    .get_mut("experiment")
                    .and_then(|experiment| experiment.get_mut("objective"))
                    .and_then(toml::Value::as_table_mut),
            ) {
                for key in ["name", "direction"] {
                    if let Some(value) = metric.get(key) {
                        objective.insert(key.to_owned(), value.clone());
                    }
                }
            }
        }
        _ => {}
    }
    let source = toml::to_string(&manifest).map_err(|error| format!("fragment: {error}"))?;
    autoresearch_config::ValidatedManifest::parse(&source)
        .map(|_| ())
        .map_err(|error| {
            format!(
                "manifest fragment invalid over a minimal manifest: {}",
                error.to_string().replace(FRAGMENT_OBJECTIVE, "<objective>")
            )
        })
}

/// A bare `[[evaluators.metrics]]` or `[evaluators.*]` fragment parses as one table, not an
/// evaluator array; it belongs to the base manifest's single evaluator. Its metrics join the
/// base metrics, displacing the base objective when the fragment declares its own.
fn merge_evaluator(evaluator: &mut toml::Table, overlay: &toml::Table) {
    for (key, value) in overlay {
        match (evaluator.get_mut(key), value) {
            (Some(toml::Value::Array(metrics)), toml::Value::Array(added)) if key == "metrics" => {
                if added.iter().any(is_objective_metric) {
                    metrics.retain(|metric| !is_objective_metric(metric));
                }
                metrics.extend(added.iter().cloned());
            }
            _ => {
                evaluator.insert(key.clone(), value.clone());
            }
        }
    }
}

fn fragment_metrics(fragment: &toml::Table) -> Vec<&toml::Value> {
    let evaluators = match fragment.get("evaluators") {
        Some(toml::Value::Array(list)) => list.iter().collect(),
        Some(table @ toml::Value::Table(_)) => vec![table],
        _ => Vec::new(),
    };
    evaluators
        .into_iter()
        .filter_map(|evaluator| evaluator.get("metrics"))
        .filter_map(toml::Value::as_array)
        .flatten()
        .collect()
}

fn is_objective_metric(metric: &toml::Value) -> bool {
    metric.get("kind").and_then(toml::Value::as_str) == Some("objective")
}

/// `relaxed` admits comments and trailing commas. An elided excerpt (`...`) is syntax-checked
/// only; a complete record carrying `protocol_version` must decode as protocol v1.
fn check_json(body: &str, relaxed: bool) -> Result<(), String> {
    let (text, elided) = normalize_json(body, relaxed);
    let value: serde_json::Value =
        serde_json::from_str(&text).map_err(|error| format!("JSON invalid: {error}"))?;
    let Some(object) = value.as_object().filter(|_| !elided) else {
        return Ok(());
    };
    if !object.contains_key("protocol_version") {
        return Ok(());
    }
    let mut record = serde_json::to_vec(&value).map_err(|error| error.to_string())?;
    record.push(b'\n');
    if object.contains_key("candidate_worktree") || object.contains_key("evaluator_id") {
        autoresearch_evaluator::decode_request(&record)
            .map(|_| ())
            .map_err(|error| format!("protocol request invalid: {error}"))
    } else {
        autoresearch_evaluator::decode_response(&record)
            .map(|_| ())
            .map_err(|error| format!("protocol response invalid: {error}"))
    }
}

fn normalize_json(body: &str, relaxed: bool) -> (String, bool) {
    let chars: Vec<char> = body.chars().collect();
    let mut out = String::with_capacity(body.len());
    let mut elided = false;
    let mut in_string = false;
    let mut escaped = false;
    let mut index = 0;
    while index < chars.len() {
        let character = chars[index];
        if in_string {
            out.push(character);
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
            }
            index += 1;
            continue;
        }
        let next = chars.get(index + 1).copied();
        if character == '"' {
            in_string = true;
        } else if relaxed && character == '/' && next == Some('/') {
            while index < chars.len() && chars[index] != '\n' {
                index += 1;
            }
            continue;
        } else if relaxed && character == '/' && next == Some('*') {
            index += 2;
            while index < chars.len() && !(chars[index] == '*' && chars.get(index + 1) == Some(&'/'))
            {
                index += 1;
            }
            index += 2;
            continue;
        } else if character == '…' || chars[index..].starts_with(&['.', '.', '.']) {
            elided = true;
            index += if character == '…' { 1 } else { 3 };
            if out.trim_end().ends_with(':') {
                out.push_str("null");
            }
            continue;
        }
        out.push(character);
        index += 1;
    }
    if relaxed || elided {
        out = drop_dangling_commas(&out);
    }
    (out, elided)
}

fn drop_dangling_commas(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut in_string = false;
    let mut escaped = false;
    for (index, &character) in chars.iter().enumerate() {
        if in_string {
            out.push(character);
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
            }
            continue;
        }
        if character == '"' {
            in_string = true;
        } else if character == ',' {
            let next = chars[index + 1..].iter().find(|c| !c.is_whitespace());
            let previous = out.chars().rev().find(|c| !c.is_whitespace());
            if matches!(next, None | Some(']' | '}' | ','))
                || matches!(previous, Some('[' | '{'))
            {
                continue;
            }
        }
        out.push(character);
    }
    out
}

struct LimitUnit {
    line: usize,
    text: String,
    /// Inside a section whose heading already negates, as in "What this does not do".
    inherited: bool,
    table_row: bool,
}

/// Every configured limit is still stated with a negation that governs its term; no sentence,
/// list item, table row, or heading affirms a limit term with claim wording in the term's clause;
/// and no un-negated clause of a sentence, list item, table cell, or heading matches a limit's
/// `claims_any` pattern.
pub fn limits_preserved(document: &Document, config: &Config) -> Gate {
    let headings = document.rendered_headings();
    let negating_section = |line: usize| {
        let mut level = usize::MAX;
        headings
            .iter()
            .rev()
            .filter(|heading| heading.line < line)
            .filter(|heading| {
                let ancestor = heading.level < level;
                level = level.min(heading.level);
                ancestor
            })
            .any(|heading| NEGATION.is_match(&heading.text) || LIMITS_HEADING.is_match(&heading.text))
    };

    let mut units = Vec::new();
    for (line, text) in document.prose_units() {
        let inherited =
            document.lines[line - 1].kind == LineKind::ListItem && negating_section(line);
        for sentence in limit_sentences(&text) {
            units.push(LimitUnit {
                line,
                text: sentence,
                inherited,
                table_row: false,
            });
        }
    }
    let mut cells = Vec::new();
    for line in &document.lines {
        if line.kind == LineKind::TableRow {
            let inherited = negating_section(line.number);
            units.push(LimitUnit {
                line: line.number,
                text: line.plain.clone(),
                inherited,
                table_row: true,
            });
            cells.extend(table_cells(&line.nocode).into_iter().map(|text| LimitUnit {
                line: line.number,
                text,
                inherited,
                table_row: false,
            }));
        }
    }
    for heading in &headings {
        units.push(LimitUnit {
            line: heading.line,
            text: heading.text.clone(),
            inherited: false,
            table_row: false,
        });
    }

    let mut failures = Vec::new();
    for group in &config.limits {
        let terms: Vec<String> = group
            .terms_any
            .iter()
            .map(|term| term.to_ascii_lowercase())
            .collect();
        let mut stated = false;
        for unit in &units {
            let lower = unit.text.to_ascii_lowercase();
            let mut contradicted = false;
            for term in &terms {
                for (start, _) in lower.match_indices(term.as_str()) {
                    let end = start + term.len();
                    if unit.inherited || scoped_negation(&lower, start, end) {
                        stated = true;
                    } else if clause_claims(&lower, start, end) {
                        contradicted = true;
                    }
                }
            }
            if contradicted {
                failures.push(format!(
                    "line {}: limit `{}` contradicted by \"{}\"",
                    unit.line,
                    group.id,
                    excerpt(&unit.text)
                ));
            }
        }
        let patterns = group
            .claims_any
            .iter()
            .map(|pattern| Regex::new(pattern))
            .collect::<Result<Vec<_>, _>>()
            .unwrap_or_else(|error| {
                let error = error.to_string().split_whitespace().collect::<Vec<_>>().join(" ");
                failures.push(format!("limit `{}` claims_any pattern invalid: {error}", group.id));
                Vec::new()
            });
        let claim_units = units
            .iter()
            .filter(|unit| !unit.table_row)
            .chain(&cells)
            .filter(|unit| !unit.inherited);
        for unit in claim_units {
            for clause in CLAUSE_BREAK.split(&unit.text) {
                if !NEGATION.is_match(clause)
                    && patterns.iter().any(|pattern| pattern.is_match(clause))
                {
                    failures.push(format!(
                        "line {}: limit `{}` overclaimed by \"{}\"",
                        unit.line,
                        group.id,
                        excerpt(clause)
                    ));
                }
            }
        }
        if !stated {
            failures.push(format!("limit `{}` no longer stated", group.id));
        }
    }
    Gate::new(
        "limits_preserved",
        &failures,
        format!("{} limits stated", config.limits.len()),
    )
}

/// Sentences, re-joined after abbreviations that end in a period.
fn limit_sentences(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for sentence in sentences(text) {
        if let Some(last) = out.last_mut() {
            let lower = last.to_ascii_lowercase();
            if ["e.g.", "i.e.", "vs.", "cf."]
                .iter()
                .any(|abbreviation| lower.ends_with(abbreviation))
            {
                last.push(' ');
                last.push_str(&sentence);
                continue;
            }
        }
        out.push(sentence);
    }
    out
}

/// Un-negated claim wording inside the clause holding the term at `start..end`; a modifier claim
/// must also sit within `MODIFIER_REACH` words of the term.
fn clause_claims(text: &str, start: usize, end: usize) -> bool {
    let from = CLAUSE_BREAK
        .find_iter(&text[..start])
        .last()
        .map_or(0, |found| found.end());
    let to = CLAUSE_BREAK
        .find(&text[end..])
        .map_or(text.len(), |found| end + found.start());
    let affirmed = |claim: &regex::Match<'_>| {
        claim.start() >= from
            && claim.end() <= to
            && !scoped_negation(text, claim.start(), claim.end())
    };
    CLAIM.find_iter(text).any(|claim| affirmed(&claim))
        || MODIFIER_CLAIM.find_iter(text).any(|claim| {
            let gap = if claim.end() <= start {
                &text[claim.end()..start]
            } else if claim.start() >= end {
                &text[end..claim.start()]
            } else {
                ""
            };
            gap.split_whitespace().count() <= MODIFIER_REACH && affirmed(&claim)
        })
}

/// Rendered text of each cell in a table row whose inline code is already blanked.
fn table_cells(row: &str) -> Vec<String> {
    let trimmed = row.trim();
    let inner = trimmed.strip_prefix('|').unwrap_or(trimmed);
    let inner = inner.strip_suffix('|').unwrap_or(inner);
    let mut cells = Vec::new();
    let mut current = String::new();
    let mut escaped = false;
    for character in inner.chars() {
        if character == '|' && !escaped {
            cells.push(std::mem::take(&mut current));
        } else {
            current.push(character);
        }
        escaped = character == '\\';
    }
    cells.push(current);
    cells
        .iter()
        .map(|cell| {
            rendered_text(cell)
                .replace("__", "")
                .replace('*', "")
                .trim()
                .to_owned()
        })
        .filter(|cell| !cell.is_empty())
        .collect()
}

/// A negation governs the term when it precedes it in the same clause, or follows it closely
/// as "is not", "out of scope", and similar.
fn scoped_negation(text: &str, start: usize, end: usize) -> bool {
    let before = &text[..start];
    let clause = match CLAUSE_BREAK.find_iter(before).last() {
        Some(found) if found.as_str() == ":" && COLON_LEAD_IN.is_match(&before[..found.start()]) => {
            return true;
        }
        Some(found) => &before[found.end()..],
        None => before,
    };
    let words: Vec<&str> = clause.split_whitespace().collect();
    let window = words[words.len().saturating_sub(NEGATION_WINDOW)..].join(" ");
    if NEGATION.is_match(&window) {
        return true;
    }
    let after = &text[end..];
    let clause = CLAUSE_BREAK
        .find(after)
        .map_or(after, |found| &after[..found.start()]);
    let clause = clause.split([',', '.']).next().unwrap_or_default();
    let window = clause
        .split_whitespace()
        .take(POSTFIX_NEGATION_WINDOW)
        .collect::<Vec<_>>()
        .join(" ");
    POSTFIX_NEGATION.is_match(&window)
}

fn excerpt(text: &str) -> String {
    let text = text.trim();
    match text.char_indices().nth(CLAIM_EXCERPT) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text.to_owned(),
    }
}

/// Fences close, table rows match their header width as GitHub splits them, one H1 across ATX,
/// setext, and `<h1>` headings, no skipped heading levels.
pub fn markdown_well_formed(document: &Document) -> Gate {
    let mut failures = Vec::new();
    for block in document.code_blocks.iter().filter(|block| !block.closed) {
        failures.push(format!("code fence at line {} never closes", block.line));
    }
    for block in document
        .blocks
        .iter()
        .filter(|block| block.kind == BlockKind::Table)
    {
        let widths: Vec<(usize, usize)> = block
            .lines
            .iter()
            .map(|&index| {
                let line = &document.lines[index];
                (line.number, table_width(&line.raw))
            })
            .collect();
        if let Some(&(_, header)) = widths.first() {
            for &(number, width) in &widths[1..] {
                if width != header {
                    failures.push(format!(
                        "table row at line {number} has {width} cells, header has {header}"
                    ));
                }
            }
        }
    }
    let headings = document.rendered_headings();
    let html_h1 = document
        .lines
        .iter()
        .filter(|line| line.kind == LineKind::Html)
        .map(|line| H1_TAG.find_iter(&line.raw).count())
        .sum::<usize>();
    let top_level = headings.iter().filter(|heading| heading.level == 1).count() + html_h1;
    if top_level > 1 {
        failures.push(format!("{top_level} H1 headings"));
    }
    for pair in headings.windows(2) {
        if pair[1].level > pair[0].level + 1 {
            failures.push(format!(
                "heading at line {} jumps from H{} to H{}",
                pair[1].line, pair[0].level, pair[1].level
            ));
        }
    }
    Gate::new(
        "markdown_well_formed",
        &failures,
        "fences, tables, and heading levels well formed".into(),
    )
}

/// Ids of configured use-case lanes no section satisfies.
///
/// A section claims a lane when its heading names it (and at most one other lane). Evidence is
/// matched on rendered content only, with numeric tokens bounded so `52.4` does not match
/// `152.47`. A token without digits is a name, so it counts only inside a table row or code.
pub fn uncovered_use_cases(document: &Document, config: &Config) -> Vec<String> {
    let headings = document.rendered_headings();
    let keywords: Vec<Vec<String>> = config
        .use_cases
        .iter()
        .map(|lane| lane.heading_any.iter().map(|keyword| normalize_title(keyword)).collect())
        .collect();
    let mut covered = vec![false; config.use_cases.len()];
    for (index, heading) in headings.iter().enumerate() {
        if heading.level < 2 {
            continue;
        }
        let title = normalize_title(&heading_source(document, heading));
        let lanes: Vec<usize> = keywords
            .iter()
            .enumerate()
            .filter(|(_, words)| words.iter().any(|keyword| title.contains(keyword.as_str())))
            .map(|(lane, _)| lane)
            .collect();
        if lanes.is_empty() || lanes.len() > LANES_PER_HEADING {
            continue;
        }
        let end = headings[index + 1..]
            .iter()
            .find(|next| next.level <= heading.level)
            .map_or(document.lines.len(), |next| next.line - 1);
        let body = &document.lines[heading.line.min(end)..end];
        let artifact = body
            .iter()
            .any(|line| matches!(line.kind, LineKind::Fence | LineKind::TableRow));
        for lane_index in lanes {
            let lane = &config.use_cases[lane_index];
            if lane.require_artifact && !artifact {
                continue;
            }
            if body.iter().any(|line| {
                lane.evidence_any
                    .iter()
                    .any(|token| evidence_in(line, token))
            }) {
                covered[lane_index] = true;
            }
        }
    }
    config
        .use_cases
        .iter()
        .zip(covered)
        .filter(|(_, covered)| !covered)
        .map(|(lane, _)| lane.id.clone())
        .collect()
}

fn heading_source(document: &Document, heading: &Heading) -> String {
    document
        .lines
        .get(heading.line - 1)
        .map_or_else(|| heading.text.clone(), |line| line.raw.clone())
}

/// Lowercase rendered heading text with `-` and `_` read as spaces and inline code kept.
fn normalize_title(raw: &str) -> String {
    let trimmed = raw.trim().trim_start_matches('#').trim_end_matches('#');
    let lower = rendered_text(trimmed).to_lowercase().replace(['-', '_'], " ");
    A11Y.replace_all(&lower, "accessibility")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn rendered_text(raw: &str) -> String {
    let text = INLINE_COMMENT.replace_all(raw, " ");
    let text = LINK_TEXT.replace_all(&text, "$1");
    let text = URL.replace_all(&text, " ");
    HTML_TAG.replace_all(&text, " ").replace('`', "")
}

fn evidence_in(line: &Line, token: &str) -> bool {
    let (text, artifact) = match line.kind {
        LineKind::Comment | LineKind::Blank | LineKind::Heading | LineKind::Fence => {
            return false;
        }
        LineKind::Code | LineKind::IndentedCode => (line.raw.clone(), true),
        LineKind::TableRow => (rendered_text(&line.raw), true),
        _ => (rendered_text(&line.raw), false),
    };
    let valued = token.chars().any(|c| c.is_ascii_digit());
    (artifact || valued) && contains_bounded(&text, token)
}

/// Substring match that refuses to extend a number: no digit, or separator plus digit, on the
/// numeric edges of `token`.
fn contains_bounded(text: &str, token: &str) -> bool {
    let numeric_start = token.chars().next().is_some_and(|c| c.is_ascii_digit());
    let numeric_end = token.chars().last().is_some_and(|c| c.is_ascii_digit());
    text.match_indices(token).any(|(start, _)| {
        !(numeric_start && extends_number(text[..start].chars().rev()))
            && !(numeric_end && extends_number(text[start + token.len()..].chars()))
    })
}

fn extends_number(mut chars: impl Iterator<Item = char>) -> bool {
    match chars.next() {
        Some(c) if c.is_ascii_digit() => true,
        Some('.' | ',') => chars.next().is_some_and(|c| c.is_ascii_digit()),
        _ => false,
    }
}

pub fn substance_floor(prose_words: usize, code_blocks: usize, floor: Option<&Substance>) -> Gate {
    let Some(floor) = floor else {
        return Gate::new(
            "substance_floor",
            &["[substance] missing from config".to_owned()],
            String::new(),
        );
    };
    let mut failures = Vec::new();
    if prose_words < floor.min_prose_words {
        failures.push(format!(
            "{prose_words} prose words, floor {}",
            floor.min_prose_words
        ));
    }
    if code_blocks < floor.min_code_blocks {
        failures.push(format!(
            "{code_blocks} code blocks, floor {}",
            floor.min_code_blocks
        ));
    }
    Gate::new(
        "substance_floor",
        &failures,
        format!("{prose_words} prose words, {code_blocks} code blocks"),
    )
}
