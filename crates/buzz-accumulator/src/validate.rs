//! Output-contract validation and the engine splice.
//!
//! Only the section STRUCTURE is validated — it is what the splice and every
//! reader depend on. Provenance is engine-owned and deterministic (the run's
//! `shown_ids` and coverage window are computed from the plan, never from
//! model output), so `[event:…]` citations in the text are best-effort links
//! for the reader, not a validated contract: a sloppy citation renders as a
//! dead chip instead of refusing a paid run.

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

/// Validate model output against the artifact contract: the output must have
/// exactly the schema's H1 sections, in order. That is the whole contract —
/// the splice and the reader depend on the structure; nothing else about the
/// text is judged. Failure is [`Error::Nonconforming`]: the caller persists
/// nothing.
pub fn validate_output(schema: &ArtifactSchema, output: &str) -> Result<(), Error> {
    let found = headings(output);
    if found != schema.sections {
        return Err(Error::Nonconforming(format!(
            "output does not conform to {}: expected H1 sections {:?}, got {:?}",
            schema.name, schema.sections, found
        )));
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
    use crate::schema::CHANNEL_DIGEST_V1;

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
        let err = validate_output(&CHANNEL_DIGEST_V1, out);
        assert!(matches!(err, Err(Error::Nonconforming(_))));
    }

    #[test]
    fn conforming_sections_pass_regardless_of_citations() {
        // Provenance is engine-owned (shown_ids + coverage window on the
        // artifact); citations in the text are best-effort links, never a
        // refusal. Unshown, missing, and malformed citations all pass —
        // an unresolvable chip in the reader beats losing a paid run.
        assert!(validate_output(&CHANNEL_DIGEST_V1, &digest("")).is_ok());
        assert!(validate_output(&CHANNEL_DIGEST_V1, &digest("- no citation here")).is_ok());
        let unshown = digest(&format!("- quoted from message text [event:{}]", id('c')));
        assert!(validate_output(&CHANNEL_DIGEST_V1, &unshown).is_ok());
        let malformed = digest("- sloppy [event:abc123]");
        assert!(validate_output(&CHANNEL_DIGEST_V1, &malformed).is_ok());
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
