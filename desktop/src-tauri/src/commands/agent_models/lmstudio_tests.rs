use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

use axum::{http::HeaderMap, routing::get, Json, Router};

use super::*;

async fn spawn_models_server(router: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind LM Studio test server");
    let address = listener.local_addr().expect("LM Studio test address");
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    format!("http://{address}")
}

fn loaded_catalog() -> serde_json::Value {
    serde_json::json!({
        "models": [{
            "type": "llm",
            "key": "google/gemma-4-26b-a4b",
            "max_context_length": 262144,
            "capabilities": {
                "vision": true,
                "trained_for_tool_use": true
            },
            "loaded_instances": [{
                "id": "gemma4-26b-official",
                "config": {"context_length": 65536, "parallel": 1}
            }]
        }]
    })
}

fn runtime_env(base_url: String) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("LM_STUDIO_BASE_URL".to_string(), base_url),
        ("LM_STUDIO_MCP_INTEGRATIONS".to_string(), "[]".to_string()),
    ])
}

#[test]
fn normalization_retains_loaded_runtime_context_and_parallelism() {
    let models = normalize_lmstudio_models(loaded_catalog()).expect("qualified catalog");
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].loaded_context_length, Some(65_536));
    assert_eq!(models[0].loaded_parallelism, Some(1));
    assert_eq!(
        models[0].loaded_instance_ids,
        ["gemma4-26b-official".to_string()]
    );
    assert_eq!(models[0].capabilities.as_ref().unwrap()["vision"], true);
    assert_eq!(
        models[0].capabilities.as_ref().unwrap()["trained_for_tool_use"],
        true
    );
}

#[tokio::test]
async fn readiness_warns_when_server_accepts_unauthenticated_probe_despite_stored_token() {
    let observed = Arc::new(Mutex::new(Vec::<Option<String>>::new()));
    let route_observed = observed.clone();
    let router = Router::new().route(
        "/api/v1/models",
        get(move |headers: HeaderMap| {
            let route_observed = route_observed.clone();
            async move {
                route_observed.lock().expect("observed headers lock").push(
                    headers
                        .get("authorization")
                        .and_then(|value| value.to_str().ok())
                        .map(str::to_string),
                );
                Json(loaded_catalog())
            }
        }),
    );
    let base_url = spawn_models_server(router).await;
    let runtime = known_acp_runtime("buzz-lmstudio-agent").expect("LM Studio runtime");

    let readiness = probe_lmstudio_readiness(
        runtime,
        &runtime_env(base_url),
        Some("google/gemma-4-26b-a4b".to_string()),
        || Some("stored-secret".to_string()),
    )
    .await;

    assert_eq!(readiness.status, LmStudioReadinessState::Ready);
    assert_eq!(
        readiness.security_warnings,
        [
            "LM Studio API authentication is not enabled.",
            "LM Studio listener exposure is unverified."
        ]
    );
    assert_eq!(
        *observed.lock().expect("observed headers lock"),
        vec![None],
        "the tokenless probe must run first and a successful probe must not send the token"
    );
}

#[tokio::test]
async fn readiness_reports_auth_required_when_tokenless_probe_is_rejected() {
    let router = Router::new().route(
        "/api/v1/models",
        get(|| async {
            (
                axum::http::StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error": "auth required"})),
            )
        }),
    );
    let base_url = spawn_models_server(router).await;
    let runtime = known_acp_runtime("buzz-lmstudio-agent").expect("LM Studio runtime");

    let readiness = probe_lmstudio_readiness(runtime, &runtime_env(base_url), None, || None).await;

    assert_eq!(readiness.status, LmStudioReadinessState::AuthRequired);
}

#[tokio::test]
async fn readiness_maps_a_valid_empty_catalog_to_no_loaded_model() {
    let router = Router::new().route(
        "/api/v1/models",
        get(|| async { Json(serde_json::json!({"models": []})) }),
    );
    let base_url = spawn_models_server(router).await;
    let runtime = known_acp_runtime("buzz-lmstudio-agent").expect("LM Studio runtime");

    let readiness = probe_lmstudio_readiness(runtime, &runtime_env(base_url), None, || None).await;

    assert_eq!(readiness.status, LmStudioReadinessState::NoLoadedModel);
}

#[tokio::test]
async fn readiness_uses_bearer_catalog_only_after_auth_enforcement_is_observed() {
    let observed = Arc::new(Mutex::new(Vec::<Option<String>>::new()));
    let route_observed = observed.clone();
    let router = Router::new().route(
        "/api/v1/models",
        get(move |headers: HeaderMap| {
            let route_observed = route_observed.clone();
            async move {
                let authorization = headers
                    .get("authorization")
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_string);
                route_observed
                    .lock()
                    .expect("observed headers lock")
                    .push(authorization.clone());
                if authorization.as_deref() == Some("Bearer stored-secret") {
                    (axum::http::StatusCode::OK, Json(loaded_catalog()))
                } else {
                    (
                        axum::http::StatusCode::FORBIDDEN,
                        Json(serde_json::json!({"error": "auth required"})),
                    )
                }
            }
        }),
    );
    let base_url = spawn_models_server(router).await;
    let runtime = known_acp_runtime("buzz-lmstudio-agent").expect("LM Studio runtime");

    let readiness = probe_lmstudio_readiness(
        runtime,
        &runtime_env(base_url),
        Some("google/gemma-4-26b-a4b".to_string()),
        || Some("stored-secret".to_string()),
    )
    .await;

    assert_eq!(readiness.status, LmStudioReadinessState::Ready);
    assert_eq!(
        readiness.security_warnings,
        ["LM Studio listener exposure is unverified."]
    );
    assert_eq!(
        *observed.lock().expect("observed headers lock"),
        vec![None, Some("Bearer stored-secret".to_string())]
    );
}

#[tokio::test]
async fn native_discovery_returns_successful_empty_and_distinguishes_invalid_schema() {
    let empty_router = Router::new().route(
        "/api/v1/models",
        get(|| async { Json(serde_json::json!({"models": []})) }),
    );
    let empty_base_url = spawn_models_server(empty_router).await;
    let response =
        discover_lmstudio_native_models("buzz-lmstudio-agent", &runtime_env(empty_base_url), None)
            .await
            .expect("valid empty discovery")
            .expect("native discovery response");
    assert!(response.models.is_empty());
    assert!(response.selected_model.is_none());
    assert!(response.agent_default_model.is_none());

    let malformed_router = Router::new().route(
        "/api/v1/models",
        get(|| async { Json(serde_json::json!({"models": "not-an-array"})) }),
    );
    let malformed_base_url = spawn_models_server(malformed_router).await;
    let malformed = discover_lmstudio_native_models(
        "buzz-lmstudio-agent",
        &runtime_env(malformed_base_url),
        None,
    )
    .await
    .expect_err("malformed native schema");
    assert!(
        malformed.contains("models response parse failed"),
        "{malformed}"
    );

    let unused_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("reserve unused port");
    let unused_address = unused_listener.local_addr().expect("unused address");
    drop(unused_listener);
    let transport = discover_lmstudio_native_models(
        "buzz-lmstudio-agent",
        &runtime_env(format!("http://{unused_address}")),
        None,
    )
    .await
    .expect_err("closed endpoint must be a transport failure");
    assert!(
        !transport.contains("models response parse failed"),
        "{transport}"
    );
    assert_ne!(malformed, transport);
}

async fn assert_discovery_keeps_stored_token_off_unauthenticated_server(
    selected_model: Option<String>,
) {
    let observed = Arc::new(Mutex::new(Vec::<Option<String>>::new()));
    let token_loaded = Arc::new(AtomicBool::new(false));
    let route_observed = observed.clone();
    let router = Router::new().route(
        "/api/v1/models",
        get(move |headers: HeaderMap| {
            let route_observed = route_observed.clone();
            async move {
                route_observed.lock().expect("observed headers lock").push(
                    headers
                        .get("authorization")
                        .and_then(|value| value.to_str().ok())
                        .map(str::to_string),
                );
                Json(loaded_catalog())
            }
        }),
    );
    let base_url = spawn_models_server(router).await;

    let loader_observed = token_loaded.clone();
    let response = discover_lmstudio_native_models_with_token_loader(
        "buzz-lmstudio-agent",
        &runtime_env(base_url),
        selected_model,
        move || {
            loader_observed.store(true, Ordering::SeqCst);
            Some("stored-secret".to_string())
        },
    )
    .await
    .expect("tokenless discovery")
    .expect("native discovery response");

    assert_eq!(response.models.len(), 1);
    assert_eq!(
        *observed.lock().expect("observed headers lock"),
        vec![None],
        "a tokenless 200 must never receive the stored bearer"
    );
    assert!(
        !token_loaded.load(Ordering::SeqCst),
        "a tokenless 200 must not even load the Keychain token"
    );
}

#[tokio::test]
async fn saved_and_unsaved_discovery_are_tokenless_first() {
    assert_discovery_keeps_stored_token_off_unauthenticated_server(Some("qwen/test".to_string()))
        .await;
    assert_discovery_keeps_stored_token_off_unauthenticated_server(None).await;
}

#[tokio::test]
async fn discovery_retries_once_with_bearer_only_after_auth_enforcement() {
    let observed = Arc::new(Mutex::new(Vec::<Option<String>>::new()));
    let route_observed = observed.clone();
    let router = Router::new().route(
        "/api/v1/models",
        get(move |headers: HeaderMap| {
            let route_observed = route_observed.clone();
            async move {
                let authorization = headers
                    .get("authorization")
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_string);
                route_observed
                    .lock()
                    .expect("observed headers lock")
                    .push(authorization.clone());
                if authorization.as_deref() == Some("Bearer stored-secret") {
                    (axum::http::StatusCode::OK, Json(loaded_catalog()))
                } else {
                    (
                        axum::http::StatusCode::UNAUTHORIZED,
                        Json(serde_json::json!({"secret": "must-not-surface"})),
                    )
                }
            }
        }),
    );
    let base_url = spawn_models_server(router).await;

    let response = discover_lmstudio_native_models_with_token_loader(
        "buzz-lmstudio-agent",
        &runtime_env(base_url),
        None,
        || Some("stored-secret".to_string()),
    )
    .await
    .expect("authenticated discovery")
    .expect("native discovery response");

    assert_eq!(response.models.len(), 1);
    assert_eq!(
        *observed.lock().expect("observed headers lock"),
        vec![None, Some("Bearer stored-secret".to_string())]
    );
}

#[tokio::test]
async fn discovery_does_not_retry_server_failure_or_leak_auth_bodies() {
    let observed = Arc::new(Mutex::new(Vec::<Option<String>>::new()));
    let token_loaded = Arc::new(AtomicBool::new(false));
    let route_observed = observed.clone();
    let router = Router::new().route(
        "/api/v1/models",
        get(move |headers: HeaderMap| {
            let route_observed = route_observed.clone();
            async move {
                route_observed.lock().expect("observed headers lock").push(
                    headers
                        .get("authorization")
                        .and_then(|value| value.to_str().ok())
                        .map(str::to_string),
                );
                (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    "stored-secret\nmust-not-surface",
                )
            }
        }),
    );
    let base_url = spawn_models_server(router).await;

    let loader_observed = token_loaded.clone();
    let error = discover_lmstudio_native_models_with_token_loader(
        "buzz-lmstudio-agent",
        &runtime_env(base_url),
        None,
        move || {
            loader_observed.store(true, Ordering::SeqCst);
            Some("stored-secret".to_string())
        },
    )
    .await
    .expect_err("server failure");

    assert_eq!(*observed.lock().expect("observed headers lock"), vec![None]);
    assert!(!error.contains("stored-secret"), "{error}");
    assert!(!error.contains("must-not-surface"), "{error}");
    assert!(!token_loaded.load(Ordering::SeqCst));
}

#[tokio::test]
async fn discovery_missing_or_rejected_token_is_fixed_auth_error() {
    async fn run(token: Option<String>) -> (String, Vec<Option<String>>) {
        let observed = Arc::new(Mutex::new(Vec::<Option<String>>::new()));
        let route_observed = observed.clone();
        let router = Router::new().route(
            "/api/v1/models",
            get(move |headers: HeaderMap| {
                let route_observed = route_observed.clone();
                async move {
                    route_observed.lock().expect("observed headers lock").push(
                        headers
                            .get("authorization")
                            .and_then(|value| value.to_str().ok())
                            .map(str::to_string),
                    );
                    (axum::http::StatusCode::UNAUTHORIZED, "secret response body")
                }
            }),
        );
        let base_url = spawn_models_server(router).await;
        let error = discover_lmstudio_native_models_with_token_loader(
            "buzz-lmstudio-agent",
            &runtime_env(base_url),
            None,
            move || token,
        )
        .await
        .expect_err("authentication must fail");
        let headers = observed.lock().expect("observed headers lock").clone();
        (error, headers)
    }

    let (missing_error, missing_headers) = run(None).await;
    assert_eq!(missing_error, "llm auth: LM Studio authentication required");
    assert_eq!(missing_headers, vec![None]);

    let (rejected_error, rejected_headers) = run(Some("rejected-secret".to_string())).await;
    assert_eq!(
        rejected_error,
        "llm auth: LM Studio authentication required"
    );
    assert_eq!(
        rejected_headers,
        vec![None, Some("Bearer rejected-secret".to_string())]
    );
    assert!(!rejected_error.contains("secret response body"));
    assert!(!rejected_error.contains("rejected-secret"));
}
