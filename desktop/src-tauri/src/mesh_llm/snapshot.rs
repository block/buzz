//! Community-wide mesh snapshot — an **allowlisted** projection of the member
//! status notes already on the relay.
//!
//! Every sharing member already publishes its whole SDK status payload as
//! kind:30003 content (see `mod.rs::status_report_payload`), but
//! `availability_from_events` reads only `serveTargets`/`models` and discards
//! the rest. This module projects the few extra fields the Share-compute UI
//! needs — per-device shared capacity, a coarse state, and a stable label —
//! and **nothing else**.
//!
//! Deliberately narrow, for two reasons:
//!   1. Routing must keep using [`super::availability_from_events`]. This
//!      snapshot is presentation-only and must never select a serve target.
//!   2. The published payload also carries a hardware fingerprint, peer
//!      topology, and operational counters. Naming each field we surface keeps
//!      the UI from quietly growing into the rest of it.
//!
//! Capacity uses the payload's `my_vram_gb`, which mesh-llm derives from a
//! `vram_bytes` already clamped to the member's own `--max-vram` cap. Raw
//! `gpus[].vram_bytes` is *not* capped, so summing GPUs would advertise more
//! than the member agreed to share.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::discovery::{
    device_name_from_status, endpoint_binding_is_valid, endpoint_id_from_status,
    latest_membership_list, owner_id_from_status_event, status_is_fresh, STATUS_FRESHNESS_SECS,
};
use super::{MeshServeTarget, MESH_STATUS_KIND};

/// Coarse per-device state. Intentionally not the SDK's `NodeState`: the UI
/// only needs "is this device giving compute, warming up, or just consuming?"
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MeshDeviceState {
    /// Advertising at least one routable model.
    Serving,
    /// Serve-mode, model not ready yet.
    Loading,
    /// Serve-mode participant with no model work (contributes capacity only).
    Standby,
    /// Client-mode: consuming someone else's compute, not sharing.
    Consuming,
}

/// One device visible on the mesh.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MeshSnapshotDevice {
    /// Stable endpoint/device id, when the note carries one.
    pub device_id: Option<String>,
    /// Human label: hostname when published, else a short endpoint id.
    pub label: String,
    /// Shared AI memory in GB, honoring the member's own cap. `None` when the
    /// member is not currently running a node (a stopped member publishes only
    /// its owner binding).
    pub capacity_gb: Option<f64>,
    /// Models this device is advertising right now.
    pub models: Vec<String>,
    pub state: MeshDeviceState,
    /// Whether this is the local member's own device.
    pub is_self: bool,
    /// Community member who signed this status note. Safe for profile lookup;
    /// endpoint tokens, owner signatures, and raw hardware remain excluded.
    pub member_pubkey: Option<String>,
    /// Relay event creation time, in Unix seconds.
    pub reported_at: Option<u64>,
    /// Approximate hosted-model footprint reported by MeshLLM, when available.
    pub model_size_gb: Option<f64>,
}

/// What the community's shared compute looks like right now.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct MeshSnapshot {
    /// Devices advertising at least one routable model.
    pub sharing_device_count: usize,
    /// Unique current members with at least one ready serving device.
    pub contributor_member_count: usize,
    /// Summed shared capacity across sharing devices. `None` when no sharing
    /// device published a figure — the UI then shows a count with no GB, never
    /// a misleading `0 GB`.
    pub shared_capacity_gb: Option<f64>,
    /// Summed model footprint only when every sharing device reports it.
    /// `None` preserves mixed-version honesty instead of presenting a partial sum.
    pub allocated_capacity_gb: Option<f64>,
    /// Distinct models ready to use, deduped across devices.
    pub models: Vec<String>,
    /// Per-device rows, for the topology view. Sharing devices first.
    pub devices: Vec<MeshSnapshotDevice>,
    /// True when the local member's own device is among the sharing devices.
    pub includes_self: bool,
    /// Members in the community right now (NIP-43 roster size).
    ///
    /// The denominator for "N of M sharing". Deliberately NOT used to estimate
    /// unshared capacity: a member who never starts a node publishes no status
    /// note, so their hardware is unknown by design — they never consented to
    /// disclose it. Ghost nodes in the topology are a count, never GB.
    pub member_count: usize,
    /// When this projection was evaluated, in Unix seconds.
    pub observed_at: u64,
    /// Maximum age of a relay-reported serving status.
    pub freshness_seconds: u64,
    /// Why the snapshot is empty, when it is. Never a hard error: an empty
    /// mesh is a normal, expected state.
    pub reason: Option<String>,
}

impl MeshSnapshot {
    fn empty(reason: impl Into<String>) -> Self {
        Self {
            observed_at: nostr::Timestamp::now().as_secs(),
            freshness_seconds: STATUS_FRESHNESS_SECS,
            reason: Some(reason.into()),
            ..Default::default()
        }
    }
}

fn f64_field(content: &serde_json::Value, key: &str) -> Option<f64> {
    content
        .get(key)
        .and_then(serde_json::Value::as_f64)
        .filter(|value| value.is_finite() && *value > 0.0)
}

/// Coarse state from the published `node_state`, falling back to whether the
/// note actually advertises routable targets.
fn device_state(content: &serde_json::Value, has_targets: bool) -> MeshDeviceState {
    let raw = content
        .get("node_state")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_ascii_lowercase();
    match raw.as_str() {
        "serving" => MeshDeviceState::Serving,
        "loading" => MeshDeviceState::Loading,
        "client" => MeshDeviceState::Consuming,
        "standby" => MeshDeviceState::Standby,
        // Unknown/absent: trust the routable evidence rather than guessing.
        _ if has_targets => MeshDeviceState::Serving,
        _ => MeshDeviceState::Standby,
    }
}

/// Project validated member status notes into a presentation snapshot.
///
/// Applies the same trust rules as routing — current NIP-43 membership, owner
/// binding, endpoint binding, and [`status_is_fresh`] — so a stale or
/// non-member note can never inflate the headline. Pure and total so it can be
/// unit-tested without a relay.
pub fn snapshot_from_events(
    events: Vec<nostr::Event>,
    self_member_pubkey: Option<&str>,
) -> MeshSnapshot {
    if events.is_empty() {
        return MeshSnapshot::empty("No shared compute has been published yet");
    }
    let Some(members) = latest_membership_list(&events) else {
        return MeshSnapshot::empty("Waiting for the current member roster");
    };

    let self_pubkey = self_member_pubkey.map(str::to_ascii_lowercase);
    let now = nostr::Timestamp::now().as_secs();
    // Status notes are replaceable per owner id, so key on that: two devices
    // signed in as the same member each publish their own note.
    let mut by_owner = BTreeMap::<String, MeshSnapshotDevice>::new();

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
        let Some(owner_id) = owner_id_from_status_event(&event) else {
            continue;
        };
        if !endpoint_binding_is_valid(&event, &content) {
            continue;
        }

        // Reuse the routing-side target parse so "sharing" here means exactly
        // what it means to the router: a validated, advertisable endpoint.
        let targets = content
            .get("serveTargets")
            .or_else(|| content.get("serve_targets"))
            .cloned()
            .and_then(|value| serde_json::from_value::<Vec<MeshServeTarget>>(value).ok())
            .unwrap_or_default()
            .into_iter()
            .filter(|target| {
                super::transport_policy::validate_advertised_endpoint(&target.endpoint_addr).is_ok()
            })
            .collect::<Vec<_>>();

        let mut models = targets
            .iter()
            .map(|target| target.model_id.clone())
            .collect::<Vec<_>>();
        models.sort();
        models.dedup();

        let endpoint_id = endpoint_id_from_status(&content, None);
        let is_self = self_pubkey
            .as_deref()
            .is_some_and(|pubkey| event.pubkey.to_hex().to_ascii_lowercase() == pubkey);

        by_owner.insert(
            owner_id,
            MeshSnapshotDevice {
                label: device_name_from_status(&content, endpoint_id.as_deref())
                    .unwrap_or_else(|| "Unknown device".to_string()),
                device_id: endpoint_id,
                capacity_gb: f64_field(&content, "my_vram_gb"),
                state: device_state(&content, !models.is_empty()),
                models,
                is_self,
                member_pubkey: Some(event.pubkey.to_hex()),
                reported_at: Some(event.created_at.as_secs()),
                model_size_gb: f64_field(&content, "model_size_gb"),
            },
        );
    }

    let mut devices = by_owner.into_values().collect::<Vec<_>>();
    // Sharing devices first, then the local device, then by label so the list
    // is stable across polls (no jitter in the topology view).
    devices.sort_by(|a, b| {
        let sharing = |d: &MeshSnapshotDevice| d.state == MeshDeviceState::Serving;
        sharing(b)
            .cmp(&sharing(a))
            .then_with(|| b.is_self.cmp(&a.is_self))
            .then_with(|| a.label.cmp(&b.label))
    });

    let sharing = devices
        .iter()
        .filter(|device| device.state == MeshDeviceState::Serving)
        .collect::<Vec<_>>();
    let sharing_device_count = sharing.len();
    let contributor_member_count = sharing
        .iter()
        .filter_map(|device| device.member_pubkey.as_ref())
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    let includes_self = sharing.iter().any(|device| device.is_self);
    // Sum only reported figures; keep `None` when nobody reported one so the
    // UI can drop the number instead of printing `0 GB`.
    let shared_capacity_gb = sharing
        .iter()
        .filter_map(|device| device.capacity_gb)
        .fold(None::<f64>, |acc, gb| Some(acc.unwrap_or(0.0) + gb));
    let allocated_capacity_gb = (!sharing.is_empty()
        && sharing.iter().all(|device| device.model_size_gb.is_some()))
    .then(|| {
        sharing
            .iter()
            .filter_map(|device| device.model_size_gb)
            .sum()
    });
    let mut models = sharing
        .iter()
        .flat_map(|device| device.models.iter().cloned())
        .collect::<Vec<_>>();
    models.sort();
    models.dedup();

    let reason = if sharing_device_count == 0 {
        Some("No one is sharing compute yet".to_string())
    } else {
        None
    };

    MeshSnapshot {
        member_count: members.len(),
        contributor_member_count,
        sharing_device_count,
        shared_capacity_gb,
        allocated_capacity_gb,
        models,
        devices,
        includes_self,
        observed_at: now,
        freshness_seconds: STATUS_FRESHNESS_SECS,
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_events_report_a_reason_not_an_error() {
        let snapshot = snapshot_from_events(Vec::new(), None);
        assert_eq!(snapshot.sharing_device_count, 0);
        assert_eq!(snapshot.shared_capacity_gb, None);
        assert!(snapshot.reason.is_some());
        assert_eq!(snapshot.freshness_seconds, STATUS_FRESHNESS_SECS);
        assert!(snapshot.observed_at > 0);
    }

    #[test]
    fn missing_membership_snapshot_is_not_treated_as_an_empty_mesh() {
        // A relay gap must not read as "nobody is sharing".
        let event = nostr::EventBuilder::new(nostr::Kind::Custom(30003), "{}")
            .sign_with_keys(&nostr::Keys::generate())
            .expect("test event signs");
        let snapshot = snapshot_from_events(vec![event], None);
        assert_eq!(
            snapshot.reason.as_deref(),
            Some("Waiting for the current member roster")
        );
    }

    #[test]
    fn unknown_node_state_falls_back_to_routable_evidence() {
        assert_eq!(
            device_state(&serde_json::json!({}), true),
            MeshDeviceState::Serving
        );
        assert_eq!(
            device_state(&serde_json::json!({}), false),
            MeshDeviceState::Standby
        );
        assert_eq!(
            device_state(&serde_json::json!({"node_state": "client"}), false),
            MeshDeviceState::Consuming
        );
    }

    #[test]
    fn capacity_ignores_zero_and_nonfinite_reports() {
        // A node that has not surveyed yet publishes 0.0; that must not count
        // as a real reported capacity.
        assert_eq!(
            f64_field(&serde_json::json!({"my_vram_gb": 0.0}), "my_vram_gb"),
            None
        );
        assert_eq!(
            f64_field(&serde_json::json!({"my_vram_gb": 36.5}), "my_vram_gb"),
            Some(36.5)
        );
    }
}
