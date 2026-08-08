//! Mesh inference readiness polling helpers.
//!
//! Extracted from `mesh_llm.rs` to stay within the file-size ratchet.
//! All items are `pub(super)` — visible to `mesh_llm` and its test submodule
//! via `use super::readiness::*`.

/// Which startup stage a mesh client is stuck at when it never becomes
/// inference-ready. The two live-observed failure modes are physically
/// distinct and want different user copy:
///
///   * `CatalogNeverSynced` — the local client node came up and connected to
///     the host at the control level (ping/RTT fine), but the served model
///     never appeared in the local `/v1/models` catalog. That catalog is
///     populated by the peer gossip exchange; when the gossip bi-stream can't
///     establish across the network (observed as iroh
///     `MultipathNotNegotiated` / unreachable direct path), the catalog stays
///     empty forever and every request is rejected "model not available".
///     Root cause is the network path between this machine and the host.
///   * `RoutingNeverCompleted` — the model *did* sync into the catalog, but
///     inference requests never completed (routing/transport to the host
///     failing per-request). The host is discoverable and advertised but not
///     actually serving us.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MeshReadinessFailure {
    CatalogNeverSynced,
    RoutingNeverCompleted,
}

/// Pure classifier: given whether the served model was ever observed in the
/// local `/v1/models` catalog during the wait, decide which stage failed.
/// Split out so the diagnosis is unit-testable without a live mesh.
pub(super) fn classify_mesh_readiness_failure(model_ever_visible: bool) -> MeshReadinessFailure {
    if model_ever_visible {
        MeshReadinessFailure::RoutingNeverCompleted
    } else {
        MeshReadinessFailure::CatalogNeverSynced
    }
}

/// Actionable, non-technical copy for a readiness failure. `last_detail` is the
/// last raw transport/HTTP error, appended for support triage.
pub(super) fn mesh_readiness_failure_message(
    failure: MeshReadinessFailure,
    model_id: &str,
    last_detail: &str,
) -> String {
    match failure {
        MeshReadinessFailure::CatalogNeverSynced => format!(
            "Buzz shared compute connected to the serving member but could not sync \
             the model list for \"{model_id}\" — this is a network path problem \
             between this machine and the host (the compute node is reachable for \
             pings but the model-sync stream did not establish). Try again, or have \
             the host and this machine on a more direct network. (last: {last_detail})"
        ),
        MeshReadinessFailure::RoutingNeverCompleted => format!(
            "Buzz shared compute found \"{model_id}\" on a serving member but inference \
             requests did not complete — the host is discoverable but not currently \
             reachable for requests. Try again shortly. (last: {last_detail})"
        ),
    }
}

/// Poll the local mesh OpenAI ingress until a real inference for `model_id`
/// succeeds, or a deadline elapses. On failure, returns a stage-specific,
/// actionable message (see [`MeshReadinessFailure`]) rather than a raw
/// `HTTP 429`, so the UI can tell "still warming up" apart from "can't reach
/// the host".
pub(super) async fn wait_for_mesh_inference(model_id: &str) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|error| format!("failed to build mesh readiness client: {error}"))?;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(120);
    let models_url = format!("{}/models", crate::managed_agents::RELAY_MESH_API_BASE_URL);
    let chat_url = format!(
        "{}/chat/completions",
        crate::managed_agents::RELAY_MESH_API_BASE_URL
    );
    let mut last_error = "mesh inference is not ready".to_string();
    // Track whether the served model ever reached the local catalog — the
    // signal that splits "catalog never synced" from "routing never completed".
    let mut model_ever_visible = false;

    while tokio::time::Instant::now() < deadline {
        // Refresh catalog visibility. "auto" delegates model choice to the
        // router, so any advertised model counts as the catalog having synced.
        if let Ok(response) = client
            .get(&models_url)
            .bearer_auth(crate::managed_agents::RELAY_MESH_API_KEY_PLACEHOLDER)
            .send()
            .await
        {
            if let Ok(body) = response.json::<serde_json::Value>().await {
                if let Some(data) = body.get("data").and_then(|d| d.as_array()) {
                    let wanted = model_id.trim().replace("@main", "");
                    let visible = !data.is_empty()
                        && (model_id == crate::mesh_llm::AUTO_MODEL_ID
                            || data.iter().any(|m| {
                                m.get("id")
                                    .and_then(|id| id.as_str())
                                    .map(|id| id.replace("@main", "") == wanted)
                                    .unwrap_or(false)
                            }));
                    model_ever_visible |= visible;
                }
            }
        }

        match client
            .post(&chat_url)
            .bearer_auth(crate::managed_agents::RELAY_MESH_API_KEY_PLACEHOLDER)
            .json(&serde_json::json!({
                "model": model_id,
                "messages": [{"role": "user", "content": "Reply OK"}],
                "max_tokens": 1,
                "stream": false
            }))
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => return Ok(()),
            Ok(response) => {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                last_error = format!("HTTP {status}: {body}");
            }
            Err(error) => last_error = error.to_string(),
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }

    let failure = classify_mesh_readiness_failure(model_ever_visible);
    Err(mesh_readiness_failure_message(
        failure,
        model_id,
        &last_error,
    ))
}
