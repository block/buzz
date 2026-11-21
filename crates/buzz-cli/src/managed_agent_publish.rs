//! Headless kind:30177 managed-agent directory publish (#2663 gaps #3–#4).
//!
//! Content schema mirrors desktop `ManagedAgentEventContent` (opt-in public
//! fields only). Sign as the **owner**; `d` tag = agent pubkey.

use nostr::{EventBuilder, Kind, PublicKey, Tag};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::CliError;
use crate::validate::validate_hex64;
use buzz_core::kind::KIND_MANAGED_AGENT;

/// Wire form of kind:30177 content (required: name, parallelism, respond_to).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManagedAgentPublishContent {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persona_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persona_source_version: Option<String>,
    pub parallelism: u32,
    /// kebab-case: owner-only | allowlist | anyone
    pub respond_to: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub respond_to_allowlist: Vec<String>,
}

/// Validate and normalize pasteable JSON for kind:30177 content.
pub fn validate_managed_agent_content(raw: &str) -> Result<ManagedAgentPublishContent, CliError> {
    let v: Value = serde_json::from_str(raw)
        .map_err(|e| CliError::Usage(format!("invalid 30177 JSON: {e}")))?;
    let obj = v
        .as_object()
        .ok_or_else(|| CliError::Usage("30177 content must be a JSON object".into()))?;

    // Reject known-secret / local-only keys so operators don't leak keys.
    const FORBIDDEN: &[&str] = &[
        "private_key_nsec",
        "private_key",
        "auth_tag",
        "env_vars",
        "backend",
        "nsec",
    ];
    for k in FORBIDDEN {
        if obj.contains_key(*k) {
            return Err(CliError::Usage(format!(
                "30177 content must not include '{k}' (secrets/local fields stay off-wire)"
            )));
        }
    }

    let name = obj
        .get("name")
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            CliError::Usage(
                "30177 content requires non-empty string field `name` (not display_name)".into(),
            )
        })?
        .to_string();

    let parallelism = obj
        .get("parallelism")
        .and_then(|x| x.as_u64())
        .ok_or_else(|| {
            CliError::Usage("30177 content requires unsigned integer field `parallelism`".into())
        })?;
    if parallelism == 0 || parallelism > 1024 {
        return Err(CliError::Usage(
            "30177 `parallelism` must be between 1 and 1024".into(),
        ));
    }

    let respond_to = obj
        .get("respond_to")
        .and_then(|x| x.as_str())
        .ok_or_else(|| {
            CliError::Usage(
                "30177 content requires `respond_to` (owner-only|allowlist|anyone)".into(),
            )
        })?;
    let respond_to = match respond_to {
        "owner-only" | "allowlist" | "anyone" => respond_to.to_string(),
        other => {
            return Err(CliError::Usage(format!(
                "invalid respond_to '{other}' (expected owner-only|allowlist|anyone)"
            )))
        }
    };

    let mut allowlist = Vec::new();
    if let Some(arr) = obj.get("respond_to_allowlist") {
        let Some(list) = arr.as_array() else {
            return Err(CliError::Usage(
                "`respond_to_allowlist` must be an array of hex pubkeys".into(),
            ));
        };
        for entry in list {
            let s = entry
                .as_str()
                .ok_or_else(|| CliError::Usage("allowlist entries must be strings".into()))?;
            validate_hex64(s)?;
            allowlist.push(s.to_ascii_lowercase());
        }
    }
    if respond_to == "allowlist" && allowlist.is_empty() {
        return Err(CliError::Usage(
            "respond_to=allowlist requires non-empty respond_to_allowlist".into(),
        ));
    }

    let opt_str = |key: &str| -> Result<Option<String>, CliError> {
        match obj.get(key) {
            None => Ok(None),
            Some(Value::Null) => Ok(None),
            Some(Value::String(s)) => Ok(Some(s.clone())),
            Some(_) => Err(CliError::Usage(format!("`{key}` must be a string"))),
        }
    };

    Ok(ManagedAgentPublishContent {
        name,
        persona_id: opt_str("persona_id")?,
        system_prompt: opt_str("system_prompt")?,
        model: opt_str("model")?,
        provider: opt_str("provider")?,
        persona_source_version: opt_str("persona_source_version")?,
        parallelism: parallelism as u32,
        respond_to,
        respond_to_allowlist: allowlist,
    })
}

pub fn build_managed_agent_event(
    agent_pubkey_hex: &str,
    content: &ManagedAgentPublishContent,
) -> Result<EventBuilder, CliError> {
    validate_hex64(agent_pubkey_hex)?;
    // Ensure pubkey parses.
    PublicKey::parse(agent_pubkey_hex)
        .map_err(|e| CliError::Usage(format!("invalid agent pubkey: {e}")))?;
    let body = serde_json::to_string(content)
        .map_err(|e| CliError::Other(format!("serialize 30177: {e}")))?;
    let d =
        Tag::parse(["d", agent_pubkey_hex]).map_err(|e| CliError::Other(format!("d-tag: {e}")))?;
    Ok(EventBuilder::new(Kind::Custom(KIND_MANAGED_AGENT as u16), body).tags([d]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_ok() -> &'static str {
        r#"{"name":"OpenClaw","parallelism":1,"respond_to":"owner-only"}"#
    }

    #[test]
    fn accepts_minimal_schema() {
        let c = validate_managed_agent_content(minimal_ok()).unwrap();
        assert_eq!(c.name, "OpenClaw");
        assert_eq!(c.parallelism, 1);
        assert_eq!(c.respond_to, "owner-only");
    }

    #[test]
    fn rejects_display_name_only() {
        let err = validate_managed_agent_content(
            r#"{"display_name":"x","parallelism":1,"respond_to":"anyone"}"#,
        )
        .unwrap_err();
        assert!(matches!(err, CliError::Usage(_)));
    }

    #[test]
    fn rejects_bad_respond_to() {
        let err = validate_managed_agent_content(
            r#"{"name":"x","parallelism":1,"respond_to":"everyone"}"#,
        )
        .unwrap_err();
        assert!(matches!(err, CliError::Usage(_)));
    }

    #[test]
    fn rejects_secrets() {
        let err = validate_managed_agent_content(
            r#"{"name":"x","parallelism":1,"respond_to":"anyone","private_key_nsec":"nsec1x"}"#,
        )
        .unwrap_err();
        assert!(matches!(err, CliError::Usage(_)));
    }

    #[test]
    fn allowlist_requires_entries() {
        let err = validate_managed_agent_content(
            r#"{"name":"x","parallelism":1,"respond_to":"allowlist"}"#,
        )
        .unwrap_err();
        assert!(matches!(err, CliError::Usage(_)));
    }

    #[test]
    fn builds_event_with_d_tag() {
        let agent = "a".repeat(64);
        let c = validate_managed_agent_content(minimal_ok()).unwrap();
        let b = build_managed_agent_event(&agent, &c).unwrap();
        // Builder existence is enough; full sign needs keys in integration.
        let _ = b;
    }
}
