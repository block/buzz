//! Build a Buzz Term `term-session` fenced card for chat handoff.
//!
//! Agents call this instead of inventing fence JSON by hand. The full handoff
//! prompt stays in JSON `prompt` (UI hides it). Chat reply should be a short
//! ack plus this tool’s returned fence only.
//!
//! When `originChannelId` / `originThreadId` / `mentionToUse` are provided,
//! ensures a **Return path (Buzz)** section is present in `prompt` so Term
//! hands back to the launch (origin) thread — not a summarized/source thread.

use rmcp::ErrorData;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const HULA: &str = "term-session";
const VERSION: u32 = 1;
const RETURN_PATH_HEADING: &str = "## Return path (Buzz)";

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TermSessionCardParams {
    /// Short title for the card / tab.
    pub name: String,
    /// Interactive harness: `claude` or `codex`.
    pub tool: String,
    /// Full handoff prompt text (never dump as plain markdown in chat).
    pub prompt: String,
    /// Optional working directory (absolute or `~` path).
    #[serde(default)]
    pub cwd: Option<String>,
    /// Optional one-line summary for the card UI.
    #[serde(default)]
    pub summary: Option<String>,
    /// Session id (uuid). Generated when omitted or empty.
    #[serde(default)]
    pub sid: Option<String>,
    /// `true` only when the agent uses OpenClaw workspace. Boolean only —
    /// never put tokens/JWTs in the card.
    #[serde(default)]
    pub openclaw_workspace: Option<bool>,
    /// Origin channel JM launched Term from (Return path). Prefer over any
    /// summarized/source thread.
    #[serde(default)]
    pub origin_channel_id: Option<String>,
    /// Origin thread JM launched Term from (Return path).
    #[serde(default)]
    pub origin_thread_id: Option<String>,
    /// Mention for hand-back drafts (e.g. `@Fable`). Do not default to the
    /// Term agent mentioning itself — use the Buzz agent that should continue.
    #[serde(default)]
    pub mention_to_use: Option<String>,
    /// When summarizing a different thread than origin, label it in Return path.
    #[serde(default)]
    pub summarized_channel_id: Option<String>,
    /// When summarizing a different thread than origin, label it in Return path.
    #[serde(default)]
    pub summarized_thread_id: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TermSessionCardPayload {
    hula: &'static str,
    v: u32,
    name: String,
    tool: String,
    sid: String,
    prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    openclaw_workspace: Option<bool>,
}

fn format_mention(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return "@agent".to_string();
    }
    if trimmed.starts_with('@') {
        trimmed.to_string()
    } else {
        format!("@{trimmed}")
    }
}

/// Build the Return path markdown block (mirrors desktop `returnPath.ts`).
pub fn build_return_path_section(
    origin_channel_id: &str,
    origin_thread_id: &str,
    mention_to_use: &str,
    summarized_channel_id: Option<&str>,
    summarized_thread_id: Option<&str>,
) -> String {
    let mention = format_mention(mention_to_use);
    let mut lines = vec![
        RETURN_PATH_HEADING.to_string(),
        format!("- origin channelId: {origin_channel_id}"),
        format!("- origin threadId:  {origin_thread_id}"),
        format!("- mention to use:   {mention}"),
    ];

    let sc = summarized_channel_id.map(str::trim).filter(|s| !s.is_empty());
    let st = summarized_thread_id.map(str::trim).filter(|s| !s.is_empty());
    if let (Some(sc), Some(st)) = (sc, st) {
        if sc != origin_channel_id || st != origin_thread_id {
            lines.push(format!("- summarized/source channelId: {sc}"));
            lines.push(format!("- summarized/source threadId:  {st}"));
        }
    }

    lines.push("- On \"report back\" / \"hand back\" / \"I'm done\":".to_string());
    lines.push("  call buzz_draft_message with the origin channelId + threadId.".to_string());
    lines.push(format!(
        "  Content = \"{mention} <text JM asked for>\". Draft only. JM clicks Send."
    ));
    lines.push(
        "- Never draft to any other channel or thread unless JM gives new IDs.".to_string(),
    );
    lines.push("- Never auto-draft progress; keep status in the Term TUI.".to_string());

    lines.join("\n")
}

/// Append Return path when origin fields are present and the prompt does not
/// already contain the heading (idempotent).
pub fn ensure_return_path_in_prompt(
    prompt: &str,
    origin_channel_id: Option<&str>,
    origin_thread_id: Option<&str>,
    mention_to_use: Option<&str>,
    summarized_channel_id: Option<&str>,
    summarized_thread_id: Option<&str>,
) -> String {
    let origin_ch = origin_channel_id.map(str::trim).filter(|s| !s.is_empty());
    let origin_th = origin_thread_id.map(str::trim).filter(|s| !s.is_empty());
    let (Some(ch), Some(th)) = (origin_ch, origin_th) else {
        return prompt.to_string();
    };
    if prompt.contains(RETURN_PATH_HEADING) {
        return prompt.to_string();
    }
    let mention = mention_to_use.unwrap_or("agent");
    let section = build_return_path_section(
        ch,
        th,
        mention,
        summarized_channel_id,
        summarized_thread_id,
    );
    let trimmed = prompt.trim_end();
    if trimmed.is_empty() {
        section
    } else {
        format!("{trimmed}\n\n{section}")
    }
}

/// Validate params and return the exact markdown fence agents must paste.
pub fn run(p: TermSessionCardParams) -> Result<String, ErrorData> {
    let name = p.name.trim();
    if name.is_empty() {
        return Err(ErrorData::invalid_params("name must be non-empty", None));
    }

    let tool = p.tool.trim();
    if tool != "claude" && tool != "codex" {
        return Err(ErrorData::invalid_params(
            "tool must be \"claude\" or \"codex\"",
            None,
        ));
    }

    if p.prompt.is_empty() {
        return Err(ErrorData::invalid_params(
            "prompt must be non-empty",
            None,
        ));
    }

    let sid = match p.sid.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(existing) => existing.to_owned(),
        None => Uuid::new_v4().to_string(),
    };

    let cwd = p
        .cwd
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    let summary = p
        .summary
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    let openclaw_workspace = match p.openclaw_workspace {
        Some(true) => Some(true),
        _ => None,
    };

    let prompt = ensure_return_path_in_prompt(
        &p.prompt,
        p.origin_channel_id.as_deref(),
        p.origin_thread_id.as_deref(),
        p.mention_to_use.as_deref(),
        p.summarized_channel_id.as_deref(),
        p.summarized_thread_id.as_deref(),
    );

    let payload = TermSessionCardPayload {
        hula: HULA,
        v: VERSION,
        name: name.to_owned(),
        tool: tool.to_owned(),
        sid,
        prompt,
        cwd,
        summary,
        openclaw_workspace,
    };

    let body = serde_json::to_string(&payload).map_err(|e| {
        ErrorData::internal_error(format!("failed to serialize term-session card: {e}"), None)
    })?;

    Ok(format!("```term-session\n{body}\n```"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn base() -> TermSessionCardParams {
        TermSessionCardParams {
            name: "Thread handoff".into(),
            tool: "claude".into(),
            prompt: "Do the thing".into(),
            cwd: None,
            summary: None,
            sid: None,
            openclaw_workspace: None,
            origin_channel_id: None,
            origin_thread_id: None,
            mention_to_use: None,
            summarized_channel_id: None,
            summarized_thread_id: None,
        }
    }

    fn parse_fence(out: &str) -> Value {
        assert!(
            out.starts_with("```term-session\n"),
            "missing fence open: {out}"
        );
        assert!(out.ends_with("\n```"), "missing fence close: {out}");
        let json = out
            .strip_prefix("```term-session\n")
            .unwrap()
            .strip_suffix("\n```")
            .unwrap();
        serde_json::from_str(json).expect("card json")
    }

    #[test]
    fn builds_fence_with_generated_sid() {
        let out = run(base()).unwrap();
        let v = parse_fence(&out);
        assert_eq!(v["hula"], "term-session");
        assert_eq!(v["v"], 1);
        assert_eq!(v["name"], "Thread handoff");
        assert_eq!(v["tool"], "claude");
        assert_eq!(v["prompt"], "Do the thing");
        let sid = v["sid"].as_str().unwrap();
        assert!(Uuid::parse_str(sid).is_ok(), "sid={sid}");
        assert!(v.get("cwd").is_none());
        assert!(v.get("summary").is_none());
        assert!(v.get("openclawWorkspace").is_none());
    }

    #[test]
    fn preserves_sid_and_optionals() {
        let mut p = base();
        p.tool = "codex".into();
        p.sid = Some("11111111-1111-1111-1111-111111111111".into());
        p.cwd = Some("~/src".into());
        p.summary = Some("ship it".into());
        p.openclaw_workspace = Some(true);
        let v = parse_fence(&run(p).unwrap());
        assert_eq!(v["tool"], "codex");
        assert_eq!(v["sid"], "11111111-1111-1111-1111-111111111111");
        assert_eq!(v["cwd"], "~/src");
        assert_eq!(v["summary"], "ship it");
        assert_eq!(v["openclawWorkspace"], true);
    }

    #[test]
    fn openclaw_false_omitted() {
        let mut p = base();
        p.openclaw_workspace = Some(false);
        let v = parse_fence(&run(p).unwrap());
        assert!(v.get("openclawWorkspace").is_none());
    }

    #[test]
    fn rejects_empty_name() {
        let mut p = base();
        p.name = "   ".into();
        let err = run(p).unwrap_err();
        assert!(err.message.contains("name"), "got: {}", err.message);
    }

    #[test]
    fn rejects_empty_prompt() {
        let mut p = base();
        p.prompt = String::new();
        let err = run(p).unwrap_err();
        assert!(err.message.contains("prompt"), "got: {}", err.message);
    }

    #[test]
    fn rejects_bad_tool() {
        let mut p = base();
        p.tool = "gemini".into();
        let err = run(p).unwrap_err();
        assert!(err.message.contains("tool"), "got: {}", err.message);
    }

    #[test]
    fn empty_sid_generates() {
        let mut p = base();
        p.sid = Some("  ".into());
        let v = parse_fence(&run(p).unwrap());
        assert!(Uuid::parse_str(v["sid"].as_str().unwrap()).is_ok());
    }

    #[test]
    fn params_accept_camel_case_openclaw() {
        let p: TermSessionCardParams = serde_json::from_str(
            r#"{
              "name": "n",
              "tool": "claude",
              "prompt": "p",
              "openclawWorkspace": true
            }"#,
        )
        .unwrap();
        assert_eq!(p.openclaw_workspace, Some(true));
    }

    #[test]
    fn params_reject_unknown_fields() {
        let res: Result<TermSessionCardParams, _> = serde_json::from_str(
            r#"{"name":"n","tool":"claude","prompt":"p","token":"nope"}"#,
        );
        assert!(res.is_err());
    }

    #[test]
    fn appends_return_path_when_origin_fields_set() {
        let mut p = base();
        p.prompt = "Ship the ClaimMiner fix.".into();
        p.origin_channel_id = Some("ch-origin".into());
        p.origin_thread_id = Some("th-origin".into());
        p.mention_to_use = Some("Fable".into());
        let v = parse_fence(&run(p).unwrap());
        let prompt = v["prompt"].as_str().unwrap();
        assert!(prompt.starts_with("Ship the ClaimMiner fix."));
        assert!(prompt.contains("## Return path (Buzz)"));
        assert!(prompt.contains("origin channelId: ch-origin"));
        assert!(prompt.contains("origin threadId:  th-origin"));
        assert!(prompt.contains("mention to use:   @Fable"));
        assert!(prompt.contains("report back"));
        assert!(prompt.contains("buzz_draft_message"));
        assert!(!prompt.contains("summarized/source"));
    }

    #[test]
    fn return_path_labels_summarized_when_distinct() {
        let section = build_return_path_section(
            "ch-o",
            "th-o",
            "@ClaimMiner",
            Some("ch-o"),
            Some("th-summarized"),
        );
        assert!(section.contains("origin threadId:  th-o"));
        assert!(section.contains("summarized/source threadId:  th-summarized"));
        assert!(section.contains("mention to use:   @ClaimMiner"));
    }

    #[test]
    fn return_path_idempotent_when_heading_already_present() {
        let mut p = base();
        p.prompt = format!(
            "Work.\n\n{}",
            build_return_path_section("ch", "th", "@Bot", None, None)
        );
        p.origin_channel_id = Some("ch".into());
        p.origin_thread_id = Some("th".into());
        p.mention_to_use = Some("@Bot".into());
        let v = parse_fence(&run(p).unwrap());
        let prompt = v["prompt"].as_str().unwrap();
        assert_eq!(prompt.matches(RETURN_PATH_HEADING).count(), 1);
    }

    #[test]
    fn params_accept_return_path_camel_case() {
        let p: TermSessionCardParams = serde_json::from_str(
            r#"{
              "name": "n",
              "tool": "claude",
              "prompt": "p",
              "originChannelId": "ch",
              "originThreadId": "th",
              "mentionToUse": "@Fable",
              "summarizedChannelId": "ch2",
              "summarizedThreadId": "th2"
            }"#,
        )
        .unwrap();
        assert_eq!(p.origin_channel_id.as_deref(), Some("ch"));
        assert_eq!(p.origin_thread_id.as_deref(), Some("th"));
        assert_eq!(p.mention_to_use.as_deref(), Some("@Fable"));
        assert_eq!(p.summarized_channel_id.as_deref(), Some("ch2"));
        assert_eq!(p.summarized_thread_id.as_deref(), Some("th2"));
    }
}
