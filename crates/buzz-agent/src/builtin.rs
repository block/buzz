//! Built-in tools that run in-process, bypassing MCP.
//!
//! Currently: `load_skill` — reads a skill's full SKILL.md body from disk
//! and returns it so the agent can load skill content on demand rather than
//! having every skill inlined into the system prompt at session start.

use serde_json::{json, Value};

use crate::hints::{SkillEntry, MAX_SKILL_BODY_BYTES};
use crate::mcp::truncate_at_boundary;
use crate::types::{ToolDef, ToolResult, ToolResultContent};

pub const LOAD_SKILL_TOOL: &str = "load_skill";

/// Return the `ToolDef` for `load_skill` to include in the LLM tool list.
pub fn load_skill_def() -> ToolDef {
    ToolDef {
        name: LOAD_SKILL_TOOL.to_owned(),
        description: "Load the full content of a skill by name. \
            Call this before using a skill — the system prompt lists skill names \
            and descriptions only; the full instructions are loaded on demand."
            .to_owned(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "The skill name as listed in the Available Skills section."
                }
            },
            "required": ["name"]
        }),
    }
}

/// Execute a `load_skill` call. Returns `Ok(ToolResult)` on success or a
/// user-visible error result if the skill is not found or cannot be read.
pub fn call_load_skill(arguments: &Value, skills: &[SkillEntry]) -> ToolResult {
    let name = match arguments.get("name").and_then(Value::as_str) {
        Some(n) => n,
        None => {
            return error_result("load_skill: missing required argument \"name\"");
        }
    };

    let entry = match skills.iter().find(|s| s.name == name) {
        Some(e) => e,
        None => {
            let available: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
            return error_result(&format!(
                "load_skill: skill {name:?} not found. Available: {available:?}"
            ));
        }
    };

    let raw = match std::fs::read_to_string(&entry.path) {
        Ok(s) => s,
        Err(e) => {
            return error_result(&format!(
                "load_skill: could not read {:?}: {e}",
                entry.path
            ));
        }
    };

    // Strip the YAML frontmatter — the agent already knows name/description
    // from the system prompt; return only the body.
    let body = strip_frontmatter(&raw);
    let body = if body.len() > MAX_SKILL_BODY_BYTES {
        truncate_at_boundary(body, MAX_SKILL_BODY_BYTES)
    } else {
        body
    };

    ToolResult {
        provider_id: String::new(),
        content: vec![ToolResultContent::Text(body.to_owned())],
        is_error: false,
    }
}

fn strip_frontmatter(content: &str) -> &str {
    let Some(rest) = content.strip_prefix("---\n") else {
        return content;
    };
    let Some(close_pos) = rest.find("\n---") else {
        return content;
    };
    let after = &rest[close_pos + 4..]; // skip "\n---"
    after.strip_prefix('\n').unwrap_or(after)
}

fn error_result(msg: &str) -> ToolResult {
    ToolResult {
        provider_id: String::new(),
        content: vec![ToolResultContent::Text(msg.to_owned())],
        is_error: true,
    }
}
