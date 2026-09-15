//! Formatting-slop rules loaded from data. Every rule yields integer hits; points are weight times hits.

use crate::markdown::{BlockKind, Document, Line, LineKind, indent_width, sentences, table_width};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;
use std::sync::LazyLock;

const EXAMPLE_LIMIT: usize = 12;
/// Not a word any token pattern can produce.
const CODE_BREAK: &str = "\u{1F}";

static WORD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[\p{L}\p{N}][\p{L}\p{N}'\x{2019}-]*").expect("static regex"));
static LIST_MARKER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\s*)([-*+]|\d{1,9}[.)])(\s+)").expect("static regex"));
static TABLE_DELIMITER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*\|?\s*:?-{3,}").expect("static regex"));
static LINK_DESTINATION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\]\([^)]*\)").expect("static regex"));
static AUTOLINK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<https?://[^>]+>").expect("static regex"));
static REFERENCE_DEFINITION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s{0,3}\[[^\]]+\]:\s*\S+").expect("static regex"));
static HTML_URL_ATTRIBUTE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)\b(?:src|href)\s*=\s*(?:"[^"]*"|'[^']*'|[^\s>]+)"#).expect("static regex")
});
static CALLOUT_LABEL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^\s*(?:\*\*|__)?(?:note|tip|warning|important|caution|hint)\b")
        .expect("static regex")
});
static LINK_ONLY_ITEM: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^\s*(?:[-*+]|\d{1,9}[.)])\s+(?:\[[^\]]+\]\([^)]*\)|`[^`]+`)\s*(?:\([^)]{0,40}\))?\s*$",
    )
    .expect("static regex")
});

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleFile {
    #[serde(rename = "rule")]
    rules: Vec<RuleSpec>,
}

/// Detection strategy. `Regex` scans a scope line by line; the rest are structural.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Regex,
    /// Paragraph sentences with at least `min_sentence_words` words that `pattern` misses.
    Telegraphic,
    HeadingDensity,
    /// Headings matching `pattern` whose section body meets every body limit.
    Section,
    SkippedHeadingLevel,
    HeadingFollowedByHeading,
    NestedListDepth,
    ListDominant,
    ConsecutiveLists,
    RuleOfThree,
    UniformFragmentList,
    HorizontalRules,
    TocShortDoc,
    OneSentenceParagraphs,
    DuplicateShingles,
    WallOfCode,
    /// One hit per table whose body rows match `pattern` enough.
    Table,
}

impl Kind {
    const fn needs_pattern(self) -> bool {
        matches!(
            self,
            Self::Regex
                | Self::Telegraphic
                | Self::Section
                | Self::HorizontalRules
                | Self::TocShortDoc
                | Self::DuplicateShingles
                | Self::Table
        )
    }
}

/// Which units a regex rule scans.
#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// Lines whose kind is listed in `lines`.
    #[default]
    Lines,
    /// Every line, code included.
    All,
    FencedCode,
    FenceOpeners,
    /// The first paragraph line after the H1, past blank, badge, image, and HTML lines.
    FirstParagraph,
    /// First lines of paragraphs in the last section or the last 20% of lines.
    ClosingParagraphs,
    /// Each paragraph, quote, or list item joined into one line of plain text.
    JoinedParagraphs,
}

/// Which view of a line a regex rule reads.
#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TextView {
    Raw,
    Nocode,
    #[default]
    Clean,
    Plain,
    /// `nocode` without link destinations, autolinks, reference definitions, or src/href values.
    LinkStripped,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Combine {
    #[default]
    All,
    Any,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScoreMode {
    #[default]
    Hits,
    /// A firing rule scores one hit.
    Once,
    /// Hits beyond `density_above` (or `density_min`) per `density_words` prose words.
    DensityExcess,
    /// Telegraphic only: hits beyond `ratio_above` of the qualifying sentences.
    RatioExcess,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Relation {
    SameOrShallower,
    Deeper,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleSpec {
    pub id: String,
    pub name: Option<String>,
    pub category: Option<String>,
    pub weight: u32,
    pub kind: Kind,
    pub pattern: Option<String>,
    #[serde(default)]
    pub scope: Scope,
    #[serde(default)]
    pub lines: Vec<LineKind>,
    #[serde(default)]
    pub text: TextView,
    /// At most one hit per scanned unit.
    #[serde(default)]
    pub per_line: bool,
    /// A matching line directly after a matching line adds no hit.
    #[serde(default)]
    pub group_consecutive: bool,
    #[serde(default)]
    pub require_blank_before: bool,
    /// The next line exists, is non-blank, and is not a heading.
    #[serde(default)]
    pub require_text_after: bool,
    /// Heading lines below this level leave the scope; other lines are unaffected.
    pub min_heading_level: Option<usize>,
    pub min_line_words: Option<usize>,
    /// Lines under a heading whose text matches this pattern leave the scope.
    pub unless_ancestor_heading: Option<String>,

    /// How a regex rule's firing clauses combine; structural kinds always require all.
    #[serde(default)]
    pub combine: Combine,
    pub min_hits: Option<usize>,
    pub min_prose_words: Option<usize>,
    pub min_scope_lines: Option<usize>,
    /// Denominator for density clauses; 100 when absent.
    pub density_words: Option<f64>,
    pub density_above: Option<f64>,
    pub density_min: Option<f64>,
    /// Inclusive ratio floor; its numerator and denominator depend on the kind.
    pub min_ratio: Option<f64>,
    /// Exclusive ratio floor; its numerator and denominator depend on the kind.
    pub ratio_above: Option<f64>,
    #[serde(default)]
    pub score: ScoreMode,
    /// Hits dropped from the front after scoring.
    pub subtract: Option<usize>,

    pub min_sentence_words: Option<usize>,
    /// Telegraphic: list-item sentences qualify from this many words; absent leaves lists out.
    pub list_item_min_words: Option<usize>,
    pub max_sentence_words: Option<usize>,
    pub min_units: Option<usize>,
    pub min_headings: Option<usize>,
    pub max_words_per_heading: Option<f64>,
    pub max_body_lines: Option<usize>,
    pub max_body_words: Option<usize>,
    pub body_pattern: Option<String>,
    pub separator_pattern: Option<String>,
    pub relation: Option<Relation>,
    pub min_depth: Option<usize>,
    pub list_items: Option<usize>,
    pub min_items: Option<usize>,
    pub max_item_words: Option<usize>,
    pub max_word_spread: Option<usize>,
    pub min_run: Option<usize>,
    pub short_prose_words: Option<usize>,
    pub short_headings: Option<usize>,
    pub shingle_size: Option<usize>,
    pub max_stopwords: Option<usize>,
    #[serde(default)]
    pub stopwords: Vec<String>,
    pub min_block_lines: Option<usize>,
    pub lookback_lines: Option<usize>,
    pub min_total_lines: Option<usize>,
    pub min_body_rows: Option<usize>,
    pub min_matches: Option<usize>,
    pub max_columns: Option<usize>,

    pub example_bad: Option<String>,
    pub example_good: Option<String>,
}

pub struct Rule {
    pub spec: RuleSpec,
    regex: Option<Regex>,
    body: Option<Regex>,
    separator: Option<Regex>,
    ancestor: Option<Regex>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Hit {
    pub line: usize,
    pub excerpt: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuleScore {
    pub id: String,
    pub weight: u32,
    pub hits: usize,
    pub points: u64,
    pub examples: Vec<Hit>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Score {
    pub points: u64,
    pub rules: Vec<RuleScore>,
}

/// Emoji rules silently match nothing when the regex build lacks these Unicode properties.
fn verify_emoji_properties() -> Result<(), String> {
    for (pattern, sample) in [
        (r"\p{Emoji_Presentation}", "\u{1F680}"),
        (r"\p{Extended_Pictographic}", "\u{2764}"),
    ] {
        let regex =
            Regex::new(pattern).map_err(|error| format!("regex build lacks {pattern}: {error}"))?;
        if !regex.is_match(sample) {
            return Err(format!("{pattern} does not match {sample}"));
        }
    }
    Ok(())
}

fn compile(id: &str, field: &str, pattern: Option<&str>) -> Result<Option<Regex>, String> {
    pattern
        .map(|pattern| Regex::new(pattern).map_err(|error| format!("rule `{id}` {field}: {error}")))
        .transpose()
}

/// Loads and compiles a rule file, rejecting duplicate ids and missing patterns.
pub fn load(path: &Path) -> Result<Vec<Rule>, String> {
    verify_emoji_properties()?;
    let source = std::fs::read_to_string(path)
        .map_err(|error| format!("read rules {}: {error}", path.display()))?;
    let file: RuleFile = toml::from_str(&source)
        .map_err(|error| format!("parse rules {}: {error}", path.display()))?;
    let mut seen = HashSet::new();
    file.rules
        .into_iter()
        .map(|spec| {
            if !seen.insert(spec.id.clone()) {
                return Err(format!("duplicate rule id `{}`", spec.id));
            }
            let regex = compile(&spec.id, "pattern", spec.pattern.as_deref())?;
            if regex.is_none() && spec.kind.needs_pattern() {
                return Err(format!("rule `{}` needs a pattern", spec.id));
            }
            let body = compile(&spec.id, "body_pattern", spec.body_pattern.as_deref())?;
            let separator = compile(
                &spec.id,
                "separator_pattern",
                spec.separator_pattern.as_deref(),
            )?;
            let ancestor = compile(
                &spec.id,
                "unless_ancestor_heading",
                spec.unless_ancestor_heading.as_deref(),
            )?;
            Ok(Rule {
                spec,
                regex,
                body,
                separator,
                ancestor,
            })
        })
        .collect()
}

/// Scores the rendered view of `document`, never its source lines.
pub fn score(document: &Document, rules: &[Rule]) -> Score {
    let context = Context::new(document.rendered());
    let rules: Vec<RuleScore> = rules
        .iter()
        .map(|rule| {
            let hits = detect(&context, rule);
            let count = hits.len();
            RuleScore {
                id: rule.spec.id.clone(),
                weight: rule.spec.weight,
                hits: count,
                points: u64::from(rule.spec.weight) * count as u64,
                examples: hits.into_iter().take(EXAMPLE_LIMIT).collect(),
            }
        })
        .collect();
    Score {
        points: rules.iter().map(|rule| rule.points).sum(),
        rules,
    }
}

/// Hits one rule scores on a document.
pub fn hits(document: &Document, rule: &Rule) -> usize {
    detect(&Context::new(document.rendered()), rule).len()
}

struct Context<'a> {
    document: &'a Document,
    prose_words: usize,
    link_stripped: Vec<String>,
}

impl<'a> Context<'a> {
    fn new(document: &'a Document) -> Self {
        let prose_words = document
            .lines
            .iter()
            .filter(|line| {
                matches!(
                    line.kind,
                    LineKind::Paragraph | LineKind::ListItem | LineKind::Quote
                )
            })
            .map(|line| word_count(&line.prose))
            .sum();
        let link_stripped = document
            .lines
            .iter()
            .map(|line| {
                let text = LINK_DESTINATION.replace_all(&line.nocode, "]");
                let text = AUTOLINK.replace_all(&text, " ");
                let text = REFERENCE_DEFINITION.replace_all(&text, " ");
                HTML_URL_ATTRIBUTE.replace_all(&text, " ").into_owned()
            })
            .collect();
        Self {
            document,
            prose_words,
            link_stripped,
        }
    }

    fn lines(&self) -> &'a [Line] {
        &self.document.lines
    }

    fn view(&self, index: usize, view: TextView) -> &str {
        let line = &self.document.lines[index];
        match view {
            TextView::Raw => &line.raw,
            TextView::Nocode => &line.nocode,
            TextView::Clean => &line.clean,
            TextView::Plain => &line.plain,
            TextView::LinkStripped => &self.link_stripped[index],
        }
    }

    fn density(&self, spec: &RuleSpec, count: usize) -> f64 {
        count as f64 * spec.density_words.unwrap_or(100.0) / self.prose_words.max(1) as f64
    }

    /// Headings enclosing line `index`, nearest first.
    fn ancestors(&self, index: usize) -> Vec<&'a str> {
        let lines = self.lines();
        let mut level = if lines[index].kind == LineKind::Heading {
            lines[index].level
        } else {
            7
        };
        let mut out = Vec::new();
        for heading in self.document.headings.iter().rev() {
            if heading.line > index {
                continue;
            }
            if heading.level < level {
                out.push(heading.text.as_str());
                level = heading.level;
            }
        }
        out
    }
}

fn word_count(text: &str) -> usize {
    WORD.find_iter(text).count()
}

fn excerpt(text: &str) -> String {
    let trimmed = text.trim();
    let mut out: String = trimmed.chars().take(80).collect();
    if trimmed.chars().count() > 80 {
        out.push('…');
    }
    out
}

fn hit(line: &Line, text: &str) -> Hit {
    Hit {
        line: line.number,
        excerpt: excerpt(text),
    }
}

fn ratio(numerator: usize, denominator: usize) -> f64 {
    match (numerator, denominator) {
        (0, _) => 0.0,
        (_, 0) => f64::INFINITY,
        _ => numerator as f64 / denominator as f64,
    }
}

fn ratio_ok(spec: &RuleSpec, value: f64) -> bool {
    spec.min_ratio.is_none_or(|floor| value >= floor)
        && spec.ratio_above.is_none_or(|floor| value > floor)
}

fn joined_plain(document: &Document, indices: &[usize]) -> String {
    indices
        .iter()
        .map(|&index| document.lines[index].plain.trim())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Top-level list items of a list block: items indented no deeper than the first item.
fn top_items<'a>(document: &'a Document, indices: &[usize]) -> Vec<&'a Line> {
    let items: Vec<&Line> = indices
        .iter()
        .map(|&index| &document.lines[index])
        .filter(|line| line.kind == LineKind::ListItem)
        .collect();
    let Some(base) = items.first().map(|line| indent_width(&line.raw)) else {
        return Vec::new();
    };
    items
        .into_iter()
        .filter(|line| indent_width(&line.raw) <= base + 1)
        .collect()
}

fn detect(context: &Context<'_>, rule: &Rule) -> Vec<Hit> {
    let spec = &rule.spec;
    let hits = match spec.kind {
        Kind::Regex => return finish(context, spec, regex_rule(context, rule)),
        Kind::Telegraphic => telegraphic(context, rule),
        Kind::HeadingDensity => heading_density(context, spec),
        Kind::Section => section(context, rule),
        Kind::SkippedHeadingLevel => skipped_heading_level(context),
        Kind::HeadingFollowedByHeading => heading_followed_by_heading(context, spec),
        Kind::NestedListDepth => nested_list_depth(context, spec),
        Kind::ListDominant => list_dominant(context, spec),
        Kind::ConsecutiveLists => consecutive_lists(context, rule),
        Kind::RuleOfThree => rule_of_three(context, spec),
        Kind::UniformFragmentList => uniform_fragment_list(context, spec),
        Kind::HorizontalRules => horizontal_rules(context, rule),
        Kind::TocShortDoc => toc_short_doc(context, rule),
        Kind::OneSentenceParagraphs => one_sentence_paragraphs(context, spec),
        Kind::DuplicateShingles => duplicate_shingles(context, rule),
        Kind::WallOfCode => wall_of_code(context, spec),
        Kind::Table => table(context, rule),
    };
    let hits = if spec.min_hits.is_some_and(|minimum| hits.len() < minimum) {
        Vec::new()
    } else {
        hits
    };
    finish(context, spec, hits)
}

fn finish(context: &Context<'_>, spec: &RuleSpec, mut hits: Vec<Hit>) -> Vec<Hit> {
    match spec.score {
        ScoreMode::Hits | ScoreMode::RatioExcess => {}
        ScoreMode::Once => hits.truncate(1),
        ScoreMode::DensityExcess => {
            let allowed = spec.density_above.or(spec.density_min).unwrap_or(0.0)
                * context.prose_words as f64
                / spec.density_words.unwrap_or(100.0);
            hits.drain(..(allowed.floor() as usize).min(hits.len()));
        }
    }
    let subtract = spec.subtract.unwrap_or(0).min(hits.len());
    hits.drain(..subtract);
    hits
}

fn regex_units(context: &Context<'_>, rule: &Rule) -> Vec<(usize, String)> {
    let spec = &rule.spec;
    let document = context.document;
    let lines = context.lines();
    let line_units = |indices: Vec<usize>| -> Vec<(usize, String)> {
        indices
            .into_iter()
            .map(|index| (index, context.view(index, spec.text).to_owned()))
            .collect()
    };
    match spec.scope {
        Scope::Lines => line_units(
            (0..lines.len())
                .filter(|&index| in_line_scope(context, rule, index))
                .collect(),
        ),
        Scope::All => line_units((0..lines.len()).collect()),
        Scope::FencedCode => line_units(
            (0..lines.len())
                .filter(|&index| lines[index].kind == LineKind::Code)
                .collect(),
        ),
        Scope::FenceOpeners => line_units(
            document
                .code_blocks
                .iter()
                .map(|block| block.line - 1)
                .collect(),
        ),
        Scope::FirstParagraph => {
            let start = document
                .headings
                .iter()
                .find(|heading| heading.level == 1)
                .map_or(0, |heading| heading.line);
            let first = (start..lines.len()).find(|&index| {
                let line = &lines[index];
                let badge = line.kind == LineKind::Paragraph && word_count(&line.prose) == 0;
                !(badge
                    || matches!(
                        line.kind,
                        LineKind::Blank | LineKind::Html | LineKind::Comment
                    ))
            });
            line_units(
                first
                    .filter(|&index| lines[index].kind == LineKind::Paragraph)
                    .into_iter()
                    .collect(),
            )
        }
        Scope::ClosingParagraphs => {
            let last_section = document.headings.last().map_or(0, |heading| heading.line);
            let tail = (lines.len() as f64 * 0.8).floor() as usize;
            line_units(
                document
                    .blocks
                    .iter()
                    .filter(|block| block.kind == BlockKind::Paragraph)
                    .map(|block| block.lines[0])
                    .filter(|&index| index >= last_section || index >= tail)
                    .collect(),
            )
        }
        Scope::JoinedParagraphs => document
            .prose_units()
            .into_iter()
            .map(|(number, text)| (number - 1, text))
            .collect(),
    }
}

fn in_line_scope(context: &Context<'_>, rule: &Rule, index: usize) -> bool {
    let spec = &rule.spec;
    let lines = context.lines();
    let line = &lines[index];
    if !spec.lines.contains(&line.kind) {
        return false;
    }
    if line.kind == LineKind::Heading
        && spec
            .min_heading_level
            .is_some_and(|minimum| line.level < minimum)
    {
        return false;
    }
    if spec
        .min_line_words
        .is_some_and(|minimum| word_count(&line.prose) < minimum)
    {
        return false;
    }
    if spec.require_blank_before && index > 0 && lines[index - 1].kind != LineKind::Blank {
        return false;
    }
    if spec.require_text_after
        && !lines
            .get(index + 1)
            .is_some_and(|next| !matches!(next.kind, LineKind::Blank | LineKind::Heading))
    {
        return false;
    }
    if let Some(ancestor) = &rule.ancestor
        && context
            .ancestors(index)
            .iter()
            .any(|text| ancestor.is_match(text))
    {
        return false;
    }
    true
}

fn regex_rule(context: &Context<'_>, rule: &Rule) -> Vec<Hit> {
    let spec = &rule.spec;
    let Some(regex) = &rule.regex else {
        return Vec::new();
    };
    let units = regex_units(context, rule);
    let lines = context.lines();
    let mut hits = Vec::new();
    let mut previous_match: Option<usize> = None;
    for (index, text) in &units {
        let found: Vec<_> = if spec.per_line {
            regex.find(text).into_iter().collect()
        } else {
            regex.find_iter(text).collect()
        };
        if found.is_empty() {
            continue;
        }
        let grouped =
            spec.group_consecutive && previous_match.is_some_and(|last| last + 1 == *index);
        previous_match = Some(*index);
        if grouped {
            continue;
        }
        hits.extend(found.into_iter().map(|m| hit(&lines[*index], m.as_str())));
    }

    let count = hits.len();
    let mut clauses = Vec::new();
    if let Some(minimum) = spec.min_hits {
        clauses.push(count >= minimum);
    }
    if let Some(minimum) = spec.min_prose_words {
        clauses.push(context.prose_words >= minimum);
    }
    if let Some(minimum) = spec.min_scope_lines {
        clauses.push(units.len() >= minimum);
    }
    let density = context.density(spec, count);
    if let Some(floor) = spec.density_above {
        clauses.push(density > floor);
    }
    if let Some(floor) = spec.density_min {
        clauses.push(density >= floor);
    }
    if spec.min_ratio.is_some() || spec.ratio_above.is_some() {
        clauses.push(ratio_ok(spec, ratio(count, units.len())));
    }
    let fires = clauses.is_empty()
        || match spec.combine {
            Combine::All => clauses.iter().all(|clause| *clause),
            Combine::Any => clauses.iter().any(|clause| *clause),
        };
    if fires { hits } else { Vec::new() }
}

fn telegraphic(context: &Context<'_>, rule: &Rule) -> Vec<Hit> {
    let spec = &rule.spec;
    let Some(article) = &rule.regex else {
        return Vec::new();
    };
    let document = context.document;
    let paragraph_minimum = spec.min_sentence_words.unwrap_or(6);
    let mut units: Vec<(&Line, String, usize)> = Vec::new();
    for block in &document.blocks {
        match block.kind {
            BlockKind::Paragraph => units.push((
                &document.lines[block.lines[0]],
                joined_plain(document, &block.lines),
                paragraph_minimum,
            )),
            BlockKind::List => {
                let Some(item_minimum) = spec.list_item_min_words else {
                    continue;
                };
                for &index in &block.lines {
                    let line = &document.lines[index];
                    match line.kind {
                        LineKind::ListItem => units.push((line, line.plain.clone(), item_minimum)),
                        LineKind::Paragraph => {
                            if let Some((_, text, _)) = units.last_mut() {
                                text.push(' ');
                                text.push_str(line.plain.trim());
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    let mut qualifying = 0;
    let mut hits = Vec::new();
    for (first, text, minimum) in units {
        if CALLOUT_LABEL.is_match(&first.clean) {
            continue;
        }
        for sentence in sentences(&text) {
            if word_count(&sentence) < minimum {
                continue;
            }
            qualifying += 1;
            if !article.is_match(&sentence) {
                hits.push(hit(first, &sentence));
            }
        }
    }
    let enough = spec.min_units.is_none_or(|minimum| qualifying >= minimum);
    if !(enough && ratio_ok(spec, ratio(hits.len(), qualifying))) {
        return Vec::new();
    }
    if spec.score == ScoreMode::RatioExcess {
        let allowed = spec.ratio_above.or(spec.min_ratio).unwrap_or(0.0) * qualifying as f64;
        hits.drain(..(allowed.floor() as usize).min(hits.len()));
    }
    hits
}

/// Headings whose own body holds a code block introduce reference material and are not counted.
fn heading_density(context: &Context<'_>, spec: &RuleSpec) -> Vec<Hit> {
    let document = context.document;
    let counted: Vec<_> = document
        .headings
        .iter()
        .enumerate()
        .filter(|(index, heading)| {
            let end = document
                .headings
                .get(index + 1)
                .map_or(document.lines.len(), |next| next.line - 1);
            !document.lines[heading.line..end].iter().any(|line| {
                matches!(
                    line.kind,
                    LineKind::Fence | LineKind::Code | LineKind::IndentedCode
                )
            })
        })
        .map(|(_, heading)| heading)
        .collect();
    let headings = counted.len();
    let words = context.prose_words;
    let dense = words > 0
        && spec.min_prose_words.is_none_or(|minimum| words >= minimum)
        && spec
            .density_above
            .is_some_and(|floor| context.density(spec, headings) > floor);
    let fragmented = headings > 0
        && spec.min_headings.is_some_and(|minimum| headings >= minimum)
        && spec
            .max_words_per_heading
            .is_some_and(|ceiling| (words as f64 / headings as f64) < ceiling);
    if !(dense || fragmented) {
        return Vec::new();
    }
    counted
        .into_iter()
        .map(|heading| hit(&document.lines[heading.line - 1], &heading.text))
        .collect()
}

fn section(context: &Context<'_>, rule: &Rule) -> Vec<Hit> {
    let spec = &rule.spec;
    let Some(heading_pattern) = &rule.regex else {
        return Vec::new();
    };
    let document = context.document;
    let mut hits = Vec::new();
    for (index, heading) in document.headings.iter().enumerate() {
        let line = &document.lines[heading.line - 1];
        if !heading_pattern.is_match(&line.nocode) {
            continue;
        }
        let (start, end) = document.section(index);
        let body = &document.lines[start..end];
        let non_blank = body
            .iter()
            .filter(|line| line.kind != LineKind::Blank)
            .count();
        let text = body
            .iter()
            .map(|line| {
                if line.plain.is_empty() {
                    line.raw.as_str()
                } else {
                    line.plain.as_str()
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
        let thin = spec
            .max_body_lines
            .is_none_or(|maximum| non_blank <= maximum)
            && spec
                .max_body_words
                .is_none_or(|maximum| word_count(&text) <= maximum)
            && rule.body.as_ref().is_none_or(|body| body.is_match(&text));
        if thin {
            hits.push(hit(line, &heading.text));
        }
    }
    hits
}

fn skipped_heading_level(context: &Context<'_>) -> Vec<Hit> {
    let document = context.document;
    document
        .headings
        .windows(2)
        .filter(|pair| pair[1].level > pair[0].level + 1)
        .map(|pair| hit(&document.lines[pair[1].line - 1], &pair[1].text))
        .collect()
}

fn heading_followed_by_heading(context: &Context<'_>, spec: &RuleSpec) -> Vec<Hit> {
    let document = context.document;
    let relation = spec.relation.unwrap_or(Relation::SameOrShallower);
    document
        .headings
        .iter()
        .filter(|heading| {
            let next = document.lines[heading.line..]
                .iter()
                .find(|line| line.kind != LineKind::Blank);
            next.is_some_and(|next| {
                next.kind == LineKind::Heading
                    && match relation {
                        Relation::SameOrShallower => next.level <= heading.level,
                        Relation::Deeper => next.level > heading.level,
                    }
            })
        })
        .map(|heading| hit(&document.lines[heading.line - 1], &heading.text))
        .collect()
}

fn nested_list_depth(context: &Context<'_>, spec: &RuleSpec) -> Vec<Hit> {
    let minimum = spec.min_depth.unwrap_or(3);
    let mut stack: Vec<(usize, usize)> = Vec::new();
    let mut hits = Vec::new();
    for line in context.lines() {
        match line.kind {
            LineKind::ListItem => {
                let Some(captures) = LIST_MARKER.captures(&line.raw) else {
                    continue;
                };
                let indent = indent_width(&captures[1]);
                let content = indent + captures[2].chars().count() + indent_width(&captures[3]);
                while stack.last().is_some_and(|top| indent < top.0) {
                    stack.pop();
                }
                if stack.last().is_some_and(|top| indent < top.1) {
                    stack.pop();
                }
                stack.push((indent, content));
                if stack.len() >= minimum && !link_only(line) {
                    hits.push(hit(line, &line.plain));
                }
            }
            LineKind::Blank => {}
            _ if indent_width(&line.raw) >= 2 => {}
            _ => stack.clear(),
        }
    }
    hits
}

fn list_dominant(context: &Context<'_>, spec: &RuleSpec) -> Vec<Hit> {
    let lines = context.lines();
    let items: Vec<&Line> = lines
        .iter()
        .filter(|line| line.kind == LineKind::ListItem && !link_only(line))
        .collect();
    let paragraphs = lines
        .iter()
        .filter(|line| line.kind == LineKind::Paragraph)
        .count();
    let enough = spec
        .min_prose_words
        .is_none_or(|minimum| context.prose_words >= minimum);
    if enough && ratio_ok(spec, ratio(items.len(), paragraphs)) {
        items
            .first()
            .map(|line| {
                hit(
                    line,
                    &format!("{} list lines, {paragraphs} paragraph lines", items.len()),
                )
            })
            .into_iter()
            .collect()
    } else {
        Vec::new()
    }
}

fn consecutive_lists(context: &Context<'_>, rule: &Rule) -> Vec<Hit> {
    let document = context.document;
    let blocks = &document.blocks;
    let separates = |block: &crate::markdown::Block| {
        block.kind == BlockKind::Heading
            || (block.kind == BlockKind::Paragraph
                && block.lines.len() == 1
                && rule.separator.as_ref().is_some_and(|pattern| {
                    pattern.is_match(&document.lines[block.lines[0]].nocode)
                }))
    };
    let mut hits = Vec::new();
    for (index, block) in blocks.iter().enumerate() {
        if block.kind != BlockKind::List {
            continue;
        }
        let mut next = index + 1;
        while next < blocks.len() && separates(&blocks[next]) {
            next += 1;
        }
        if let Some(following) = blocks.get(next)
            && following.kind == BlockKind::List
        {
            let line = &document.lines[following.lines[0]];
            hits.push(hit(line, &line.plain));
        }
    }
    hits
}

fn rule_of_three(context: &Context<'_>, spec: &RuleSpec) -> Vec<Hit> {
    let document = context.document;
    let target = spec.list_items.unwrap_or(3);
    let mut multi_item = 0;
    let mut hits = Vec::new();
    for block in document
        .blocks
        .iter()
        .filter(|block| block.kind == BlockKind::List)
    {
        let items = top_items(document, &block.lines);
        if items.len() >= 2 {
            multi_item += 1;
        }
        if items.len() == target {
            hits.push(hit(items[0], &items[0].plain));
        }
    }
    if ratio_ok(spec, ratio(hits.len(), multi_item)) {
        hits
    } else {
        Vec::new()
    }
}

fn uniform_fragment_list(context: &Context<'_>, spec: &RuleSpec) -> Vec<Hit> {
    let document = context.document;
    let mut hits = Vec::new();
    for block in document
        .blocks
        .iter()
        .filter(|block| block.kind == BlockKind::List)
    {
        let all = top_items(document, &block.lines);
        let links = all.iter().filter(|item| link_only(item)).count();
        if links * 2 > all.len() {
            continue;
        }
        let items: Vec<&Line> = all.into_iter().filter(|item| !link_only(item)).collect();
        if items.len() < spec.min_items.unwrap_or(4) {
            continue;
        }
        let counts: Vec<usize> = items.iter().map(|item| word_count(&item.plain)).collect();
        let longest = counts.iter().copied().max().unwrap_or(0);
        let shortest = counts.iter().copied().min().unwrap_or(0);
        let terminal = items
            .iter()
            .filter(|item| item.plain.trim_end().ends_with(['.', '!', '?']))
            .count();
        if longest <= spec.max_item_words.unwrap_or(6)
            && terminal * 2 <= items.len()
            && longest - shortest <= spec.max_word_spread.unwrap_or(2)
        {
            hits.push(hit(items[0], &items[0].plain));
        }
    }
    hits
}

fn horizontal_rules(context: &Context<'_>, rule: &Rule) -> Vec<Hit> {
    let Some(pattern) = &rule.regex else {
        return Vec::new();
    };
    let lines = context.lines();
    let front_matter = lines.first().is_some_and(|line| line.raw.trim() == "---");
    let front_closer = front_matter
        .then(|| {
            lines
                .iter()
                .skip(1)
                .position(|line| matches!(line.raw.trim(), "---" | "..."))
                .map(|position| position + 1)
        })
        .flatten();
    let rules: Vec<usize> = (0..lines.len())
        .filter(|&index| {
            let line = &lines[index];
            line.kind == LineKind::Rule
                && pattern.is_match(&line.raw)
                && !(front_matter && index == 0)
                && front_closer != Some(index)
                && !(index > 0 && lines[index - 1].kind != LineKind::Blank)
        })
        .collect();
    rules
        .into_iter()
        .filter(|&index| {
            lines[index + 1..]
                .iter()
                .find(|line| line.kind != LineKind::Blank)
                .is_some_and(|next| next.kind == LineKind::Heading)
        })
        .map(|index| hit(&lines[index], &lines[index].raw))
        .collect()
}

fn toc_short_doc(context: &Context<'_>, rule: &Rule) -> Vec<Hit> {
    let spec = &rule.spec;
    let Some(pattern) = &rule.regex else {
        return Vec::new();
    };
    let lines = context.lines();
    let mut first: Option<&Line> = None;
    let mut run = 0;
    let mut longest_run = 0;
    let mut toc_heading = false;
    for line in lines {
        let matched = matches!(line.kind, LineKind::Heading | LineKind::ListItem)
            && pattern.is_match(&line.nocode);
        if matched && first.is_none() {
            first = Some(line);
        }
        if matched && line.kind == LineKind::Heading {
            toc_heading = true;
        }
        if matched && line.kind == LineKind::ListItem {
            run += 1;
            longest_run = longest_run.max(run);
        } else if line.kind != LineKind::Blank {
            run = 0;
        }
    }
    let has_toc = (toc_heading && longest_run >= 2) || longest_run >= spec.min_run.unwrap_or(4);
    let short = spec
        .short_prose_words
        .is_some_and(|ceiling| context.prose_words < ceiling)
        || spec
            .short_headings
            .is_some_and(|ceiling| context.document.headings.len() < ceiling);
    match first {
        Some(line) if has_toc && short => vec![hit(line, &line.raw)],
        _ => Vec::new(),
    }
}

fn one_sentence_paragraphs(context: &Context<'_>, spec: &RuleSpec) -> Vec<Hit> {
    let document = context.document;
    let max_words = spec.max_sentence_words.unwrap_or(25);
    let min_run = spec.min_run.unwrap_or(4);
    let mut hits = Vec::new();
    let mut run = 0;
    let mut total = 0;
    let mut single = 0;
    let mut first_single: Option<&Line> = None;
    for (index, block) in document.blocks.iter().enumerate() {
        if block.kind == BlockKind::Paragraph {
            total += 1;
            let text = joined_plain(document, &block.lines);
            let lead_in = text.trim_end().ends_with(':')
                || document
                    .blocks
                    .get(index + 1)
                    .is_some_and(|next| next.kind == BlockKind::Code);
            let words = word_count(&text);
            if !lead_in && words > 1 && sentences(&text).len() == 1 && words <= max_words {
                let line = &document.lines[block.lines[0]];
                single += 1;
                first_single.get_or_insert(line);
                run += 1;
                if run == min_run {
                    hits.push(hit(line, "run of one-sentence paragraphs"));
                }
                continue;
            }
        }
        run = 0;
    }
    let document_level = spec.min_units.is_some_and(|minimum| total >= minimum)
        && ratio_ok(spec, ratio(single, total));
    if hits.is_empty()
        && document_level
        && let Some(line) = first_single
    {
        hits.push(hit(line, "one-sentence paragraphs dominate"));
    }
    hits
}

fn duplicate_shingles(context: &Context<'_>, rule: &Rule) -> Vec<Hit> {
    let spec = &rule.spec;
    let Some(token) = &rule.regex else {
        return Vec::new();
    };
    let document = context.document;
    let size = spec.shingle_size.unwrap_or(6).max(1);
    let max_stopwords = spec.max_stopwords.unwrap_or(size);
    let stopwords: HashSet<&str> = spec.stopwords.iter().map(String::as_str).collect();

    let mut units: Vec<(usize, usize, String)> = Vec::new();
    let mut section = 0;
    for block in &document.blocks {
        match block.kind {
            BlockKind::Heading => {
                if document.lines[block.lines[0]].level <= 2 {
                    section += 1;
                }
            }
            BlockKind::Paragraph | BlockKind::Quote => {
                let text = block
                    .lines
                    .iter()
                    .map(|&index| document.lines[index].plain.trim())
                    .collect::<Vec<_>>()
                    .join(" ");
                units.push((section, block.lines[0], text));
            }
            BlockKind::List => {
                let mut skipping = false;
                for &index in &block.lines {
                    let line = &document.lines[index];
                    match line.kind {
                        LineKind::ListItem => {
                            skipping = link_only(line);
                            if !skipping {
                                units.push((section, index, line.plain.clone()));
                            }
                        }
                        LineKind::Paragraph if !skipping => {
                            if let Some((_, _, text)) = units.last_mut() {
                                text.push(' ');
                                text.push_str(line.plain.trim());
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    // `plain` marks each inline code span with ` CODE `; a window never spans one, or the words
    // on either side of stripped code would read as a repeated phrase.
    let tokenized: Vec<(usize, usize, Vec<String>)> = units
        .into_iter()
        .map(|(section, index, text)| {
            let mut tokens = Vec::new();
            for (position, segment) in text.split(" CODE ").enumerate() {
                if position > 0 {
                    tokens.push(CODE_BREAK.to_owned());
                }
                let lower = segment.to_lowercase();
                tokens.extend(token.find_iter(&lower).map(|m| m.as_str().to_owned()));
            }
            (section, index, tokens)
        })
        .collect();
    let counted = |window: &[String]| {
        !window.iter().any(|word| word == CODE_BREAK)
            && window
                .iter()
                .filter(|word| stopwords.contains(word.as_str()))
                .count()
                <= max_stopwords
    };
    let mut owners: HashMap<String, (BTreeSet<usize>, usize)> = HashMap::new();
    for (unit, (section, _, tokens)) in tokenized.iter().enumerate() {
        for window in tokens.windows(size).filter(|window| counted(window)) {
            owners
                .entry(window.join(" "))
                .or_insert_with(|| (BTreeSet::new(), unit))
                .0
                .insert(*section);
        }
    }

    // Overlapping repeated windows in their first unit merge into one span, so a repeated
    // clause scores once however long it is.
    let mut hits = Vec::new();
    for (unit, (_, index, tokens)) in tokenized.iter().enumerate() {
        let mut span: Option<(usize, usize)> = None;
        for (offset, window) in tokens.windows(size).enumerate() {
            let repeated = counted(window)
                && owners
                    .get(&window.join(" "))
                    .is_some_and(|(sections, owner)| sections.len() >= 2 && *owner == unit);
            if !repeated {
                continue;
            }
            span = match span {
                Some((start, last)) if offset < last + size => Some((start, offset)),
                Some((start, last)) => {
                    hits.push(hit(
                        &document.lines[*index],
                        &tokens[start..last + size].join(" "),
                    ));
                    Some((offset, offset))
                }
                None => Some((offset, offset)),
            };
        }
        if let Some((start, last)) = span {
            hits.push(hit(
                &document.lines[*index],
                &tokens[start..last + size].join(" "),
            ));
        }
    }
    hits
}

/// A list item holding only one link or one code span, as in a docs index or a table of contents.
fn link_only(line: &Line) -> bool {
    LINK_ONLY_ITEM.is_match(&line.raw)
}

fn wall_of_code(context: &Context<'_>, spec: &RuleSpec) -> Vec<Hit> {
    let document = context.document;
    let lines = context.lines();
    let lookback = spec.lookback_lines.unwrap_or(3);
    let mut hits = Vec::new();
    for block in &document.code_blocks {
        if block.body.lines().count() < spec.min_block_lines.unwrap_or(30) {
            continue;
        }
        let opener = block.line - 1;
        let introduced = lines[..opener]
            .iter()
            .rev()
            .filter(|line| line.kind != LineKind::Blank)
            .take(lookback)
            .any(|line| line.kind == LineKind::Paragraph);
        if !introduced {
            hits.push(hit(&lines[opener], &lines[opener].raw));
        }
    }
    let code = lines
        .iter()
        .filter(|line| matches!(line.kind, LineKind::Code | LineKind::IndentedCode))
        .count();
    let prose = lines
        .iter()
        .filter(|line| {
            matches!(
                line.kind,
                LineKind::Paragraph | LineKind::ListItem | LineKind::Quote
            )
        })
        .count();
    let long = spec
        .min_total_lines
        .is_none_or(|minimum| lines.len() >= minimum);
    if long && code > 0 && ratio_ok(spec, ratio(code, code + prose)) {
        hits.push(Hit {
            line: 1,
            excerpt: format!("{code} code lines, {prose} prose lines"),
        });
    }
    hits
}

fn table(context: &Context<'_>, rule: &Rule) -> Vec<Hit> {
    let spec = &rule.spec;
    let Some(pattern) = &rule.regex else {
        return Vec::new();
    };
    let document = context.document;
    let mut hits = Vec::new();
    for block in document
        .blocks
        .iter()
        .filter(|block| block.kind == BlockKind::Table)
    {
        let header = &document.lines[block.lines[0]];
        let body: Vec<&Line> = block.lines[1..]
            .iter()
            .map(|&index| &document.lines[index])
            .filter(|line| !TABLE_DELIMITER.is_match(&line.raw))
            .collect();
        let matches = body
            .iter()
            .filter(|line| pattern.is_match(&line.nocode))
            .count();
        let fires = spec
            .min_body_rows
            .is_none_or(|minimum| body.len() >= minimum)
            && matches >= spec.min_matches.unwrap_or(1)
            && spec
                .max_columns
                .is_none_or(|maximum| table_width(&header.nocode) <= maximum)
            && ratio_ok(spec, ratio(matches, body.len()));
        if fires {
            hits.push(hit(header, &header.raw));
        }
    }
    hits
}
