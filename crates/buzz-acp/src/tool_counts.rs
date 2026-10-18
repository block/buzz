//! Per-turn tool-call and skill-invocation counters for NIP-AM turn metrics.
//!
//! Fed from `session/update` `tool_call` / `tool_call_update` notifications and
//! drained once when the turn metric is published. Calls are deduped by
//! `toolCallId`, because a tool's name or arguments may only appear on a later
//! `tool_call_update`.
//!
//! Name resolution (first present wins): a stable harness tool name
//! (`toolName`, `_meta.claudeCode.toolName`, `rawInput.toolName`), then `title`
//! when it is a bare identifier (buzz-agent puts the tool name there; free-text
//! titles like "Read src/foo.rs" are high-cardinality and rejected), then ACP
//! `kind`. Skills: `load_skill` -> `rawInput.name` (a `/` marks a supporting-file
//! read, not an invocation); Claude Code `Skill` -> `rawInput.skill`.

use std::collections::{BTreeMap, HashMap};

use buzz_core::agent_turn_metric::{MAX_TURN_METRIC_NAMES, MAX_TURN_METRIC_NAME_BYTES};
use serde_json::Value;

/// Name -> count map as published in NIP-AM `tools` / `skills`.
pub(crate) type NameCounts = BTreeMap<String, u32>;

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
    t.len() <= 64
        && t.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | ':'))
}

impl ToolCallCounter {
    /// Discard all counts; called when a new turn begins.
    pub(crate) fn begin_turn(&mut self) {
        self.calls.clear();
    }

    /// Fold a `tool_call` or `tool_call_update` payload into the counter.
    pub(crate) fn observe(&mut self, update: &Value) {
        let Some(id) = s(update, "toolCallId") else {
            return;
        };
        if !self.calls.contains_key(id) && self.calls.len() >= MAX_CALLS {
            return;
        }
        let call = self.calls.entry(id.to_string()).or_default();
        let raw = update.get("rawInput").unwrap_or(&Value::Null);
        let meta = &update["_meta"]["claudeCode"];
        let fill = |slot: &mut Option<String>, v: Option<&str>| {
            if slot.is_none() {
                *slot = v.map(str::to_string);
            }
        };
        fill(
            &mut call.stable,
            s(update, "toolName")
                .or_else(|| s(meta, "toolName"))
                .or_else(|| s(raw, "toolName")),
        );
        fill(
            &mut call.title,
            s(update, "title").filter(|t| is_identifier(t)),
        );
        fill(&mut call.kind, s(update, "kind"));
        fill(&mut call.name_arg, s(raw, "name"));
        fill(&mut call.skill_arg, s(raw, "skill"));
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
            let norm = name
                .rsplit("__")
                .next()
                .unwrap_or(name)
                .to_ascii_lowercase();
            let skill = match norm.replace('-', "_").as_str() {
                "load_skill" => call.name_arg.as_deref().filter(|n| !n.contains('/')),
                "skill" => call.skill_arg.as_deref(),
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

/// Count `name`, folding into [`OTHER`] when over the byte or distinct-name cap.
fn bump(map: &mut BTreeMap<String, u32>, name: &str) {
    let key = if name.len() > MAX_TURN_METRIC_NAME_BYTES
        || (!map.contains_key(name)
            && map.len() - usize::from(map.contains_key(OTHER)) >= MAX_TURN_METRIC_NAMES - 1)
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

    fn take(c: &mut ToolCallCounter) -> (BTreeMap<String, u32>, BTreeMap<String, u32>) {
        let (t, s) = c.take();
        (t.unwrap_or_default(), s.unwrap_or_default())
    }

    #[test]
    fn dedupes_by_id_and_prefers_stable_name_over_title() {
        let mut c = ToolCallCounter::default();
        c.observe(&json!({"toolCallId":"1","title":"Read src/foo.rs","kind":"read"}));
        c.observe(&json!({"toolCallId":"1","_meta":{"claudeCode":{"toolName":"Read"}}}));
        c.observe(&json!({"toolCallId":"1","status":"completed"}));
        c.observe(&json!({"toolCallId":"2","title":"Run ls -la","kind":"execute"}));
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
        c.observe(&json!({"toolCallId":"1","title":"shell","kind":"other"}));
        assert_eq!(take(&mut c).0, BTreeMap::from([("shell".into(), 1)]));
    }

    #[test]
    fn skills_detected_and_supporting_file_reads_excluded() {
        let mut c = ToolCallCounter::default();
        c.observe(&json!({"toolCallId":"1","title":"load_skill","rawInput":{"name":"git"}}));
        c.observe(
            &json!({"toolCallId":"2","title":"load_skill","rawInput":{"name":"git/refs/a.md"}}),
        );
        c.observe(&json!({"toolCallId":"3","_meta":{"claudeCode":{"toolName":"Skill"}}}));
        c.observe(&json!({"toolCallId":"3","rawInput":{"skill":"review"}}));
        c.observe(
            &json!({"toolCallId":"4","title":"buzz-dev-mcp__load_skill","rawInput":{"name":"git"}}),
        );
        let (tools, skills) = take(&mut c);
        assert_eq!(tools["load_skill"], 2);
        assert_eq!(tools["Skill"], 1);
        assert_eq!(
            skills,
            BTreeMap::from([("git".into(), 2), ("review".into(), 1)])
        );
    }

    #[test]
    fn caps_distinct_names_and_name_length() {
        let mut c = ToolCallCounter::default();
        for i in 0..100 {
            c.observe(&json!({"toolCallId":i.to_string(),"toolName":format!("t{i}")}));
        }
        c.observe(&json!({"toolCallId":"long","toolName":"x".repeat(200)}));
        let (tools, _) = take(&mut c);
        assert_eq!(tools.len(), MAX_TURN_METRIC_NAMES);
        assert_eq!(tools[OTHER], 100 - (MAX_TURN_METRIC_NAMES as u32 - 1) + 1);
    }

    #[test]
    fn empty_turn_is_none_and_begin_turn_resets() {
        let mut c = ToolCallCounter::default();
        assert_eq!(c.take(), (None, None));
        c.observe(&json!({"toolCallId":"1","toolName":"Read"}));
        c.begin_turn();
        assert_eq!(c.take(), (None, None));
    }
}
