//! Line-oriented Markdown scanner. Enough structure for gates and slop rules; no rendering.

mod render;

use regex::Regex;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::{LazyLock, OnceLock};

static LIST_ITEM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\s*)([-*+]|\d{1,9}[.)])\s+(.*)$").expect("static regex"));
static IMAGE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"!\[[^\]]*\]\([^)]*\)").expect("static regex"));
static LINK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[([^\]]*)\]\([^)]*\)").expect("static regex"));
static URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<?https?://[^\s)>\]]+>?").expect("static regex"));
static HTML_TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"</?[A-Za-z][^>]*>").expect("static regex"));
static INLINE_COMMENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<!--.*?-->").expect("static regex"));

/// Classification of one source line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineKind {
    Blank,
    Heading,
    ListItem,
    TableRow,
    Quote,
    Html,
    /// Inside or opening an HTML comment.
    Comment,
    Rule,
    Paragraph,
    Fence,
    /// Inside a fenced code block.
    Code,
    /// Indented (4+ columns) code block outside any list.
    IndentedCode,
}

/// One source line with progressively cleaned views.
#[derive(Debug, Clone)]
pub struct Line {
    /// 1-based line number.
    pub number: usize,
    pub kind: LineKind,
    pub raw: String,
    /// Raw line with inline code spans blanked; link targets and block markers intact.
    pub nocode: String,
    /// `nocode` without inline HTML comments or URLs; markup and block markers intact.
    pub clean: String,
    /// Content without block markers, inline code, images, URLs, link targets, HTML tags, or emphasis.
    pub plain: String,
    /// `plain` with inline code removed rather than replaced by a placeholder word.
    pub prose: String,
    /// Heading level (1-6) or list nesting depth (0 = top level).
    pub level: usize,
    /// Ends in a hard line break; only the rendered view sets it.
    pub hard_break: bool,
}

impl Line {
    fn bare(number: usize, kind: LineKind, raw: &str) -> Self {
        Self {
            number,
            kind,
            raw: raw.to_owned(),
            nocode: String::new(),
            clean: String::new(),
            plain: String::new(),
            prose: String::new(),
            level: 0,
            hard_break: false,
        }
    }
}

/// Fenced code block.
#[derive(Debug, Clone)]
pub struct CodeBlock {
    /// 1-based line number of the opening fence.
    pub line: usize,
    /// Lowercased first word of the info string; empty when absent.
    pub lang: String,
    pub body: String,
    pub closed: bool,
}

/// ATX heading with its GitHub anchor slug.
#[derive(Debug, Clone)]
pub struct Heading {
    pub line: usize,
    pub level: usize,
    pub text: String,
    pub slug: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockKind {
    Heading,
    Paragraph,
    List,
    Code,
    Table,
    Quote,
    Html,
    Rule,
}

/// Consecutive lines forming one Markdown block. `lines` holds indices into `Document::lines`.
#[derive(Debug, Clone)]
pub struct Block {
    pub kind: BlockKind,
    pub lines: Vec<usize>,
}

#[derive(Debug)]
pub struct Document {
    pub lines: Vec<Line>,
    pub blocks: Vec<Block>,
    pub code_blocks: Vec<CodeBlock>,
    pub headings: Vec<Heading>,
    is_rendered: bool,
    rendered: OnceLock<Box<Document>>,
}

/// Parses Markdown source into lines, blocks, code blocks, and headings.
pub fn parse(source: &str) -> Document {
    let sources: Vec<String> = source.lines().map(str::to_owned).collect();
    build(&sources, &[], false)
}

/// `breaks[i]` marks a hard line break at the end of line `i`; `rendered` selects the
/// classification used by the rendered view.
fn build(sources: &[String], breaks: &[bool], rendered: bool) -> Document {
    let mut lines = Vec::new();
    let mut code_blocks = Vec::new();
    let mut open: Option<(char, usize, CodeBlock)> = None;
    let mut in_comment = false;
    let mut list_open = false;

    for (index, raw) in sources.iter().map(String::as_str).enumerate() {
        let number = index + 1;
        if let Some((fence_char, fence_len, mut block)) = open.take() {
            if closes_fence(raw, fence_char, fence_len) {
                block.closed = true;
                code_blocks.push(block);
                lines.push(Line::bare(number, LineKind::Fence, raw));
            } else {
                block.body.push_str(raw);
                block.body.push('\n');
                lines.push(Line::bare(number, LineKind::Code, raw));
                open = Some((fence_char, fence_len, block));
            }
            continue;
        }
        if in_comment {
            if raw.contains("-->") {
                in_comment = false;
            }
            lines.push(Line::bare(number, LineKind::Comment, raw));
            continue;
        }
        if let Some((fence_char, fence_len, lang)) = opens_fence(raw) {
            if indent_width(raw) < 2 {
                list_open = false;
            }
            let block = CodeBlock {
                line: number,
                lang,
                body: String::new(),
                closed: false,
            };
            open = Some((fence_char, fence_len, block));
            lines.push(Line::bare(number, LineKind::Fence, raw));
            continue;
        }
        let trimmed = raw.trim_start();
        if trimmed.starts_with("<!--") {
            in_comment = !trimmed.contains("-->");
            lines.push(Line::bare(number, LineKind::Comment, raw));
            continue;
        }
        let mut line = classify(number, raw, rendered);
        line.hard_break = breaks.get(index).copied().unwrap_or(false);
        let previous = lines.last().map(|previous: &Line| previous.kind);
        let after_break =
            previous.is_none_or(|kind| matches!(kind, LineKind::Blank | LineKind::IndentedCode));
        if line.kind == LineKind::Paragraph && indent_width(raw) >= 4 && after_break && !list_open {
            line = Line::bare(number, LineKind::IndentedCode, raw);
        }
        match line.kind {
            LineKind::ListItem => list_open = true,
            LineKind::Blank | LineKind::IndentedCode => {}
            LineKind::Paragraph if indent_width(raw) >= 2 || previous != Some(LineKind::Blank) => {}
            _ => list_open = false,
        }
        lines.push(line);
    }
    if let Some((_, _, block)) = open {
        code_blocks.push(block);
    }

    let headings = collect_headings(&lines);
    let blocks = group_blocks(&lines);
    Document {
        lines,
        blocks,
        code_blocks,
        headings,
        is_rendered: rendered,
        rendered: OnceLock::new(),
    }
}

/// Paragraph lines `start..=text_end` underlined at `text_end + 1` as a setext heading.
struct SetextRun {
    start: usize,
    text_end: usize,
    level: usize,
}

impl Document {
    /// The document as a reader sees it on GitHub, for slop rules.
    ///
    /// Inline HTML becomes the Markdown it imitates, setext and `<hN>` headings become ATX,
    /// entities are decoded, and invisible or look-alike code points are folded. Fenced and
    /// indented code lines keep their source bytes. Gates must keep reading `self`.
    pub fn rendered(&self) -> &Document {
        if self.is_rendered {
            return self;
        }
        self.rendered.get_or_init(|| Box::new(render::render(self)))
    }

    fn setext_runs(&self) -> Vec<SetextRun> {
        let mut runs = Vec::new();
        for (index, line) in self.lines.iter().enumerate() {
            if line.kind != LineKind::Paragraph || indent_width(&line.raw) > 3 {
                continue;
            }
            let Some(next) = self.lines.get(index + 1) else {
                continue;
            };
            let underline = next.raw.trim();
            let level = if indent_width(&next.raw) > 3 || underline.is_empty() {
                continue;
            } else if next.kind == LineKind::Paragraph && underline.chars().all(|c| c == '=') {
                1
            } else if next.kind == LineKind::Rule && underline.chars().all(|c| c == '-') {
                2
            } else {
                continue;
            };
            let mut start = index;
            while start > 0 && self.lines[start - 1].kind == LineKind::Paragraph {
                start -= 1;
            }
            if start > 0
                && matches!(
                    self.lines[start - 1].kind,
                    LineKind::ListItem | LineKind::Quote | LineKind::TableRow
                )
            {
                continue;
            }
            runs.push(SetextRun {
                start,
                text_end: index,
                level,
            });
        }
        runs
    }

    /// Words in paragraph, list, and quote prose, excluding code.
    pub fn prose_words(&self) -> usize {
        self.lines
            .iter()
            .filter(|line| {
                matches!(
                    line.kind,
                    LineKind::Paragraph | LineKind::ListItem | LineKind::Quote
                )
            })
            .map(|line| words(&line.plain))
            .sum()
    }

    /// Line-index range `[start, end)` of the body below heading `heading_index`.
    pub fn section(&self, heading_index: usize) -> (usize, usize) {
        let heading = &self.headings[heading_index];
        let start = heading.line;
        let end = self.headings[heading_index + 1..]
            .iter()
            .find(|next| next.level <= heading.level)
            .map_or(self.lines.len(), |next| next.line - 1);
        (start, end)
    }

    /// ATX and setext headings in document order, slugs deduplicated across both kinds.
    ///
    /// `headings` stays ATX-only; `rendered()` rewrites setext headings as ATX instead.
    pub fn rendered_headings(&self) -> Vec<Heading> {
        let setext: HashMap<usize, SetextRun> = self
            .setext_runs()
            .into_iter()
            .map(|run| (run.text_end, run))
            .collect();
        let mut found: Vec<(usize, usize, String, String)> = Vec::new();
        for (index, line) in self.lines.iter().enumerate() {
            if line.kind == LineKind::Heading {
                found.push((
                    line.number,
                    line.level,
                    line.plain.trim().to_owned(),
                    line.raw.clone(),
                ));
                continue;
            }
            let Some(&SetextRun { start, level, .. }) = setext.get(&index) else {
                continue;
            };
            let run = &self.lines[start..=index];
            let join = |view: fn(&Line) -> &str| {
                run.iter()
                    .map(|member| view(member).trim())
                    .collect::<Vec<_>>()
                    .join(" ")
            };
            found.push((
                self.lines[start].number,
                level,
                join(|member| &member.plain),
                join(|member| &member.raw),
            ));
        }
        let mut seen: HashMap<String, usize> = HashMap::new();
        found
            .into_iter()
            .map(|(line, level, text, source)| {
                let base = github_slug(&source);
                let count = seen.entry(base.clone()).or_insert(0);
                let slug = if *count == 0 {
                    base
                } else {
                    format!("{base}-{count}")
                };
                *count += 1;
                Heading {
                    line,
                    level,
                    text,
                    slug,
                }
            })
            .collect()
    }

    /// Prose units for sentence rules: one per paragraph or quote block, one per list item.
    pub fn prose_units(&self) -> Vec<(usize, String)> {
        let mut units = Vec::new();
        for block in &self.blocks {
            match block.kind {
                BlockKind::Paragraph | BlockKind::Quote => {
                    let text = block
                        .lines
                        .iter()
                        .map(|&index| self.lines[index].plain.trim())
                        .collect::<Vec<_>>()
                        .join(" ");
                    units.push((self.lines[block.lines[0]].number, text));
                }
                BlockKind::List => {
                    for &index in &block.lines {
                        let line = &self.lines[index];
                        match line.kind {
                            LineKind::ListItem => units.push((line.number, line.plain.clone())),
                            LineKind::Paragraph => {
                                if let Some((_, text)) = units.last_mut() {
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
        units
    }
}

/// Counts whitespace-separated tokens containing at least one alphanumeric character.
pub fn words(text: &str) -> usize {
    text.split_whitespace()
        .filter(|word| word.chars().any(char::is_alphanumeric))
        .count()
}

/// Splits prose on terminal punctuation followed by whitespace or end of text.
pub fn sentences(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut current = String::new();
    for (index, &character) in chars.iter().enumerate() {
        current.push(character);
        let terminal = matches!(character, '.' | '!' | '?')
            && chars.get(index + 1).is_none_or(|next| next.is_whitespace());
        if terminal {
            let sentence = current.trim();
            if !sentence.is_empty() {
                out.push(sentence.to_owned());
            }
            current.clear();
        }
    }
    let rest = current.trim();
    if !rest.is_empty() {
        out.push(rest.to_owned());
    }
    out
}

/// Replaces each inline code span with `replacement`.
pub fn blank_code_spans(text: &str, replacement: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '`' {
            let run = backtick_run(&chars, index);
            if let Some(end) = closing_run(&chars, index + run, run) {
                out.push_str(replacement);
                index = end + run;
            } else {
                out.extend(std::iter::repeat_n('`', run));
                index += run;
            }
            continue;
        }
        out.push(chars[index]);
        index += 1;
    }
    out
}

/// Splits text into `(is_code, content)` pieces; code content excludes the backtick delimiters.
pub fn code_span_pieces(text: &str) -> Vec<(bool, String)> {
    let chars: Vec<char> = text.chars().collect();
    let mut pieces = Vec::new();
    let mut plain = String::new();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '`' {
            let run = backtick_run(&chars, index);
            if let Some(end) = closing_run(&chars, index + run, run) {
                if !plain.is_empty() {
                    pieces.push((false, std::mem::take(&mut plain)));
                }
                pieces.push((true, chars[index + run..end].iter().collect()));
                index = end + run;
            } else {
                plain.extend(std::iter::repeat_n('`', run));
                index += run;
            }
            continue;
        }
        plain.push(chars[index]);
        index += 1;
    }
    if !plain.is_empty() {
        pieces.push((false, plain));
    }
    pieces
}

/// Inline code span contents, trimmed.
pub fn code_spans(text: &str) -> Vec<String> {
    code_span_pieces(text)
        .into_iter()
        .filter(|(is_code, _)| *is_code)
        .map(|(_, content)| content.trim().to_owned())
        .collect()
}

fn backtick_run(chars: &[char], start: usize) -> usize {
    chars[start..].iter().take_while(|&&c| c == '`').count()
}

fn closing_run(chars: &[char], start: usize, run: usize) -> Option<usize> {
    let mut index = start;
    while index < chars.len() {
        if chars[index] == '`' {
            let length = backtick_run(chars, index);
            if length == run {
                return Some(index);
            }
            index += length;
        } else {
            index += 1;
        }
    }
    None
}

/// Cells in a table row, counting unescaped pipes between optional outer pipes.
pub fn table_width(row: &str) -> usize {
    let trimmed = row.trim();
    let inner = trimmed.strip_prefix('|').unwrap_or(trimmed);
    let inner = inner.strip_suffix('|').unwrap_or(inner);
    let mut cells = 1;
    let mut escaped = false;
    for character in inner.chars() {
        if character == '|' && !escaped {
            cells += 1;
        }
        escaped = character == '\\';
    }
    cells
}

/// Leading whitespace width with a tab counted as 4 columns.
pub fn indent_width(raw: &str) -> usize {
    raw.chars()
        .take_while(|c| c.is_whitespace())
        .map(|c| if c == '\t' { 4 } else { 1 })
        .sum()
}

pub fn opens_fence(raw: &str) -> Option<(char, usize, String)> {
    if indent_width(raw) > 3 {
        return None;
    }
    let trimmed = raw.trim_start();
    let fence_char = trimmed.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let length = trimmed.chars().take_while(|c| *c == fence_char).count();
    if length < 3 {
        return None;
    }
    let info = &trimmed[length * fence_char.len_utf8()..];
    if fence_char == '`' && info.contains('`') {
        return None;
    }
    let lang = info
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_lowercase();
    Some((fence_char, length, lang))
}

pub fn closes_fence(raw: &str, fence_char: char, fence_len: usize) -> bool {
    if indent_width(raw) > 3 {
        return false;
    }
    let trimmed = raw.trim();
    let length = trimmed.chars().take_while(|c| *c == fence_char).count();
    length >= fence_len && trimmed.chars().all(|c| c == fence_char)
}

fn is_rule(trimmed: &str) -> bool {
    let marks: Vec<char> = trimmed.chars().filter(|c| !c.is_whitespace()).collect();
    marks.len() >= 3 && matches!(marks[0], '-' | '*' | '_') && marks.iter().all(|c| *c == marks[0])
}

/// In the rendered view a line opening with a tag is HTML only when no text is visible.
fn classify(number: usize, raw: &str, rendered: bool) -> Line {
    let trimmed = raw.trim_start();
    let nocode = blank_code_spans(raw, " ");
    if trimmed.is_empty() {
        return Line::bare(number, LineKind::Blank, raw);
    }

    let (kind, level, content) = if let Some(level) = heading_level(raw) {
        let body = trimmed[level..].trim();
        let body = body.trim_end_matches('#').trim_end();
        (LineKind::Heading, level, body.to_owned())
    } else if indent_width(raw) <= 3 && is_rule(trimmed) {
        (LineKind::Rule, 0, String::new())
    } else if let Some(captures) = LIST_ITEM.captures(raw) {
        let depth = indent_width(&captures[1]) / 2;
        (LineKind::ListItem, depth, captures[3].to_owned())
    } else if trimmed.starts_with('|') {
        let separator = trimmed.chars().all(|c| matches!(c, '|' | ':' | '-' | ' '));
        let content = if separator {
            String::new()
        } else {
            trimmed.replace('|', " ")
        };
        (LineKind::TableRow, 0, content)
    } else if trimmed.starts_with('>') {
        (
            LineKind::Quote,
            0,
            trimmed.trim_start_matches('>').trim_start().to_owned(),
        )
    } else if trimmed.starts_with('<')
        && trimmed
            .chars()
            .nth(1)
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '/' || c == '!')
        && (!rendered || render::visible_text(trimmed).trim().is_empty())
    {
        (LineKind::Html, 0, String::new())
    } else {
        (LineKind::Paragraph, 0, trimmed.to_owned())
    };

    let strip_emphasis = |text: String| text.replace("**", "").replace("__", "").replace('*', "");
    let plain = strip_emphasis(clean_inline(&content, " CODE "));
    let prose = strip_emphasis(clean_inline(&content, " "));
    let clean = URL
        .replace_all(&INLINE_COMMENT.replace_all(&nocode, " "), " ")
        .into_owned();
    Line {
        number,
        kind,
        raw: raw.to_owned(),
        nocode,
        clean,
        plain,
        prose,
        level,
        hard_break: false,
    }
}

fn heading_level(raw: &str) -> Option<usize> {
    if indent_width(raw) > 3 {
        return None;
    }
    let trimmed = raw.trim_start();
    let level = trimmed.chars().take_while(|c| *c == '#').count();
    let after = trimmed[level..].chars().next();
    ((1..=6).contains(&level) && after.is_none_or(char::is_whitespace)).then_some(level)
}

/// `code` replaces each inline code span; a word there lets sentence rules see a subject or object.
fn clean_inline(content: &str, code: &str) -> String {
    let text = blank_code_spans(content, code);
    let text = IMAGE.replace_all(&text, " ");
    let text = LINK.replace_all(&text, "$1");
    let text = URL.replace_all(&text, " ");
    HTML_TAG.replace_all(&text, " ").into_owned()
}

fn collect_headings(lines: &[Line]) -> Vec<Heading> {
    let mut seen: HashMap<String, usize> = HashMap::new();
    lines
        .iter()
        .filter(|line| line.kind == LineKind::Heading)
        .map(|line| {
            let base = github_slug(&line.raw);
            let count = seen.entry(base.clone()).or_insert(0);
            let slug = if *count == 0 {
                base
            } else {
                format!("{base}-{count}")
            };
            *count += 1;
            Heading {
                line: line.number,
                level: line.level,
                text: line.plain.trim().to_owned(),
                slug,
            }
        })
        .collect()
}

/// GitHub-compatible anchor for a raw ATX heading line or setext heading text.
pub fn github_slug(raw: &str) -> String {
    let trimmed = raw.trim_start();
    let content = trimmed.trim_start_matches('#').trim().trim_end_matches('#');
    let content = LINK.replace_all(content, "$1");
    let mut rendered = String::new();
    for (is_code, piece) in code_span_pieces(&content) {
        if is_code {
            rendered.push_str(&piece);
        } else {
            let text = HTML_TAG
                .replace_all(&piece, "")
                .replace('*', "")
                .replace("~~", "");
            rendered.push_str(&strip_underscore_delimiters(&text));
        }
    }
    rendered
        .trim()
        .to_lowercase()
        .chars()
        .filter_map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                Some(c)
            } else if c == ' ' {
                Some('-')
            } else {
                None
            }
        })
        .collect()
}

/// Drops `_` emphasis delimiters; an underscore between two alphanumerics is literal.
fn strip_underscore_delimiters(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    chars
        .iter()
        .enumerate()
        .filter(|&(index, &c)| {
            c != '_'
                || (index > 0
                    && chars[index - 1].is_alphanumeric()
                    && chars
                        .get(index + 1)
                        .is_some_and(|next| next.is_alphanumeric()))
        })
        .map(|(_, &c)| c)
        .collect()
}

fn group_blocks(lines: &[Line]) -> Vec<Block> {
    let mut next_non_blank = vec![None; lines.len() + 1];
    for index in (0..lines.len()).rev() {
        next_non_blank[index] = if lines[index].kind == LineKind::Blank {
            next_non_blank[index + 1]
        } else {
            Some(index)
        };
    }
    let mut blocks = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let line = &lines[index];
        match line.kind {
            LineKind::Blank | LineKind::Code => index += 1,
            LineKind::IndentedCode => {
                let start = index;
                index += 1;
                while index < lines.len()
                    && matches!(lines[index].kind, LineKind::IndentedCode | LineKind::Blank)
                {
                    index += 1;
                }
                while index > start + 1 && lines[index - 1].kind == LineKind::Blank {
                    index -= 1;
                }
                blocks.push(Block {
                    kind: BlockKind::Code,
                    lines: (start..index).collect(),
                });
            }
            LineKind::Heading | LineKind::Rule => {
                let kind = if line.kind == LineKind::Heading {
                    BlockKind::Heading
                } else {
                    BlockKind::Rule
                };
                blocks.push(Block {
                    kind,
                    lines: vec![index],
                });
                index += 1;
            }
            LineKind::Fence => {
                let start = index;
                index += 1;
                while index < lines.len() && lines[index].kind == LineKind::Code {
                    index += 1;
                }
                if index < lines.len() && lines[index].kind == LineKind::Fence {
                    index += 1;
                }
                blocks.push(Block {
                    kind: BlockKind::Code,
                    lines: (start..index).collect(),
                });
            }
            LineKind::ListItem => {
                let start = index;
                index += 1;
                loop {
                    let Some(next) = lines.get(index) else { break };
                    let continues = match next.kind {
                        LineKind::ListItem => true,
                        LineKind::Paragraph => {
                            lines[index - 1].kind != LineKind::Blank || indent_width(&next.raw) >= 2
                        }
                        LineKind::Blank => next_non_blank[index + 1]
                            .map(|later| &lines[later])
                            .is_some_and(|later| {
                                later.kind == LineKind::ListItem
                                    || (later.kind == LineKind::Paragraph
                                        && indent_width(&later.raw) >= 2)
                            }),
                        _ => false,
                    };
                    if !continues {
                        break;
                    }
                    index += 1;
                }
                let members = (start..index)
                    .filter(|&member| lines[member].kind != LineKind::Blank)
                    .collect();
                blocks.push(Block {
                    kind: BlockKind::List,
                    lines: members,
                });
            }
            LineKind::TableRow
            | LineKind::Quote
            | LineKind::Paragraph
            | LineKind::Html
            | LineKind::Comment => {
                let kind = line.kind;
                let start = index;
                index += 1;
                while index < lines.len()
                    && lines[index].kind == kind
                    && !(kind == LineKind::Paragraph && lines[index - 1].hard_break)
                {
                    index += 1;
                }
                let block_kind = match kind {
                    LineKind::TableRow => BlockKind::Table,
                    LineKind::Quote => BlockKind::Quote,
                    LineKind::Html | LineKind::Comment => BlockKind::Html,
                    _ => BlockKind::Paragraph,
                };
                blocks.push(Block {
                    kind: block_kind,
                    lines: (start..index).collect(),
                });
            }
        }
    }
    blocks
}
