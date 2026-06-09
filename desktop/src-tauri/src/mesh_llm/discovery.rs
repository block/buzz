use std::collections::BTreeMap;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};

use super::{dedupe_models, MeshAvailability, MeshModelOption, MeshServeTarget, MESH_STATUS_KIND};

fn dedupe_targets(targets: Vec<MeshServeTarget>) -> Vec<MeshServeTarget> {
    let mut by_endpoint = BTreeMap::<String, MeshServeTarget>::new();
    for target in targets {
        by_endpoint
            .entry(target.endpoint_addr.clone())
            .or_insert(target);
    }
    by_endpoint.into_values().collect()
}

pub fn availability_from_events(events: Vec<nostr::Event>) -> MeshAvailability {
    if events.is_empty() {
        return MeshAvailability::unavailable("relay mesh status is not published yet");
    }

    // Relay status is now per reporter (d=sprout-relay-mesh:<pubkey>), so a
    // query returns multiple replaceable events. Aggregate them; do not pick the
    // newest single event or one member's machines hide everyone else's.
    let mut all_targets = Vec::<MeshServeTarget>::new();
    let mut all_models = Vec::<MeshModelOption>::new();
    let mut saw_valid_status = false;

    for event in events {
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
        return MeshAvailability::unavailable("relay mesh status is malformed");
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
            Some("no relay mesh serve targets are available".to_string())
        },
        models,
        serve_targets,
    }
}

pub fn mesh_status_filter() -> serde_json::Value {
    serde_json::json!({
        "kinds": [MESH_STATUS_KIND],
        "#k": ["sprout-mesh-status"],
        "limit": 100
    })
}

fn reporter_pubkey_from_status_event(event: &nostr::Event) -> Option<String> {
    event.tags.iter().find_map(|tag| {
        let slice = tag.as_slice();
        let d = slice.get(1)?;
        if slice.first().is_some_and(|name| name == "d") {
            d.strip_prefix("sprout-relay-mesh:")
                .map(ToString::to_string)
        } else {
            None
        }
    })
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
