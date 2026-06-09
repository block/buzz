//! Relay-signed mesh-LLM status publication.
//!
//! The relay owns the Nostr publication surface for Sprout mesh status. mesh-llm
//! remains the source of live runtime truth, but the relay sanitizes that status
//! and republishes a member-readable, relay-signed parameterized event. This
//! keeps discovery inside Sprout's relay-membership boundary and avoids mesh's
//! public-relay publisher path.

use std::sync::Arc;

use nostr::{EventBuilder, Kind, Tag};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sprout_core::kind::KIND_MESH_LLM_RELAY_STATUS;
use tracing::info;

use crate::handlers::event::dispatch_persistent_event;
use crate::state::AppState;

/// d-tag prefix for the relay's mesh status events. The full d-tag is
/// `"<prefix>:<reporter_pubkey_hex>"` so each reporting member gets an isolated,
/// independently-replaceable kind:30621 note (NIP-33 keys on `(kind, pubkey,
/// d_tag)`, and the author is always the relay key here). This avoids two
/// members' reports clobbering each other and sidesteps any read-modify-write
/// race on a single shared note. Discovery reads all notes with the `k` tag.
pub const MESH_STATUS_D_TAG_PREFIX: &str = "sprout-relay-mesh";

/// Build the per-reporter d-tag for a mesh status note.
pub fn mesh_status_d_tag(reporter_pubkey_hex: &str) -> String {
    format!("{MESH_STATUS_D_TAG_PREFIX}:{reporter_pubkey_hex}")
}

/// Content schema discriminator.
pub const MESH_STATUS_TYPE: &str = "sprout-mesh-status";

/// Sanitized relay-published mesh status.
///
/// Carries dial metadata (mesh identity + per-target `endpoint_addr` invite
/// tokens), not access grants. iroh admission via NIP-98 → relay membership is
/// the only gate; possession of these fields confers no ability to dial a
/// serving node that has not admitted the caller's pubkey.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SproutMeshStatus {
    /// Schema version.
    pub v: u32,
    /// Constant discriminator: `sprout-mesh-status`.
    #[serde(rename = "type")]
    pub status_type: String,
    /// Unix timestamp when the relay generated this projection.
    pub updated_at: u64,
    /// Mesh identity, if mesh-llm has joined/created one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mesh_id: Option<String>,
    /// Human mesh name, if configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mesh_name: Option<String>,
    /// Serving targets reachable by relay members.
    pub serve_targets: Vec<MeshServeTarget>,
    /// Deduplicated model options for agent/provider pickers.
    pub models: Vec<MeshModelOption>,
    /// Aggregate peer count from mesh status.
    pub peer_count: usize,
}

/// Model value + display label. `id` is the API/routing value; `name` is UI-only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeshModelOption {
    /// Stable routable OpenAI model id/ref. This is the value clients pass to `/v1`.
    pub id: String,
    /// Optional UI label. Must never replace [`Self::id`] for routing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// A model served at an EndpointAddr dial pointer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeshServeTarget {
    /// Stable routable OpenAI model id/ref served at this target.
    pub model_id: String,
    /// Optional UI label for [`Self::model_id`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_name: Option<String>,
    /// mesh-llm invite token: base64(json(EndpointAddr)). This is dial
    /// metadata, not an access grant; iroh admission remains the gate.
    pub endpoint_addr: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Optional node label/hostname for UI display.
    pub node_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Optional serving capacity hint for this target.
    pub capacity: Option<MeshTargetCapacity>,
}

/// Capacity hint for a serving target.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeshTargetCapacity {
    /// Advertised VRAM capacity in GB, if known.
    pub vram_gb: Option<f64>,
}

/// Build the relay-owned status projection from mesh's `/api/status` JSON.
pub fn sanitize_mesh_status(payload: &Value, now_unix: u64) -> SproutMeshStatus {
    let endpoint_addr = string_field(payload, "token").unwrap_or_default();
    let node_id = string_field(payload, "node_id");
    let mesh_id = string_field(payload, "mesh_id");
    let mesh_name = string_field(payload, "mesh_name");
    let my_vram_gb = payload.get("my_vram_gb").and_then(Value::as_f64);

    let mut models = Vec::<MeshModelOption>::new();
    let mut serve_targets = Vec::<MeshServeTarget>::new();

    for model_id in string_array_field(payload, "hosted_models")
        .into_iter()
        .chain(string_array_field(payload, "serving_models"))
    {
        push_model(&mut models, &model_id, None);
        if !endpoint_addr.is_empty() {
            push_target(
                &mut serve_targets,
                MeshServeTarget {
                    model_id,
                    model_name: None,
                    endpoint_addr: endpoint_addr.clone(),
                    node_name: node_id.clone(),
                    capacity: Some(MeshTargetCapacity {
                        vram_gb: my_vram_gb,
                    }),
                },
            );
        }
    }

    let peers = payload
        .get("peers")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    for peer in &peers {
        let peer_endpoint = string_field(peer, "invite_token")
            .or_else(|| string_field(peer, "endpoint_addr"))
            .or_else(|| string_field(peer, "endpointAddr"));
        let peer_name = string_field(peer, "hostname").or_else(|| string_field(peer, "id"));
        let peer_vram_gb = peer.get("vram_gb").and_then(Value::as_f64);

        for model_id in string_array_field(peer, "hosted_models")
            .into_iter()
            .chain(string_array_field(peer, "serving_models"))
        {
            push_model(&mut models, &model_id, None);
            if let Some(endpoint_addr) = peer_endpoint.clone() {
                push_target(
                    &mut serve_targets,
                    MeshServeTarget {
                        model_id,
                        model_name: None,
                        endpoint_addr,
                        node_name: peer_name.clone(),
                        capacity: Some(MeshTargetCapacity {
                            vram_gb: peer_vram_gb,
                        }),
                    },
                );
            }
        }
    }

    models.sort_by(|a, b| a.id.cmp(&b.id));
    serve_targets.sort_by(|a, b| {
        a.model_id
            .cmp(&b.model_id)
            .then_with(|| a.endpoint_addr.cmp(&b.endpoint_addr))
    });

    SproutMeshStatus {
        v: 1,
        status_type: MESH_STATUS_TYPE.to_string(),
        updated_at: now_unix,
        mesh_id,
        mesh_name,
        serve_targets,
        models,
        peer_count: peers.len(),
    }
}

/// Publish sanitized mesh status as a relay-signed kind:30621 event, keyed to
/// the reporting member's pubkey so each member's note is isolated.
pub async fn publish_mesh_status_from_payload(
    state: &Arc<AppState>,
    reporter_pubkey_hex: &str,
    payload: &Value,
) -> anyhow::Result<()> {
    let now_unix = chrono::Utc::now().timestamp().max(0) as u64;
    let status = sanitize_mesh_status(payload, now_unix);
    publish_mesh_status(state, reporter_pubkey_hex, &status).await
}

/// Publish a pre-sanitized mesh status. Exposed for tests and integration seams.
pub async fn publish_mesh_status(
    state: &Arc<AppState>,
    reporter_pubkey_hex: &str,
    status: &SproutMeshStatus,
) -> anyhow::Result<()> {
    let content = serde_json::to_string(status)?;
    let d_tag = mesh_status_d_tag(reporter_pubkey_hex);
    let tags = vec![
        Tag::parse(["-"]).map_err(|e| anyhow::anyhow!("failed to build '-' tag: {e}"))?,
        Tag::parse(["d", &d_tag]).map_err(|e| anyhow::anyhow!("failed to build d tag: {e}"))?,
        Tag::parse(["k", MESH_STATUS_TYPE])
            .map_err(|e| anyhow::anyhow!("failed to build k tag: {e}"))?,
    ];

    let event = EventBuilder::new(Kind::Custom(KIND_MESH_LLM_RELAY_STATUS as u16), content)
        .tags(tags)
        .sign_with_keys(&state.relay_keypair)
        .map_err(|e| anyhow::anyhow!("failed to sign kind:{KIND_MESH_LLM_RELAY_STATUS}: {e}"))?;

    let (stored, was_inserted) = state
        .db
        .replace_parameterized_event(&event, &d_tag, None)
        .await?;
    if was_inserted {
        let relay_pubkey_hex = state.relay_keypair.public_key().to_hex();
        dispatch_persistent_event(
            state,
            &stored,
            KIND_MESH_LLM_RELAY_STATUS,
            &relay_pubkey_hex,
        )
        .await;
    }

    info!(
        targets = status.serve_targets.len(),
        models = status.models.len(),
        "mesh-LLM relay status published"
    );
    Ok(())
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn string_array_field(value: &Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn push_model(models: &mut Vec<MeshModelOption>, id: &str, name: Option<String>) {
    if models.iter().any(|model| model.id == id) {
        return;
    }
    models.push(MeshModelOption {
        id: id.to_string(),
        name,
    });
}

fn push_target(targets: &mut Vec<MeshServeTarget>, target: MeshServeTarget) {
    if targets.iter().any(|existing| {
        existing.model_id == target.model_id && existing.endpoint_addr == target.endpoint_addr
    }) {
        return;
    }
    targets.push(target);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn d_tag_is_per_reporter_so_members_never_clobber() {
        let a = "aa".repeat(32);
        let b = "bb".repeat(32);
        let d_a = mesh_status_d_tag(&a);
        let d_b = mesh_status_d_tag(&b);
        // Two different members → two different d-tags → isolated 30621 notes
        // (NIP-33 keys on (kind, pubkey, d); the relay is always the author, so
        // the d-tag is the only thing distinguishing reporters).
        assert_ne!(d_a, d_b, "distinct reporters must get distinct d-tags");
        assert!(d_a.starts_with(MESH_STATUS_D_TAG_PREFIX));
        assert!(d_a.ends_with(&a));
        // Same reporter → same d-tag → its later report replaces its own note.
        assert_eq!(d_a, mesh_status_d_tag(&a));
    }

    #[test]
    fn sanitizer_projects_models_and_endpoint_addr_without_raw_runtime() {
        let payload = serde_json::json!({
            "token": "endpoint-token-a",
            "node_id": "node-a",
            "mesh_id": "mesh-1",
            "mesh_name": "sprout",
            "hosted_models": ["Qwen3-8B-Q4_K_M"],
            "serving_models": ["Qwen3-8B-Q4_K_M"],
            "my_vram_gb": 12.0,
            "runtime": { "stages": [{ "source_model_path": "/secret/model.gguf" }] },
            "local_instances": [{ "runtime_dir": "/tmp/private" }],
            "peers": [{
                "id": "peer-b",
                "hostname": "gpu-b",
                "invite_token": "endpoint-token-b",
                "hosted_models": ["TinyLlama"],
                "vram_gb": 8.0
            }]
        });

        let status = sanitize_mesh_status(&payload, 123);
        assert_eq!(status.status_type, MESH_STATUS_TYPE);
        assert_eq!(status.mesh_id.as_deref(), Some("mesh-1"));
        assert_eq!(status.models.len(), 2);
        assert!(status.models.iter().any(|m| m.id == "Qwen3-8B-Q4_K_M"));
        assert!(status.models.iter().any(|m| m.id == "TinyLlama"));
        assert_eq!(status.serve_targets.len(), 2);
        assert!(status
            .serve_targets
            .iter()
            .any(|target| target.model_id == "Qwen3-8B-Q4_K_M"
                && target.endpoint_addr == "endpoint-token-a"));
        assert!(status
            .serve_targets
            .iter()
            .any(|target| target.model_id == "TinyLlama"
                && target.endpoint_addr == "endpoint-token-b"));

        let serialized = serde_json::to_string(&status).unwrap();
        assert!(!serialized.contains("source_model_path"));
        assert!(!serialized.contains("runtime_dir"));
        assert!(!serialized.contains("/secret"));
    }

    #[test]
    fn sanitizer_keeps_model_id_separate_from_label() {
        let payload = serde_json::json!({
            "token": "endpoint-token-a",
            "hosted_models": ["hf://meshllm/demo@main"],
            "peers": []
        });

        let status = sanitize_mesh_status(&payload, 123);
        assert_eq!(status.models[0].id, "hf://meshllm/demo@main");
        assert_eq!(status.models[0].name, None);
        assert_eq!(status.serve_targets[0].model_id, "hf://meshllm/demo@main");
        assert_eq!(status.serve_targets[0].model_name, None);
    }
}
