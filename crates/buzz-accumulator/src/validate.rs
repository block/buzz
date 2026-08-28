//! Output-contract validation and the engine splice.
//!
//! Nonconforming output is refused and nothing persists. The rules here are
//! ones a model can genuinely meet — exact H1 sections, citations drawn only
//! from the ids it was shown — never rules it could only satisfy by lying.

use std::collections::BTreeSet;

use crate::error::Error;
use crate::schema::ArtifactSchema;

/// True for a line the heading grammar recognizes as an H1: `# ` followed by
/// at least one non-`#` character.
fn is_h1(line: &str) -> bool {
    match line.strip_prefix("# ") {
        Some(rest) => rest.chars().next().is_some_and(|c| c != '#'),
        None => false,
    }
}

/// Byte offsets of the section body for H1 `name`: (start just after the
/// heading's newline, end at the next H1 line or end of input). `None` when
/// the heading is absent (a heading with no trailing newline has no body).
fn section_span(output: &str, name: &str) -> Option<(usize, usize)> {
    let mut pos = 0;
    let mut found: Option<usize> = None;
    for line in output.split_inclusive('\n') {
        let bare = line.strip_suffix('\n');
        let text = bare.unwrap_or(line);
        match found {
            None => {
                // Heading line: `# {name}` plus optional trailing whitespace,
                // and it must be newline-terminated to have a body.
                let matches = text
                    .strip_prefix("# ")
                    .is_some_and(|rest| rest.trim_end() == name && rest.trim_end() == rest.trim());
                if matches && bare.is_some() {
                    found = Some(pos + line.len());
                }
            }
            Some(start) => {
                if is_h1(text) {
                    return Some((start, pos));
                }
            }
        }
        pos += line.len();
    }
    found.map(|start| (start, output.len()))
}

/// The trimmed body of H1 section `name`, or `""` when absent.
pub fn section(output: &str, name: &str) -> String {
    match section_span(output, name) {
        Some((start, end)) => output[start..end].trim().to_string(),
        None => String::new(),
    }
}

/// Replace the body of H1 section `name` with `content`, leaving the rest of
/// the document byte-identical. A missing heading leaves the document
/// unchanged.
pub fn replace_section(output: &str, name: &str, content: &str) -> String {
    match section_span(output, name) {
        Some((start, end)) => {
            format!("{}\n{}\n\n{}", &output[..start], content, &output[end..])
        }
        None => output.to_string(),
    }
}

/// All H1 headings in document order (text after `# `, verbatim to line end).
pub fn headings(output: &str) -> Vec<String> {
    output
        .lines()
        .filter(|line| is_h1(line))
        .map(|line| line[2..].to_string())
        .collect()
}

/// Extract cited event ids from `[event:…]` brackets, splitting brackets that
/// jam several full ids together.
///
/// `[event:<64 hex>]` yields that id; `[event:<id>, event:<id>]` yields both.
/// A bracket without a full 64-hex id inside (shortened id, prose) is kept
/// verbatim so it still fails validation loudly.
pub fn cited_event_ids(citation_text: &str) -> BTreeSet<String> {
    let mut cited = BTreeSet::new();
    let mut rest = citation_text;
    while let Some(open) = rest.find("[event:") {
        let inner_start = open + "[event:".len();
        let Some(close_rel) = rest[inner_start..].find(']') else {
            break;
        };
        let chunk = &rest[inner_start..inner_start + close_rel];
        rest = &rest[inner_start + close_rel + 1..];
        if chunk.is_empty() {
            continue;
        }
        let before = cited.len();
        // Consecutive non-overlapping 64-char windows over each maximal hex run.
        let mut run_start: Option<usize> = None;
        for (i, c) in chunk.char_indices().chain([(chunk.len(), ' ')]) {
            if matches!(c, '0'..='9' | 'a'..='f') {
                run_start.get_or_insert(i);
            } else if let Some(s) = run_start.take() {
                let mut at = s;
                while i - at >= 64 {
                    cited.insert(chunk[at..at + 64].to_string());
                    at += 64;
                }
            }
        }
        if cited.len() == before {
            cited.insert(chunk.trim().to_string());
        }
    }
    cited
}

/// Validate model output against the artifact contract.
///
/// - Freeform schemas (no sections) require only non-empty output.
/// - Sectioned schemas require exactly the schema's H1 sections, in order.
/// - When signals were shown (`source_event_ids` non-empty), the append
///   sections must cite ≥ 1 of them and may cite nothing else. Citations the
///   model merely re-emitted from the prior artifact's append sections are
///   stripped before checking, so only this run's citations are judged
///   against this run's shown ids.
///
/// Any failure is [`Error::Nonconforming`]: the caller persists nothing.
pub fn validate_output(
    schema: &ArtifactSchema,
    output: &str,
    previous_output: Option<&str>,
    source_event_ids: &[String],
) -> Result<(), Error> {
    if schema.sections.is_empty() {
        if output.trim().is_empty() {
            return Err(Error::Nonconforming(format!(
                "schema {:?} requires non-empty output",
                schema.name
            )));
        }
        return Ok(());
    }
    let found = headings(output);
    if found != schema.sections {
        return Err(Error::Nonconforming(format!(
            "output does not conform to {}: expected H1 sections {:?}, got {:?}",
            schema.name, schema.sections, found
        )));
    }
    if !source_event_ids.is_empty() {
        if schema.append_sections.is_empty() {
            return Err(Error::Nonconforming(format!(
                "schema {:?} has no append section for event citations",
                schema.name
            )));
        }
        let joined = |doc: &str| {
            schema
                .append_sections
                .iter()
                .map(|name| section(doc, name))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let citation_text = joined(output);
        let prior_citation_text = joined(previous_output.unwrap_or(""));
        let new_citation_text = if !prior_citation_text.is_empty() {
            citation_text.replacen(&prior_citation_text, "", 1)
        } else {
            citation_text
        };
        let cited = cited_event_ids(&new_citation_text);
        let allowed: BTreeSet<&str> = source_event_ids.iter().map(String::as_str).collect();
        let fabricated: Vec<&str> = cited
            .iter()
            .map(String::as_str)
            .filter(|id| !allowed.contains(id))
            .collect();
        if !fabricated.is_empty() {
            return Err(Error::Nonconforming(format!(
                "append sections cite unknown source event ids: {fabricated:?}"
            )));
        }
        if cited.is_empty() {
            return Err(Error::Nonconforming(
                "append sections are missing a source event citation".into(),
            ));
        }
    }
    Ok(())
}

/// Rebuild append sections so prior content is retained by construction.
///
/// The model contributes only this run's NEW entries; the engine prepends the
/// prior section verbatim. A model that repeats the prior section anyway is
/// deduplicated (exact-substring strip). History cannot be rewritten or
/// dropped no matter what the model outputs — and the model never has to
/// re-emit an ever-growing log, which kept run latency flat along the chain.
pub fn splice_append_sections(
    schema: &ArtifactSchema,
    previous_output: Option<&str>,
    output: &str,
) -> String {
    let Some(previous) = previous_output.filter(|p| !p.is_empty()) else {
        return output.to_string();
    };
    let mut doc = output.to_string();
    for name in schema.append_sections {
        let prior = section(previous, name);
        if prior.is_empty() {
            continue;
        }
        let model_part = section(&doc, name);
        let new_part = if model_part.contains(&prior) {
            model_part.replacen(&prior, "", 1)
        } else {
            model_part
        };
        let new_part = new_part.trim();
        let combined = if new_part.is_empty() {
            prior
        } else {
            format!("{prior}\n{new_part}")
        };
        doc = replace_section(&doc, name, &combined);
    }
    doc
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{CHANNEL_DIGEST_V1, FREEFORM_V1};

    fn id(c: char) -> String {
        std::iter::repeat_n(c, 64).collect()
    }

    fn digest(log_body: &str) -> String {
        format!("# Working Context\n\nSummary.\n\n# Log\n\n{log_body}\n")
    }

    #[test]
    fn section_and_replace_roundtrip() {
        let doc = digest("- entry one");
        assert_eq!(section(&doc, "Log"), "- entry one");
        assert_eq!(section(&doc, "Working Context"), "Summary.");
        assert_eq!(section(&doc, "Missing"), "");
        let replaced = replace_section(&doc, "Log", "- swapped");
        assert_eq!(section(&replaced, "Log"), "- swapped");
        assert_eq!(section(&replaced, "Working Context"), "Summary.");
    }

    #[test]
    fn headings_ignore_h2_and_capture_in_order() {
        let doc = "# One\n\n## nested\n\n# Two\nbody\n";
        assert_eq!(headings(doc), vec!["One", "Two"]);
    }

    #[test]
    fn heading_mismatch_is_refused() {
        let out = "# Wrong\n\nx\n\n# Log\n\n- e\n";
        let err = validate_output(&CHANNEL_DIGEST_V1, out, None, &[id('a')]);
        assert!(matches!(err, Err(Error::Nonconforming(_))));
    }

    #[test]
    fn freeform_requires_only_non_empty() {
        assert!(validate_output(&FREEFORM_V1, "  \n", None, &[]).is_err());
        assert!(validate_output(&FREEFORM_V1, "anything", None, &[]).is_ok());
    }

    #[test]
    fn fabricated_citation_is_refused() {
        let out = digest(&format!("- made up [event:{}]", id('c')));
        let err = validate_output(&CHANNEL_DIGEST_V1, &out, None, &[id('a')]);
        assert!(matches!(err, Err(Error::Nonconforming(_))));
    }

    #[test]
    fn missing_citation_is_refused() {
        let out = digest("- no citation here");
        let err = validate_output(&CHANNEL_DIGEST_V1, &out, None, &[id('a')]);
        assert!(matches!(err, Err(Error::Nonconforming(_))));
    }

    #[test]
    fn valid_citation_passes() {
        let out = digest(&format!("- did a thing [event:{}]", id('a')));
        assert!(validate_output(&CHANNEL_DIGEST_V1, &out, None, &[id('a')]).is_ok());
    }

    #[test]
    fn multi_id_bracket_splits_and_short_id_fails_loudly() {
        let jam = format!("[event:{}, event:{}]", id('a'), id('b'));
        let cited = cited_event_ids(&jam);
        assert_eq!(cited.len(), 2);
        assert!(cited.contains(&id('a')) && cited.contains(&id('b')));
        let short = cited_event_ids("[event:abc123]");
        assert_eq!(short.into_iter().collect::<Vec<_>>(), vec!["abc123"]);
    }

    #[test]
    fn prior_citations_do_not_satisfy_a_new_run() {
        let prior = digest(&format!("- old entry [event:{}]", id('a')));
        // Model re-emits the prior Log verbatim and adds nothing new: refused.
        let repeat_only = digest(&format!("- old entry [event:{}]", id('a')));
        let err = validate_output(&CHANNEL_DIGEST_V1, &repeat_only, Some(&prior), &[id('b')]);
        assert!(matches!(err, Err(Error::Nonconforming(_))));
        // Repeat plus a genuinely new cited entry: the repeat is stripped, the
        // new citation is judged against this run's shown ids.
        let repeat_plus_new = digest(&format!(
            "- old entry [event:{}]\n- new entry [event:{}]",
            id('a'),
            id('b')
        ));
        assert!(validate_output(
            &CHANNEL_DIGEST_V1,
            &repeat_plus_new,
            Some(&prior),
            &[id('b')]
        )
        .is_ok());
    }

    #[test]
    fn splice_retains_prior_history_when_model_omits_it() {
        let prior = digest(&format!("- old entry [event:{}]", id('a')));
        let fresh = digest(&format!("- new entry [event:{}]", id('b')));
        let doc = splice_append_sections(&CHANNEL_DIGEST_V1, Some(&prior), &fresh);
        let log = section(&doc, "Log");
        assert_eq!(
            log,
            format!(
                "- old entry [event:{}]\n- new entry [event:{}]",
                id('a'),
                id('b')
            )
        );
    }

    #[test]
    fn splice_dedupes_a_repeated_prior_section() {
        let prior = digest(&format!("- old entry [event:{}]", id('a')));
        let repeat_plus_new = digest(&format!(
            "- old entry [event:{}]\n- new entry [event:{}]",
            id('a'),
            id('b')
        ));
        let doc = splice_append_sections(&CHANNEL_DIGEST_V1, Some(&prior), &repeat_plus_new);
        let log = section(&doc, "Log");
        assert_eq!(log.matches("- old entry").count(), 1);
        assert!(log.contains("- new entry"));
    }

    #[test]
    fn splice_with_no_new_entries_keeps_prior_exactly() {
        let prior = digest(&format!("- old entry [event:{}]", id('a')));
        let empty_log = digest("");
        let doc = splice_append_sections(&CHANNEL_DIGEST_V1, Some(&prior), &empty_log);
        assert_eq!(
            section(&doc, "Log"),
            format!("- old entry [event:{}]", id('a'))
        );
    }
}
