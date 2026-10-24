use super::*;
use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post, put},
    Json, Router,
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

#[derive(Default)]
struct Data {
    blobs: HashMap<String, Vec<u8>>,
    profiles: Vec<nostr::Event>,
    requests: Vec<(String, nostr::Event)>,
    fail_upload: bool,
    reject_profile: bool,
    redirect: Option<String>,
    upload_url: Option<String>,
    pause_read: Option<(Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>)>,
}
#[derive(Clone)]
struct ServerState {
    data: Arc<Mutex<Data>>,
    base: String,
}
struct Server {
    state: ServerState,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn auth(headers: &HeaderMap, method: &str, state: &ServerState) {
    let encoded = headers["authorization"]
        .to_str()
        .unwrap()
        .strip_prefix("Nostr ")
        .unwrap();
    let bytes = URL_SAFE_NO_PAD
        .decode(encoded)
        .or_else(|_| base64::engine::general_purpose::STANDARD.decode(encoded))
        .unwrap();
    let event = nostr::Event::from_json(bytes).unwrap();
    event.verify().unwrap();
    if event.kind.as_u16() == 24242 {
        let authority = state.base.strip_prefix("http://").unwrap();
        assert!(event
            .tags
            .iter()
            .any(|tag| tag.as_slice() == ["server", authority]));
    }
    state
        .data
        .lock()
        .unwrap()
        .requests
        .push((method.into(), event));
}
async fn read(
    State(state): State<ServerState>,
    Path(path): Path<String>,
    headers: HeaderMap,
) -> Response {
    auth(&headers, "read", &state);
    let pause = state.data.lock().unwrap().pause_read.clone();
    if let Some((entered, release)) = pause {
        entered.notify_one();
        release.notified().await;
    }
    let data = state.data.lock().unwrap();
    if let Some(url) = &data.redirect {
        return (StatusCode::TEMPORARY_REDIRECT, [("location", url.clone())]).into_response();
    }
    match data.blobs.get(&path) {
        Some(bytes) => ([("content-type", "image/png")], bytes.clone()).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
async fn upload(State(state): State<ServerState>, headers: HeaderMap, bytes: Bytes) -> Response {
    auth(&headers, "upload", &state);
    let mut data = state.data.lock().unwrap();
    if data.fail_upload {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    let hash = hex::encode(Sha256::digest(&bytes));
    assert_eq!(headers["x-sha-256"].to_str().unwrap(), hash);
    data.blobs.insert(format!("{hash}.png"), bytes.to_vec());
    Json(serde_json::json!({
        "url": data.upload_url.clone().unwrap_or_else(|| format!("{}/media/{hash}.png", state.base)),
        "sha256": hash, "size": bytes.len(), "type": "image/png", "uploaded": 1
    })).into_response()
}
async fn publish(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Json(event): Json<nostr::Event>,
) -> Response {
    auth(&headers, "profile", &state);
    event.verify().unwrap();
    let mut data = state.data.lock().unwrap();
    let accepted = !data.reject_profile;
    let id = event.id.to_hex();
    if accepted {
        data.profiles.push(event);
    }
    Json(serde_json::json!({"event_id": id, "accepted": accepted, "message": "test verdict"}))
        .into_response()
}
async fn query(State(state): State<ServerState>, headers: HeaderMap) -> Response {
    auth(&headers, "query", &state);
    let data = state.data.lock().unwrap();
    if let Some(url) = &data.redirect {
        return (StatusCode::TEMPORARY_REDIRECT, [("location", url.clone())]).into_response();
    }
    Json(
        data.profiles
            .last()
            .into_iter()
            .cloned()
            .collect::<Vec<_>>(),
    )
    .into_response()
}
async fn server() -> Server {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let state = ServerState {
        data: Arc::new(Mutex::new(Data::default())),
        base: format!("http://{}", listener.local_addr().unwrap()),
    };
    let app = Router::new()
        .route("/media/{path}", get(read))
        .route("/upload", put(upload))
        .route("/events", post(publish))
        .route("/query", post(query))
        .with_state(state.clone());
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Server { state, task }
}
fn image(seed: u8) -> Vec<u8> {
    let image = image::RgbaImage::from_pixel(2, 2, image::Rgba([seed, 0, 0, 255]));
    let mut output = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut output, image::ImageFormat::Png)
        .unwrap();
    output.into_inner()
}
fn seed(server: &Server, bytes: Vec<u8>) -> String {
    let path = format!("{}.png", hex::encode(Sha256::digest(&bytes)));
    server
        .state
        .data
        .lock()
        .unwrap()
        .blobs
        .insert(path.clone(), bytes);
    format!("{}/media/{path}", server.state.base)
}
fn state(servers: &[&Server]) -> AppState {
    let state = crate::app_state::build_app_state();
    *state.agent_avatar_communities.lock().unwrap() =
        servers.iter().map(|s| s.state.base.clone()).collect();
    state
}
fn picture(server: &Server) -> String {
    let data = server.state.data.lock().unwrap();
    let event = data.profiles.last().unwrap();
    serde_json::from_str::<serde_json::Value>(&event.content).unwrap()["picture"]
        .as_str()
        .unwrap()
        .into()
}
async fn sync(state: &AppState, target: &Server, keys: &Keys, avatar: &str) -> Result<(), String> {
    crate::relay::sync_managed_agent_profile(
        state,
        &target.state.base,
        keys,
        "Carl",
        Some(avatar),
        None,
        None,
    )
    .await
}

#[tokio::test]
async fn two_communities_add_edit_restart_keeps_local_images_and_fixed_agent_signer() {
    let a = server().await;
    let b = server().await;
    let c = server().await;
    let state = state(&[&a, &b]);
    let keys = Keys::generate();
    let original = seed(&a, image(1));
    sync(&state, &a, &keys, &original).await.unwrap();
    sync(&state, &b, &keys, &original).await.unwrap();
    assert_eq!(picture(&b), original.replace(&a.state.base, &b.state.base));
    assert_eq!(picture(&a), original);
    let edited = seed(&b, image(2));
    sync(&state, &b, &keys, &edited).await.unwrap();
    // A pending A task must not use a newly active C or its owner signer.
    *state.relay_url_override.lock().unwrap() = Some(c.state.base.clone());
    *state.keys.lock().unwrap() = Keys::generate();
    sync(&state, &a, &keys, &edited).await.unwrap();
    let projected = edited.replace(&b.state.base, &a.state.base);
    assert_eq!(picture(&a), projected);
    assert_eq!(picture(&b), edited);
    // The copied bytes survive source loss; subsequent reconciliation only HEADs A.
    b.state.data.lock().unwrap().blobs.clear();
    sync(&state, &a, &keys, &edited).await.unwrap();
    assert_eq!(picture(&a), projected);
    assert!(c.state.data.lock().unwrap().requests.is_empty());
    for server in [&a, &b] {
        let data = server.state.data.lock().unwrap();
        assert!(data
            .requests
            .iter()
            .all(|(_, event)| event.pubkey == keys.public_key()));
    }
    let path = projected.split("/media/").nth(1).unwrap();
    assert_eq!(a.state.data.lock().unwrap().blobs[path], image(2));
}

#[tokio::test]
async fn failed_transfer_or_profile_rejection_preserves_previous_picture_and_retry_succeeds() {
    let a = server().await;
    let b = server().await;
    let state = state(&[&a, &b]);
    let keys = Keys::generate();
    let old = seed(&b, image(1));
    sync(&state, &b, &keys, &old).await.unwrap();
    let new = seed(&a, image(2));
    b.state.data.lock().unwrap().fail_upload = true;
    assert!(sync(&state, &b, &keys, &new).await.is_err());
    assert_eq!(picture(&b), old);
    b.state.data.lock().unwrap().fail_upload = false;
    b.state.data.lock().unwrap().reject_profile = true;
    assert!(sync(&state, &b, &keys, &new)
        .await
        .unwrap_err()
        .contains("rejected agent profile"));
    assert_eq!(picture(&b), old);
    b.state.data.lock().unwrap().reject_profile = false;
    sync(&state, &b, &keys, &new).await.unwrap();
    assert_eq!(picture(&b), new.replace(&a.state.base, &b.state.base));
}

#[tokio::test]
async fn untrusted_origin_redirect_hash_and_descriptor_mismatch_cannot_publish() {
    let a = server().await;
    let b = server().await;
    let trap = server().await;
    let state = state(&[&a, &b]);
    let keys = Keys::generate();
    let avatar = seed(&a, image(1));
    let unknown = avatar.replace(&a.state.base, &trap.state.base);
    sync(&state, &b, &keys, &unknown).await.unwrap();
    b.state.data.lock().unwrap().profiles.clear();
    assert!(trap.state.data.lock().unwrap().requests.is_empty());
    a.state.data.lock().unwrap().redirect = Some(unknown);
    assert!(sync(&state, &b, &keys, &avatar).await.is_err());
    assert!(trap.state.data.lock().unwrap().requests.is_empty());
    a.state.data.lock().unwrap().redirect = None;
    let path = avatar.split("/media/").nth(1).unwrap();
    a.state
        .data
        .lock()
        .unwrap()
        .blobs
        .insert(path.into(), image(2));
    assert!(sync(&state, &b, &keys, &avatar)
        .await
        .unwrap_err()
        .contains("content hash"));
    a.state
        .data
        .lock()
        .unwrap()
        .blobs
        .insert(path.into(), image(1));
    b.state.data.lock().unwrap().upload_url = Some(avatar.clone());
    assert!(sync(&state, &b, &keys, &avatar)
        .await
        .unwrap_err()
        .contains("mismatched"));
    assert!(b.state.data.lock().unwrap().profiles.is_empty());
}

#[tokio::test]
async fn non_media_external_and_inline_avatars_are_not_fetched() {
    let b = server().await;
    let state = state(&[&b]);
    let keys = Keys::generate();
    for avatar in [
        "https://example.org/avatar.png",
        "https://example.org/media/avatar.png",
        "data:image/svg+xml,%3Csvg/%3E",
    ] {
        sync(&state, &b, &keys, avatar).await.unwrap();
        assert_eq!(picture(&b), avatar);
    }
    assert!(b
        .state
        .data
        .lock()
        .unwrap()
        .requests
        .iter()
        .all(|(kind, _)| kind == "profile"));
}

#[test]
fn trusted_community_and_original_image_validation() {
    for value in [
        "wss://a.example",
        "https://a.example/",
        "ws://localhost:1234",
    ] {
        assert!(community_base(value).is_ok(), "{value}");
    }
    for value in [
        "http://a.example",
        "https://user@a.example",
        "https://a.example/path",
        "https://a.example?query",
    ] {
        assert!(community_base(value).is_err(), "{value}");
    }
    let hash = "a".repeat(64);
    for suffix in [".png", ""] {
        assert!(media_hash(
            &url::Url::parse(&format!("https://a.example/media/{hash}{suffix}")).unwrap()
        )
        .is_ok());
    }
    for suffix in [".thumb.jpg", ".png?query", ".png#fragment", "/other"] {
        assert!(media_hash(
            &url::Url::parse(&format!("https://a.example/media/{hash}{suffix}")).unwrap()
        )
        .is_err());
    }
}

#[tokio::test]
async fn removing_source_community_revokes_authenticated_fetches() {
    let a = server().await;
    let b = server().await;
    let state = state(&[&a, &b]);
    let keys = Keys::generate();
    let avatar = seed(&a, image(1));
    *state.agent_avatar_communities.lock().unwrap() = vec![b.state.base.clone()];
    // Unknown public hosts retain passthrough compatibility, but never get an
    // authenticated fetch after removal from the configured list.
    sync(&state, &b, &keys, &avatar).await.unwrap();
    assert!(a.state.data.lock().unwrap().requests.is_empty());
    assert_eq!(picture(&b), avatar, "never rewrite an arbitrary public URL");
}

#[tokio::test]
async fn source_removed_while_target_head_pending_cannot_start_authenticated_get() {
    let a = server().await;
    let b = server().await;
    let state = Arc::new(state(&[&a, &b]));
    let keys = Keys::generate();
    let avatar = seed(&a, image(1));
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    b.state.data.lock().unwrap().pause_read = Some((entered.clone(), release.clone()));
    let task = tokio::spawn({
        let state = state.clone();
        let target = b.state.base.clone();
        async move { localize_avatar(&state, &target, &keys, Some(&avatar), None).await }
    });
    tokio::time::timeout(TIMEOUT, entered.notified())
        .await
        .unwrap();
    *state.agent_avatar_communities.lock().unwrap() = vec![b.state.base.clone()];
    release.notify_one();
    assert!(task
        .await
        .unwrap()
        .unwrap_err()
        .contains("removed during transfer"));
    assert!(a.state.data.lock().unwrap().requests.is_empty());
    assert!(b.state.data.lock().unwrap().profiles.is_empty());
}

#[tokio::test]
async fn startup_reconcile_compares_local_projection_retries_saved_edit_and_honors_opt_out() {
    use crate::commands::{reconcile_profile_at, ProfileReconcileData, ProfileReconcileOutcome};
    use nostr::ToBech32;
    let a = server().await;
    let b = server().await;
    let state = state(&[&a, &b]);
    let keys = Keys::generate();
    let source = seed(&a, image(1));
    let mut data = ProfileReconcileData {
        private_key_nsec: keys.secret_key().to_bech32().unwrap(),
        name: "Carl".into(),
        relay_url: a.state.base.clone(),
        target_relay_url: Some(b.state.base.clone()),
        avatar_url: Some(source.clone()),
        auth_tag: None,
        pubkey: keys.public_key().to_hex(),
        agent_command: "goose".into(),
        persona_id: None,
        about: None,
    };
    async fn run(
        state: &AppState,
        target: &str,
        data: &ProfileReconcileData,
        existing: Option<crate::relay::AgentProfileInfo>,
    ) -> Result<ProfileReconcileOutcome, String> {
        reconcile_profile_at(
            state,
            target,
            data,
            data.avatar_url.as_deref(),
            existing.as_ref(),
        )
        .await
    }
    assert_eq!(
        run(&state, &b.state.base, &data, None).await.unwrap(),
        ProfileReconcileOutcome::Reconciled
    );
    let first = picture(&b);
    let existing = crate::relay::AgentProfileInfo {
        display_name: Some("Carl".into()),
        picture: Some(first.clone()),
        about: None,
    };
    run(&state, &b.state.base, &data, Some(existing.clone()))
        .await
        .unwrap();
    assert_eq!(
        b.state.data.lock().unwrap().profiles.len(),
        1,
        "local projection must not cause perpetual republish"
    );
    data.avatar_url = Some(seed(&a, image(2)));
    b.state.data.lock().unwrap().fail_upload = true;
    assert!(run(&state, &b.state.base, &data, Some(existing.clone()))
        .await
        .is_err());
    assert_eq!(picture(&b), first);
    b.state.data.lock().unwrap().fail_upload = false;
    b.state.data.lock().unwrap().reject_profile = true;
    assert!(run(&state, &b.state.base, &data, Some(existing.clone()))
        .await
        .is_err());
    assert_eq!(picture(&b), first);
    b.state.data.lock().unwrap().reject_profile = false;
    run(&state, &b.state.base, &data, Some(existing.clone()))
        .await
        .unwrap();
    let second = picture(&b);
    assert_ne!(second, first);
    // Removing A revokes reads, not the identical copy already published in B.
    *state.agent_avatar_communities.lock().unwrap() = vec![b.state.base.clone()];
    let source_requests = a.state.data.lock().unwrap().requests.len();
    data.name = "Renamed Carl".into();
    data.about = Some("New description".into());
    run(
        &state,
        &b.state.base,
        &data,
        Some(crate::relay::AgentProfileInfo {
            picture: Some(second.clone()),
            ..existing.clone()
        }),
    )
    .await
    .unwrap();
    assert_eq!(picture(&b), second);
    // Direct rename/persona-about writers use this same production writer.
    crate::relay::sync_managed_agent_profile(
        &state,
        &b.state.base,
        &keys,
        "Another name",
        data.avatar_url.as_deref(),
        Some("Another about"),
        None,
    )
    .await
    .unwrap();
    assert_eq!(picture(&b), second);
    assert_eq!(a.state.data.lock().unwrap().requests.len(), source_requests);
    state
        .managed_agent_profile_reconcile_enabled()
        .store(false, std::sync::atomic::Ordering::Release);
    let requests = b.state.data.lock().unwrap().requests.len();
    assert_eq!(
        run(&state, &b.state.base, &data, Some(existing))
            .await
            .unwrap(),
        ProfileReconcileOutcome::SkippedDisabled
    );
    assert_eq!(b.state.data.lock().unwrap().requests.len(), requests);
}

#[tokio::test]
async fn oversized_and_non_image_sources_do_not_upload_or_replace_profile() {
    let a = server().await;
    let b = server().await;
    let state = state(&[&a, &b]);
    let keys = Keys::generate();
    for bytes in [vec![0u8; MAX_AVATAR_BYTES + 1], b"not an image".to_vec()] {
        let avatar = seed(&a, bytes);
        assert!(sync(&state, &b, &keys, &avatar).await.is_err());
    }
    let data = b.state.data.lock().unwrap();
    assert!(data.profiles.is_empty());
    assert!(data.requests.iter().all(|(method, _)| method != "upload"));
}

#[tokio::test]
async fn removed_source_projection_query_uses_fixed_signer_and_cannot_redirect() {
    let a = server().await;
    let b = server().await;
    let trap = server().await;
    let state = state(&[&a, &b]);
    let keys = Keys::generate();
    let avatar = seed(&a, image(1));
    sync(&state, &b, &keys, &avatar).await.unwrap();
    let projected = picture(&b);
    *state.agent_avatar_communities.lock().unwrap() = vec![b.state.base.clone()];
    *state.relay_url_override.lock().unwrap() = Some(trap.state.base.clone());
    *state.keys.lock().unwrap() = Keys::generate();
    sync(&state, &b, &keys, &avatar).await.unwrap();
    assert_eq!(picture(&b), projected);
    assert!(b
        .state
        .data
        .lock()
        .unwrap()
        .requests
        .iter()
        .all(|(_, event)| event.pubkey == keys.public_key()));
    b.state.data.lock().unwrap().redirect = Some(format!("{}/query", trap.state.base));
    assert!(sync(&state, &b, &keys, &avatar).await.is_err());
    assert!(trap.state.data.lock().unwrap().requests.is_empty());
    assert_eq!(picture(&b), projected);
}
