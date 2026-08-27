use std::collections::{BTreeMap, BTreeSet};

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};

use super::{dedupe_models, MeshAvailability, MeshModelOption, MeshServeTarget, MESH_STATUS_KIND};

/// Running-node status notes are refreshed every 45 seconds. Routing ignores
/// notes older than two minutes so crashed/offline devices stop contributing
/// compute without a relay-side cleanup job. Admission intentionally does not:
/// Buzz membership, rather than device liveness, is the trust boundary.
pub(super) const STATUS_FRESHNESS_SECS: u64 = 120;
pub(crate) const MESH_STATUS_PAGE_SIZE: usize = 100;

pub(super) fn status_is_fresh(event: &nostr::Event, now: u64) -> bool {
    event
        .created_at
        .as_secs()
        .saturating_add(STATUS_FRESHNESS_SECS)
        >= now
}

/// Whether the relay enforces NIP-43 relay membership.
///
/// This is a property of the *relay deployment*, not of any particular query
/// result, and it is resolved from the relay's NIP-11 document (NIP-43 is
/// advertised only when membership is actually enforced). It must never be
/// inferred from an absent membership snapshot: on a closed relay a snapshot
/// can also be missing because of a transient replication gap, and treating
/// that as "open" would silently drop admission enforcement exactly when the
/// relay is least healthy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MeshRelayMode {
    /// Membership is enforced (`BUZZ_REQUIRE_RELAY_MEMBERSHIP=true`). Every
    /// participant is a NIP-43 direct member, so the membership snapshot is the
    /// trust boundary for both admission and routing.
    ClosedMembershipEnforced,
    /// Membership is not enforced. Signing in never writes a `relay_members`
    /// row, so the snapshot lists at most whoever was added out of band and is
    /// structurally useless as a trust boundary. Mesh runs unenforced (stock
    /// MeshLLM behaviour) and is scoped by the relay-derived mesh topic.
    OpenNoMembership,
}

impl MeshRelayMode {
    pub(crate) fn is_open(self) -> bool {
        matches!(self, Self::OpenNoMembership)
    }
}

/// Resolve [`MeshRelayMode`] from a relay's NIP-11 document.
///
/// `buzz-relay` lists NIP-43 in `supported_nips` only when it has a stable
/// signing key *and* `BUZZ_REQUIRE_RELAY_MEMBERSHIP=true`, so the advertisement
/// is a faithful proxy for "membership is enforced here" when the field is
/// well formed. A missing or malformed `supported_nips` field is rejected so
/// callers keep the fail-closed fallback rather than treating an ambiguous
/// response as evidence that membership is disabled.
pub fn relay_mesh_mode_from_nip11(document: &serde_json::Value) -> Result<MeshRelayMode, String> {
    let nips = document
        .get("supported_nips")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| "NIP-11 document has no supported_nips array".to_string())?;
    let nips = nips
        .iter()
        .enumerate()
        .map(|(index, nip)| {
            nip.as_u64()
                .filter(|value| *value <= u32::MAX as u64)
                .map(|value| value as u32)
                .ok_or_else(|| {
                    format!("NIP-11 supported_nips[{index}] is not a valid u32 NIP number")
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if nips.contains(&43) {
        Ok(MeshRelayMode::ClosedMembershipEnforced)
    } else {
        Ok(MeshRelayMode::OpenNoMembership)
    }
}

fn dedupe_targets(targets: Vec<MeshServeTarget>) -> Vec<MeshServeTarget> {
    let mut by_endpoint_and_model = BTreeMap::<(String, String), MeshServeTarget>::new();
    for target in targets {
        by_endpoint_and_model
            .entry((target.endpoint_addr.clone(), target.model_id.clone()))
            .or_insert(target);
    }
    by_endpoint_and_model.into_values().collect()
}

/// Resolve the mesh admission roster from relay status and membership events.
///
/// Only status notes signed by a currently listed NIP-43 direct member
/// contribute an owner id. This removes stale notes from former members and
/// ignores notes from nonmembers. If the relay has no membership snapshot, the
/// roster is empty and MeshLLM admission therefore remains self-only.
///
/// Admission deliberately ignores status freshness: membership is the trust
/// boundary, and a member whose device is offline (stale status) is still a
/// member. Gating admission on freshness caused the allowlist — and therefore
/// the serving node — to churn whenever any member's app went online or
/// offline. Freshness still gates *routing* (see `availability_from_events`):
/// stale nodes are never selected as serve targets. Revocation is unaffected:
/// a member removed from the NIP-43 roster leaves the intersection at the next
/// roster poll regardless of how fresh their last status is.
pub fn owner_ids_from_events(events: &[nostr::Event]) -> Vec<String> {
    let Some(members) = latest_membership_list(events) else {
        return Vec::new();
    };
    let mut ids: Vec<String> = events
        .iter()
        .filter(|event| event.kind.as_u16() as u64 == MESH_STATUS_KIND)
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

pub(super) fn latest_membership_list(events: &[nostr::Event]) -> Option<BTreeSet<String>> {
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

pub(crate) fn current_member_pubkeys(events: &[nostr::Event]) -> Vec<String> {
    latest_membership_list(events)
        .map(BTreeSet::into_iter)
        .map(Iterator::collect)
        .unwrap_or_default()
}

/// Whether the relay actually returned a NIP-43 membership snapshot (kind
/// 13534) in `events`.
///
/// The relay publishes an explicit membership event even for a zero-member
/// community, so its presence is what makes an empty roster *authoritative*.
/// Callers use this to distinguish "the community genuinely has no members"
/// (snapshot present, zero `member` tags) from "no snapshot came back at all"
/// (a transient relay gap / replication lag). Only the former may shrink the
/// admission roster; the latter must be surfaced as an error so the reconcile
/// loop keeps the current allowlist instead of restarting to self-only.
pub(crate) fn has_membership_snapshot(events: &[nostr::Event]) -> bool {
    events.iter().any(|event| event.kind.as_u16() == 13_534)
}

pub(super) fn owner_id_from_status_event(event: &nostr::Event) -> Option<String> {
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

pub(super) fn endpoint_binding_is_valid(event: &nostr::Event, content: &serde_json::Value) -> bool {
    let Some(endpoint_tokens) = super::identity::advertised_endpoint_tokens(content) else {
        return false;
    };
    let Some(verifying_key_bytes) = content
        .get("ownerVerifyingKey")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .and_then(|value| hex::decode(value).ok())
        .and_then(|value| <[u8; 32]>::try_from(value).ok())
    else {
        return false;
    };
    let Some(signature) = content
        .get("ownerEndpointBindingSig")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .and_then(|value| hex::decode(value).ok())
        .and_then(|value| Signature::from_slice(&value).ok())
    else {
        return false;
    };
    let Ok(verifying_key) = VerifyingKey::from_bytes(&verifying_key_bytes) else {
        return false;
    };
    verifying_key
        .verify(
            &super::identity::member_endpoint_binding_bytes(
                &event.pubkey.to_hex(),
                &endpoint_tokens,
            ),
            &signature,
        )
        .is_ok()
}

#[cfg(test)]
pub fn availability_from_events(events: Vec<nostr::Event>) -> MeshAvailability {
    availability_from_events_for_mode(events, MeshRelayMode::ClosedMembershipEnforced)
}

/// Resolve availability from relay events, scoped to the relay's membership mode.
///
/// On a membership-enforcing relay the NIP-43 snapshot gates routing: a serve
/// target is selectable only when its status note is signed by a current
/// member. On a relay that does not enforce membership there is no such
/// roster to intersect with — requiring one would make every serving peer
/// permanently undiscoverable — so status notes are accepted on their own
/// merits (fresh, well-formed, and carrying a valid owner-signed endpoint
/// binding). The mesh topic is derived from the relay origin, so this widens
/// discovery to that relay's participants, not to the public internet.
pub fn availability_from_events_for_mode(
    events: Vec<nostr::Event>,
    mode: MeshRelayMode,
) -> MeshAvailability {
    if events.is_empty() {
        return MeshAvailability::unavailable("Buzz shared compute status is not published yet");
    }
    // On an open relay the snapshot is not a trust boundary even when one
    // happens to exist (added out of band): admission runs `TrustPolicy::Off`
    // there, so intersecting routing with a stray roster would hide peers the
    // mesh happily admits. Ignore membership entirely and accept status notes
    // on their own merits.
    let members = if mode.is_open() {
        None
    } else {
        match latest_membership_list(&events) {
            Some(members) => Some(members),
            // An enforcing relay always publishes a snapshot (even a
            // zero-member one), so its absence means an incomplete read:
            // stay closed.
            None => {
                return MeshAvailability::unavailable(
                    "Buzz shared compute is waiting for the current member roster",
                );
            }
        }
    };

    // Status is replaceable per member pubkey, so a query returns multiple
    // events. Aggregate only current members: a removed member's last status
    // must not remain selectable after their admission is revoked.
    let mut all_targets = Vec::<MeshServeTarget>::new();
    let mut all_models = Vec::<MeshModelOption>::new();
    let mut saw_valid_status = false;

    let now = nostr::Timestamp::now().as_secs();
    for event in events {
        if event.kind.as_u16() as u64 != MESH_STATUS_KIND || !status_is_fresh(&event, now) {
            continue;
        }
        if let Some(members) = members.as_ref() {
            if !members.contains(&event.pubkey.to_hex().to_ascii_lowercase()) {
                continue;
            }
        }
        let Ok(content) = serde_json::from_str::<serde_json::Value>(&event.content) else {
            continue;
        };
        let Some(owner_id) = owner_id_from_status_event(&event) else {
            continue;
        };
        if !endpoint_binding_is_valid(&event, &content) {
            continue;
        }
        saw_valid_status = true;
        let reporter_pubkey = event.pubkey.to_hex().to_ascii_lowercase();
        let mut serve_targets = content
            .get("serveTargets")
            .or_else(|| content.get("serve_targets"))
            .cloned()
            .and_then(|value| serde_json::from_value::<Vec<MeshServeTarget>>(value).ok())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|mut target| {
                let validated =
                    super::transport_policy::validate_advertised_endpoint(&target.endpoint_addr)
                        .ok()?;
                target.endpoint_addr = validated.join_token;
                target.reporter_pubkey = Some(reporter_pubkey.clone());
                target.owner_id = Some(owner_id.clone());
                if target.endpoint_id.is_none() {
                    target.endpoint_id = Some(validated.endpoint_id);
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
                Some(target)
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
        reason: if available {
            None
        } else {
            Some("no Buzz shared compute serving members are available".to_string())
        },
        models,
        serve_targets,
    }
}

/// Status filter for admission and availability queries.
///
/// Deliberately has no `since` bound: status events are parameterized
/// replaceable (one per member), and admission must see a member's latest
/// owner binding even when that member has been offline for longer than
/// [`STATUS_FRESHNESS_SECS`]. Freshness is applied *after* the query, and only
/// where it belongs — routing (`availability_from_events`), never admission
/// (`owner_ids_from_events`).
pub fn mesh_status_filter() -> serde_json::Value {
    serde_json::json!({
        "kinds": [MESH_STATUS_KIND],
        "#k": ["buzz-mesh-status"],
        "limit": MESH_STATUS_PAGE_SIZE
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
    super::transport_policy::validate_advertised_endpoint(invite_token)
        .ok()
        .map(|validated| validated.endpoint_id)
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
