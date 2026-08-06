//! Pure helpers for bridging ACP `elicitation/create` (form mode) to Buzz
//! question-card events and back.
//!
//! Claude Code's built-in `AskUserQuestion` tool surfaces over ACP as an
//! `elicitation/create` request with `mode: "form"` (see
//! `RESEARCH/ACP_ELICITATION_ASKUSERQUESTION.md` in the Buzz workspace). The
//! adapter only enables the tool when the client advertises the
//! `elicitation.form` capability; otherwise it drops `AskUserQuestion` into
//! `disallowedTools` and the model falls back to prose.
//!
//! This module is the pure, testable core of the bridge:
//! - [`parse_elicitation_form`] turns a request's `requestedSchema` into a list
//!   of [`ElicitationQuestion`]s (one Buzz card is published per question).
//! - [`parse_card_answer`] reads a Buzz answer event's content JSON.
//! - [`build_elicitation_response`] folds the per-card answers back into the ACP
//!   `CreateElicitationResponse` the adapter expects, keyed by the schema's
//!   `question_<n>` / `question_<n>_custom` property names (a non-empty custom
//!   answer wins over the picked option, matching the adapter's own semantics).
//!
//! The async round-trip (publish each card, await the owner's taps) lives in the
//! pool, which owns the relay client and channel context.

use serde_json::{Map, Value};

/// One selectable option within a question.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElicitationOption {
    pub label: String,
    pub description: Option<String>,
}

/// One question parsed from an `elicitation/create` form schema.
///
/// One Buzz `KIND_ELICITATION_REQUEST` card is published per question.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElicitationQuestion {
    /// Schema property key, e.g. `question_0`. The picked answer is written back
    /// under this key and the free-text override under `<key>_custom` — the two
    /// key shapes the adapter's `applyAskElicitationResponse` reads.
    pub key: String,
    /// Short header/title, when the schema supplied one.
    pub header: Option<String>,
    /// The question prompt. For a single-question form the adapter carries the
    /// text in the top-level `message` rather than the field description, so
    /// callers pass `message` as the fallback.
    pub prompt: String,
    /// `true` when the field accepts multiple selections (schema `type: array`).
    pub multi_select: bool,
    /// `true` when a `<key>_custom` free-text companion field is present.
    pub allow_custom: bool,
    pub options: Vec<ElicitationOption>,
}

/// The owner's answer to a single question card (parsed from a
/// `KIND_ELICITATION_RESPONSE` event's content JSON).
#[derive(Debug, Clone, PartialEq)]
pub enum CardAnswer {
    /// The owner picked option(s) and/or typed a custom answer.
    Accept {
        /// Picked label (string) or labels (array), if any.
        answer: Option<Value>,
        /// Free-text override, if the owner used the "Other…" field.
        custom: Option<String>,
    },
    /// The owner explicitly skipped this question.
    Decline,
    /// The owner cancelled — aborts the whole tool call.
    Cancel,
}

const CUSTOM_SUFFIX: &str = "_custom";

/// Parse an `elicitation/create` form `requestedSchema` into ordered questions.
///
/// `message` is the request's top-level human-readable message, used as the
/// prompt fallback for a single-question form. Returns an empty vec when the
/// schema has no usable question properties.
pub fn parse_elicitation_form(requested_schema: &Value, message: &str) -> Vec<ElicitationQuestion> {
    let Some(properties) = requested_schema
        .get("properties")
        .and_then(Value::as_object)
    else {
        return Vec::new();
    };

    // Property keys ending in `_custom` are free-text companions, not questions.
    let mut questions: Vec<ElicitationQuestion> = properties
        .iter()
        .filter(|(key, _)| !key.ends_with(CUSTOM_SUFFIX))
        .map(|(key, schema)| {
            let multi_select = schema.get("type").and_then(Value::as_str) == Some("array");
            let header = schema
                .get("title")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_owned);
            let prompt = schema
                .get("description")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| message.to_owned());
            let allow_custom = properties.contains_key(&format!("{key}{CUSTOM_SUFFIX}"));
            ElicitationQuestion {
                key: key.clone(),
                header,
                prompt,
                multi_select,
                allow_custom,
                options: parse_options(schema, multi_select),
            }
        })
        .collect();

    // Order by the numeric suffix of `question_<n>` so cards are published in
    // the model's asked order (a plain string sort would place `_10` before
    // `_2`); keys without a numeric suffix keep a stable, lexical fallback.
    questions.sort_by(|a, b| {
        numeric_suffix(&a.key)
            .cmp(&numeric_suffix(&b.key))
            .then_with(|| a.key.cmp(&b.key))
    });
    questions
}

/// Extract option entries from a question schema.
///
/// Single-select questions carry options under `oneOf`; multi-select questions
/// nest them under `items.anyOf`. Each option is an `EnumOption`
/// (`{ const, title, description? }`).
fn parse_options(schema: &Value, multi_select: bool) -> Vec<ElicitationOption> {
    let raw = if multi_select {
        schema.get("items").and_then(|items| items.get("anyOf"))
    } else {
        schema.get("oneOf")
    };
    let Some(entries) = raw.and_then(Value::as_array) else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|opt| {
            // Prefer the human `title`; fall back to the machine `const` value.
            let label = opt
                .get("title")
                .and_then(Value::as_str)
                .or_else(|| opt.get("const").and_then(Value::as_str))?;
            let description = opt
                .get("description")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_owned);
            Some(ElicitationOption {
                label: label.to_owned(),
                description,
            })
        })
        .collect()
}

/// Return the trailing integer of a `question_<n>` key, or `u64::MAX` when there
/// is no numeric suffix (so unsuffixed keys sort last, then lexically).
fn numeric_suffix(key: &str) -> u64 {
    key.rsplit('_')
        .next()
        .and_then(|tail| tail.parse::<u64>().ok())
        .unwrap_or(u64::MAX)
}

/// Parse a `KIND_ELICITATION_RESPONSE` content JSON into a [`CardAnswer`].
///
/// Shape: `{ "action": "accept" | "decline" | "cancel", "answer"?: string |
/// string[], "custom"?: string }`. Unknown or missing actions are treated as
/// `Cancel` (fail-closed: an unparseable answer aborts rather than inventing a
/// selection).
pub fn parse_card_answer(content: &Value) -> CardAnswer {
    match content.get("action").and_then(Value::as_str) {
        Some("accept") => {
            let answer = content.get("answer").filter(|v| !v.is_null()).cloned();
            let custom = content
                .get("custom")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_owned);
            CardAnswer::Accept { answer, custom }
        }
        Some("decline") => CardAnswer::Decline,
        _ => CardAnswer::Cancel,
    }
}

/// Fold per-card answers into an ACP `CreateElicitationResponse`.
///
/// - Any `Cancel` short-circuits the whole response to `{ "action": "cancel" }`
///   (a single cancelled card aborts the tool call).
/// - Otherwise the result is `{ "action": "accept", "content": { … } }`, with
///   each accepted question contributing `content[key]` (the picked value) and,
///   when the owner typed one, `content[key_custom]` (the free-text override).
/// - When every card is declined and none accepted, the response is
///   `{ "action": "decline" }` (the adapter reports the user skipped and the
///   turn continues).
///
/// `items` pairs each question with its answer, in card order.
pub fn build_elicitation_response(items: &[(ElicitationQuestion, CardAnswer)]) -> Value {
    if items
        .iter()
        .any(|(_, answer)| matches!(answer, CardAnswer::Cancel))
    {
        return serde_json::json!({ "action": "cancel" });
    }

    let mut content = Map::new();
    for (question, answer) in items {
        if let CardAnswer::Accept { answer, custom } = answer {
            if let Some(value) = answer {
                content.insert(question.key.clone(), value.clone());
            }
            if let Some(text) = custom {
                content.insert(
                    format!("{}{CUSTOM_SUFFIX}", question.key),
                    Value::String(text.clone()),
                );
            }
        }
    }

    if content.is_empty() {
        return serde_json::json!({ "action": "decline" });
    }
    serde_json::json!({ "action": "accept", "content": Value::Object(content) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The exact single-question shape `askUserQuestionsToCreateRequest` emits:
    /// prompt in the top-level `message`, options under `oneOf`, plus a
    /// `_custom` companion field.
    fn single_question_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "question_0": {
                    "type": "string",
                    "title": "Weight",
                    "oneOf": [
                        { "const": "QUICK", "title": "QUICK", "description": "hack/throwaway" },
                        { "const": "STANDARD", "title": "STANDARD" },
                        { "const": "FLAGSHIP", "title": "FLAGSHIP" }
                    ]
                },
                "question_0_custom": {
                    "type": "string",
                    "title": "Other",
                    "description": "Type your own answer instead of choosing…"
                }
            }
        })
    }

    #[test]
    fn parses_single_question_with_options_and_custom() {
        let questions = parse_elicitation_form(&single_question_schema(), "How heavy is this?");
        assert_eq!(questions.len(), 1);
        let q = &questions[0];
        assert_eq!(q.key, "question_0");
        assert_eq!(q.header.as_deref(), Some("Weight"));
        // No field description → falls back to the form message.
        assert_eq!(q.prompt, "How heavy is this?");
        assert!(!q.multi_select);
        assert!(q.allow_custom);
        assert_eq!(q.options.len(), 3);
        assert_eq!(q.options[0].label, "QUICK");
        assert_eq!(q.options[0].description.as_deref(), Some("hack/throwaway"));
        assert_eq!(q.options[1].description, None);
    }

    #[test]
    fn parses_multi_select_from_items_anyof() {
        let schema = json!({
            "type": "object",
            "properties": {
                "question_0": {
                    "type": "array",
                    "description": "Pick any that apply",
                    "items": { "anyOf": [
                        { "const": "A", "title": "A" },
                        { "const": "B", "title": "B" }
                    ] }
                }
            }
        });
        let questions = parse_elicitation_form(&schema, "unused");
        assert_eq!(questions.len(), 1);
        assert!(questions[0].multi_select);
        assert!(!questions[0].allow_custom);
        assert_eq!(questions[0].prompt, "Pick any that apply");
        assert_eq!(questions[0].options.len(), 2);
    }

    #[test]
    fn orders_questions_by_numeric_suffix_not_lexically() {
        let mut props = Map::new();
        for i in [0u32, 2, 10, 1] {
            props.insert(
                format!("question_{i}"),
                json!({ "type": "string", "description": format!("q{i}"), "oneOf": [] }),
            );
        }
        let schema = json!({ "type": "object", "properties": Value::Object(props) });
        let keys: Vec<_> = parse_elicitation_form(&schema, "m")
            .into_iter()
            .map(|q| q.key)
            .collect();
        assert_eq!(
            keys,
            ["question_0", "question_1", "question_2", "question_10"]
        );
    }

    #[test]
    fn empty_or_missing_schema_yields_no_questions() {
        assert!(parse_elicitation_form(&json!({}), "m").is_empty());
        assert!(parse_elicitation_form(&json!({ "properties": {} }), "m").is_empty());
    }

    #[test]
    fn parse_card_answer_variants() {
        assert_eq!(
            parse_card_answer(&json!({ "action": "accept", "answer": "STANDARD" })),
            CardAnswer::Accept {
                answer: Some(json!("STANDARD")),
                custom: None
            }
        );
        assert_eq!(
            parse_card_answer(&json!({ "action": "accept", "custom": "my own" })),
            CardAnswer::Accept {
                answer: None,
                custom: Some("my own".to_owned())
            }
        );
        // Empty custom string is treated as absent.
        assert_eq!(
            parse_card_answer(&json!({ "action": "accept", "answer": "X", "custom": "" })),
            CardAnswer::Accept {
                answer: Some(json!("X")),
                custom: None
            }
        );
        assert_eq!(
            parse_card_answer(&json!({ "action": "decline" })),
            CardAnswer::Decline
        );
        assert_eq!(
            parse_card_answer(&json!({ "action": "cancel" })),
            CardAnswer::Cancel
        );
        // Fail closed on garbage.
        assert_eq!(parse_card_answer(&json!({})), CardAnswer::Cancel);
    }

    fn q(key: &str) -> ElicitationQuestion {
        ElicitationQuestion {
            key: key.to_owned(),
            header: None,
            prompt: "p".to_owned(),
            multi_select: false,
            allow_custom: true,
            options: vec![],
        }
    }

    #[test]
    fn builds_accept_response_keyed_by_schema_property() {
        let items = vec![(
            q("question_0"),
            CardAnswer::Accept {
                answer: Some(json!("STANDARD")),
                custom: None,
            },
        )];
        assert_eq!(
            build_elicitation_response(&items),
            json!({ "action": "accept", "content": { "question_0": "STANDARD" } })
        );
    }

    #[test]
    fn custom_answer_is_emitted_under_custom_key() {
        let items = vec![(
            q("question_0"),
            CardAnswer::Accept {
                answer: None,
                custom: Some("bespoke".to_owned()),
            },
        )];
        assert_eq!(
            build_elicitation_response(&items),
            json!({ "action": "accept", "content": { "question_0_custom": "bespoke" } })
        );
    }

    #[test]
    fn multi_select_array_answer_passes_through() {
        let items = vec![(
            q("question_0"),
            CardAnswer::Accept {
                answer: Some(json!(["A", "B"])),
                custom: None,
            },
        )];
        assert_eq!(
            build_elicitation_response(&items),
            json!({ "action": "accept", "content": { "question_0": ["A", "B"] } })
        );
    }

    #[test]
    fn any_cancel_short_circuits_to_cancel() {
        let items = vec![
            (
                q("question_0"),
                CardAnswer::Accept {
                    answer: Some(json!("A")),
                    custom: None,
                },
            ),
            (q("question_1"), CardAnswer::Cancel),
        ];
        assert_eq!(
            build_elicitation_response(&items),
            json!({ "action": "cancel" })
        );
    }

    #[test]
    fn all_declined_yields_decline() {
        let items = vec![(q("question_0"), CardAnswer::Decline)];
        assert_eq!(
            build_elicitation_response(&items),
            json!({ "action": "decline" })
        );
    }

    #[test]
    fn declined_card_is_omitted_from_multi_question_content() {
        let items = vec![
            (
                q("question_0"),
                CardAnswer::Accept {
                    answer: Some(json!("A")),
                    custom: None,
                },
            ),
            (q("question_1"), CardAnswer::Decline),
        ];
        assert_eq!(
            build_elicitation_response(&items),
            json!({ "action": "accept", "content": { "question_0": "A" } })
        );
    }
}
