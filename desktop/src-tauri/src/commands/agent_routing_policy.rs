//! Tauri commands for per-agent model routing policies.
//!
//! `buzz-acp` reads its routing policy from the file named by
//! `BUZZ_ROUTING_POLICY` (see `crates/buzz-acp/src/routing.rs`). These commands
//! own the other half of that contract: they write the file and report where it
//! lives, so the agent dialog can point the env var at it.
//!
//! Setting the env var is deliberately NOT done here. The edit dialog holds the
//! whole `env_vars` map in local state and replaces it wholesale on submit, so a
//! backend-side patch would be silently overwritten by the next save. The
//! frontend merges the returned path into that map instead.
//!
//! The types below mirror the serde shape of `buzz_acp::routing::Policy`. They
//! are duplicated rather than imported because `buzz-acp` is a sidecar the
//! desktop talks to across a process boundary, not a library it links.
//! `policy_shape_matches_the_harness_contract` pins the emitted JSON so a rename
//! on either side fails a test instead of silently disabling routing.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::managed_agents::managed_agents_base_dir;

/// How a rule matches the prompt text. Mirrors `buzz_acp::routing::MatchKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MatchKind {
    /// Any needle appears in the prompt.
    #[default]
    Contains,
    /// Every needle appears in the prompt.
    ContainsAll,
}

/// One deterministic routing rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutingRule {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default)]
    pub match_kind: MatchKind,
    #[serde(default)]
    pub any: Vec<String>,
    pub model: String,
}

/// Optional local classifier, consulted only when no rule matched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutingClassifier {
    pub url: String,
    pub model: String,
    #[serde(default)]
    pub labels: Vec<RoutingLabelTarget>,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
}

fn default_timeout_ms() -> u64 {
    20_000
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutingLabelTarget {
    pub label: String,
    pub model: String,
}

/// A routing policy, in the exact shape `buzz-acp` deserializes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RoutingPolicy {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub rules: Vec<RoutingRule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classifier: Option<RoutingClassifier>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,
}

/// Where an agent's policy lives, and what is currently in it.
///
/// `path` is always populated — the UI needs it to set `BUZZ_ROUTING_POLICY`
/// even on the save that creates the file. `policy` is `None` when nothing has
/// been written yet, or when the file on disk is unreadable/unparseable, which
/// is the same thing the harness would conclude.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRoutingPolicyFile {
    pub path: String,
    pub policy: Option<RoutingPolicy>,
}

/// Read the routing policy for `pubkey`, if one has been written.
#[tauri::command]
pub fn get_agent_routing_policy(
    pubkey: String,
    app: AppHandle,
) -> Result<AgentRoutingPolicyFile, String> {
    let path = routing_policy_path(&app, &pubkey)?;
    let policy = std::fs::read_to_string(&path)
        .ok()
        .and_then(|raw| serde_json::from_str::<RoutingPolicy>(&raw).ok());
    Ok(AgentRoutingPolicyFile {
        path: path.to_string_lossy().into_owned(),
        policy,
    })
}

/// Write (or, with `policy: None`, delete) the routing policy for `pubkey`.
///
/// Returns the path in both cases so the caller can set or clear
/// `BUZZ_ROUTING_POLICY` without recomputing it.
#[tauri::command]
pub fn set_agent_routing_policy(
    pubkey: String,
    policy: Option<RoutingPolicy>,
    app: AppHandle,
) -> Result<AgentRoutingPolicyFile, String> {
    let path = routing_policy_path(&app, &pubkey)?;

    let Some(policy) = policy else {
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("failed to delete routing policy: {error}")),
        }
        return Ok(AgentRoutingPolicyFile {
            path: path.to_string_lossy().into_owned(),
            policy: None,
        });
    };

    let policy = normalize_policy(policy)?;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create routing policy dir: {error}"))?;
    }
    let json = serde_json::to_string_pretty(&policy)
        .map_err(|error| format!("failed to serialize routing policy: {error}"))?;
    std::fs::write(&path, json)
        .map_err(|error| format!("failed to write routing policy: {error}"))?;

    Ok(AgentRoutingPolicyFile {
        path: path.to_string_lossy().into_owned(),
        policy: Some(policy),
    })
}

fn routing_policy_path(app: &AppHandle, pubkey: &str) -> Result<PathBuf, String> {
    Ok(managed_agents_base_dir(app)?
        .join("routing")
        .join(format!("{}.json", validated_pubkey(pubkey)?)))
}

/// The pubkey becomes a filename, so it must not be able to escape the routing
/// directory. Agent pubkeys are hex (or npub), both strictly alphanumeric —
/// rejecting everything else closes the traversal hole at the boundary rather
/// than trusting the caller.
fn validated_pubkey(pubkey: &str) -> Result<&str, String> {
    if pubkey.is_empty() || pubkey.len() > 128 {
        return Err("routing policy: agent pubkey has an invalid length".to_string());
    }
    if !pubkey.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err("routing policy: agent pubkey must be alphanumeric".to_string());
    }
    Ok(pubkey)
}

/// Trim user input and reject rules the harness would silently ignore.
///
/// A rule with no needles never matches (`routing.rs` makes that explicit so
/// "always route here" cannot be created by omission), and a rule with no model
/// has nothing to route to. Both are user mistakes worth naming at save time
/// rather than discovering as a policy that quietly does nothing.
fn normalize_policy(mut policy: RoutingPolicy) -> Result<RoutingPolicy, String> {
    for (index, rule) in policy.rules.iter_mut().enumerate() {
        rule.name = rule
            .name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_string);
        rule.model = rule.model.trim().to_string();
        rule.any = rule
            .any
            .iter()
            .map(|needle| needle.trim().to_string())
            .filter(|needle| !needle.is_empty())
            .collect();

        let label = rule
            .name
            .clone()
            .unwrap_or_else(|| format!("rule {}", index + 1));
        if rule.model.is_empty() {
            return Err(format!("{label} needs a model to route to."));
        }
        if rule.any.is_empty() {
            return Err(format!(
                "{label} needs at least one phrase to match. To route everything, set a default model instead."
            ));
        }
    }

    policy.default_model = policy
        .default_model
        .as_deref()
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(str::to_string);

    if let Some(classifier) = policy.classifier.as_mut() {
        classifier.url = classifier.url.trim().to_string();
        classifier.model = classifier.model.trim().to_string();
        classifier
            .labels
            .retain(|target| !target.label.trim().is_empty() && !target.model.trim().is_empty());
        if classifier.url.is_empty() || classifier.model.is_empty() {
            return Err("The classifier needs both a URL and a model.".to_string());
        }
    }

    Ok(policy)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(model: &str, any: &[&str]) -> RoutingRule {
        RoutingRule {
            name: None,
            match_kind: MatchKind::Contains,
            any: any.iter().map(|s| (*s).to_string()).collect(),
            model: model.to_string(),
        }
    }

    /// Pins the on-disk contract with `crates/buzz-acp/src/routing.rs`. If a
    /// field is renamed on either side, routing silently stops working — the
    /// harness's `from_env` swallows a parse failure by design. This test is
    /// the tripwire; its counterpart is
    /// `routing::tests::a_desktop_written_policy_parses`.
    #[test]
    fn policy_shape_matches_the_harness_contract() {
        let policy = RoutingPolicy {
            enabled: true,
            rules: vec![RoutingRule {
                name: Some("db".to_string()),
                match_kind: MatchKind::ContainsAll,
                any: vec!["migration".to_string()],
                model: "codex-model".to_string(),
            }],
            classifier: Some(RoutingClassifier {
                url: "http://localhost:11434".to_string(),
                model: "gemma3:27b".to_string(),
                labels: vec![RoutingLabelTarget {
                    label: "database".to_string(),
                    model: "db-model".to_string(),
                }],
                timeout_ms: 20_000,
            }),
            default_model: Some("fallback".to_string()),
        };

        let json: serde_json::Value = serde_json::to_value(&policy).expect("serialize");
        assert_eq!(
            json,
            serde_json::json!({
                "enabled": true,
                "rules": [{
                    "name": "db",
                    "match_kind": "contains_all",
                    "any": ["migration"],
                    "model": "codex-model"
                }],
                "classifier": {
                    "url": "http://localhost:11434",
                    "model": "gemma3:27b",
                    "labels": [{ "label": "database", "model": "db-model" }],
                    "timeout_ms": 20000
                },
                "default_model": "fallback"
            })
        );
    }

    #[test]
    fn a_disabled_empty_policy_serializes_without_null_noise() {
        let json = serde_json::to_value(RoutingPolicy::default()).expect("serialize");
        assert_eq!(json, serde_json::json!({ "enabled": false, "rules": [] }));
    }

    #[test]
    fn normalize_trims_and_drops_blank_needles() {
        let policy = normalize_policy(RoutingPolicy {
            enabled: true,
            rules: vec![RoutingRule {
                name: Some("  db  ".to_string()),
                any: vec!["  migration ".to_string(), "   ".to_string()],
                model: " codex ".to_string(),
                ..rule("x", &["y"])
            }],
            default_model: Some("   ".to_string()),
            ..Default::default()
        })
        .expect("normalize");

        assert_eq!(policy.rules[0].name.as_deref(), Some("db"));
        assert_eq!(policy.rules[0].any, vec!["migration".to_string()]);
        assert_eq!(policy.rules[0].model, "codex");
        assert_eq!(
            policy.default_model, None,
            "a whitespace-only default model is no default model"
        );
    }

    #[test]
    fn a_rule_with_no_needles_is_rejected_by_name() {
        let err = normalize_policy(RoutingPolicy {
            enabled: true,
            rules: vec![RoutingRule {
                name: Some("catch-all".to_string()),
                ..rule("m", &[])
            }],
            ..Default::default()
        })
        .unwrap_err();
        assert!(
            err.contains("catch-all"),
            "error should name the rule: {err}"
        );
        assert!(
            err.contains("default model"),
            "error should point at the fix: {err}"
        );
    }

    #[test]
    fn an_unnamed_bad_rule_is_reported_by_its_position() {
        let err = normalize_policy(RoutingPolicy {
            enabled: true,
            rules: vec![rule("ok-model", &["x"]), rule("", &["y"])],
            ..Default::default()
        })
        .unwrap_err();
        assert!(
            err.starts_with("rule 2"),
            "expected a 1-based position: {err}"
        );
    }

    #[test]
    fn a_classifier_missing_its_url_is_rejected() {
        let err = normalize_policy(RoutingPolicy {
            enabled: true,
            classifier: Some(RoutingClassifier {
                url: "  ".to_string(),
                model: "gemma3:27b".to_string(),
                labels: vec![],
                timeout_ms: 20_000,
            }),
            ..Default::default()
        })
        .unwrap_err();
        assert!(err.contains("classifier"), "{err}");
    }

    #[test]
    fn a_pubkey_that_could_escape_the_routing_dir_is_rejected() {
        for bad in ["../../etc/passwd", "a/b", "a\\b", "", "a.b"] {
            assert!(
                validated_pubkey(bad).is_err(),
                "{bad:?} must not be accepted as a filename"
            );
        }
        assert!(validated_pubkey("deadbeef00").is_ok());
    }
}
