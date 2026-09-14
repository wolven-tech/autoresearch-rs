//! The rendered view: each source line rewritten into the Markdown a GitHub reader sees.
//!
//! Line count and numbering never change, so hits still point at source lines.

use super::{Document, HTML_TAG, INLINE_COMMENT, LineKind, backtick_run, build, closing_run, opens_fence};
use regex::{Captures, Regex};
use std::collections::HashMap;
use std::sync::LazyLock;

static EMOJI_IMG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)<img\b[^>]*?\bsrc\s*=\s*["']?[^"'\s>]*/emoji/(?:unicode/)?([0-9a-f]{4,6}(?:-[0-9a-f]{4,6})*)?[^"'\s>]*["']?[^>]*>"#,
    )
    .expect("static regex")
});
static HR_TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^<hr\b[^>]*>$").expect("static regex"));
static HEADING_LINE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^<h([1-6])\b[^>]*>(.*?)(?:</h[1-6]\s*>\s*)?$").expect("static regex")
});
static HEADING_CLOSE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^</h[1-6]\s*>$").expect("static regex"));
static LIST_ITEM_TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^<li\b[^>]*>").expect("static regex"));
static SUMMARY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)<summary\b[^>]*>(.*?)</summary\s*>").expect("static regex")
});
static EMPHASIS_TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)</?(?:b|strong|i|em)\b[^>]*>").expect("static regex"));
static BOLD_TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)</?(?:b|strong)\b[^>]*>").expect("static regex"));
static ITALIC_TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)</?(?:i|em)\b[^>]*>").expect("static regex"));
static CODE_TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)</?(?:code|tt)\b[^>]*>").expect("static regex"));
static BREAK_AT_END: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)<br\b[^>]*>\s*$").expect("static regex"));
static WRAPPER_TAG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)</?(?:p|div|span|details|summary|center|sup|sub|small|big|u|ins|del|s|strike|mark|kbd|samp|var|abbr|cite|q|font|section|article|header|footer|nav|main|aside|figure|figcaption|picture|source|table|thead|tbody|tfoot|tr|td|th|ul|ol|li|dl|dt|dd|blockquote|br|wbr|g-emoji|h[1-6])\b[^>]*>",
    )
    .expect("static regex")
});
static ENTITY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"&(?:#[0-9]{1,7}|#[xX][0-9a-fA-F]{1,6}|[A-Za-z][A-Za-z0-9]{1,31});")
        .expect("static regex")
});
static KEYCAP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[0-9#*]\x{FE0F}?\x{20E3}").expect("static regex"));
static TEXT_PICTOGRAPH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"([\p{Extended_Pictographic}--\p{Emoji_Presentation}--[\x{A9}\x{AE}\x{2122}\x{2190}-\x{21FF}\x{2934}\x{2935}\x{25AA}\x{25AB}\x{25B6}\x{25C0}\x{25FB}-\x{25FE}\x{3030}\x{303D}\x{2139}\x{24C2}\x{3297}\x{3299}\x{203C}\x{2049}]])([\x{FE0E}\x{FE0F}]?)",
    )
    .expect("static regex")
});
static REFERENCE_DEFINITION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"^\s{0,3}\[([^\]\n]+)\]:\s*<?([^\s>]+)>?(?:\s+(?:"[^"]*"|'[^']*'|\([^)]*\)))?\s*$"#)
        .expect("static regex")
});
static REFERENCE_USE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(!?)\[([^\]\n]*)\]\[([^\]\n]*)\]").expect("static regex"));
static BADGE_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"https?://(?:img\.shields\.io|(?:flat\.)?badgen\.net)/[^\s)"'>]+"#)
        .expect("static regex")
});

/// Emoji shortcodes GitHub renders, mapped to one representative code point each.
const SHORTCODES: &[(&str, &str)] = &[
    ("+1", "\u{1F44D}"), ("-1", "\u{1F44E}"), ("100", "\u{1F4AF}"), ("art", "\u{1F3A8}"),
    ("bar_chart", "\u{1F4CA}"), ("beetle", "\u{1FAB2}"), ("bell", "\u{1F514}"),
    ("bookmark", "\u{1F516}"), ("books", "\u{1F4DA}"), ("book", "\u{1F4D6}"),
    ("boom", "\u{1F4A5}"), ("brain", "\u{1F9E0}"), ("bug", "\u{1F41B}"), ("bulb", "\u{1F4A1}"),
    ("calendar", "\u{1F4C6}"), ("chart_with_upwards_trend", "\u{1F4C8}"),
    ("checkered_flag", "\u{1F3C1}"), ("clap", "\u{1F44F}"), ("clipboard", "\u{1F4CB}"),
    ("coffee", "\u{2615}"), ("compass", "\u{1F9ED}"), ("computer", "\u{1F4BB}"),
    ("confetti_ball", "\u{1F38A}"), ("construction", "\u{1F6A7}"), ("crab", "\u{1F980}"),
    ("crystal_ball", "\u{1F52E}"), ("dart", "\u{1F3AF}"), ("dizzy", "\u{1F4AB}"),
    ("electric_plug", "\u{1F50C}"), ("exclamation", "\u{2757}"), ("eyes", "\u{1F440}"),
    ("fast_forward", "\u{23E9}"), ("file_folder", "\u{1F4C1}"), ("fire", "\u{1F525}"),
    ("floppy_disk", "\u{1F4BE}"), ("gear", "\u{2699}\u{FE0F}"), ("gem", "\u{1F48E}"),
    ("gift", "\u{1F381}"), ("globe_with_meridians", "\u{1F310}"), ("hammer", "\u{1F528}"),
    ("handshake", "\u{1F91D}"), ("heart", "\u{2764}\u{FE0F}"),
    ("heavy_check_mark", "\u{2714}\u{FE0F}"), ("hourglass", "\u{231B}"),
    ("information_source", "\u{2139}\u{FE0F}"), ("jigsaw", "\u{1F9E9}"), ("key", "\u{1F511}"),
    ("label", "\u{1F3F7}\u{FE0F}"), ("large_blue_circle", "\u{1F535}"),
    ("light_bulb", "\u{1F4A1}"), ("link", "\u{1F517}"), ("lock", "\u{1F512}"),
    ("loudspeaker", "\u{1F4E2}"), ("mag", "\u{1F50D}"), ("magic_wand", "\u{1FA84}"),
    ("mega", "\u{1F4E3}"), ("memo", "\u{1F4DD}"), ("microscope", "\u{1F52C}"),
    ("muscle", "\u{1F4AA}"), ("new", "\u{1F195}"), ("no_entry", "\u{26D4}"),
    ("ok_hand", "\u{1F44C}"), ("one", "\u{0031}\u{FE0F}\u{20E3}"),
    ("open_file_folder", "\u{1F4C2}"), ("package", "\u{1F4E6}"), ("pencil", "\u{1F4DD}"),
    ("pencil2", "\u{270F}\u{FE0F}"), ("point_down", "\u{1F447}"), ("point_right", "\u{1F449}"),
    ("pushpin", "\u{1F4CC}"), ("question", "\u{2753}"), ("rainbow", "\u{1F308}"),
    ("raised_hands", "\u{1F64C}"), ("recycle", "\u{267B}\u{FE0F}"), ("red_circle", "\u{1F534}"),
    ("repeat", "\u{1F501}"), ("robot", "\u{1F916}"), ("rocket", "\u{1F680}"),
    ("rotating_light", "\u{1F6A8}"), ("round_pushpin", "\u{1F4CD}"),
    ("satellite", "\u{1F4E1}"), ("seedling", "\u{1F331}"), ("shield", "\u{1F6E1}\u{FE0F}"),
    ("smile", "\u{1F604}"), ("sparkle", "\u{2747}\u{FE0F}"), ("sparkles", "\u{2728}"),
    ("speech_balloon", "\u{1F4AC}"), ("star", "\u{2B50}"), ("star2", "\u{1F31F}"),
    ("stars", "\u{1F320}"), ("stopwatch", "\u{23F1}\u{FE0F}"), ("sunglasses", "\u{1F60E}"),
    ("tada", "\u{1F389}"), ("test_tube", "\u{1F9EA}"), ("thumbsup", "\u{1F44D}"),
    ("three", "\u{0033}\u{FE0F}\u{20E3}"), ("toolbox", "\u{1F9F0}"), ("trophy", "\u{1F3C6}"),
    ("two", "\u{0032}\u{FE0F}\u{20E3}"), ("unlock", "\u{1F513}"),
    ("warning", "\u{26A0}\u{FE0F}"), ("wave", "\u{1F44B}"),
    ("white_check_mark", "\u{2705}"), ("wrench", "\u{1F527}"), ("x", "\u{274C}"),
    ("zap", "\u{26A1}"),
];
static SHORTCODE_LOOKUP: LazyLock<HashMap<&str, &str>> =
    LazyLock::new(|| SHORTCODES.iter().copied().collect());
/// Stands in for an emoji whose code point is unknown; not a status emoji.
const GENERIC_EMOJI: &str = "\u{1F536}";

/// Text of a line with comments and tags removed; emoji images count as visible.
pub(super) fn visible_text(text: &str) -> String {
    let text = INLINE_COMMENT.replace_all(text, "");
    let text = EMOJI_IMG.replace_all(&text, GENERIC_EMOJI);
    HTML_TAG
        .replace_all(&text, "")
        .replace("&nbsp;", " ")
        .replace("&#160;", " ")
}

pub(super) fn render(document: &Document) -> Document {
    let count = document.lines.len();
    let mut sources: Vec<String> = Vec::with_capacity(count);
    let mut breaks = vec![false; count];
    let mut pending_heading: Option<usize> = None;
    for (index, line) in document.lines.iter().enumerate() {
        if !rewritable(line.kind, &line.raw) {
            sources.push(line.raw.clone());
            continue;
        }
        let (mut text, hard_break) = normalize_line(&line.raw);
        breaks[index] = hard_break;
        let trimmed = text.trim();
        let tag_only = visible_text(trimmed).trim().is_empty();
        if HEADING_CLOSE.is_match(trimmed) {
            pending_heading = None;
        } else if tag_only && let Some(captures) = HEADING_LINE.captures(trimmed) {
            pending_heading = captures[1].parse().ok();
        } else if !tag_only && let Some(level) = pending_heading.take() {
            text = format!("{} {}", "#".repeat(level), trimmed);
        }
        sources.push(text);
    }
    resolve_references(document, &mut sources);
    for (index, source) in sources.iter_mut().enumerate() {
        if rewritable(document.lines[index].kind, &document.lines[index].raw) {
            *source = outside_code(source, |piece| {
                BADGE_URL
                    .replace_all(piece, |captures: &Captures<'_>| percent_decode(&captures[0]))
                    .into_owned()
            });
        }
    }

    let first = build(&sources, &breaks, true);
    let front_matter = sources.first().is_some_and(|line| line.trim() == "---");
    let runs: Vec<_> = first
        .setext_runs()
        .into_iter()
        .filter(|run| !(front_matter && run.start == 1))
        .collect();
    if runs.is_empty() {
        return first;
    }
    for run in runs {
        let text = sources[run.start..=run.text_end]
            .iter()
            .map(|line| line.trim())
            .collect::<Vec<_>>()
            .join(" ");
        sources[run.start] = format!("{} {text}", "#".repeat(run.level));
        breaks[run.start] = false;
        for index in run.start + 1..=run.text_end + 1 {
            sources[index].clear();
            breaks[index] = false;
        }
    }
    build(&sources, &breaks, true)
}

/// Code lines keep their bytes; a comment line is rewritten only when it closes on itself.
fn rewritable(kind: LineKind, raw: &str) -> bool {
    match kind {
        LineKind::Code | LineKind::Fence | LineKind::IndentedCode => false,
        LineKind::Comment => {
            let trimmed = raw.trim_start();
            trimmed.starts_with("<!--") && trimmed.contains("-->")
        }
        _ => true,
    }
}

/// Returns the rewritten line and whether it ends in a hard line break.
fn normalize_line(raw: &str) -> (String, bool) {
    let indent_len = raw.len() - raw.trim_start_matches([' ', '\t']).len();
    let (indent, rest) = raw.split_at(indent_len);
    let text = outside_code(rest, |piece| {
        let piece = INLINE_COMMENT.replace_all(piece, "");
        EMOJI_IMG.replace_all(&piece, emoji_image).into_owned()
    });
    if visible_text(&text).trim().is_empty() {
        let text = if HR_TAG.is_match(text.trim()) {
            "---".to_owned()
        } else {
            text
        };
        return (format!("{indent}{text}"), false);
    }

    let mut hard_break = BREAK_AT_END.is_match(&text);
    let text = convert_block_tags(&text);
    let text = outside_code(&text, convert_inline_tags);
    let mut text = outside_code(&text, |piece| {
        let piece = expand_shortcodes(piece);
        let piece = ENTITY.replace_all(&piece, decode_entity);
        let piece = fold_characters(&piece);
        let piece = KEYCAP.replace_all(&piece, "\u{1F522}");
        let piece = TEXT_PICTOGRAPH.replace_all(&piece, |captures: &Captures<'_>| {
            if captures[2].is_empty() {
                format!("{}\u{FE0F}", &captures[1])
            } else {
                captures[0].to_owned()
            }
        });
        collapse_triple_emphasis(&piece)
    });
    if text.ends_with('\\') && !text.ends_with("\\\\") {
        text.pop();
        hard_break = true;
    }

    let mut rest = text.trim_start().to_owned();
    if opens_fence(&rest).is_some() && opens_fence(raw).is_none() {
        rest.insert(0, '\\');
    }
    if rest.contains("<!--") && !raw.contains("<!--") {
        rest = rest.replace("<!--", "< !--");
    }
    (format!("{indent}{rest}"), hard_break)
}

/// Whole-line forms: `<hN>` headings, `<li>` items, and `<summary>` labels without emphasis.
fn convert_block_tags(text: &str) -> String {
    let text = SUMMARY.replace_all(text, |captures: &Captures<'_>| {
        EMPHASIS_TAG.replace_all(&captures[1], "").into_owned()
    });
    let trimmed = text.trim();
    if let Some(captures) = HEADING_LINE.captures(trimmed) {
        let level: usize = captures[1].parse().unwrap_or(1);
        return format!("{} {}", "#".repeat(level), captures[2].trim());
    }
    LIST_ITEM_TAG.replace(trimmed, "- ").into_owned()
}

fn convert_inline_tags(piece: &str) -> String {
    let piece = BOLD_TAG.replace_all(piece, "**");
    let piece = ITALIC_TAG.replace_all(&piece, "*");
    let piece = CODE_TAG.replace_all(&piece, "`");
    WRAPPER_TAG.replace_all(&piece, " ").into_owned()
}

/// Applies `map` to the text between inline code spans; spans keep their delimiters and bytes.
fn outside_code(text: &str, mut map: impl FnMut(&str) -> String) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut plain = String::new();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '`' {
            let run = backtick_run(&chars, index);
            if let Some(end) = closing_run(&chars, index + run, run) {
                out.push_str(&map(&std::mem::take(&mut plain)));
                out.extend(&chars[index..end + run]);
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
    out.push_str(&map(&plain));
    out
}

fn emoji_image(captures: &Captures<'_>) -> String {
    captures
        .get(1)
        .and_then(|codes| {
            codes
                .as_str()
                .split('-')
                .map(|code| u32::from_str_radix(code, 16).ok().and_then(char::from_u32))
                .collect::<Option<String>>()
        })
        .unwrap_or_else(|| GENERIC_EMOJI.to_owned())
}

/// `:name:` for a known alias, or for any multi-word alias such as `:heavy_minus_sign:`.
fn expand_shortcodes(text: &str) -> String {
    if !text.contains(':') {
        return text.to_owned();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut last_end = usize::MAX;
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == ':'
            && let Some(close) = chars[index + 1..]
                .iter()
                .take(40)
                .position(|&c| c == ':')
                .map(|offset| index + 1 + offset)
        {
            let name: String = chars[index + 1..close].iter().collect();
            let before_ok = index == 0
                || last_end == index
                || !(chars[index - 1].is_alphanumeric() || chars[index - 1] == ':');
            let after_ok = chars
                .get(close + 1)
                .is_none_or(|&next| !next.is_alphanumeric());
            let valid = !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "_+-".contains(c));
            let multi_word = name.contains('_')
                && name.split('_').all(|part| !part.is_empty())
                && name.starts_with(|c: char| c.is_ascii_lowercase());
            if before_ok && after_ok && valid {
                if let Some(emoji) = SHORTCODE_LOOKUP.get(name.as_str()) {
                    out.push_str(emoji);
                } else if multi_word {
                    out.push_str(GENERIC_EMOJI);
                } else {
                    out.push(':');
                    index += 1;
                    continue;
                }
                index = close + 1;
                last_end = index;
                continue;
            }
        }
        out.push(chars[index]);
        index += 1;
    }
    out
}

/// Decodes an entity unless it would produce Markdown syntax, which GitHub renders literally.
fn decode_entity(captures: &Captures<'_>) -> String {
    let entity = &captures[0];
    let body = &entity[1..entity.len() - 1];
    let decoded: Option<String> = if let Some(number) = body.strip_prefix('#') {
        let value = match number.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok(),
            None => number.parse().ok(),
        };
        value.and_then(char::from_u32).map(String::from)
    } else {
        named_entity(body).map(str::to_owned)
    };
    match decoded {
        Some(text)
            if !text
                .chars()
                .any(|c| c.is_control() || "`#*_-+><|[]()=~\\".contains(c)) =>
        {
            text
        }
        _ => entity.to_owned(),
    }
}

fn named_entity(name: &str) -> Option<&'static str> {
    Some(match name {
        "amp" => "&",
        "nbsp" | "ensp" | "emsp" | "thinsp" => " ",
        "shy" => "\u{AD}",
        "zwj" => "\u{200D}",
        "zwnj" => "\u{200C}",
        "mdash" => "\u{2014}",
        "ndash" => "\u{2013}",
        "horbar" => "\u{2015}",
        "hellip" => "\u{2026}",
        "rarr" => "\u{2192}",
        "larr" => "\u{2190}",
        "harr" => "\u{2194}",
        "rArr" => "\u{21D2}",
        "lArr" => "\u{21D0}",
        "hArr" => "\u{21D4}",
        "uarr" => "\u{2191}",
        "darr" => "\u{2193}",
        "laquo" => "\u{AB}",
        "raquo" => "\u{BB}",
        "lsquo" => "\u{2018}",
        "rsquo" => "\u{2019}",
        "ldquo" => "\u{201C}",
        "rdquo" => "\u{201D}",
        "quot" => "\"",
        "apos" => "'",
        "excl" => "!",
        "quest" => "?",
        "comma" => ",",
        "period" => ".",
        "colon" => ":",
        "semi" => ";",
        "copy" => "\u{A9}",
        "reg" => "\u{AE}",
        "trade" => "\u{2122}",
        "bull" => "\u{2022}",
        "middot" => "\u{B7}",
        "times" => "\u{D7}",
        "minus" => "\u{2212}",
        "deg" => "\u{B0}",
        "check" => "\u{2713}",
        "cross" => "\u{2717}",
        "starf" => "\u{2605}",
        "star" => "\u{2606}",
        "hearts" => "\u{2665}",
        _ => return None,
    })
}

/// Folds invisible, look-alike, fullwidth, and mathematical alphanumeric code points.
/// A run of bold mathematical letters becomes `**…**`.
fn fold_characters(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut index = 0;
    while index < chars.len() {
        if math_alphanumeric(chars[index]).is_some_and(|(_, bold)| bold) {
            let mut end = index + 1;
            let mut cursor = index + 1;
            while let Some(&next) = chars.get(cursor) {
                if math_alphanumeric(next).is_some_and(|(_, bold)| bold) {
                    cursor += 1;
                    end = cursor;
                } else if matches!(next, ' ' | '-' | '\'' | '\u{2019}') {
                    cursor += 1;
                } else {
                    break;
                }
            }
            out.push_str("**");
            for position in index..end {
                fold_one(&chars, position, &mut out);
            }
            out.push_str("**");
            index = end;
            continue;
        }
        fold_one(&chars, index, &mut out);
        index += 1;
    }
    out
}

fn fold_one(chars: &[char], index: usize, out: &mut String) {
    let c = chars[index];
    let alphanumeric_neighbour = (index > 0 && chars[index - 1].is_alphanumeric())
        || chars.get(index + 1).is_some_and(|next| next.is_alphanumeric());
    match c {
        '\u{00AD}' | '\u{200B}' | '\u{200C}' | '\u{200E}' | '\u{200F}' | '\u{2060}'..='\u{2064}'
        | '\u{FEFF}' | '\u{180E}' | '\u{034F}' | '\u{061C}' => {}
        '\u{200D}' if alphanumeric_neighbour || index == 0 || index + 1 == chars.len() => {}
        '\u{00A0}' | '\u{2000}'..='\u{200A}' | '\u{202F}' | '\u{205F}' | '\u{3000}' => out.push(' '),
        '\u{2010}' | '\u{2011}' | '\u{FE63}' | '\u{FF0D}' => out.push('-'),
        '\u{2012}' => out.push('\u{2013}'),
        '\u{2015}' | '\u{2E3A}' | '\u{2E3B}' | '\u{FE58}' | '\u{FE31}' | '\u{FE32}' => {
            out.push('\u{2014}');
        }
        '\u{203C}' => out.push_str("!!"),
        '\u{2049}' => out.push_str("!?"),
        '\u{FE57}' | '\u{01C3}' => out.push('!'),
        '\u{FE56}' => out.push('?'),
        '\u{FF01}'..='\u{FF5E}' => {
            let ascii = char::from_u32(c as u32 - 0xFEE0).unwrap_or(c);
            if ascii.is_ascii_alphanumeric() || ".,:;!?'\"%&@$/".contains(ascii) {
                out.push(ascii);
            } else {
                out.push(c);
            }
        }
        _ => match math_alphanumeric(c) {
            Some((ascii, _)) => out.push(ascii),
            None => out.push(c),
        },
    }
}

/// ASCII letter or digit for a mathematical alphanumeric or letterlike symbol, and whether
/// its style is bold.
fn math_alphanumeric(c: char) -> Option<(char, bool)> {
    let code = c as u32;
    if (0x1D400..=0x1D6A3).contains(&code) {
        let offset = code - 0x1D400;
        let index = (offset % 52) as u8;
        let letter = if index < 26 { b'A' + index } else { b'a' + index - 26 };
        return Some((char::from(letter), matches!(offset / 52, 0 | 2 | 4 | 7 | 9 | 11)));
    }
    if (0x1D7CE..=0x1D7FF).contains(&code) {
        let offset = code - 0x1D7CE;
        let digit = b'0' + (offset % 10) as u8;
        return Some((char::from(digit), matches!(offset / 10, 0 | 3)));
    }
    let letter = match c {
        '\u{210E}' => 'h',
        '\u{212C}' => 'B',
        '\u{2130}' => 'E',
        '\u{2131}' => 'F',
        '\u{210B}' | '\u{210C}' | '\u{210D}' => 'H',
        '\u{2110}' | '\u{2111}' => 'I',
        '\u{2112}' => 'L',
        '\u{2133}' => 'M',
        '\u{211B}' | '\u{211C}' | '\u{211D}' => 'R',
        '\u{212F}' => 'e',
        '\u{210A}' => 'g',
        '\u{2134}' => 'o',
        '\u{212D}' | '\u{2102}' => 'C',
        '\u{2128}' | '\u{2124}' => 'Z',
        '\u{2115}' => 'N',
        '\u{2119}' => 'P',
        '\u{211A}' => 'Q',
        _ => return None,
    };
    Some((letter, false))
}

/// `***x***` and `___x___` as bold; a bare `***` line stays a thematic break.
fn collapse_triple_emphasis(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut index = 0;
    while index < chars.len() {
        let c = chars[index];
        if matches!(c, '*' | '_')
            && chars.get(index + 1) == Some(&c)
            && chars.get(index + 2) == Some(&c)
            && chars.get(index + 3) != Some(&c)
            && (index == 0 || chars[index - 1] != c)
        {
            let touches_text = (index > 0 && !chars[index - 1].is_whitespace())
                || chars.get(index + 3).is_some_and(|next| !next.is_whitespace());
            out.push(c);
            out.push(c);
            if !touches_text {
                out.push(c);
            }
            index += 3;
            continue;
        }
        out.push(c);
        index += 1;
    }
    out
}

/// Rewrites `![alt][ref]` and `[text][ref]` to inline form and blanks the definitions.
fn resolve_references(document: &Document, sources: &mut [String]) {
    let mut definitions: HashMap<String, String> = HashMap::new();
    let mut definition_lines = Vec::new();
    for (index, source) in sources.iter().enumerate() {
        if document.lines[index].kind != LineKind::Paragraph {
            continue;
        }
        if let Some(captures) = REFERENCE_DEFINITION.captures(source) {
            definitions
                .entry(reference_key(&captures[1]))
                .or_insert_with(|| captures[2].to_owned());
            definition_lines.push(index);
        }
    }
    if definitions.is_empty() {
        return;
    }
    for index in definition_lines {
        sources[index].clear();
    }
    for (index, source) in sources.iter_mut().enumerate() {
        if !rewritable(document.lines[index].kind, &document.lines[index].raw) {
            continue;
        }
        *source = outside_code(source, |piece| {
            REFERENCE_USE
                .replace_all(piece, |captures: &Captures<'_>| {
                    let label = if captures[3].trim().is_empty() {
                        &captures[2]
                    } else {
                        &captures[3]
                    };
                    match definitions.get(&reference_key(label)) {
                        Some(url) => format!("{}[{}]({url})", &captures[1], &captures[2]),
                        None => captures[0].to_owned(),
                    }
                })
                .into_owned()
        });
    }
}

fn reference_key(label: &str) -> String {
    label.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// Shields reads `_` and `%20` alike as a space; decoding to `_` keeps the URL one token.
fn percent_decode(url: &str) -> String {
    let bytes = url.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && let Some(value) = url
                .get(index + 1..index + 3)
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
        {
            out.push(if value == b' ' { b'_' } else { value });
            index += 3;
            continue;
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| url.to_owned())
}
