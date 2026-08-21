//! In-memory session task list with `_Stop` and `_PostCompact` hooks.
//!
//! - `todo`: read or full-list-replace the task list. Empty args reads.
//! - `_Stop`: hook called by the agent before honoring end_turn. Returns
//!   objection text if any items are open, empty otherwise.
//! - `_PostCompact`: hook called after context compaction/handoff.
//!   Returns the full list state so the agent can re-inject it.
//!
//! State is per-process (Vec<Item> behind a Mutex). Items are
//! `{text, done}` — no ids; the LLM provides a full replacement list.
//! On replacement, open items that disappear without being marked done
//! trigger a soft warning appended to the tool response.

use rmcp::model::{CallToolResult, Content};
use rmcp::ErrorData;
use schemars::JsonSchema;
use serde::Deserialize;
use std::sync::Mutex;

const MAX_ITEMS: usize = 50;
const MAX_TEXT_CHARS: usize = 200;

#[derive(Debug, Clone, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Item {
    #[schemars(length(min = 1, max = 200))]
    pub text: String,
    #[serde(default)]
    pub done: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TodoParams {
    /// Full replacement list (max 50 items). Omit to read.
    // Both omitted and explicit null mean "read current state".
    #[serde(default)]
    #[schemars(length(max = 50))]
    pub todos: Option<Vec<Item>>,
}

/// Empty params struct for the hook tools. Hooks take no arguments but
/// rmcp requires Parameters<T> for the macro.
#[derive(Debug, Deserialize, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct HookParams {}

#[derive(Debug, Default)]
pub struct TodoState {
    items: Mutex<Vec<Item>>,
}

impl TodoState {
    pub fn new() -> Self {
        Self::default()
    }

    fn with_items<R>(&self, f: impl FnOnce(&mut Vec<Item>) -> R) -> R {
        let mut g = match self.items.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        f(&mut g)
    }

    /// Replace-or-read. Returns the rendered list, with a warning
    /// appended if open items were silently removed. Mutation, warning
    /// computation, and rendering all occur under a single lock hold so
    /// that the rendered output reflects exactly the list that was just
    /// written (no interleaving with concurrent calls).
    pub fn handle_todo(&self, params: TodoParams) -> Result<String, String> {
        if let Some(mut new_items) = params.todos {
            validate(&new_items)?;
            // Normalize: trim text on storage so duplicate detection
            // and silent-removal diff operate on the same canonical
            // form. Validation already rejected empty-after-trim text,
            // duplicates after trim, and invalid characters.
            for it in &mut new_items {
                it.text = it.text.trim().to_owned();
            }
            return Ok(self.with_items(|items| {
                let warning = silent_removal_warning(items, &new_items);
                *items = new_items;
                let mut out = render_items(items);
                if !warning.is_empty() {
                    out.push('\n');
                    out.push_str(&warning);
                }
                out
            }));
        }
        Ok(self.render())
    }

    pub fn render(&self) -> String {
        self.with_items(|items| render_items(items))
    }

    /// Objection text if open items exist, empty string otherwise.
    pub fn stop_objection(&self) -> String {
        self.with_items(|items| {
            if items.iter().any(|i| !i.done) {
                format!(
                    "You have open todo items. Keep working.\n\n{}",
                    render_items(items)
                )
            } else {
                String::new()
            }
        })
    }

    /// Re-injection block for after a handoff. Empty if no items.
    pub fn post_compact(&self) -> String {
        self.with_items(|items| {
            if items.is_empty() {
                String::new()
            } else {
                format!("# Todo List\n{}", render_items(items))
            }
        })
    }
}

/// Reject characters that aren't safe for single-line display. ASCII
/// space is the only whitespace allowed; everything else (control
/// chars, exotic whitespace, bidi/format chars, zero-width joiners,
/// BOM) could be used to spoof the rendered todo output.
fn invalid_text_char(c: char) -> bool {
    if c.is_control() {
        return true;
    }
    if c.is_whitespace() && c != ' ' {
        // Catches \t, \n, \r, NBSP (\u{00A0}), line/paragraph
        // separators (\u{2028}, \u{2029}), and other exotic spaces.
        return true;
    }
    matches!(c,
        '\u{200B}'..='\u{200F}' // zero-width spaces, LRM/RLM
        | '\u{202A}'..='\u{202E}' // bidi embedding/override
        | '\u{2060}'..='\u{206F}' // word joiner, invisible operators
        | '\u{FEFF}' // BOM / zero-width no-break space
    )
}

fn validate(items: &[Item]) -> Result<(), String> {
    if items.len() > MAX_ITEMS {
        return Err(format!("too many items (max {MAX_ITEMS})"));
    }
    for (i, it) in items.iter().enumerate() {
        if it.text.trim().is_empty() {
            return Err(format!("item {}: text is empty", i + 1));
        }
        if it.text.trim().chars().count() > MAX_TEXT_CHARS {
            return Err(format!(
                "item {}: text exceeds {MAX_TEXT_CHARS} characters",
                i + 1
            ));
        }
        // Reject control characters and Unicode trickery that could
        // spoof the rendered output: line/paragraph separators, bidi
        // overrides, zero-width joiners, BOM, etc. See
        // `invalid_text_char` for the full list.
        if it.text.chars().any(invalid_text_char) {
            return Err(format!("item {}: text contains invalid characters", i + 1));
        }
    }
    // Reject duplicates (after trim). Without ids, duplicates are
    // ambiguous: silent-removal detection can't tell which copy was
    // dropped, and they have no semantic meaning anyway.
    for (i, a) in items.iter().enumerate() {
        let a_trim = a.text.trim();
        for (j, b) in items.iter().enumerate().skip(i + 1) {
            if a_trim == b.text.trim() {
                return Err(format!(
                    "item {}: duplicate text (matches item {})",
                    j + 1,
                    i + 1
                ));
            }
        }
    }
    Ok(())
}

/// Diff old vs new. Any open (`!done`) item in `old` whose `text` is
/// absent from `new` (any position, any done-state) is "silently
/// removed". Returns the warning string, or empty if none.
///
/// `validate()` rejects duplicate text in `new`, so a `HashSet` lookup
/// is unambiguous here.
fn silent_removal_warning(old: &[Item], new: &[Item]) -> String {
    let new_texts: std::collections::HashSet<&str> = new.iter().map(|i| i.text.as_str()).collect();
    let removed: Vec<&str> = old
        .iter()
        .filter(|i| !i.done && !new_texts.contains(i.text.as_str()))
        .map(|i| i.text.as_str())
        .collect();
    if removed.is_empty() {
        return String::new();
    }
    let mut out = format!(
        "⚠️ {} open item{} {} removed from the list:\n",
        removed.len(),
        if removed.len() == 1 { "" } else { "s" },
        if removed.len() == 1 { "was" } else { "were" },
    );
    for t in &removed {
        out.push_str("  - \"");
        out.push_str(t);
        out.push_str("\"\n");
    }
    out.push_str(
        "If accidental, re-add them. If they are complete, prefer marking items done before removing.",
    );
    out
}

fn render_items(items: &[Item]) -> String {
    if items.is_empty() {
        return "(todo list is empty)".into();
    }
    let next = items.iter().position(|i| !i.done);
    let mut out = String::with_capacity(64 * items.len());
    for (i, it) in items.iter().enumerate() {
        let box_ = if it.done { "[x]" } else { "[ ]" };
        out.push_str(box_);
        out.push(' ');
        out.push_str(&(i + 1).to_string());
        out.push_str(". ");
        out.push_str(&it.text);
        if Some(i) == next {
            out.push_str("  ← next");
        }
        out.push('\n');
    }
    out
}

/// Wrap a string result as an MCP CallToolResult with text content.
pub fn text_result(s: String) -> Result<CallToolResult, ErrorData> {
    Ok(CallToolResult::success(vec![Content::text(s)]))
}

/// Wrap an error string as an MCP CallToolResult with isError=true.
pub fn error_result(s: String) -> Result<CallToolResult, ErrorData> {
    Ok(CallToolResult::error(vec![Content::text(s)]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk(items: &[(&str, bool)]) -> Vec<Item> {
        items
            .iter()
            .map(|(text, done)| Item {
                text: (*text).to_owned(),
                done: *done,
            })
            .collect()
    }

    fn write(s: &TodoState, items: &[(&str, bool)]) -> String {
        s.handle_todo(TodoParams {
            todos: Some(mk(items)),
        })
        .unwrap()
    }

    #[test]
    fn empty_read_returns_placeholder() {
        let s = TodoState::new();
        let out = s.handle_todo(TodoParams { todos: None }).unwrap();
        assert!(out.contains("empty"));
    }

    #[test]
    fn rejects_empty_text() {
        let s = TodoState::new();
        let err = s
            .handle_todo(TodoParams {
                todos: Some(mk(&[("   ", false)])),
            })
            .unwrap_err();
        assert!(err.contains("text is empty"));
    }

    #[test]
    fn rejects_too_many_items() {
        let s = TodoState::new();
        let many: Vec<Item> = (0..=MAX_ITEMS)
            .map(|i| Item {
                text: format!("item {i}"),
                done: false,
            })
            .collect();
        let err = s.handle_todo(TodoParams { todos: Some(many) }).unwrap_err();
        assert!(err.contains("too many items"));
    }

    #[test]
    fn rejects_duplicate_text() {
        let s = TodoState::new();
        let err = s
            .handle_todo(TodoParams {
                todos: Some(mk(&[("same", false), ("same", true)])),
            })
            .unwrap_err();
        assert!(err.contains("duplicate"), "got: {err}");
    }

    #[test]
    fn rejects_duplicate_text_after_trim() {
        let s = TodoState::new();
        let err = s
            .handle_todo(TodoParams {
                todos: Some(mk(&[("foo", false), ("  foo  ", false)])),
            })
            .unwrap_err();
        assert!(err.contains("duplicate"), "got: {err}");
    }

    #[test]
    fn rejects_control_characters() {
        let s = TodoState::new();
        let err_n = s
            .handle_todo(TodoParams {
                todos: Some(mk(&[("line1\nline2", false)])),
            })
            .unwrap_err();
        assert!(err_n.contains("invalid characters"), "got: {err_n}");
        let err_t = s
            .handle_todo(TodoParams {
                todos: Some(mk(&[("col1\tcol2", false)])),
            })
            .unwrap_err();
        assert!(err_t.contains("invalid characters"), "got: {err_t}");
    }

    #[test]
    fn rejects_unicode_line_separator() {
        let s = TodoState::new();
        let err = s
            .handle_todo(TodoParams {
                todos: Some(mk(&[("line1\u{2028}line2", false)])),
            })
            .unwrap_err();
        assert!(err.contains("invalid characters"), "got: {err}");
    }

    #[test]
    fn rejects_bidi_override() {
        let s = TodoState::new();
        let err = s
            .handle_todo(TodoParams {
                todos: Some(mk(&[("text\u{202E}reversed", false)])),
            })
            .unwrap_err();
        assert!(err.contains("invalid characters"), "got: {err}");
    }

    #[test]
    fn rejects_zero_width() {
        let s = TodoState::new();
        let err = s
            .handle_todo(TodoParams {
                todos: Some(mk(&[("zero\u{200B}width", false)])),
            })
            .unwrap_err();
        assert!(err.contains("invalid characters"), "got: {err}");
    }

    #[test]
    fn leading_trailing_spaces_trimmed_on_store() {
        let s = TodoState::new();
        // Send with surrounding spaces.
        let out = write(&s, &[("  task  ", false)]);
        // Render should have the trimmed form; no leading/trailing
        // spaces in the displayed text.
        assert!(out.contains("[ ] 1. task"));
        assert!(!out.contains("  task  "));
        // A subsequent write of the trimmed form should not falsely
        // warn that "  task  " was silently removed: stored text is
        // already "task", so the diff sees no removal.
        let out2 = write(&s, &[("task", false)]);
        assert!(!out2.contains("⚠️"), "unexpected warning: {out2}");
    }

    #[test]
    fn hook_params_rejects_unknown_fields() {
        let res: Result<HookParams, _> = serde_json::from_str(r#"{"x":1}"#);
        assert!(res.is_err(), "expected error, got: {res:?}");
        // Sanity: empty object parses.
        let ok: Result<HookParams, _> = serde_json::from_str("{}");
        assert!(ok.is_ok(), "expected ok, got: {ok:?}");
    }

    #[test]
    fn explicit_null_is_read() {
        let s = TodoState::new();
        write(&s, &[("alpha", false)]);
        // {"todos": null} should behave the same as omitting the field
        // — return the current list, don't clear it.
        let params: TodoParams = serde_json::from_str(r#"{"todos":null}"#).unwrap();
        let out = s.handle_todo(params).unwrap();
        assert!(out.contains("[ ] 1. alpha"), "got: {out}");
        // Verify state was not mutated.
        let again = s.render();
        assert!(again.contains("[ ] 1. alpha"));
    }

    #[test]
    fn rejects_unknown_fields() {
        // Unknown field on Item: `complete` instead of `done`.
        let bad_item = r#"{"todos":[{"text":"x","complete":true}]}"#;
        let res: Result<TodoParams, _> = serde_json::from_str(bad_item);
        assert!(res.is_err(), "expected error, got: {res:?}");
        // Unknown field on TodoParams.
        let bad_root = r#"{"todos":[{"text":"x"}],"extra":1}"#;
        let res: Result<TodoParams, _> = serde_json::from_str(bad_root);
        assert!(res.is_err(), "expected error, got: {res:?}");
        // Sanity: the correct shape parses.
        let good = r#"{"todos":[{"text":"x","done":true}]}"#;
        let res: Result<TodoParams, _> = serde_json::from_str(good);
        assert!(res.is_ok(), "expected ok, got: {res:?}");
    }

    #[test]
    fn text_at_boundary_200_chars() {
        let s = TodoState::new();
        let exactly_200 = "a".repeat(200);
        let out = s
            .handle_todo(TodoParams {
                todos: Some(mk(&[(exactly_200.as_str(), false)])),
            })
            .unwrap();
        assert!(out.contains(&exactly_200));
        let too_long = "b".repeat(201);
        let err = s
            .handle_todo(TodoParams {
                todos: Some(mk(&[(too_long.as_str(), false)])),
            })
            .unwrap_err();
        assert!(err.contains("exceeds"), "got: {err}");
    }

    #[test]
    fn write_and_render_atomic() {
        // The string returned from a write must reflect the list that
        // was just written, not stale state. With the atomic
        // implementation the response IS computed under the same lock
        // hold as the mutation, so this is guaranteed structurally; we
        // assert the contract.
        let s = TodoState::new();
        let out = write(&s, &[("alpha", false), ("beta", true)]);
        assert!(out.contains("[ ] 1. alpha"));
        assert!(out.contains("[x] 2. beta"));
        // And a subsequent render returns the same content.
        let again = s.render();
        assert!(again.contains("[ ] 1. alpha"));
        assert!(again.contains("[x] 2. beta"));
    }

    #[test]
    fn render_uses_position_numbers() {
        let s = TodoState::new();
        let out = write(&s, &[("first", true), ("second", false), ("third", false)]);
        assert!(out.contains("[x] 1. first"));
        assert!(out.contains("[ ] 2. second"));
        assert!(out.contains("[ ] 3. third"));
        // ← next on first open item
        assert!(out.contains("[ ] 2. second  ← next"));
        assert!(!out.contains("[ ] 3. third  ← next"));
    }

    #[test]
    fn stop_returns_objection_when_open_items_exist() {
        let s = TodoState::new();
        write(&s, &[("a", false), ("b", false)]);
        let obj = s.stop_objection();
        assert!(!obj.is_empty(), "expected non-empty objection");
        assert!(obj.contains("open todo items"));
        assert!(obj.contains("a"));
    }

    #[test]
    fn stop_returns_empty_when_all_done() {
        let s = TodoState::new();
        write(&s, &[("a", true), ("b", true)]);
        assert_eq!(s.stop_objection(), "");
    }

    #[test]
    fn stop_returns_empty_when_list_is_empty() {
        assert_eq!(TodoState::new().stop_objection(), "");
    }

    #[test]
    fn post_compact_renders_when_populated() {
        let s = TodoState::new();
        write(&s, &[("a", false)]);
        let block = s.post_compact();
        assert!(block.starts_with("# Todo List\n"));
    }

    #[test]
    fn post_compact_empty_when_no_items() {
        assert_eq!(TodoState::new().post_compact(), "");
    }

    #[test]
    fn silent_removal_warns() {
        let s = TodoState::new();
        write(
            &s,
            &[
                ("fix the flaky test", false),
                ("write unit tests", false),
                ("ship it", false),
            ],
        );
        let out = write(&s, &[("ship it", false)]);
        assert!(out.contains("⚠️"), "expected warning, got: {out}");
        assert!(out.contains("2 open items"));
        assert!(out.contains("\"fix the flaky test\""));
        assert!(out.contains("\"write unit tests\""));
    }

    #[test]
    fn removal_of_done_item_no_warning() {
        let s = TodoState::new();
        write(&s, &[("done thing", true), ("open thing", false)]);
        let out = write(&s, &[("open thing", false)]);
        assert!(!out.contains("⚠️"), "unexpected warning: {out}");
    }

    #[test]
    fn rename_triggers_warning() {
        let s = TodoState::new();
        write(&s, &[("write tests", false)]);
        let out = write(&s, &[("write unit tests", false)]);
        assert!(out.contains("⚠️"), "expected warning, got: {out}");
        assert!(out.contains("\"write tests\""));
    }

    #[test]
    fn marking_done_then_removing_no_warning() {
        let s = TodoState::new();
        write(&s, &[("task", false)]);
        // Mark done.
        let out1 = write(&s, &[("task", true)]);
        assert!(!out1.contains("⚠️"));
        // Now remove (it's done in the prior list).
        let out2 = write(&s, &[]);
        assert!(!out2.contains("⚠️"), "unexpected warning: {out2}");
    }
}
