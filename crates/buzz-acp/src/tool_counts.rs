//! Per-turn tool-call and skill-invocation counters for NIP-AM turn metrics.
//!
//! Fed from the in-flight session's `session/update` `tool_call` /
//! `tool_call_update` notifications and drained once when the turn metric is
//! published. Only `tool_call` starts a call; updates enrich a call already
//! started this turn (a tool's name or arguments may only appear on a later
//! update), so late updates from a cancelled earlier turn are not counted.
//!
//! Name resolution (first present wins), names kept exactly as the harness
//! emits them, MCP server prefix included:
//! 1. A stable harness tool name: `toolName`, `_meta.claudeCode.toolName`
//!    (claude-agent-acp), `_meta.goose.toolCall.toolName` (goose),
//!    or `rawInput.toolName`.
//! 2. A name parsed from `title`: codex `"Tool: server/tool"` -> `server/tool`, hermes
//!    `"name: preview"` -> `name`, or a bare identifier title (buzz-agent, pi).
//!    A bare `execute` title is skipped because pi/codex put the shell command
//!    there. Free-text titles like "Read src/foo.rs" are high-cardinality and
//!    rejected.
//! 3. ACP `kind`, then `"unknown"`.
//!
//! Skills (exact tool names only, so an MCP tool merely named `skill` is not
//! one): `load_skill` (buzz-agent) / `skills__load_skill` (goose) ->
//! `rawInput.name` (a `/` marks a supporting-file read, not an invocation);
//! Claude Code `Skill` -> `rawInput.skill` or `_meta.claudeCode.skill`.

use std::collections::{BTreeMap, HashMap};

use serde_json::Value;

/// Name -> count map as published in NIP-AM `tools` / `skills`.
pub(crate) type NameCounts = BTreeMap<String, u32>;

/// Maximum distinct entries per `tools` / `skills` map, `_other` included.
const MAX_NAMES: usize = 64;
/// Maximum byte length of a `tools` / `skills` key.
const MAX_NAME_BYTES: usize = 128;
/// Overflow bucket for names beyond the distinct-name cap or over the length cap.
const OTHER: &str = "_other";
/// Upper bound on distinct tool calls tracked per turn (memory bound).
const MAX_CALLS: usize = 4096;

#[derive(Debug, Default)]
struct Call {
    stable: Option<String>,
    title: Option<String>,
    kind: Option<String>,
    name_arg: Option<String>,
    skill_arg: Option<String>,
}

/// Accumulates tool calls for the in-flight turn.
#[derive(Debug, Default)]
pub(crate) struct ToolCallCounter {
    calls: HashMap<String, Call>,
}

fn s<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str).filter(|x| !x.is_empty())
}

fn is_identifier(t: &str) -> bool {
    !t.is_empty()
        && t.len() <= 64
        && t.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | ':' | '/'))
}

/// Tool name parsed from an ACP `title`, or `None` when it is free text.
fn title_name<'a>(title: &'a str, kind: Option<&str>) -> Option<&'a str> {
    let name = if let Some(rest) = title.strip_prefix("Tool: ") {
        rest
    } else if let Some((prefix, _)) = title.split_once(": ") {
        prefix
    } else if kind == Some("execute") {
        return None;
    } else {
        title
    };
    is_identifier(name).then_some(name)
}

impl ToolCallCounter {
    /// Discard all counts; called when a new turn begins.
    pub(crate) fn begin_turn(&mut self) {
        self.calls.clear();
    }

    /// Fold a `tool_call` (`start`) or `tool_call_update` payload into the
    /// counter. Updates for calls not started this turn are ignored.
    pub(crate) fn observe(&mut self, update: &Value, start: bool) {
        let Some(id) = s(update, "toolCallId") else {
            return;
        };
        if start && !self.calls.contains_key(id) && self.calls.len() < MAX_CALLS {
            self.calls.insert(id.to_string(), Call::default());
        }
        let Some(call) = self.calls.get_mut(id) else {
            return;
        };
        let raw = update.get("rawInput").unwrap_or(&Value::Null);
        let claude = &update["_meta"]["claudeCode"];
        let kind = s(update, "kind");
        fill(&mut call.stable, || {
            s(update, "toolName")
                .or_else(|| s(claude, "toolName"))
                .or_else(|| s(&update["_meta"]["goose"]["toolCall"], "toolName"))
                .or_else(|| s(raw, "toolName"))
        });
        fill(&mut call.title, || {
            s(update, "title").and_then(|t| title_name(t, kind))
        });
        fill(&mut call.kind, || kind);
        fill(&mut call.name_arg, || s(raw, "name"));
        fill(&mut call.skill_arg, || {
            s(raw, "skill").or_else(|| s(claude, "skill"))
        });
    }

    /// Drain into `(tools, skills)` maps; `None` when no tool calls were seen.
    pub(crate) fn take(&mut self) -> (Option<NameCounts>, Option<NameCounts>) {
        let mut tools = BTreeMap::new();
        let mut skills = BTreeMap::new();
        for call in std::mem::take(&mut self.calls).into_values() {
            let name = call
                .stable
                .as_deref()
                .or(call.title.as_deref())
                .or(call.kind.as_deref())
                .unwrap_or("unknown");
            bump(&mut tools, name);
            let skill = match name {
                "load_skill" | "skills__load_skill" => {
                    call.name_arg.as_deref().filter(|n| !n.contains('/'))
                }
                "Skill" => call.skill_arg.as_deref(),
                _ => None,
            };
            if let Some(skill) = skill {
                bump(&mut skills, skill);
            }
        }
        let some = |m: BTreeMap<String, u32>| (!m.is_empty()).then_some(m);
        (some(tools), some(skills))
    }
}

/// Set an empty slot from `v`, allocating only when the slot is still empty.
fn fill<'a>(slot: &mut Option<String>, v: impl FnOnce() -> Option<&'a str>) {
    if slot.is_none() {
        *slot = v().map(str::to_string);
    }
}

/// Count `name`, folding into [`OTHER`] when over the byte or distinct-name cap.
fn bump(map: &mut BTreeMap<String, u32>, name: &str) {
    let key = if name.len() > MAX_NAME_BYTES
        || (!map.contains_key(name)
            && map.len() - usize::from(map.contains_key(OTHER)) >= MAX_NAMES - 1)
    {
        OTHER
    } else {
        name
    };
    let n = map.entry(key.to_string()).or_insert(0);
    *n = n.saturating_add(1);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn start(c: &mut ToolCallCounter, v: &Value) {
        c.observe(v, true);
    }

    fn take(c: &mut ToolCallCounter) -> (BTreeMap<String, u32>, BTreeMap<String, u32>) {
        let (t, s) = c.take();
        (t.unwrap_or_default(), s.unwrap_or_default())
    }

    #[test]
    fn dedupes_by_id_and_prefers_stable_name_over_title() {
        let mut c = ToolCallCounter::default();
        start(
            &mut c,
            &json!({"toolCallId":"1","title":"Read src/foo.rs","kind":"read"}),
        );
        start(
            &mut c,
            &json!({"toolCallId":"1","_meta":{"claudeCode":{"toolName":"Read"}}}),
        );
        start(&mut c, &json!({"toolCallId":"1","status":"completed"}));
        start(
            &mut c,
            &json!({"toolCallId":"2","title":"Run ls -la","kind":"execute"}),
        );
        let (tools, skills) = take(&mut c);
        assert_eq!(
            tools,
            BTreeMap::from([("Read".into(), 1), ("execute".into(), 1)])
        );
        assert!(skills.is_empty());
    }

    #[test]
    fn bare_identifier_title_is_the_tool_name() {
        let mut c = ToolCallCounter::default();
        start(
            &mut c,
            &json!({"toolCallId":"1","title":"shell","kind":"other"}),
        );
        assert_eq!(take(&mut c).0, BTreeMap::from([("shell".into(), 1)]));
    }

    #[test]
    fn skills_detected_and_supporting_file_reads_excluded() {
        let mut c = ToolCallCounter::default();
        start(
            &mut c,
            &json!({"toolCallId":"1","title":"load_skill","rawInput":{"name":"git"}}),
        );
        start(
            &mut c,
            &json!({"toolCallId":"2","title":"load_skill","rawInput":{"name":"git/refs/a.md"}}),
        );
        start(
            &mut c,
            &json!({"toolCallId":"3","_meta":{"claudeCode":{"toolName":"Skill"}}}),
        );
        c.observe(
            &json!({"toolCallId":"3","rawInput":{"skill":"review"}}),
            false,
        );
        // An MCP tool merely named `skill` is not a skill invocation.
        start(
            &mut c,
            &json!({"toolCallId":"4","toolName":"mcp__notes__skill","rawInput":{"skill":"py"}}),
        );
        let (tools, skills) = take(&mut c);
        assert_eq!(tools["load_skill"], 2);
        assert_eq!(tools["Skill"], 1);
        assert_eq!(
            skills,
            BTreeMap::from([("git".into(), 1), ("review".into(), 1)])
        );
    }

    #[test]
    fn caps_distinct_names_and_name_length() {
        let mut c = ToolCallCounter::default();
        for i in 0..100 {
            start(
                &mut c,
                &json!({"toolCallId":i.to_string(),"toolName":format!("t{i}")}),
            );
        }
        start(
            &mut c,
            &json!({"toolCallId":"long","toolName":"x".repeat(200)}),
        );
        let (tools, _) = take(&mut c);
        assert_eq!(tools.len(), MAX_NAMES);
        assert_eq!(tools[OTHER], 100 - (MAX_NAMES as u32 - 1) + 1);
    }

    #[test]
    fn empty_turn_is_none_and_begin_turn_resets() {
        let mut c = ToolCallCounter::default();
        assert_eq!(c.take(), (None, None));
        start(&mut c, &json!({"toolCallId":"1","toolName":"Read"}));
        c.begin_turn();
        // A late update for a call started before this turn is not counted.
        c.observe(&json!({"toolCallId":"1","status":"cancelled"}), false);
        assert_eq!(c.take(), (None, None));
    }

    /// Field shapes taken from each adapter's source (see the module docs).
    #[test]
    fn resolves_names_across_harness_wire_formats() {
        let mut c = ToolCallCounter::default();
        // goose: name only in `_meta.goose.toolCall.toolName`, free-text title.
        start(
            &mut c,
            &json!({"toolCallId":"g1","title":"developer: shell · cargo test","kind":"other",
            "_meta":{"goose":{"toolCall":{"toolName":"developer__shell","extensionName":"developer"}}}}),
        );
        start(
            &mut c,
            &json!({"toolCallId":"g2","title":"skills: load skill · git","kind":"other",
            "rawInput":{"name":"git"},
            "_meta":{"goose":{"toolCall":{"toolName":"skills__load_skill"}}}}),
        );
        // claude-agent-acp: MCP prefix kept; skill name arrives on the update.
        start(
            &mut c,
            &json!({"toolCallId":"c1","title":"mcp__docs__search","kind":"other",
            "_meta":{"claudeCode":{"toolName":"mcp__docs__search"}}}),
        );
        start(
            &mut c,
            &json!({"toolCallId":"c2","title":"Load skill","rawInput":{},
            "_meta":{"claudeCode":{"toolName":"Skill"}}}),
        );
        start(
            &mut c,
            &json!({"toolCallId":"c2","title":"Load skill: commits",
            "_meta":{"claudeCode":{"toolName":"Skill","skill":"commits"}}}),
        );
        // codex-acp: MCP via the "Tool: server/tool" title; shell command title skipped.
        start(
            &mut c,
            &json!({"toolCallId":"x1","title":"Tool: github/create_issue","kind":"other",
            "rawInput":{"server":"github","tool":"create_issue","arguments":{}}}),
        );
        start(
            &mut c,
            &json!({"toolCallId":"x2","title":"Tool: lookup","kind":"other","rawInput":{}}),
        );
        start(
            &mut c,
            &json!({"toolCallId":"x3","title":"ls","kind":"execute"}),
        );
        // hermes: "name: preview" title, no rawInput.
        start(
            &mut c,
            &json!({"toolCallId":"h1","title":"terminal: ls -la","kind":"execute"}),
        );
        // pi: bare tool-name title.
        start(
            &mut c,
            &json!({"toolCallId":"p1","title":"read","kind":"read","rawInput":{"path":"a"}}),
        );
        let (tools, skills) = take(&mut c);
        assert_eq!(
            tools,
            BTreeMap::from([
                ("Skill".into(), 1),
                ("developer__shell".into(), 1),
                ("execute".into(), 1),
                ("github/create_issue".into(), 1),
                ("lookup".into(), 1),
                ("mcp__docs__search".into(), 1),
                ("read".into(), 1),
                ("skills__load_skill".into(), 1),
                ("terminal".into(), 1),
            ])
        );
        assert_eq!(
            skills,
            BTreeMap::from([("commits".into(), 1), ("git".into(), 1)])
        );
    }
}
