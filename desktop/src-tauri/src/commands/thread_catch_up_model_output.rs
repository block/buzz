//! Parsing the local model's recap answer (see `thread_catch_up.rs`).
//!
//! Only well-formed points that cite messages from the frozen model input are
//! kept; references are resolved against that input and nothing else.
use std::collections::{HashMap, HashSet};

use serde_json::Value;

use super::thread_catch_up::{
    CatchUpCategory, CatchUpPoint, MAX_POINTS, MAX_POINT_TEXT_BYTES, MAX_RECAP_TEXT_BYTES,
};

/// Generated points are one sentence; longer model text, or text over the
/// shared byte limit, is rejected.
const MAX_GENERATED_POINT_CHARS: usize = 600;
const MAX_GENERATED_POINT_SOURCES: usize = 8;
pub(super) const UNPARSEABLE: &str =
    "The local model's answer was not in the expected format. Try again or choose a different model.";

/// One message the model was shown: its prompt reference, its event id, and
/// whether it is in the unread window.
pub(super) struct Citable<'a> {
    pub(super) reference: &'a str,
    pub(super) id: &'a str,
    pub(super) is_new: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct ParsedPoints {
    pub(super) points: Vec<CatchUpPoint>,
    pub(super) warnings: Vec<String>,
}

/// Accept only well-formed points that cite messages from the frozen input,
/// at least one of them new. Unknown references are dropped, never resolved.
/// Points past the recap's text budget are left out, so the result can always
/// be shared.
pub(super) fn parse_model_points(
    reply: &str,
    input: &[Citable<'_>],
) -> Result<ParsedPoints, String> {
    let answer = match reply.rfind("</think>") {
        Some(end) => &reply[end + "</think>".len()..],
        None if reply.contains("<think>") => return Err(UNPARSEABLE.into()),
        None => reply,
    };
    let (Some(start), Some(end)) = (answer.find('{'), answer.rfind('}')) else {
        return Err(UNPARSEABLE.into());
    };
    if end < start {
        return Err(UNPARSEABLE.into());
    }
    let value: Value =
        serde_json::from_str(&answer[start..=end]).map_err(|_| UNPARSEABLE.to_string())?;
    let Some(raw_points) = value.get("points").and_then(Value::as_array) else {
        return Err(UNPARSEABLE.into());
    };

    let by_ref: HashMap<&str, &Citable<'_>> = input
        .iter()
        .map(|message| (message.reference, message))
        .collect();
    let mut points = Vec::new();
    let mut seen_text = HashSet::new();
    let (mut malformed, mut uncited, mut unknown_refs, mut over_cap, mut over_budget) =
        (0usize, 0usize, 0usize, 0usize, 0usize);
    let mut text_bytes = 0usize;

    for raw in raw_points {
        let category = match raw
            .get("category")
            .and_then(Value::as_str)
            .map(|c| c.trim().to_ascii_lowercase())
            .as_deref()
        {
            Some("decision" | "decisions") => CatchUpCategory::Decision,
            Some("question" | "questions" | "open question") => CatchUpCategory::Question,
            _ => {
                malformed += 1;
                continue;
            }
        };
        let text = raw
            .get("text")
            .and_then(Value::as_str)
            .map(clean_point_text)
            .unwrap_or_default();
        if text.is_empty()
            || text.chars().count() > MAX_GENERATED_POINT_CHARS
            || text.len() > MAX_POINT_TEXT_BYTES
        {
            malformed += 1;
            continue;
        }
        let refs: Vec<&str> = match raw.get("sources").or_else(|| raw.get("sourceIds")) {
            Some(Value::Array(items)) => items.iter().filter_map(Value::as_str).collect(),
            Some(Value::String(single)) => vec![single.as_str()],
            _ => Vec::new(),
        };
        let mut source_ids: Vec<String> = Vec::new();
        let mut cites_new = false;
        for reference in refs {
            let key = reference
                .trim()
                .trim_matches(|c: char| c == '[' || c == ']')
                .to_ascii_lowercase();
            match by_ref.get(key.as_str()) {
                Some(message) => {
                    if !source_ids.iter().any(|id| id == message.id)
                        && source_ids.len() < MAX_GENERATED_POINT_SOURCES
                    {
                        source_ids.push(message.id.to_string());
                        cites_new |= message.is_new;
                    }
                }
                None => unknown_refs += 1,
            }
        }
        if !cites_new {
            uncited += 1;
            continue;
        }
        if !seen_text.insert(text.to_lowercase()) {
            continue;
        }
        if points.len() >= MAX_POINTS {
            over_cap += 1;
            continue;
        }
        if text_bytes + text.len() > MAX_RECAP_TEXT_BYTES {
            over_budget += 1;
            continue;
        }
        text_bytes += text.len();
        points.push(CatchUpPoint {
            id: format!("p{}", points.len() + 1),
            category,
            text,
            source_ids,
        });
    }

    let mut warnings = Vec::new();
    if uncited > 0 {
        warnings.push(format!(
            "Left out {uncited} suggested point(s) that did not cite a new message."
        ));
    }
    if unknown_refs > 0 {
        warnings.push(format!(
            "Ignored {unknown_refs} citation(s) to messages the model was not given."
        ));
    }
    if malformed > 0 {
        warnings.push(format!(
            "Left out {malformed} malformed suggested point(s)."
        ));
    }
    if over_cap > 0 {
        warnings.push(format!("Kept the first {MAX_POINTS} points."));
    }
    if over_budget > 0 {
        warnings.push(format!(
            "Left out {over_budget} point(s) to keep the recap within its size limit."
        ));
    }
    Ok(ParsedPoints { points, warnings })
}

/// One line of plain text: whitespace runs collapse and controls are removed.
fn clean_point_text(raw: &str) -> String {
    raw.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .to_string()
}
