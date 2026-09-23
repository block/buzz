//! Deterministic display-label grammar for Databricks endpoint ids.
//!
//! The last tier of [`crate::model_capabilities::databricks_registry_label`],
//! reached only after an exact-record miss and a unique-alias miss. The result
//! is either a label built solely from the id's own tokens or `None`, in which
//! case callers show the raw id. It is presentation-only: capabilities, wire
//! routing, and the saved model id never depend on it.
//!
//! Mirrored by `desktop/src/features/agents/ui/databricksLabelGrammar.ts`; both
//! replay `scripts/databricks-label-fixtures.json`.

use crate::model_capabilities::is_databricks_model_service_fqn;

/// Endpoint-name wrappers that precede the model name in Databricks ids.
const WRAPPERS: [&str; 4] = ["databricks-", "goose-", "kgoose-", "builderbot-"];

enum Part {
    /// Numeric version (`5`, `5.5`) — the one part GPT/GLM join with a hyphen.
    Version(String),
    Word(String),
}

/// Parse a Databricks endpoint id into a display label, or `None` when any part
/// of the id falls outside the grammar.
pub(crate) fn generate_databricks_label(raw_model_id: &str) -> Option<String> {
    let id = raw_model_id.trim().to_ascii_lowercase();
    let is_fqn = is_databricks_model_service_fqn(&id);
    let service = if is_fqn {
        id.rsplit('.').next()?
    } else {
        id.as_str()
    };
    // Only a UC FQN or a wrapped endpoint name is attributable to Databricks; a
    // bare id such as `gpt-5` stays raw.
    let body = match WRAPPERS.iter().find_map(|w| service.strip_prefix(w)) {
        Some(body) => body,
        None if is_fqn => service,
        None => return None,
    };
    let body = body
        .strip_prefix("meta-")
        .filter(|rest| rest.starts_with("llama-"))
        .unwrap_or(body);

    let mut tokens = body.split('-');
    let (family, stem) = split_family(tokens.next()?)?;
    let mut rest: Vec<&str> = tokens.collect();
    if family == "claude" {
        // Numeric-first Claude ids (`claude-4-7-opus`) name the tier after the version.
        let digits = rest.iter().take_while(|t| is_version(t)).count();
        if digits > 0 && digits < rest.len() {
            rest[..=digits].rotate_right(1);
        }
    }

    let mut parts = Vec::new();
    let mut has_number = stem.is_some();
    let mut after_minor = false;
    let mut rest = rest.into_iter().peekable();
    while let Some(tok) = rest.next() {
        if is_date(tok) && rest.peek().is_none() {
            break;
        }
        let part = if is_version(tok) {
            let minor = rest.next_if(|t| is_version(t));
            after_minor = minor.is_some();
            Part::Version(minor.map_or_else(|| tok.to_string(), |m| format!("{tok}.{m}")))
        } else if is_letter_version(tok) {
            let minor = rest.next_if(|t| is_version(t));
            let major = capitalize(tok);
            Part::Word(minor.map_or_else(|| major.clone(), |m| format!("{major}.{m}")))
        } else if is_size(tok) {
            Part::Word(tok.to_ascii_uppercase())
        } else if !tok.is_empty() && tok.bytes().all(|b| b.is_ascii_lowercase()) {
            Part::Word(match tok {
                "oss" => "OSS".to_string(),
                "mini" | "nano" if family == "gpt" && after_minor => tok.to_string(),
                _ => capitalize(tok),
            })
        } else {
            return None;
        };
        has_number |= match &part {
            Part::Version(_) => true,
            Part::Word(word) => word.bytes().any(|b| b.is_ascii_digit()),
        };
        parts.push(part);
    }
    // A versionless id (`builderbot-pr-reviews`) is a named endpoint, not a model.
    if !has_number {
        return None;
    }

    let (brand, hyphenated) = match family {
        "gpt" => ("GPT".to_string(), true),
        "glm" => ("GLM".to_string(), true),
        "deepseek" => ("DeepSeek".to_string(), false),
        _ => (capitalize(family), false),
    };
    let mut label = brand + &stem.unwrap_or_default();
    for (index, part) in parts.iter().enumerate() {
        let (sep, text) = match part {
            Part::Version(v) if index == 0 && hyphenated => ('-', v),
            Part::Version(text) | Part::Word(text) => (' ', text),
        };
        label.push(sep);
        label.push_str(text);
    }
    Some(label)
}

/// Split the leading token into its family name and an optional in-name
/// version (`qwen3` → `3`, `qwen35` → `3.5`).
fn split_family(tok: &str) -> Option<(&str, Option<String>)> {
    let alpha = tok.bytes().take_while(u8::is_ascii_lowercase).count();
    let (family, digits) = tok.split_at(alpha);
    if family.is_empty() || digits.len() > 2 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let stem = match digits.as_bytes() {
        [] => None,
        [major, minor] => Some(format!("{}.{}", *major as char, *minor as char)),
        _ => Some(digits.to_string()),
    };
    Some((family, stem))
}

/// `0` or a 1–2-digit number without a leading zero.
fn is_version(tok: &str) -> bool {
    match tok.as_bytes() {
        [d] => d.is_ascii_digit(),
        [a, b] => (b'1'..=b'9').contains(a) && b.is_ascii_digit(),
        _ => false,
    }
}

/// A 4- or 8-digit date stamp (`0731`, `20260101`).
fn is_date(tok: &str) -> bool {
    matches!(tok.len(), 4 | 8) && tok.bytes().all(|b| b.is_ascii_digit())
}

/// A letter-prefixed version such as `v4` or `k2`.
fn is_letter_version(tok: &str) -> bool {
    matches!(tok.as_bytes(), [l, digits @ ..] if l.is_ascii_lowercase()
        && !digits.is_empty()
        && digits.iter().all(u8::is_ascii_digit))
}

/// A parameter-size token: `120b`, or letter-prefixed `a3b`.
fn is_size(tok: &str) -> bool {
    let Some(body) = tok.strip_suffix('b') else {
        return false;
    };
    let digits = body
        .strip_prefix(|c: char| c.is_ascii_lowercase())
        .unwrap_or(body);
    !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
}

fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_ascii_uppercase().to_string() + chars.as_str()
    })
}

#[cfg(test)]
mod tests {
    use super::generate_databricks_label;
    use crate::model_capabilities::databricks_registry_label;
    use serde_json::Value;

    const FIXTURES_JSON: &str = include_str!("../../../scripts/databricks-label-fixtures.json");
    const MANIFEST_JSON: &str = include_str!("../../../scripts/model-capabilities.json");

    fn json(source: &str) -> Value {
        serde_json::from_str(source).expect("fixture JSON must parse")
    }

    /// `(raw_model_id, registry_label)` for every `databricks_v2` exact record.
    fn exact_records() -> Vec<(String, String)> {
        json(MANIFEST_JSON)["exact_records"]
            .as_array()
            .expect("exact_records")
            .iter()
            .filter(|rec| rec["provider"] == "databricks_v2")
            .map(|rec| {
                let field = |key: &str| rec[key].as_str().expect(key).to_string();
                (field("raw_model_id"), field("registry_label"))
            })
            .collect()
    }

    #[test]
    fn shared_fixtures_replay_through_the_label_helper() {
        let fixtures = json(FIXTURES_JSON);
        let records = exact_records();
        for row in fixtures["lookup"].as_array().expect("lookup") {
            let id = row["id"].as_str().expect("id");
            let label = row["label"].as_str();
            assert_eq!(databricks_registry_label(id).as_deref(), label, "id={id:?}");
            let has_exact = records.iter().any(|(raw, _)| raw.eq_ignore_ascii_case(id));
            match row["tier"].as_str().expect("tier") {
                "exact" => assert!(has_exact, "exact fixture lacks a record: {id}"),
                "generated" => {
                    assert!(!has_exact, "generated fixture is masked by a record: {id}");
                    assert_eq!(generate_databricks_label(id).as_deref(), label, "id={id}");
                }
                "alias" | "raw" => assert!(!has_exact, "fixture has a record: {id}"),
                tier => panic!("unknown tier {tier:?} for {id}"),
            }
        }
    }

    #[test]
    fn grammar_reproduces_every_curated_label_except_curator_only_ones() {
        let fixtures = json(FIXTURES_JSON);
        let curator_only: Vec<&str> = fixtures["curator_only_labels"]
            .as_array()
            .expect("curator_only_labels")
            .iter()
            .map(|id| id.as_str().expect("id"))
            .collect();
        let misses: Vec<String> = exact_records()
            .into_iter()
            .filter(|(raw, label)| generate_databricks_label(raw).as_deref() != Some(label))
            .map(|(raw, _)| raw)
            .collect();
        assert_eq!(misses, curator_only);
    }
}
