//! Reconcile the harness identity's public kind:10100 agent directory profile.
//!
//! Desktop mention admission uses this replaceable event as affirmative proof
//! that a remote/headless agent is invocable. The harness owns the fields it
//! can observe directly (memberships and inbound-author policy) and preserves
//! every unknown field so future profile extensions are not erased.

use std::collections::{HashMap, HashSet};

use buzz_core::kind::KIND_AGENT_PROFILE;
use nostr::{EventBuilder, Filter, Keys, Kind};
use serde_json::{json, Map, Value};
use uuid::Uuid;

use crate::config::{ChannelAddPolicy, RespondTo};
use crate::relay::{ChannelInfo, RestClient};

#[derive(Debug, PartialEq, Eq)]
pub enum ReconcileOutcome {
    Published(String),
    Unchanged,
    SkippedNoChannelAddPolicy,
}

fn event_content(event: &Value, kind: u32) -> Result<Map<String, Value>, String> {
    let content = event
        .get("content")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("kind:{kind} event has no string content"))?;
    serde_json::from_str::<Value>(content)
        .map_err(|error| format!("kind:{kind} content is invalid JSON: {error}"))?
        .as_object()
        .cloned()
        .ok_or_else(|| format!("kind:{kind} content is not an object"))
}

fn first_event(events: &Value) -> Option<&Value> {
    events.as_array().and_then(|items| items.first())
}

fn existing_policy(
    existing: &Map<String, Value>,
    configured: Option<ChannelAddPolicy>,
) -> Result<Option<String>, String> {
    if let Some(policy) = configured {
        return Ok(Some(policy.to_string()));
    }
    let Some(value) = existing.get("channel_add_policy") else {
        return Ok(None);
    };
    let Some(policy) = value.as_str() else {
        return Err("kind:10100 channel_add_policy is not a string".into());
    };
    if !matches!(policy, "anyone" | "owner_only" | "nobody") {
        return Err(format!(
            "kind:10100 channel_add_policy has unsupported value {policy:?}"
        ));
    }
    Ok(Some(policy.to_string()))
}

fn build_content(
    mut existing: Map<String, Value>,
    kind0: Option<&Map<String, Value>>,
    channels: &HashMap<Uuid, ChannelInfo>,
    respond_to: &RespondTo,
    respond_to_allowlist: &HashSet<String>,
    owner: Option<&str>,
    channel_add_policy: &str,
) -> Value {
    if let Some(profile) = kind0 {
        for field in ["name", "display_name", "picture"] {
            if !existing.contains_key(field) {
                if let Some(value) = profile.get(field) {
                    existing.insert(field.to_string(), value.clone());
                }
            }
        }
        if !existing.contains_key("name") {
            if let Some(value) = profile.get("display_name") {
                existing.insert("name".into(), value.clone());
            }
        }
    }

    let mut memberships: Vec<(String, String)> = channels
        .iter()
        .map(|(id, info)| (id.to_string(), info.name.clone()))
        .collect();
    memberships.sort_by(|left, right| left.0.cmp(&right.0));

    let channel_ids: Vec<Value> = memberships
        .iter()
        .map(|(id, _)| Value::String(id.clone()))
        .collect();
    let channel_names: Vec<Value> = memberships
        .into_iter()
        .map(|(_, name)| Value::String(name))
        .collect();
    let (directory_respond_to, mut allowlist): (&str, Vec<String>) = match respond_to {
        RespondTo::Anyone => ("anyone", Vec::new()),
        RespondTo::Nobody => ("nobody", Vec::new()),
        RespondTo::Allowlist => ("allowlist", respond_to_allowlist.iter().cloned().collect()),
        // Desktop's relay-directory admission cannot resolve the implicit
        // runtime owner from `owner-only`. Project it to an equivalent public
        // allowlist so that owner-authored mentions remain selectable.
        RespondTo::OwnerOnly if owner.is_some() => ("allowlist", Vec::new()),
        RespondTo::OwnerOnly => ("owner-only", Vec::new()),
    };
    if matches!(respond_to, RespondTo::Allowlist | RespondTo::OwnerOnly) {
        if let Some(owner) = owner {
            allowlist.push(owner.to_string());
        }
    }
    allowlist.sort();
    allowlist.dedup();

    existing.insert("agent_type".into(), json!("agent"));
    existing.insert("channels".into(), Value::Array(channel_names));
    existing.insert("channel_ids".into(), Value::Array(channel_ids));
    existing.entry("capabilities").or_insert_with(|| json!([]));
    existing.insert("respond_to".into(), json!(directory_respond_to));
    existing.insert("respond_to_allowlist".into(), json!(allowlist));
    existing.insert("channel_add_policy".into(), json!(channel_add_policy));
    Value::Object(existing)
}

/// Publish the current complete directory profile when it differs from the
/// relay head. A missing first-time channel-add policy is a safe skip: the
/// harness must not guess and accidentally widen an operator's relay policy.
pub async fn reconcile_agent_profile(
    rest: &RestClient,
    keys: &Keys,
    channels: &HashMap<Uuid, ChannelInfo>,
    respond_to: &RespondTo,
    respond_to_allowlist: &HashSet<String>,
    owner: Option<&str>,
    configured_channel_add_policy: Option<ChannelAddPolicy>,
) -> Result<ReconcileOutcome, String> {
    let author = keys.public_key();
    let profile_filter = Filter::new()
        .kind(Kind::Custom(KIND_AGENT_PROFILE as u16))
        .author(author)
        .limit(1);
    let profile_events = rest
        .query(&[profile_filter])
        .await
        .map_err(|error| format!("kind:10100 query failed: {error}"))?;
    let existing_event = first_event(&profile_events);
    let existing = existing_event
        .map(|event| event_content(event, KIND_AGENT_PROFILE))
        .transpose()?
        .unwrap_or_default();

    let Some(channel_add_policy) = existing_policy(&existing, configured_channel_add_policy)?
    else {
        return Ok(ReconcileOutcome::SkippedNoChannelAddPolicy);
    };

    let kind0_filter = Filter::new().kind(Kind::Metadata).author(author).limit(1);
    let kind0_events = rest
        .query(&[kind0_filter])
        .await
        .map_err(|error| format!("kind:0 query failed: {error}"))?;
    let kind0 = first_event(&kind0_events)
        .map(|event| event_content(event, 0))
        .transpose()?;

    let content = build_content(
        existing,
        kind0.as_ref(),
        channels,
        respond_to,
        respond_to_allowlist,
        owner,
        &channel_add_policy,
    );
    if existing_event
        .and_then(|event| event.get("content"))
        .and_then(Value::as_str)
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .as_ref()
        == Some(&content)
    {
        return Ok(ReconcileOutcome::Unchanged);
    }

    let serialized = serde_json::to_string(&content)
        .map_err(|error| format!("kind:10100 serialize failed: {error}"))?;
    let event = EventBuilder::new(Kind::Custom(KIND_AGENT_PROFILE as u16), serialized)
        .tags([])
        .sign_with_keys(keys)
        .map_err(|error| format!("kind:10100 sign failed: {error}"))?;
    let event_id = event.id.to_hex();
    rest.submit_event(&event)
        .await
        .map_err(|error| format!("kind:10100 publish failed: {error}"))?;
    Ok(ReconcileOutcome::Published(event_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel(name: &str) -> ChannelInfo {
        ChannelInfo {
            name: name.into(),
            channel_type: "stream".into(),
            description: None,
        }
    }

    #[test]
    fn build_content_sorts_memberships_and_preserves_unknown_fields() {
        let first = Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap();
        let second = Uuid::parse_str("22222222-2222-2222-2222-222222222222").unwrap();
        let channels = HashMap::from([(second, channel("second")), (first, channel("first"))]);
        let existing = Map::from_iter([("future_field".into(), json!({"kept": true}))]);
        let kind0 = Map::from_iter([("display_name".into(), json!("Scout"))]);
        let allowlist = HashSet::from(["b".repeat(64), "a".repeat(64)]);

        let content = build_content(
            existing,
            Some(&kind0),
            &channels,
            &RespondTo::Allowlist,
            &allowlist,
            Some(&"c".repeat(64)),
            "owner_only",
        );

        assert_eq!(content["name"], json!("Scout"));
        assert_eq!(content["display_name"], json!("Scout"));
        assert_eq!(content["future_field"], json!({"kept": true}));
        assert_eq!(content["channels"], json!(["first", "second"]));
        assert_eq!(
            content["channel_ids"],
            json!([first.to_string(), second.to_string()])
        );
        assert_eq!(
            content["respond_to_allowlist"],
            json!(["a".repeat(64), "b".repeat(64), "c".repeat(64)])
        );
    }

    #[test]
    fn owner_only_projects_to_owner_allowlist_for_directory_admission() {
        let owner = "d".repeat(64);
        let content = build_content(
            Map::new(),
            None,
            &HashMap::new(),
            &RespondTo::OwnerOnly,
            &HashSet::new(),
            Some(&owner),
            "anyone",
        );
        assert_eq!(content["respond_to"], json!("allowlist"));
        assert_eq!(content["respond_to_allowlist"], json!([owner]));
    }

    #[test]
    fn configured_policy_wins_and_missing_policy_skips() {
        let existing = Map::from_iter([("channel_add_policy".into(), json!("nobody"))]);
        assert_eq!(
            existing_policy(&existing, Some(ChannelAddPolicy::OwnerOnly)).unwrap(),
            Some("owner_only".into())
        );
        assert_eq!(existing_policy(&Map::new(), None).unwrap(), None);
    }

    #[test]
    fn invalid_existing_policy_fails_closed() {
        let existing = Map::from_iter([("channel_add_policy".into(), json!("open"))]);
        assert!(existing_policy(&existing, None).is_err());
    }
}
