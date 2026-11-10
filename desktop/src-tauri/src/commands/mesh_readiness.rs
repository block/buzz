//! Startup readiness for Buzz shared compute.
//!
//! Mesh can bind its HTTP ingress and advertise a model shortly before the
//! router has installed a usable target. These helpers probe the exact chat
//! path agents use, so startup cannot race that gap
//! (`single target None unavailable`), and classify a timeout into copy that
//! names the actual stage rather than a raw `HTTP 429`.

use super::CmdResult;

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
enum MeshReadinessFailure {
    CatalogNeverSynced,
    RoutingNeverCompleted,
}

/// Pure classifier: given whether the served model was ever observed in the
/// local `/v1/models` catalog during the wait, decide which stage failed.
/// Split out so the diagnosis is unit-testable without a live mesh.
/// Whether the catalog has synced enough to count, for the model actually being
/// requested (a wire name — see `relay_mesh_wire_model`).
///
/// The virtual `mesh` model delegates the choice to the router, so any
/// advertised model proves the catalog synced. It has to work that way: MeshLLM
/// only advertises `mesh` itself once two non-virtual models are reachable
/// (`should_advertise_virtual_mesh`), so requiring it by name would leave a
/// single-worker mesh looking permanently unsynced and misreport a slow model
/// load as a network path problem.
fn mesh_catalog_shows_model(advertised: &[String], wire_model: &str) -> bool {
    if advertised.is_empty() {
        return false;
    }
    if wire_model == crate::managed_agents::RELAY_MESH_VIRTUAL_MODEL_ID {
        return true;
    }
    let wanted = wire_model.trim().replace("@main", "");
    advertised
        .iter()
        .any(|id| id.replace("@main", "") == wanted)
}

fn classify_mesh_readiness_failure(model_ever_visible: bool) -> MeshReadinessFailure {
    if model_ever_visible {
        MeshReadinessFailure::RoutingNeverCompleted
    } else {
        MeshReadinessFailure::CatalogNeverSynced
    }
}

/// Actionable, non-technical copy for a readiness failure. `last_detail` is the
/// last raw transport/HTTP error, appended for support triage.
fn mesh_readiness_failure_message(
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
pub(crate) async fn wait_for_mesh_inference(model_id: &str) -> CmdResult<()> {
    // Probe the name that will actually be requested. Callers pass a stored
    // value, which for shared-compute `auto` is not a model the mesh
    // advertises: probing it would validate a route no agent uses, and could
    // fail readiness while the real route works. Named models pass through
    // unchanged, so this is safe for the serve-side callers too.
    let requested_model = model_id;
    let model_id = crate::managed_agents::relay_mesh_wire_model(model_id);
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
        // Refresh catalog visibility. The virtual `mesh` model delegates the
        // choice to the router, so any advertised model counts as the catalog
        // having synced — and it must, because MeshLLM only advertises `mesh`
        // itself once two non-virtual models are reachable
        // (`should_advertise_virtual_mesh`). Requiring it by name would leave a
        // single-worker mesh looking permanently unsynced.
        if let Ok(response) = client
            .get(&models_url)
            .bearer_auth(crate::managed_agents::RELAY_MESH_API_KEY_PLACEHOLDER)
            .send()
            .await
        {
            if let Ok(body) = response.json::<serde_json::Value>().await {
                if let Some(data) = body.get("data").and_then(|d| d.as_array()) {
                    let advertised: Vec<String> = data
                        .iter()
                        .filter_map(|m| m.get("id").and_then(|id| id.as_str()))
                        .map(str::to_owned)
                        .collect();
                    model_ever_visible |= mesh_catalog_shows_model(&advertised, model_id);
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
        requested_model,
        &last_error,
    ))
}
#[cfg(test)]
mod tests {
    use super::*;

    /// The regression this guards: MeshLLM only advertises the virtual `mesh` model
    /// once two non-virtual models are reachable, so a single-worker mesh never
    /// lists it by name. Keying visibility on the literal name would report a lone
    /// host that is still loading weights as a network path problem.
    #[test]
    fn virtual_mesh_counts_any_advertised_model_as_a_synced_catalog() {
        let one_worker = vec!["unsloth/gemma-4-E4B-it-GGUF:Q4_K_M".to_string()];
        assert!(mesh_catalog_shows_model(
            &one_worker,
            crate::managed_agents::RELAY_MESH_VIRTUAL_MODEL_ID
        ));
    }

    #[test]
    fn an_empty_catalog_is_never_synced_even_for_the_virtual_model() {
        assert!(!mesh_catalog_shows_model(
            &[],
            crate::managed_agents::RELAY_MESH_VIRTUAL_MODEL_ID
        ));
    }

    #[test]
    fn a_named_model_must_actually_be_advertised() {
        let advertised = vec!["unsloth/gemma-4-E4B-it-GGUF:Q4_K_M".to_string()];
        assert!(mesh_catalog_shows_model(
            &advertised,
            "unsloth/gemma-4-E4B-it-GGUF:Q4_K_M"
        ));
        assert!(!mesh_catalog_shows_model(&advertised, "some/other-model"));
    }

    #[test]
    fn a_named_model_ignores_the_main_revision_suffix() {
        let advertised = vec!["unsloth/gemma-4-E4B-it-GGUF:Q4_K_M@main".to_string()];
        assert!(mesh_catalog_shows_model(
            &advertised,
            "unsloth/gemma-4-E4B-it-GGUF:Q4_K_M"
        ));
    }

    #[test]
    fn readiness_failure_is_catalog_sync_when_model_never_visible() {
        assert_eq!(
            classify_mesh_readiness_failure(false),
            MeshReadinessFailure::CatalogNeverSynced
        );
    }

    #[test]
    fn readiness_failure_is_routing_when_model_was_visible() {
        assert_eq!(
            classify_mesh_readiness_failure(true),
            MeshReadinessFailure::RoutingNeverCompleted
        );
    }

    #[test]
    fn readiness_messages_are_distinct_and_actionable() {
        let catalog = mesh_readiness_failure_message(
            MeshReadinessFailure::CatalogNeverSynced,
            "auto",
            "HTTP 429",
        );
        let routing = mesh_readiness_failure_message(
            MeshReadinessFailure::RoutingNeverCompleted,
            "auto",
            "HTTP 503",
        );
        // Distinct diagnoses, each names the model and carries the raw detail.
        assert_ne!(catalog, routing);
        assert!(catalog.contains("network path"));
        assert!(catalog.contains("HTTP 429"));
        assert!(routing.contains("did not complete"));
        assert!(routing.contains("HTTP 503"));
    }
}
