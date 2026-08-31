//! Scheme-independent embedded HTTP authorization redaction.

use regex::Regex;
use std::sync::OnceLock;

fn header_prefix_regex() -> Result<&'static Regex, String> {
    static REGEX: OnceLock<Result<Regex, String>> = OnceLock::new();
    match REGEX.get_or_init(|| {
        Regex::new(r#"(?i)(?:proxy-)?authorization\s*:\s*"#).map_err(|error| {
            format!("embedded authorization header prefix regex is invalid: {error}")
        })
    }) {
        Ok(regex) => Ok(regex),
        Err(error) => Err(error.clone()),
    }
}

fn is_outer_quote_boundary(text: &str, quote_index: usize) -> bool {
    let trailing = &text[quote_index + 1..];
    let Some(next) = trailing.chars().next() else {
        return true;
    };
    if next.is_ascii_whitespace() || matches!(next, ';' | '|' | '&' | ')' | ']' | '}') {
        return true;
    }
    if next != ',' {
        return false;
    }
    trailing[1..]
        .trim_start_matches(char::is_whitespace)
        .chars()
        .next()
        .is_some_and(|ch| matches!(ch, '\'' | '"'))
}

/// Redact through the header's outer closing quote, ignoring quoted Digest
/// parameters inside a single-quoted shell argument. Without an outer quote,
/// fail closed by consuming the rest of the line.
pub(super) fn redact_embedded_authorization_headers(
    text: &str,
    redaction_marker: &str,
) -> Result<(String, usize), String> {
    let regex = header_prefix_regex()?;
    let mut output = String::with_capacity(text.len());
    let mut redactions = 0;
    let mut copied_until = 0;
    let mut search_from = 0;
    while let Some(prefix) = regex.find_at(text, search_from) {
        let preceding = text[..prefix.start()].chars().next_back();
        if preceding.is_some_and(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-')) {
            search_from = prefix.end();
            continue;
        }
        let outer_quote = preceding.filter(|ch| matches!(ch, '\'' | '"'));
        let value_end = match outer_quote {
            Some(quote) => text[prefix.end()..]
                .char_indices()
                .find_map(|(offset, ch)| {
                    if ch != quote {
                        return None;
                    }
                    let index = prefix.end() + offset;
                    let backslashes = text.as_bytes()[..index]
                        .iter()
                        .rev()
                        .take_while(|byte| **byte == b'\\')
                        .count();
                    (backslashes % 2 == 0 && is_outer_quote_boundary(text, index)).then_some(index)
                })
                .unwrap_or(text.len()),
            None => text[prefix.end()..]
                .find(['\r', '\n'])
                .map_or(text.len(), |offset| prefix.end() + offset),
        };
        let value = &text[prefix.end()..value_end];
        output.push_str(&text[copied_until..prefix.end()]);
        if value.trim() == redaction_marker {
            output.push_str(value);
        } else {
            output.push_str(redaction_marker);
            redactions += 1;
        }
        copied_until = value_end;
        search_from = value_end;
        if value_end == text.len() {
            break;
        }
    }
    if redactions == 0 {
        Ok((text.to_owned(), 0))
    } else {
        output.push_str(&text[copied_until..]);
        Ok((output, redactions))
    }
}
