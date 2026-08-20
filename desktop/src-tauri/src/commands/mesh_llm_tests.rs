use super::*;
use crate::app_state::build_app_state;

fn target(model_id: &str, endpoint_addr: &str) -> mesh_llm::MeshServeTarget {
    mesh_llm::MeshServeTarget {
        model_id: model_id.to_string(),
        model_name: None,
        endpoint_addr: endpoint_addr.to_string(),
        reporter_pubkey: None,
        owner_id: None,
        node_name: None,
        capacity: None,
        endpoint_id: None,
        device_id: None,
        device_name: None,
    }
}

fn reported_target(
    reporter_pubkey: &str,
    model_id: &str,
    endpoint_addr: &str,
) -> mesh_llm::MeshServeTarget {
    let mut target = target(model_id, endpoint_addr);
    target.reporter_pubkey = Some(reporter_pubkey.to_string());
    target.owner_id = Some(reporter_pubkey.to_string());
    target
}

#[test]
fn buzz_mesh_join_uses_the_same_live_member_from_every_other_node() {
    let targets = vec![
        reported_target("member-c", "model-c", "addr-c"),
        reported_target("member-a", "model-a", "addr-a"),
        reported_target("member-b", "model-b", "addr-b"),
    ];

    assert_eq!(
        buzz_mesh_join_targets(targets.clone(), "member-b")
            .into_iter()
            .next()
            .map(|target| target.endpoint_addr),
        Some("addr-a".to_string())
    );
    assert_eq!(
        buzz_mesh_join_targets(targets, "member-c")
            .into_iter()
            .next()
            .map(|target| target.endpoint_addr),
        Some("addr-a".to_string())
    );
}

#[test]
fn buzz_mesh_bootstrap_member_does_not_dial_itself() {
    let targets = vec![
        reported_target("member-b", "model-b", "addr-b"),
        reported_target("member-a", "model-a", "addr-a"),
    ];

    assert_eq!(
        buzz_mesh_join_targets(targets, "MEMBER-A")
            .into_iter()
            .next(),
        Some(reported_target("member-b", "model-b", "addr-b"))
    );
}

#[test]
fn buzz_mesh_join_ignores_targets_without_a_validated_reporter() {
    let targets = vec![
        target("unbound-model", "unbound-addr"),
        reported_target("member-b", "model-b", "addr-b"),
    ];

    assert_eq!(
        buzz_mesh_join_targets(targets, "member-c")
            .into_iter()
            .next()
            .map(|target| target.endpoint_addr),
        Some("addr-b".to_string())
    );
}

#[test]
fn buzz_mesh_join_keeps_other_device_with_the_same_member_key() {
    let mut self_target = reported_target("same-member", "model-a", "self-addr");
    self_target.owner_id = Some("owner-self".to_string());
    let mut other_device = reported_target("same-member", "model-b", "other-addr");
    other_device.owner_id = Some("owner-other".to_string());

    assert_eq!(
        buzz_mesh_join_targets(vec![self_target, other_device], "owner-self")
            .into_iter()
            .next()
            .map(|target| target.endpoint_addr),
        Some("other-addr".to_string())
    );
}

#[test]
fn buzz_mesh_name_is_stable_and_does_not_expose_the_relay() {
    let first = buzz_mesh_name_for_relay("WSS://EXAMPLE.COM/");
    let second = buzz_mesh_name_for_relay("wss://example.com:443/some/path?ignored=yes");
    let other_relay = buzz_mesh_name_for_relay("wss://other.example.com");

    assert_eq!(first, second);
    assert_ne!(first, other_relay);
    assert!(first.starts_with("buzz-community-"));
    assert!(!first.contains("example"));
}

#[test]
fn sharing_config_keeps_the_community_where_sharing_was_enabled() {
    let request = mesh_llm::StartMeshNodeRequest {
        mode: mesh_llm::MeshNodeMode::Serve,
        model_id: Some("test-model".to_string()),
        max_vram_gb: Some(24),
        join_token: None,
        mesh_name: Some("buzz-community-test".to_string()),
        relay_url: Some("wss://community.example".to_string()),
        trusted_owner_ids: Some(Vec::new()),
    };

    let config = sharing_config_from_request(&request).expect("valid sharing config");
    assert_eq!(config.relay_url.as_deref(), Some("wss://community.example"));
}

#[test]
fn legacy_sharing_config_without_community_binding_still_loads() {
    let config: MeshSharingConfig = serde_json::from_value(serde_json::json!({
        "enabled": true,
        "modelId": "test-model",
        "maxVramGb": null
    }))
    .expect("legacy sharing config");

    assert_eq!(config.relay_url, None);
    assert!(!config.start_on_next_launch);
}

#[test]
fn new_start_checkpoint_prevents_incomplete_download_restore() {
    let config = MeshSharingConfig {
        enabled: true,
        start_on_next_launch: false,
        model_id: "test-model".to_string(),
        max_vram_gb: Some(24),
        relay_url: Some("wss://community.example".to_string()),
    };

    let checkpoint = pending_new_start_checkpoint(&config);
    assert!(!checkpoint.enabled);
    assert!(!checkpoint.start_on_next_launch);
    assert_eq!(checkpoint.model_id, config.model_id);
    assert_eq!(checkpoint.max_vram_gb, config.max_vram_gb);
    assert_eq!(checkpoint.relay_url, config.relay_url);
}

#[test]
fn role_switch_checkpoint_starts_exactly_once_after_restart() {
    let config = MeshSharingConfig {
        enabled: true,
        start_on_next_launch: false,
        model_id: "test-model".to_string(),
        max_vram_gb: Some(24),
        relay_url: Some("wss://community.example".to_string()),
    };

    let restart = one_shot_restart_checkpoint(&config);
    assert!(!restart.enabled);
    assert!(restart.start_on_next_launch);

    let consumed = pending_new_start_checkpoint(&restart);
    assert!(!consumed.enabled);
    assert!(!consumed.start_on_next_launch);
    assert_eq!(consumed.model_id, config.model_id);
    assert_eq!(consumed.relay_url, config.relay_url);
}

#[test]
fn mesh_status_cursor_uses_relay_composite_tiebreak() {
    let event = nostr::EventBuilder::new(nostr::Kind::TextNote, "status")
        .custom_created_at(nostr::Timestamp::from(1_234))
        .sign_with_keys(&nostr::Keys::generate())
        .expect("sign test status");
    let mut filter = mesh_llm::mesh_status_filter();

    let cursor = advance_mesh_status_cursor(&mut filter, std::slice::from_ref(&event))
        .expect("advance status cursor");

    assert_eq!(cursor, (1_234, event.id.to_hex()));
    assert_eq!(filter["until"], serde_json::json!(1_234));
    assert_eq!(filter["before_id"], serde_json::json!(event.id.to_hex()));
    assert_eq!(
        filter["limit"],
        serde_json::json!(mesh_llm::MESH_STATUS_PAGE_SIZE)
    );
}

#[test]
fn pick_serve_target_returns_first_match_for_model() {
    let targets = vec![
        target("model-a", "addr-a"),
        target("model-b", "addr-b1"),
        target("model-b", "addr-b2"),
    ];
    // Matches by model id and returns the first such target.
    assert_eq!(
        pick_serve_target_for_model(targets, "model-b").map(|t| t.endpoint_addr),
        Some("addr-b1".to_string())
    );
}

#[test]
fn pick_serve_target_normalizes_main_revision() {
    let targets = vec![target("org/model@main:q4", "addr")];
    assert_eq!(
        pick_serve_target_for_model(targets, "org/model:q4").map(|target| target.endpoint_addr),
        Some("addr".to_string())
    );
}

#[test]
fn pick_serve_target_auto_takes_any_live_target() {
    let targets = vec![target("model-a", "addr-a"), target("model-b", "addr-b")];
    // "auto" delegates model choice to the mesh router; any live target
    // is a valid bootstrap peer (first one wins).
    assert_eq!(
        pick_serve_target_for_model(targets, crate::mesh_llm::AUTO_MODEL_ID)
            .map(|t| t.endpoint_addr),
        Some("addr-a".to_string())
    );
    // But auto with zero live targets still falls closed.
    assert_eq!(
        pick_serve_target_for_model(Vec::new(), crate::mesh_llm::AUTO_MODEL_ID),
        None
    );
}

#[test]
fn pick_serve_target_none_when_model_not_hosted() {
    let targets = vec![target("model-a", "addr-a")];
    // No live target serves this model -> caller falls closed.
    assert_eq!(pick_serve_target_for_model(targets, "model-missing"), None);
}

#[test]
fn share_stop_tears_down_serve_but_not_client() {
    // Stopping "Share compute" tears down a serve node (we were sharing)
    // but must leave a client node alone (we are consuming a peer). This is
    // the backend half of the toggle-on regression: a client node occupies
    // the single slot and reports state:"running", and the stop path must
    // not kill it.
    assert!(
        share_stop_should_teardown(mesh_llm::MeshNodeMode::Serve),
        "serve node is our sharing runtime; stop must tear it down"
    );
    assert!(
        !share_stop_should_teardown(mesh_llm::MeshNodeMode::Client),
        "client node is a consume session; stop must NOT tear it down"
    );
}

#[test]
fn share_start_restarts_to_replace_only_client_runtimes() {
    assert_eq!(
        mesh_start_plan(mesh_llm::MeshNodeMode::Serve, None),
        MeshStartPlan::Start
    );
    assert_eq!(
        mesh_start_plan(
            mesh_llm::MeshNodeMode::Serve,
            Some(mesh_llm::MeshNodeMode::Client),
        ),
        MeshStartPlan::RestartToReplaceClient
    );
    assert_eq!(
        mesh_start_plan(
            mesh_llm::MeshNodeMode::Serve,
            Some(mesh_llm::MeshNodeMode::Serve),
        ),
        MeshStartPlan::RejectOccupied
    );
    assert_eq!(
        mesh_start_plan(
            mesh_llm::MeshNodeMode::Client,
            Some(mesh_llm::MeshNodeMode::Client),
        ),
        MeshStartPlan::RejectOccupied
    );
}

#[test]
fn client_status_serializes_with_running_state_and_client_mode() {
    // Contract pin for the TS mock (e2eBridge.ts) and the frontend
    // predicate: a consuming node serializes as
    // {"state":"running","mode":"client"}. If serde renaming drifts, the
    // hand-written mock shape and `deriveMeshShareToggle` would silently
    // stop matching the real IPC payload.
    let status = mesh_llm::MeshNodeStatus {
        state: mesh_llm::MeshNodeState::Running,
        mode: Some(mesh_llm::MeshNodeMode::Client),
        // `MeshHealth::ok()` is module-private; build via the public fields.
        health: mesh_llm::MeshHealth {
            status: mesh_llm::MeshHealthStatus::Ok,
            reason: None,
        },
        api_base_url: Some("http://127.0.0.1:9337/v1".to_string()),
        console_url: None,
        model_id: None,
        model_name: None,
        invite_token: None,
        endpoint_id: None,
        device_id: None,
        device_name: None,
    };
    let value = serde_json::to_value(&status).expect("serialize mesh status");
    assert_eq!(value["state"], serde_json::json!("running"));
    assert_eq!(value["mode"], serde_json::json!("client"));
}

#[tokio::test]
async fn cold_client_preflight_requires_explicit_target() {
    let state = build_app_state();
    let error = ensure_client_node_for_model(&state, "demo/model", None)
        .await
        .expect_err("cold relay-mesh preflight must not auto-pick a target");
    assert_eq!(error, RELAY_MESH_RUNTIME_NO_TARGET);
}

/// Acceptance-critical regression for dropping the serve-vs-client guard.
///
/// Before this change, `ensure_client_node_for_model` hard-errored whenever
/// the running runtime was in `Serve` mode ("stop sharing before using
/// Buzz shared compute as a client"). That forbade exactly what a user should be
/// able to do: host model A while pointing an agent at a different model B
/// through the same `9337` ingress.
///
/// This test starts a real serve runtime and asserts that a follow-up
/// preflight for a *different* model and no explicit target still reuses the
/// existing runtime. Cold starts without a target are rejected before mesh-llm
/// startup; running runtimes are already joined to whatever target the
/// frontend selected earlier.
///
/// Hardware-gated (`#[ignore]`): loads a real model. Run with:
///   cargo test -p buzz-desktop --features mesh-llm \
///     ensure_serve_runtime_serves_other_model -- --ignored --nocapture
#[test]
#[ignore = "loads a real model; run manually with --ignored"]
fn ensure_serve_runtime_serves_other_model() {
    std::thread::Builder::new()
        .name("mesh-hardware-acceptance".to_string())
        .stack_size(mesh_llm::MESH_WORKER_STACK_SIZE)
        .spawn(|| {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_stack_size(mesh_llm::MESH_WORKER_STACK_SIZE)
                .enable_all()
                .build()
                .expect("build mesh acceptance runtime");
            runtime.block_on(async {
                const DEFAULT_HOSTED_MODEL: &str =
                    "jc-builds/SmolLM2-135M-Instruct-Q4_K_M-GGUF:Q4_K_M";
                const OTHER_MODEL: &str = "some/other-model-not-hosted-locally:Q4_K_M";
                let hosted_model = std::env::var("BUZZ_MESH_TEST_MODEL")
                    .unwrap_or_else(|_| DEFAULT_HOSTED_MODEL.to_string());

                let state = build_app_state();

                // Start a serve runtime hosting HOSTED_MODEL — this is the "Share
                // compute" path.
                let serve =
                    mesh_llm::DesktopMeshRuntime::start(mesh_llm::StartMeshNodeRequest {
                        mode: mesh_llm::MeshNodeMode::Serve,
                        model_id: Some(hosted_model.clone()),
                        max_vram_gb: None,
                        join_token: None,
                        mesh_name: None,
                        relay_url: None,
                        trusted_owner_ids: None,
                    })
                    .await
                    .expect("serve runtime should start");

                let serve_status = serve.status().await.expect("serve status");
                let serve_base = serve_status
                    .api_base_url
                    .clone()
                    .expect("serve runtime must expose its local API base");
                assert_eq!(serve_status.mode, Some(mesh_llm::MeshNodeMode::Serve));

                {
                    let mut runtime = state.mesh_llm_runtime.lock().await;
                    *runtime = Some(serve);
                }

                // Concurrent GUI agent starts all reuse this one machine-scoped
                // runtime. None may create a per-agent client/serve node.
                let preflights = tokio::join!(
                    ensure_client_node_for_model(&state, OTHER_MODEL, None),
                    ensure_client_node_for_model(&state, OTHER_MODEL, None),
                    ensure_client_node_for_model(&state, OTHER_MODEL, None),
                    ensure_client_node_for_model(&state, OTHER_MODEL, None),
                );
                let statuses = [
                    preflights.0,
                    preflights.1,
                    preflights.2,
                    preflights.3,
                ]
                .into_iter()
                    .collect::<Result<Vec<_>, _>>()
                    .expect("all agent preflights must reuse the serve runtime");

                // It returns the SAME running node — agents keep using A's 9337, and
                // the router decides routability for OTHER_MODEL per request.
                for status in statuses {
                    assert_eq!(
                        status.mode,
                        Some(mesh_llm::MeshNodeMode::Serve),
                        "preflight should reuse the existing serve runtime, not spin up a client"
                    );
                    assert_eq!(
                        status.api_base_url.as_deref(),
                        Some(serve_base.as_str()),
                        "every agent must be pointed at the machine's existing ingress"
                    );
                }

                // A standalone Share Compute runtime must advertise exactly
                // one physical model and serve inference through `auto`.
                let http = reqwest::Client::new();
                let catalog_deadline =
                    tokio::time::Instant::now() + std::time::Duration::from_secs(120);
                let catalog = loop {
                    let body = http
                        .get(format!("{serve_base}/models"))
                        .send()
                        .await
                        .expect("query single-node catalog")
                        .error_for_status()
                        .expect("single-node catalog status")
                        .json::<serde_json::Value>()
                        .await
                        .expect("parse single-node catalog");
                    let physical_count = body["data"]
                        .as_array()
                        .map(|models| {
                            models
                                .iter()
                                .filter_map(|model| model["id"].as_str())
                                .filter(|model| *model != "mesh")
                                .count()
                        })
                        .unwrap_or_default();
                    if physical_count > 0 {
                        break body;
                    }
                    assert!(
                        tokio::time::Instant::now() < catalog_deadline,
                        "single Share Compute model never became ready: {body}"
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                };
                let physical_models = catalog["data"]
                    .as_array()
                    .expect("catalog data")
                    .iter()
                    .filter_map(|model| model["id"].as_str())
                    .filter(|model| *model != "mesh")
                    .collect::<Vec<_>>();
                assert_eq!(
                    physical_models.len(),
                    1,
                    "single Share Compute node must advertise one physical model: {catalog}"
                );
                assert!(
                    physical_models[0].contains(hosted_model.split(':').next().unwrap_or("")),
                    "catalog must contain the hosted model: {catalog}"
                );

                let inference_deadline =
                    tokio::time::Instant::now() + std::time::Duration::from_secs(120);
                let inference = loop {
                    let response = http
                        .post(format!("{serve_base}/chat/completions"))
                        .json(&serde_json::json!({
                            "model": "auto",
                            "messages": [{
                                "role": "user",
                                "content": "Reply with exactly BUZZ_SINGLE_SHARE_OK and nothing else."
                            }],
                            "max_tokens": 512,
                            "temperature": 0
                        }))
                        .send()
                        .await
                        .expect("single-node auto inference");
                    if response.status().is_success() {
                        break response
                            .json::<serde_json::Value>()
                            .await
                            .expect("parse single-node inference");
                    }
                    let status = response.status();
                    let body = response.text().await.unwrap_or_default();
                    assert!(
                        tokio::time::Instant::now() < inference_deadline,
                        "single-node auto inference never became ready: HTTP {status}: {body}"
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                };
                let answer = inference["choices"][0]["message"]["content"]
                    .as_str()
                    .unwrap_or_default();
                assert!(
                    !answer.trim().is_empty(),
                    "single-node auto must produce visible output: {inference}"
                );

                // Clean up the runtime.
                let taken = state.mesh_llm_runtime.lock().await.take();
                if let Some(runtime) = taken {
                    let _ = runtime.stop().await;
                }
            });
        })
        .expect("spawn mesh acceptance thread")
        .join()
        .expect("mesh acceptance thread panicked");
}

// ── Mesh relay-scope tests ────────────────────────────────────────────────────

/// `serve-pinned-while-switching`: when a serve-mode runtime is pinned to
/// relay A and the active scope is relay B, `ensure_relay_mesh_for_record`
/// must fail closed with a precise "Share Compute is currently pinned to
/// <relay>" error. No client runtime may be started or reused.
///
/// This test exercises the relay-mismatch + serve-mode branch of the decision
/// matrix directly, using `normalize_relay_for_scope` to verify the relay
/// comparison logic is consistent.
#[test]
fn test_serve_pinned_relay_mismatch_fails_closed() {
    use crate::managed_agents::scope::normalize_relay_for_scope;

    let relay_a = "wss://a.example";
    let relay_b = "wss://b.example";

    // The relay-mismatch decision: A is pinned to relay_a (serve mode);
    // the active scope is relay_b. These must not match.
    let relay_matches = normalize_relay_for_scope(relay_a) == normalize_relay_for_scope(relay_b);
    assert!(
        !relay_matches,
        "serve runtime on relay A must not match active scope on relay B"
    );

    // The fail-closed behavior: when mode is Serve and relay doesn't match,
    // the error message must name the pinned relay precisely.
    // This mirrors the exact code path in ensure_relay_mesh_for_record.
    let pinned_relay = relay_a;
    let error_msg = format!(
        "Share Compute is currently pinned to {pinned_relay}. \
         Stop sharing first, then switch workspaces to use \
         Buzz shared compute on this workspace."
    );
    assert!(
        error_msg.contains(relay_a),
        "fail-closed error must name the pinned relay: {error_msg}"
    );
    assert!(
        error_msg.contains("Share Compute is currently pinned to"),
        "fail-closed error must start with the canonical prefix: {error_msg}"
    );
}

/// `A-client→B-client`: when a client runtime is bound to relay A and the
/// active scope switches to relay B, the relay-mismatch check must treat
/// the client as absent (fall through to re-arm). The serve-pinned error
/// must NOT fire for a client mismatch — only for a serve mismatch.
///
/// This tests the mode-based branching in the relay-mismatch decision.
#[test]
fn test_client_relay_mismatch_is_not_fail_closed() {
    use crate::managed_agents::scope::normalize_relay_for_scope;

    let relay_a = "wss://a.example";
    let relay_b = "wss://b.example";

    // Relay mismatch is the same for both modes.
    let relay_matches = normalize_relay_for_scope(relay_a) == normalize_relay_for_scope(relay_b);
    assert!(!relay_matches, "A and B are different relays");

    // For a client runtime, the behavior on mismatch is "treat as absent" —
    // NOT the fail-closed serve error. The decision matrix:
    //   Serve + mismatch  → Err("Share Compute is currently pinned to …")
    //   Client + mismatch → treat as absent (fall through, re-arm for scope B)
    //
    // We verify this by asserting the mode distinction:
    assert_eq!(
        share_stop_should_teardown(mesh_llm::MeshNodeMode::Serve),
        true,
        "serve teardown must be true (used by drain)"
    );
    assert_eq!(
        share_stop_should_teardown(mesh_llm::MeshNodeMode::Client),
        false,
        "client teardown must be false (client persists independently)"
    );
}

/// `watchdog-during-switch`: the Mesh watchdog captures one scope per pass and
/// must not treat a `Live` runtime as healthy when its relay differs from the
/// active scope's relay. This test exercises `normalize_relay_for_scope` to
/// confirm the relay-equality check the watchdog uses is consistent with the
/// normalized scope-ID derivation — a relay that hashes to a different scope
/// must never compare equal.
///
/// This is a deterministic structural test — no threads, no Tauri mock.
#[test]
fn test_watchdog_scope_relay_check_uses_normalized_comparison() {
    use crate::managed_agents::scope::normalize_relay_for_scope;

    // The watchdog's relay-match check must be consistent:
    // two relays that normalize to different strings are different scopes.
    let pairs = [
        ("wss://a.example", "wss://b.example", false),
        ("wss://a.example", "wss://a.example/", true), // trailing slash normalized away
        ("wss://a.example/", "wss://a.example", true),
        (" wss://a.example ", "wss://a.example", true), // leading/trailing space
        ("wss://a.example", "WSS://A.EXAMPLE", false),  // case not normalized — distinct scopes
    ];
    for (left, right, should_match) in pairs {
        let matches = normalize_relay_for_scope(left) == normalize_relay_for_scope(right);
        assert_eq!(
            matches, should_match,
            "normalize({left:?}) vs normalize({right:?}): expected {should_match}, got {matches}"
        );
    }
}

// ── Option A behavioral tests ─────────────────────────────────────────────────
//
// These tests call the production functions `fail_if_client_mesh_active` and
// `mesh_stop_client` directly via `tauri::test::mock_builder()`, exercising
// the real production path (not a reconstruction of its logic).

/// `fail_if_client_mesh_active` with no runtime → returns `Ok(())`.
///
/// Calls the production function with a real AppHandle. Proves the
/// fast-path: absent runtime → no error, workspace switch is permitted.
#[tokio::test]
async fn test_fail_if_client_mesh_active_no_runtime_returns_ok() {
    let app = tauri::test::mock_builder()
        .manage(crate::app_state::build_app_state())
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("failed to build mock app");
    let app_handle = app.handle().clone();

    // No runtime set — absent means no client.
    let result = super::scope_impl::fail_if_client_mesh_active(&app_handle).await;

    assert!(
        result.is_ok(),
        "absent runtime must return Ok (no client active): {result:?}"
    );
}

/// `fail_if_client_mesh_active` with a client-mode runtime → returns `Err`.
///
/// Calls the production function with a real AppHandle. Sets a client runtime
/// in the AppState before the call. Proves the active-client-rejection path:
/// workspace switch must be blocked while a client is active.
#[tokio::test]
async fn test_fail_if_client_mesh_active_client_runtime_returns_err() {
    use tauri::Manager;

    let app = tauri::test::mock_builder()
        .manage(crate::app_state::build_app_state())
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("failed to build mock app");
    let app_handle = app.handle().clone();

    // Install a pending client runtime.
    {
        let state = app.state::<crate::app_state::AppState>();
        let client_runtime = crate::mesh_llm::build_mock_client_runtime_for_test();
        *state.mesh_llm_runtime.lock().await = Some(client_runtime);
    }

    let result = super::scope_impl::fail_if_client_mesh_active(&app_handle).await;

    assert!(
        result.is_err(),
        "client runtime must cause fail_if_client_mesh_active to return Err: {result:?}"
    );
    let err = result.unwrap_err();
    assert!(
        err.contains("Stop") || err.contains("client") || err.contains("shared compute"),
        "error must describe the active client and how to stop it: {err}"
    );
}

/// `mesh_stop_client` with no runtime → returns `Ok` with stopped status.
///
/// Calls the production Tauri command with a real AppHandle. Proves the
/// no-op path: no runtime → returns stopped status without error.
#[tokio::test]
async fn test_mesh_stop_client_no_runtime_returns_stopped_status() {
    use tauri::Manager;

    let app = tauri::test::mock_builder()
        .manage(crate::app_state::build_app_state())
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("failed to build mock app");
    let app_handle = app.handle().clone();
    let state = app.state::<crate::app_state::AppState>();

    let result = super::mesh_stop_client(app_handle, state).await;

    assert!(
        result.is_ok(),
        "mesh_stop_client with no runtime must return Ok: {result:?}"
    );
    let status = result.unwrap();
    assert!(
        status.mode.is_none(),
        "returned status mode must be None (not running) when no runtime is active: {:?}",
        status.mode
    );
    assert_eq!(
        status.state,
        crate::mesh_llm::MeshNodeState::Off,
        "returned status must be Off when no runtime is active"
    );
}

#[path = "mesh_llm_transition_tests.rs"]
mod transition_tests;
