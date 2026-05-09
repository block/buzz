//! In-memory session task list with `_Stop` and `_PostCompact` hooks.
//!
//! - `todo`: read or full-list-replace the task list. Empty args reads.
//! - `_Stop`: hook called by the agent before honoring end_turn. Returns
//!   objection text if any items are open, empty otherwise.
//! - `_PostCompact`: hook called after context compaction/handoff.
//!   Returns the full list state so the agent can re-inject it.
//!
//! State is per-process (Vec<Item> behind a Mutex). Same shape as the
//! original built-in todo it replaces.

use rmcp::model::{CallToolResult, Content};
use rmcp::ErrorData;
use schemars::JsonSchema;
use serde::Deserialize;
use std::sync::Mutex;

const MAX_ITEMS: usize = 50;
const MAX_ID: u32 = 9999;
const MAX_TITLE_CHARS: usize = 200;

#[derive(Debug, Clone, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct Item {
    #[schemars(range(max = 9999))]
    pub id: u32,
    #[schemars(length(min = 1, max = 200))]
    pub title: String,
    #[serde(default)]
    pub done: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TodoParams {
    /// Full replacement list (max 50 items). Omit to read.
    #[serde(default)]
    #[schemars(length(max = 50))]
    pub todos: Option<Vec<Item>>,
}

/// Empty params struct for the hook tools. Hooks take no arguments but
/// rmcp requires Parameters<T> for the macro.
#[derive(Debug, Deserialize, JsonSchema, Default)]
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

    /// Replace-or-read. Returns the rendered list.
    pub fn handle_todo(&self, params: TodoParams) -> Result<String, String> {
        if let Some(new_items) = params.todos {
            validate(&new_items)?;
            self.with_items(|items| *items = new_items);
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

fn validate(items: &[Item]) -> Result<(), String> {
    if items.len() > MAX_ITEMS {
        return Err(format!("too many items (max {MAX_ITEMS})"));
    }
    let mut seen = std::collections::HashSet::with_capacity(items.len());
    for it in items {
        if it.id > MAX_ID {
            return Err(format!("id {} exceeds max {MAX_ID}", it.id));
        }
        if !seen.insert(it.id) {
            return Err(format!("duplicate id {}", it.id));
        }
        if it.title.trim().is_empty() {
            return Err(format!("item {}: title is empty", it.id));
        }
        if it.title.chars().count() > MAX_TITLE_CHARS {
            return Err(format!(
                "item {}: title exceeds {MAX_TITLE_CHARS} characters",
                it.id
            ));
        }
    }
    Ok(())
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
        out.push_str(&it.id.to_string());
        out.push_str(". ");
        out.push_str(&it.title);
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

    fn mk(items: &[(u32, &str, bool)]) -> Vec<Item> {
        items
            .iter()
            .map(|(id, title, done)| Item {
                id: *id,
                title: (*title).to_owned(),
                done: *done,
            })
            .collect()
    }

    #[test]
    fn empty_read_returns_placeholder() {
        let s = TodoState::new();
        let out = s.handle_todo(TodoParams { todos: None }).unwrap();
        assert!(out.contains("empty"));
    }

    #[test]
    fn rejects_duplicate_ids() {
        let s = TodoState::new();
        let err = s
            .handle_todo(TodoParams {
                todos: Some(mk(&[(1, "a", false), (1, "b", false)])),
            })
            .unwrap_err();
        assert!(err.contains("duplicate id 1"));
    }

    #[test]
    fn rejects_empty_title() {
        let s = TodoState::new();
        let err = s
            .handle_todo(TodoParams {
                todos: Some(mk(&[(1, "   ", false)])),
            })
            .unwrap_err();
        assert!(err.contains("title is empty"));
    }

    #[test]
    fn rejects_too_many_items() {
        let s = TodoState::new();
        let many: Vec<Item> = (0u32..=(MAX_ITEMS as u32))
            .map(|i| Item {
                id: i,
                title: "x".into(),
                done: false,
            })
            .collect();
        let err = s
            .handle_todo(TodoParams { todos: Some(many) })
            .unwrap_err();
        assert!(err.contains("too many items"));
    }

    #[test]
    fn stop_returns_objection_when_open_items_exist() {
        let s = TodoState::new();
        s.handle_todo(TodoParams {
            todos: Some(mk(&[(1, "a", false), (2, "b", false)])),
        })
        .unwrap();
        let obj = s.stop_objection();
        assert!(!obj.is_empty(), "expected non-empty objection");
        assert!(obj.contains("open todo items"));
        assert!(obj.contains("a"));
    }

    #[test]
    fn stop_returns_empty_when_all_done() {
        let s = TodoState::new();
        s.handle_todo(TodoParams {
            todos: Some(mk(&[(1, "a", true), (2, "b", true)])),
        })
        .unwrap();
        assert_eq!(s.stop_objection(), "");
    }

    #[test]
    fn stop_returns_empty_when_list_is_empty() {
        assert_eq!(TodoState::new().stop_objection(), "");
    }

    #[test]
    fn post_compact_renders_when_populated() {
        let s = TodoState::new();
        s.handle_todo(TodoParams {
            todos: Some(mk(&[(1, "a", false)])),
        })
        .unwrap();
        let block = s.post_compact();
        assert!(block.starts_with("# Todo List\n"));
    }

    #[test]
    fn post_compact_empty_when_no_items() {
        assert_eq!(TodoState::new().post_compact(), "");
    }
}
