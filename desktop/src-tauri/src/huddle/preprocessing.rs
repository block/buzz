//! Text preprocessing for TTS output.
//!
//! Mental model:
//!
//! ```text
//! raw agent text
//!   → strip fenced code blocks  → "code block omitted"
//!   → strip inline code         → bare text
//!   → strip URLs                → "link omitted"
//!   → strip markdown markers    → plain text
//!   → strip emoji               → (removed)
//!   → numbers → words           → "forty two"
//!   → collapse whitespace       → clean string
//! ```
//!
//! Also provides `split_sentences` — the single sentence-boundary splitter used
//! by both the TTS batching pipeline and the Supertonic text chunker.

use regex::Regex;
use std::sync::LazyLock;

// ── Sentence splitting ────────────────────────────────────────────────────────

/// Regex: a sentence-ending punctuation mark followed by whitespace.
static RE_SENTENCE_BOUNDARY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"([.!?])\s+").unwrap());

/// Common abbreviations that end with a period but are NOT sentence boundaries.
const ABBREVIATIONS: &[&str] = &[
    "Dr.", "Mr.", "Mrs.", "Ms.", "Prof.", "Sr.", "Jr.", "St.", "Ave.", "Rd.", "Blvd.", "Dept.",
    "Inc.", "Ltd.", "Co.", "Corp.", "etc.", "vs.", "i.e.", "e.g.", "Ph.D.",
];

/// Split text into sentence-sized chunks.
///
/// Combines regex-based boundary detection with:
/// - Abbreviation awareness (`Dr.`, `Mr.`, etc. don't split)
/// - Digit-before-period check (avoids splitting `1.` `2.` numbered lists)
/// - `\n` and `—` treated as sentence breaks
///
/// Returns non-empty, trimmed strings.
pub fn split_sentences(text: &str) -> Vec<String> {
    // First, split on newlines and em-dashes to get coarse segments.
    let coarse: Vec<&str> = text.split(['\n', '—']).collect();

    let mut sentences = Vec::new();

    for segment in coarse {
        let segment = segment.trim();
        if segment.is_empty() {
            continue;
        }
        // Within each segment, split on sentence-ending punctuation.
        let matches: Vec<_> = RE_SENTENCE_BOUNDARY.find_iter(segment).collect();
        if matches.is_empty() {
            sentences.push(segment.to_string());
            continue;
        }

        let mut last_end = 0usize;
        for m in &matches {
            let before = &segment[last_end..m.start()];
            let punc_char = &segment[m.start()..m.start() + 1];

            // Skip if this looks like an abbreviation.
            let combined = format!("{}{}", before.trim(), punc_char);
            let is_abbrev = ABBREVIATIONS.iter().any(|a| combined.ends_with(a));

            // Skip if the character before the period is a digit (numbered list).
            let is_digit_period = punc_char == "."
                && !before.is_empty()
                && before.ends_with(|c: char| c.is_ascii_digit());

            if !is_abbrev && !is_digit_period {
                let piece = segment[last_end..m.end()].trim();
                if !piece.is_empty() {
                    sentences.push(piece.to_string());
                }
                last_end = m.end();
            }
        }

        if last_end < segment.len() {
            let tail = segment[last_end..].trim();
            if !tail.is_empty() {
                sentences.push(tail.to_string());
            }
        }
    }

    if sentences.is_empty() {
        vec![text.to_string()]
    } else {
        sentences
    }
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Prepare `text` for TTS synthesis.
///
/// Applies in order:
/// 1. Fenced code blocks → "code block omitted"
/// 2. Inline code → bare content (backticks stripped)
/// 3. URLs → "link omitted"
/// 4. Markdown bold/italic/underline markers stripped
/// 5. Emoji stripped
/// 6. Numbers → words (integers 0–999, times HH:MM)
/// 7. Excess whitespace collapsed
pub fn preprocess_for_tts(text: &str) -> String {
    let s = strip_fenced_code_blocks(text);
    let s = strip_inline_code(&s);
    let s = strip_urls(&s);
    let s = strip_markdown_markers(&s);
    let s = strip_emoji(&s);
    let s = expand_numbers(&s);
    let s = collapse_whitespace(&s);
    // Filter trivially short results — ".", ",", etc. would be spoken as
    // "period", "comma" by TTS. Agents that have nothing relevant to say
    // should not respond at all, but defense-in-depth catches edge cases.
    if s.len() <= 1 {
        return String::new();
    }
    s
}

// ── Step implementations ──────────────────────────────────────────────────────

/// Replace fenced code blocks with "code block omitted".
///
/// Handles both ` ``` ` and `~~~` fences. Multi-line aware.
fn strip_fenced_code_blocks(text: &str) -> String {
    let s = replace_fenced(text, "```");
    replace_fenced(&s, "~~~")
}

fn replace_fenced(text: &str, fence: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        match rest.find(fence) {
            None => {
                out.push_str(rest);
                break;
            }
            Some(start) => {
                // Everything before the opening fence.
                out.push_str(&rest[..start]);
                rest = &rest[start + fence.len()..];
                // Skip optional language tag on the same line.
                if let Some(nl) = rest.find('\n') {
                    rest = &rest[nl + 1..];
                }
                // Find the closing fence.
                match rest.find(fence) {
                    None => {
                        // Unclosed fence — treat rest as omitted.
                        out.push_str(" code block omitted ");
                        break;
                    }
                    Some(end) => {
                        out.push_str(" code block omitted ");
                        rest = &rest[end + fence.len()..];
                        // Skip trailing newline after closing fence.
                        if rest.starts_with('\n') {
                            rest = &rest[1..];
                        }
                    }
                }
            }
        }
    }
    out
}

/// Strip backtick-delimited inline code, leaving the inner text.
///
/// Single-backtick only — triple backtick already handled above.
fn strip_inline_code(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        match rest.find('`') {
            None => {
                out.push_str(rest);
                break;
            }
            Some(start) => {
                out.push_str(&rest[..start]);
                rest = &rest[start + 1..];
                match rest.find('`') {
                    None => {
                        // Unclosed — emit as-is.
                        out.push_str(rest);
                        break;
                    }
                    Some(end) => {
                        out.push_str(&rest[..end]);
                        rest = &rest[end + 1..];
                    }
                }
            }
        }
    }
    out
}

/// Replace http/https URLs with "link omitted".
///
/// Trailing sentence-ending punctuation (`.`, `!`, `?`) that immediately follows
/// a URL and is at end-of-string or followed by whitespace is preserved so that
/// sentence splitting and TTS prosody are not degraded.
///
/// Example: `"See https://x.y/z."` → `"See link omitted."`
fn strip_urls(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        // Find the earliest URL prefix.
        let http = rest.find("http://");
        let https = rest.find("https://");
        let url_start = match (http, https) {
            (None, None) => {
                out.push_str(rest);
                break;
            }
            (Some(a), None) => a,
            (None, Some(b)) => b,
            (Some(a), Some(b)) => a.min(b),
        };
        out.push_str(&rest[..url_start]);
        rest = &rest[url_start..];
        // Consume until whitespace or structural delimiter.
        let url_end = rest
            .find(|c: char| c.is_whitespace() || c == ')' || c == ']' || c == '"' || c == '\'')
            .unwrap_or(rest.len());
        let url_token = &rest[..url_end];
        rest = &rest[url_end..];

        // Check if the URL token ends with sentence-ending punctuation that
        // belongs to the surrounding sentence rather than the URL itself.
        // A trailing `.`, `!`, or `?` is preserved when it is at end-of-string
        // or followed by whitespace (i.e. it is a sentence boundary).
        let trailing_punct = if url_token.ends_with(['.', '!', '?']) {
            let after = rest; // rest is already past url_end
            if after.is_empty() || after.starts_with(|c: char| c.is_whitespace()) {
                // Preserve the trailing punctuation.
                &url_token[url_token.len() - 1..]
            } else {
                ""
            }
        } else {
            ""
        };

        out.push_str("link omitted");
        out.push_str(trailing_punct);
    }
    out
}

/// Strip `**`, `*`, `__`, `_emphasis_`, `~~` markdown markers.
///
/// Underscores are only stripped when they wrap a word (`_text_`).
/// Standalone underscores (e.g. `snake_case` identifiers) are preserved.
fn strip_markdown_markers(text: &str) -> String {
    // Order matters: strip multi-char markers before single-char.
    let s = text.replace("**", "");
    let s = s.replace("__", "");
    let s = s.replace("~~", "");
    let s = s.replace('*', "");
    strip_underscore_emphasis(&s)
}

/// Strip `_text_` emphasis markers while preserving underscores in identifiers.
///
/// A `_` is treated as an emphasis delimiter only when it is preceded by
/// whitespace or the start of the string AND followed by a non-whitespace char,
/// or vice-versa for the closing delimiter.
fn strip_underscore_emphasis(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut i = 0;
    while i < len {
        if chars[i] == '_' {
            // Opening delimiter: preceded by whitespace/start, followed by non-whitespace.
            let prev_is_boundary = i == 0 || chars[i - 1].is_whitespace();
            let next_is_nonspace = i + 1 < len && !chars[i + 1].is_whitespace();
            if prev_is_boundary && next_is_nonspace {
                // Look for a matching closing `_`.
                if let Some(close) = (i + 1..len).find(|&j| {
                    chars[j] == '_'
                        && !chars[j - 1].is_whitespace()
                        && (j + 1 >= len
                            || chars[j + 1].is_whitespace()
                            || chars[j + 1].is_ascii_punctuation())
                }) {
                    // Emit the inner text without the delimiters.
                    for &ch in &chars[i + 1..close] {
                        out.push(ch);
                    }
                    i = close + 1;
                    continue;
                }
            }
            // Not an emphasis delimiter — emit as-is.
            out.push('_');
        } else {
            out.push(chars[i]);
        }
        i += 1;
    }
    out
}

/// Strip Unicode emoji (characters in common emoji ranges).
///
/// Covers the main Emoji block (U+1F300–U+1FAFF) and supplemental ranges.
/// ASCII emoticons like `:)` are left as-is.
fn strip_emoji(text: &str) -> String {
    text.chars().filter(|&c| !is_emoji(c)).collect()
}

#[inline]
fn is_emoji(c: char) -> bool {
    matches!(c,
        '\u{1F300}'..='\u{1FAFF}' // Misc symbols, emoticons, transport, etc.
        | '\u{2600}'..='\u{27BF}'  // Misc symbols, dingbats
        | '\u{FE00}'..='\u{FE0F}'  // Variation selectors
        | '\u{1F000}'..='\u{1F02F}'// Mahjong/domino tiles
        | '\u{1F0A0}'..='\u{1F0FF}'// Playing cards
        | '\u{200D}'               // Zero-width joiner (used in emoji sequences)
        | '\u{20E3}'               // Combining enclosing keycap
    )
}

/// Expand numbers to spoken words.
///
/// Handles:
/// - Times: `HH:MM` → "eleven thirty"
/// - Integers 0–999,999
/// - Leaves other numeric strings (e.g. "3.14", "1000000+") as-is.
fn expand_numbers(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.char_indices().peekable();

    while let Some((i, c)) = chars.next() {
        if c.is_ascii_digit() {
            // Collect the full token (digits, colon, dots).
            let start = i;
            let mut end = i + c.len_utf8();
            while let Some(&(j, nc)) = chars.peek() {
                if nc.is_ascii_digit() || nc == ':' || nc == '.' {
                    end = j + nc.len_utf8();
                    chars.next();
                } else {
                    break;
                }
            }
            let token = &text[start..end];
            out.push_str(&expand_numeric_token(token));
        } else {
            out.push(c);
        }
    }
    out
}

fn expand_numeric_token(token: &str) -> String {
    // Strip trailing punctuation that the token collector may have included
    // (e.g. "11:30." from "at 11:30.") before attempting to parse.
    let token = token.trim_end_matches(|c: char| !c.is_ascii_digit());

    // Time: HH:MM
    if let Some(colon) = token.find(':') {
        let h = &token[..colon];
        let m = &token[colon + 1..];
        if let (Ok(hh), Ok(mm)) = (h.parse::<u32>(), m.parse::<u32>()) {
            if hh < 24 && mm < 60 {
                let hour_word = int_to_words(hh);
                let min_word = if mm == 0 {
                    String::new()
                } else if mm < 10 {
                    // "9:05" → "nine oh five" (not "nine five")
                    format!(" oh {}", int_to_words(mm))
                } else {
                    format!(" {}", int_to_words(mm))
                };
                return format!("{}{}", hour_word, min_word);
            }
        }
        // Not a valid time — return as-is.
        return token.to_string();
    }

    // Plain integer 0–999,999.
    if token.chars().all(|c| c.is_ascii_digit()) {
        if let Ok(n) = token.parse::<u32>() {
            if n <= 999_999 {
                return int_to_words(n);
            }
        }
    }

    // Anything else (decimals, millions+) — leave as-is.
    token.to_string()
}

/// Convert an integer 0–999,999 to English words.
fn int_to_words(n: u32) -> String {
    const ONES: &[&str] = &[
        "zero",
        "one",
        "two",
        "three",
        "four",
        "five",
        "six",
        "seven",
        "eight",
        "nine",
        "ten",
        "eleven",
        "twelve",
        "thirteen",
        "fourteen",
        "fifteen",
        "sixteen",
        "seventeen",
        "eighteen",
        "nineteen",
    ];
    const TENS: &[&str] = &[
        "", "", "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety",
    ];

    if n < 20 {
        return ONES[n as usize].to_string();
    }
    if n < 100 {
        let ten = TENS[(n / 10) as usize];
        let one = n % 10;
        return if one == 0 {
            ten.to_string()
        } else {
            format!("{} {}", ten, ONES[one as usize])
        };
    }
    if n < 1000 {
        let hundreds = n / 100;
        let remainder = n % 100;
        let hundred_word = format!("{} hundred", ONES[hundreds as usize]);
        return if remainder == 0 {
            hundred_word
        } else {
            format!("{} {}", hundred_word, int_to_words(remainder))
        };
    }
    // 1,000–999,999
    let thousands = n / 1000;
    let remainder = n % 1000;
    let thousand_word = format!("{} thousand", int_to_words(thousands));
    if remainder == 0 {
        thousand_word
    } else {
        format!("{} {}", thousand_word, int_to_words(remainder))
    }
}

/// Collapse runs of whitespace (spaces, tabs, newlines) to a single space.
/// Trims leading/trailing whitespace.
fn collapse_whitespace(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut prev_space = true; // Start true to trim leading whitespace.
    for c in text.chars() {
        if c.is_whitespace() {
            if !prev_space {
                out.push(' ');
                prev_space = true;
            }
        } else {
            out.push(c);
            prev_space = false;
        }
    }
    // Trim trailing space.
    if out.ends_with(' ') {
        out.pop();
    }
    out
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_fenced_code_block() {
        let input = "Here is some code:\n```rust\nfn main() {}\n```\nDone.";
        let out = preprocess_for_tts(input);
        assert!(out.contains("code block omitted"), "got: {out}");
        assert!(!out.contains("fn main"), "got: {out}");
    }

    #[test]
    fn strips_inline_code() {
        let out = preprocess_for_tts("Call `foo()` now.");
        assert_eq!(out, "Call foo() now.");
    }

    #[test]
    fn strips_urls() {
        let out = preprocess_for_tts("See https://example.com for details.");
        assert!(out.contains("link omitted"), "got: {out}");
        assert!(!out.contains("example.com"), "got: {out}");
    }

    #[test]
    fn strips_url_preserves_trailing_period() {
        // Trailing `.` at end of sentence must be preserved for sentence splitting.
        let out = strip_urls("See https://x.y/z.");
        assert_eq!(out, "See link omitted.", "got: {out}");
    }

    #[test]
    fn strips_url_preserves_trailing_exclamation() {
        let out = strip_urls("Visit https://example.com!");
        assert_eq!(out, "Visit link omitted!", "got: {out}");
    }

    #[test]
    fn strips_url_preserves_trailing_question() {
        let out = strip_urls("Did you see https://example.com?");
        assert_eq!(out, "Did you see link omitted?", "got: {out}");
    }

    #[test]
    fn strips_url_mid_sentence_no_punct_preserved() {
        // URL in the middle of a sentence — no trailing punct to preserve.
        let out = strip_urls("Check https://example.com for more info.");
        assert_eq!(out, "Check link omitted for more info.", "got: {out}");
    }

    #[test]
    fn strips_bold_italic() {
        let out = preprocess_for_tts("**bold** and *italic* and _under_");
        assert_eq!(out, "bold and italic and under");
    }

    #[test]
    fn preserves_standalone_underscores() {
        // snake_case identifiers should not be mangled.
        let out = preprocess_for_tts("call foo_bar() or baz_qux");
        assert!(out.contains("foo_bar"), "got: {out}");
        assert!(out.contains("baz_qux"), "got: {out}");
    }

    #[test]
    fn strips_tilde_fenced_block() {
        let input = "Here:\n~~~python\nprint('hi')\n~~~\nDone.";
        let out = preprocess_for_tts(input);
        assert!(out.contains("code block omitted"), "got: {out}");
        assert!(!out.contains("print"), "got: {out}");
    }

    #[test]
    fn expands_integers() {
        assert_eq!(preprocess_for_tts("42"), "forty two");
        assert_eq!(preprocess_for_tts("0"), "zero");
        assert_eq!(preprocess_for_tts("11"), "eleven");
        assert_eq!(preprocess_for_tts("100"), "one hundred");
    }

    #[test]
    fn expands_thousands() {
        assert_eq!(preprocess_for_tts("1000"), "one thousand");
        assert_eq!(
            preprocess_for_tts("1234"),
            "one thousand two hundred thirty four"
        );
        assert_eq!(preprocess_for_tts("10000"), "ten thousand");
        assert_eq!(preprocess_for_tts("100000"), "one hundred thousand");
        assert_eq!(
            preprocess_for_tts("999999"),
            "nine hundred ninety nine thousand nine hundred ninety nine"
        );
    }

    #[test]
    fn expands_times() {
        assert_eq!(preprocess_for_tts("11:30"), "eleven thirty");
        assert_eq!(preprocess_for_tts("9:00"), "nine");
        assert_eq!(preprocess_for_tts("9:05"), "nine oh five");
        assert_eq!(preprocess_for_tts("10:09"), "ten oh nine");
    }

    #[test]
    fn collapses_whitespace() {
        let out = preprocess_for_tts("  hello   world  ");
        assert_eq!(out, "hello world");
    }

    #[test]
    fn split_sentences_basic() {
        let result = split_sentences("Hello world. How are you? I'm fine!");
        assert_eq!(result, vec!["Hello world.", "How are you?", "I'm fine!"]);
    }

    #[test]
    fn split_sentences_newline_break() {
        let result = split_sentences("First line.\nSecond line.");
        assert_eq!(result, vec!["First line.", "Second line."]);
    }

    #[test]
    fn split_sentences_em_dash_break() {
        let result = split_sentences("Start here—then continue.");
        assert_eq!(result, vec!["Start here", "then continue."]);
    }

    #[test]
    fn split_sentences_abbreviations() {
        let result = split_sentences("Dr. Smith went home. He was tired.");
        assert_eq!(result, vec!["Dr. Smith went home.", "He was tired."]);
    }

    #[test]
    fn split_sentences_numbered_list() {
        let result = split_sentences("1. First item. 2. Second item.");
        // "1." and "2." should NOT cause a split (digit before period).
        assert_eq!(result, vec!["1. First item.", "2. Second item."]);
    }

    #[test]
    fn split_sentences_single() {
        let result = split_sentences("Just one sentence");
        assert_eq!(result, vec!["Just one sentence"]);
    }

    #[test]
    fn split_sentences_empty() {
        let result = split_sentences("");
        assert_eq!(result, vec![""]);
    }

    #[test]
    fn filters_trivial_responses() {
        assert_eq!(preprocess_for_tts("."), "");
        assert_eq!(preprocess_for_tts(","), "");
        assert_eq!(preprocess_for_tts("!"), "");
        assert_eq!(preprocess_for_tts(" "), "");
        assert_eq!(preprocess_for_tts("ok"), "ok");
    }

    #[test]
    fn full_pipeline() {
        let input =
            "**Agent says:** check https://relay.example.com at 11:30.\n```\nsome code\n```";
        let out = preprocess_for_tts(input);
        assert!(!out.contains("**"), "got: {out}");
        assert!(!out.contains("https://"), "got: {out}");
        assert!(out.contains("eleven thirty"), "got: {out}");
        assert!(out.contains("code block omitted"), "got: {out}");
    }
}
