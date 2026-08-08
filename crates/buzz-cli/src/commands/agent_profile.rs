//! Agent profile (kind:10100) read and write.
//!
//! Kind:10100 is the agent-authored directory record. Buzz Desktop discovers
//! agents by querying it unfiltered, so it is how an agent that runs on its own
//! machine becomes visible, mentionable, and addressable in a workspace without
//! the Desktop owning or supervising it.
//!
//! # Why every write is read-modify-write
//!
//! Kind:10100 is a **replaceable** event: the relay keeps only the newest one
//! per author. A writer that publishes a partial profile does not merge into
//! the previous one, it *replaces* it, and every field it omitted is gone.
//!
//! That has a second, quieter consequence. The relay derives a stored
//! `channel_add_policy` column from this event in a side effect, and side
//! effect failures are logged rather than rejected — the event is still
//! accepted and still becomes the author's profile. So a profile published
//! without `channel_add_policy` replaces the visible record *and* leaves the
//! relay's stored policy at its previous value, with nothing but a relay-side
//! warning to show for it. The event log and the database disagree, silently.
//!
//! Both problems have the same fix, applied here: read the current profile,
//! layer the caller's changes onto it, and always emit a complete document.

use nostr::{EventBuilder, Kind};

use crate::client::BuzzClient;
use crate::error::CliError;

/// Policy values the relay's `channel_add_policy` side effect accepts.
pub const VALID_ADD_POLICIES: [&str; 3] = ["anyone", "owner_only", "nobody"];

/// Fields a caller may set on an agent profile.
///
/// Every field is optional: `None` means "leave whatever the current profile
/// has", which is what makes partial updates safe against the replaceable-event
/// clobber described in the module docs.
#[derive(Debug, Default, Clone)]
pub struct ProfileUpdate {
    pub display_name: Option<String>,
    pub agent_type: Option<String>,
    pub capabilities: Option<Vec<String>>,
    pub status: Option<String>,
    pub channel_add_policy: Option<String>,
}

impl ProfileUpdate {
    fn is_empty(&self) -> bool {
        self.display_name.is_none()
            && self.agent_type.is_none()
            && self.capabilities.is_none()
            && self.status.is_none()
            && self.channel_add_policy.is_none()
    }
}

/// Status values the Desktop renders. `agents_from_events` defaults an
/// absent or non-string status to `offline`, so anything outside this set
/// would round-trip to something the caller did not write.
const VALID_STATUSES: [&str; 3] = ["online", "away", "offline"];

// Note on unknown content keys: an unrecognized key in an existing profile is
// preserved untouched rather than filtered out. A newer Buzz may have added a
// field this CLI build does not know about, and dropping it would be the very
// clobber this module exists to prevent. Only *incoming* values are restricted,
// via `ProfileUpdate`'s typed fields.

/// Validate a policy string against the set the relay side effect accepts.
pub fn validate_add_policy(policy: &str) -> Result<(), CliError> {
    if VALID_ADD_POLICIES.contains(&policy) {
        return Ok(());
    }
    Err(CliError::Usage(format!(
        "--policy must be one of {} (got: {policy})",
        VALID_ADD_POLICIES.join(", ")
    )))
}

fn validate_status(status: &str) -> Result<(), CliError> {
    if VALID_STATUSES.contains(&status) {
        return Ok(());
    }
    Err(CliError::Usage(format!(
        "--status must be one of {} (got: {status})",
        VALID_STATUSES.join(", ")
    )))
}

/// Validate the caller's own values, independent of any existing profile.
///
/// Split out from [`merge_profile`] so the write path can reject bad input
/// *before* it queries the relay. Ordering the fetch first would turn a typo in
/// `--policy` into a network error (exit 2) instead of an input error (exit 1),
/// and would spend a round trip discovering something already knowable.
fn validate_update(update: &ProfileUpdate) -> Result<(), CliError> {
    if let Some(policy) = &update.channel_add_policy {
        validate_add_policy(policy)?;
    }
    if let Some(status) = &update.status {
        validate_status(status)?;
    }
    Ok(())
}

/// Merge `update` onto `current`, returning the complete profile document to
/// publish.
///
/// `current` is the author's existing profile content, or `None` when they have
/// never published one. Pure so the merge semantics — the whole point of this
/// module — are testable without a relay.
///
/// Fails when the result would carry no `channel_add_policy`: publishing such a
/// profile desyncs the relay's stored policy from the event log (see module
/// docs), so it is refused rather than written.
pub fn merge_profile(
    current: Option<&serde_json::Value>,
    update: &ProfileUpdate,
) -> Result<serde_json::Value, CliError> {
    validate_update(update)?;

    // Start from the existing document so unknown-to-this-build fields survive.
    let mut merged = match current.and_then(|v| v.as_object()) {
        Some(obj) => obj.clone(),
        None => serde_json::Map::new(),
    };

    // A stored `pubkey` is never authoritative — the Desktop overwrites it with
    // the event author on read. Carrying it forward would preserve a stale or
    // forged value in the document for no benefit.
    merged.remove("pubkey");

    if let Some(v) = &update.display_name {
        merged.insert("display_name".into(), serde_json::json!(v));
    }
    if let Some(v) = &update.agent_type {
        merged.insert("agent_type".into(), serde_json::json!(v));
    }
    if let Some(v) = &update.capabilities {
        merged.insert("capabilities".into(), serde_json::json!(v));
    }
    if let Some(v) = &update.status {
        merged.insert("status".into(), serde_json::json!(v));
    }
    if let Some(v) = &update.channel_add_policy {
        merged.insert("channel_add_policy".into(), serde_json::json!(v));
    }

    // Refuse rather than guess. Defaulting to a policy the caller did not
    // choose would silently widen or narrow who may add this agent to channels.
    let policy_ok = merged
        .get("channel_add_policy")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|p| VALID_ADD_POLICIES.contains(&p));
    if !policy_ok {
        return Err(CliError::Usage(format!(
            "channel_add_policy is required and must be one of {}. This identity has no \
             existing profile to inherit it from, so pass --policy explicitly. Publishing \
             a profile without it would replace the stored record while leaving the relay's \
             policy unchanged.",
            VALID_ADD_POLICIES.join(", ")
        )));
    }

    Ok(serde_json::Value::Object(merged))
}

/// Fetch the signing identity's current kind:10100 content, if any.
///
/// Returns `None` when the identity has never published a profile. A stored
/// profile whose content is not a JSON object is also treated as `None`: it
/// carries nothing mergeable, and preserving unparseable bytes would just
/// propagate the corruption into the next write.
pub async fn fetch_current_profile(
    client: &BuzzClient,
) -> Result<Option<serde_json::Value>, CliError> {
    let me = client.keys().public_key().to_hex();
    let filter = serde_json::json!({
        "kinds": [buzz_sdk::kind::KIND_AGENT_PROFILE],
        "authors": [me],
        "limit": 1,
    });
    let events = client.query_paginated(filter, 1).await?;
    let Some(event) = events.first() else {
        return Ok(None);
    };
    let Some(content) = event.get("content").and_then(serde_json::Value::as_str) else {
        return Ok(None);
    };
    match serde_json::from_str::<serde_json::Value>(content) {
        Ok(value) if value.is_object() => Ok(Some(value)),
        _ => Ok(None),
    }
}

/// Sign and submit a complete profile document as kind:10100.
async fn publish_profile(
    client: &BuzzClient,
    content: &serde_json::Value,
) -> Result<String, CliError> {
    let builder = EventBuilder::new(
        Kind::Custom(buzz_sdk::kind::KIND_AGENT_PROFILE as u16),
        content.to_string(),
    )
    .tags([]);
    let event = client.sign_event(builder)?;
    client.submit_event(event).await
}

/// Read-modify-write entry point shared by `agents profile set` and
/// `channels set-add-policy`, so the two can never disagree about how a
/// partial update is applied.
pub async fn apply_profile_update(
    client: &BuzzClient,
    update: &ProfileUpdate,
) -> Result<String, CliError> {
    // Validate before the fetch: bad input must fail as an input error without
    // a network round trip. `merge_profile` re-checks, which is cheap and keeps
    // it safe to call directly.
    validate_update(update)?;
    let current = fetch_current_profile(client).await?;
    let merged = merge_profile(current.as_ref(), update)?;
    publish_profile(client, &merged).await
}

/// Run `buzz agents profile get`.
pub async fn cmd_profile_get(client: &BuzzClient) -> Result<(), CliError> {
    let current = fetch_current_profile(client).await?;
    let report = serde_json::json!({
        "pubkey": client.keys().public_key().to_hex(),
        "profile": current,
    });
    println!("{report}");
    Ok(())
}

/// Run `buzz agents profile set`.
pub async fn cmd_profile_set(client: &BuzzClient, update: &ProfileUpdate) -> Result<(), CliError> {
    if update.is_empty() {
        return Err(CliError::Usage(
            "no fields to set: pass at least one of --display-name, --agent-type, \
             --capabilities, --status, --policy"
                .into(),
        ));
    }
    let resp = apply_profile_update(client, update).await?;
    println!("{}", crate::client::normalize_write_response(&resp));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(json: serde_json::Value) -> serde_json::Value {
        json
    }

    #[test]
    fn policy_only_update_preserves_every_other_field() {
        // This is the regression that motivates the module. Before the
        // read-modify-write refactor, `channels set-add-policy` published a
        // document containing only `channel_add_policy`, wiping the agent's
        // identity fields from the replaceable event.
        let current = profile(serde_json::json!({
            "display_name": "Scout",
            "agent_type": "researcher",
            "capabilities": ["search", "summarize"],
            "status": "online",
            "channel_add_policy": "owner_only",
        }));
        let update = ProfileUpdate {
            channel_add_policy: Some("anyone".into()),
            ..Default::default()
        };

        let merged = merge_profile(Some(&current), &update).unwrap();
        assert_eq!(merged["channel_add_policy"], "anyone");
        assert_eq!(merged["display_name"], "Scout");
        assert_eq!(merged["agent_type"], "researcher");
        assert_eq!(
            merged["capabilities"],
            serde_json::json!(["search", "summarize"])
        );
        assert_eq!(merged["status"], "online");
    }

    #[test]
    fn field_update_preserves_existing_policy() {
        // The mirror case: renaming the agent must not drop the policy and
        // desync the relay's stored value.
        let current = profile(serde_json::json!({
            "display_name": "Scout",
            "channel_add_policy": "nobody",
        }));
        let update = ProfileUpdate {
            display_name: Some("Scout II".into()),
            ..Default::default()
        };

        let merged = merge_profile(Some(&current), &update).unwrap();
        assert_eq!(merged["display_name"], "Scout II");
        assert_eq!(merged["channel_add_policy"], "nobody");
    }

    #[test]
    fn unknown_fields_from_a_newer_buzz_survive() {
        // A field this CLI build does not know about must not be dropped —
        // dropping it is the same clobber, one release later.
        let current = profile(serde_json::json!({
            "channel_add_policy": "anyone",
            "some_future_field": {"nested": true},
        }));
        let update = ProfileUpdate {
            display_name: Some("Scout".into()),
            ..Default::default()
        };

        let merged = merge_profile(Some(&current), &update).unwrap();
        assert_eq!(
            merged["some_future_field"],
            serde_json::json!({"nested": true})
        );
        assert_eq!(merged["display_name"], "Scout");
    }

    #[test]
    fn first_profile_requires_an_explicit_policy() {
        let update = ProfileUpdate {
            display_name: Some("Scout".into()),
            ..Default::default()
        };
        let err = merge_profile(None, &update).expect_err("expected usage error");
        assert!(matches!(err, CliError::Usage(_)));
    }

    #[test]
    fn first_profile_succeeds_with_a_policy() {
        let update = ProfileUpdate {
            display_name: Some("Scout".into()),
            channel_add_policy: Some("owner_only".into()),
            ..Default::default()
        };
        let merged = merge_profile(None, &update).unwrap();
        assert_eq!(merged["display_name"], "Scout");
        assert_eq!(merged["channel_add_policy"], "owner_only");
    }

    #[test]
    fn a_current_profile_with_an_invalid_policy_is_not_inherited() {
        // Garbage in the stored document must not be laundered into a new
        // write just because it was already there.
        let current = profile(serde_json::json!({
            "display_name": "Scout",
            "channel_add_policy": "everyone",
        }));
        let update = ProfileUpdate {
            display_name: Some("Scout II".into()),
            ..Default::default()
        };
        let err = merge_profile(Some(&current), &update).expect_err("expected usage error");
        assert!(matches!(err, CliError::Usage(_)));
    }

    #[test]
    fn rejects_invalid_policy_and_status() {
        let base = serde_json::json!({"channel_add_policy": "anyone"});

        let bad_policy = ProfileUpdate {
            channel_add_policy: Some("everyone".into()),
            ..Default::default()
        };
        assert!(matches!(
            merge_profile(Some(&base), &bad_policy),
            Err(CliError::Usage(_))
        ));

        let bad_status = ProfileUpdate {
            status: Some("busy".into()),
            ..Default::default()
        };
        assert!(matches!(
            merge_profile(Some(&base), &bad_status),
            Err(CliError::Usage(_))
        ));
    }

    #[test]
    fn stale_pubkey_in_content_is_dropped() {
        // The Desktop overwrites `pubkey` with the event author on read, so a
        // stored value is at best redundant and at worst a forged claim.
        let current = profile(serde_json::json!({
            "pubkey": "deadbeef",
            "channel_add_policy": "anyone",
        }));
        let update = ProfileUpdate {
            display_name: Some("Scout".into()),
            ..Default::default()
        };
        let merged = merge_profile(Some(&current), &update).unwrap();
        assert!(merged.get("pubkey").is_none());
    }

    #[test]
    fn bad_input_is_rejected_without_needing_a_current_profile() {
        // Regression: `apply_profile_update` used to fetch before validating, so
        // a typo in --policy surfaced as a network error (exit 2) rather than an
        // input error (exit 1). `validate_update` is what the write path calls
        // first, so it must reject on the caller's values alone.
        assert!(matches!(
            validate_update(&ProfileUpdate {
                channel_add_policy: Some("everyone".into()),
                ..Default::default()
            }),
            Err(CliError::Usage(_))
        ));
        assert!(matches!(
            validate_update(&ProfileUpdate {
                status: Some("busy".into()),
                ..Default::default()
            }),
            Err(CliError::Usage(_))
        ));
        // A valid update passes with no profile and no relay in sight.
        assert!(validate_update(&ProfileUpdate {
            display_name: Some("Scout".into()),
            channel_add_policy: Some("anyone".into()),
            ..Default::default()
        })
        .is_ok());
    }

    #[test]
    fn empty_update_is_detected() {
        assert!(ProfileUpdate::default().is_empty());
        assert!(!ProfileUpdate {
            status: Some("online".into()),
            ..Default::default()
        }
        .is_empty());
    }

    #[test]
    fn capabilities_are_replaced_not_appended() {
        // Set semantics, not merge semantics: a caller passing --capabilities
        // is stating the full list. Appending would make removal impossible.
        let current = profile(serde_json::json!({
            "capabilities": ["old"],
            "channel_add_policy": "anyone",
        }));
        let update = ProfileUpdate {
            capabilities: Some(vec!["new".into()]),
            ..Default::default()
        };
        let merged = merge_profile(Some(&current), &update).unwrap();
        assert_eq!(merged["capabilities"], serde_json::json!(["new"]));
    }
}
