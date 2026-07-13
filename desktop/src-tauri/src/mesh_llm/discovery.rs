use std::collections::{BTreeMap, BTreeSet};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};

use super::{dedupe_models, MeshAvailability, MeshModelOption, MeshServeTarget, MESH_STATUS_KIND};

/// Status notes are refreshed every 15 seconds. Ignore notes older than two
/// minutes so crashed/offline devices stop contributing compute or admission
/// identities without requiring a relay-side cleanup job.
const STATUS_FRESHNESS_SECS: u64 = 120;

fn newest_event_timestamp(events: &[nostr::Event]) -> u64 {
    events
        .iter()
        .map(|event| event.created_at.as_secs())
        .max()
        .unwrap_or_default()
}

fn status_is_fresh(event: &nostr::Event, now: u64) -> bool {
    event
        .created_at
        .as_secs()
        .saturating_add(STATUS_FRESHNESS_SECS)
        >= now
}

fn dedupe_targets(targets: Vec<MeshServeTarget>) -> Vec<MeshServeTarget> {
    let mut by_endpoint = BTreeMap::<String, MeshServeTarget>::new();
    for target in targets {
        by_endpoint
            .entry(target.endpoint_addr.clone())
            .or_insert(target);
    }
    by_endpoint.into_values().collect()
}

/// Resolve the mesh admission roster from relay status and membership events.
///
/// Only status notes signed by a currently listed NIP-43 direct member
/// contribute an owner id. This removes stale notes from former members and
/// ignores notes from nonmembers. If the relay has no membership snapshot, the
/// roster is empty and MeshLLM admission therefore remains self-only.
pub fn owner_ids_from_events(events: &[nostr::Event]) -> Vec<String> {
    let Some(members) = latest_membership_list(events) else {
        return Vec::new();
    };
    let now = newest_event_timestamp(events);
    let mut ids: Vec<String> = events
        .iter()
        .filter(|event| {
            event.kind.as_u16() as u64 == MESH_STATUS_KIND && status_is_fresh(event, now)
        })
        .filter(|event| {
            reporter_pubkey_from_status_event(event)
                .is_some_and(|reporter| members.contains(&reporter.to_ascii_lowercase()))
        })
        .filter_map(owner_id_from_status_event)
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

fn latest_membership_list(events: &[nostr::Event]) -> Option<BTreeSet<String>> {
    events
        .iter()
        .filter(|event| event.kind.as_u16() == 13_534)
        .max_by_key(|event| event.created_at)
        .map(|event| {
            event
                .tags
                .iter()
                .filter_map(|tag| {
                    let slice = tag.as_slice();
                    let name = slice.first()?;
                    if name != "member" && name != "p" {
                        return None;
                    }
                    slice
                        .get(1)
                        .map(|pubkey| pubkey.trim().to_ascii_lowercase())
                })
                .filter(|pubkey| !pubkey.is_empty())
                .collect()
        })
}

fn owner_id_from_status_event(event: &nostr::Event) -> Option<String> {
    let content = serde_json::from_str::<serde_json::Value>(&event.content).ok()?;
    let owner_id = content
        .get("ownerId")
        .or_else(|| content.get("owner_id"))?
        .as_str()?
        .trim();
    let verifying_key_bytes: [u8; 32] =
        hex::decode(content.get("ownerVerifyingKey")?.as_str()?.trim())
            .ok()?
            .try_into()
            .ok()?;
    let derived_owner = hex::encode(Sha256::digest(verifying_key_bytes));
    if owner_id != derived_owner {
        return None;
    }
    let signature_bytes = hex::decode(content.get("ownerBindingSig")?.as_str()?.trim()).ok()?;
    let signature = Signature::from_slice(&signature_bytes).ok()?;
    let verifying_key = VerifyingKey::from_bytes(&verifying_key_bytes).ok()?;
    verifying_key
        .verify(
            &super::identity::member_binding_bytes(&event.pubkey.to_hex()),
            &signature,
        )
        .ok()?;
    Some(owner_id.to_string())
}

pub fn availability_from_events(events: Vec<nostr::Event>) -> MeshAvailability {
    if events.is_empty() {
        return MeshAvailability::unavailable("Buzz shared compute status is not published yet");
    }
    let Some(members) = latest_membership_list(&events) else {
        return MeshAvailability::unavailable(
            "Buzz shared compute is waiting for the current member roster",
        );
    };

    // Status is replaceable per member pubkey, so a query returns multiple
    // events. Aggregate only current members: a removed member's last status
    // must not remain selectable after their admission is revoked.
    let mut all_targets = Vec::<MeshServeTarget>::new();
    let mut all_models = Vec::<MeshModelOption>::new();
    let mut saw_valid_status = false;

    let now = newest_event_timestamp(&events);
    for event in events {
        if event.kind.as_u16() as u64 != MESH_STATUS_KIND
            || !status_is_fresh(&event, now)
            || !members.contains(&event.pubkey.to_hex().to_ascii_lowercase())
        {
            continue;
        }
        let Ok(content) = serde_json::from_str::<serde_json::Value>(&event.content) else {
            continue;
        };
        saw_valid_status = true;
        let reporter_pubkey = reporter_pubkey_from_status_event(&event);
        let mut serve_targets = content
            .get("serveTargets")
            .or_else(|| content.get("serve_targets"))
            .cloned()
            .and_then(|value| serde_json::from_value::<Vec<MeshServeTarget>>(value).ok())
            .unwrap_or_default()
            .into_iter()
            .map(|mut target| {
                if target.reporter_pubkey.is_none() {
                    target.reporter_pubkey = reporter_pubkey.clone();
                }
                if target.endpoint_id.is_none() {
                    target.endpoint_id = endpoint_id_from_invite_token(&target.endpoint_addr);
                }
                if target.device_id.is_none() {
                    target.device_id = target.endpoint_id.clone();
                }
                if target.device_name.is_none() {
                    target.device_name = target
                        .node_name
                        .clone()
                        .or_else(|| target.endpoint_id.as_deref().map(short_endpoint_label));
                }
                target
            })
            .collect::<Vec<_>>();

        let mut models = content
            .get("models")
            .cloned()
            .and_then(|value| serde_json::from_value::<Vec<MeshModelOption>>(value).ok())
            .unwrap_or_else(|| {
                dedupe_models(
                    serve_targets
                        .iter()
                        .map(|target| MeshModelOption {
                            id: target.model_id.clone(),
                            name: target.model_name.clone(),
                        })
                        .collect(),
                )
            });
        all_targets.append(&mut serve_targets);
        all_models.append(&mut models);
    }

    if !saw_valid_status {
        return MeshAvailability::unavailable("Buzz shared compute status is malformed");
    }

    let serve_targets = dedupe_targets(all_targets);
    let models = dedupe_models(all_models);
    let available = !serve_targets.is_empty();
    MeshAvailability {
        capable: true,
        admitted: true,
        available,
        reason: if available {
            None
        } else {
            Some("no Buzz shared compute serving members are available".to_string())
        },
        models,
        serve_targets,
    }
}

pub fn mesh_status_filter() -> serde_json::Value {
    serde_json::json!({
        "kinds": [MESH_STATUS_KIND],
        "#k": ["buzz-mesh-status"],
        "limit": 100
    })
}

pub fn relay_membership_filter() -> serde_json::Value {
    serde_json::json!({
        "kinds": [13534],
        "limit": 1
    })
}

fn reporter_pubkey_from_status_event(event: &nostr::Event) -> Option<String> {
    // Discovery notes are signed by the member that owns the MeshLLM identity.
    // The generic relay only stores/queries them; it is not an identity oracle.
    Some(event.pubkey.to_hex())
}

pub(super) fn enrich_status_payload_identity(
    payload: &mut serde_json::Value,
    invite_token: Option<&str>,
) {
    let endpoint_id = endpoint_id_from_status(payload, invite_token);
    let device_name = device_name_from_status(payload, endpoint_id.as_deref());
    if let Some(endpoint_id) = endpoint_id {
        payload["endpointId"] = serde_json::Value::String(endpoint_id.clone());
        payload["deviceId"] = serde_json::Value::String(endpoint_id);
    }
    if let Some(device_name) = device_name {
        payload["deviceName"] = serde_json::Value::String(device_name);
    }
}

pub(super) fn endpoint_id_from_status(
    payload: &serde_json::Value,
    invite_token: Option<&str>,
) -> Option<String> {
    string_value(payload, "endpointId")
        .or_else(|| string_value(payload, "endpoint_id"))
        .or_else(|| string_value(payload, "node_id"))
        .or_else(|| invite_token.and_then(endpoint_id_from_invite_token))
}

pub(super) fn device_name_from_status(
    payload: &serde_json::Value,
    endpoint_id: Option<&str>,
) -> Option<String> {
    string_value(payload, "deviceName")
        .or_else(|| string_value(payload, "device_name"))
        .or_else(|| string_value(payload, "my_hostname"))
        .or_else(|| string_value(payload, "hostname"))
        .or_else(|| endpoint_id.map(short_endpoint_label))
}

fn endpoint_id_from_invite_token(invite_token: &str) -> Option<String> {
    let json = URL_SAFE_NO_PAD.decode(invite_token).ok()?;
    let value = serde_json::from_slice::<serde_json::Value>(&json).ok()?;
    string_value(&value, "id")
}

fn string_value(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn short_endpoint_label(endpoint_id: &str) -> String {
    endpoint_id.chars().take(12).collect()
}
