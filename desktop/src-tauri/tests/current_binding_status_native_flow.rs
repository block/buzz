//! Real loopback transport coverage for the native current-binding projection.
//!
//! This target composes the production native WebSocket/session manager with a synthetic,
//! loopback-only relay. The test owns neither a projection fold nor a browser-state fixture:
//! every trace value comes back through the production projection channel and getter.

#[path = "../src/client_binding_status_session.rs"]
mod client_binding_status_session;

mod app_state {
    use nostr::Keys;

    pub(crate) struct AppState {
        keys: Keys,
        relay_url: String,
    }

    impl AppState {
        pub(crate) fn synthetic(keys: Keys, relay_url: String) -> Self {
            Self { keys, relay_url }
        }

        pub(crate) fn signing_keys(&self) -> Result<Keys, String> {
            Ok(self.keys.clone())
        }

        pub(crate) fn relay_url(&self) -> &str {
            &self.relay_url
        }
    }
}

mod relay {
    pub(crate) fn relay_ws_url_with_override(state: &crate::app_state::AppState) -> String {
        state.relay_url().to_owned()
    }
}

mod egress_guard {
    pub(crate) fn assert_no_key_backup(_: &str, _: &str) -> Result<(), String> {
        Ok(())
    }

    pub(crate) fn assert_no_key_backup_bytes(_: &[u8], _: &str) -> Result<(), String> {
        Ok(())
    }
}

#[allow(dead_code)]
mod native_websocket {
    include!("../src/native_websocket.rs");

    pub(super) async fn connect_status_for_test(
        manager: &WebSocketManager,
        state: &crate::app_state::AppState,
        url: String,
        on_message: Channel<serde_json::Value>,
        on_projection: Channel<serde_json::Value>,
    ) -> Result<Id, String> {
        connect_internal(manager, state, url, on_message, Some(on_projection)).await
    }

    pub(super) async fn current_projection_for_test(
        manager: &WebSocketManager,
    ) -> Option<crate::client_binding_status_session::CurrentProjection> {
        manager.projection.lock().await.current.clone()
    }

    pub(super) async fn connection_present_for_test(manager: &WebSocketManager, id: Id) -> bool {
        manager.connections.lock().await.contains_key(&id)
    }

    pub(super) fn unix_now_for_test() -> u64 {
        unix_now()
    }
}

use std::{
    env,
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

use buzz_core_pkg::{
    client_binding_bootstrap::{
        ClientBindingBootstrapInputV1, ClientBindingEpoch, ClientBindingScopeV1,
        CLIENT_BINDING_BOOTSTRAP_SUB_ID, CLIENT_BINDING_SCOPE_TAG, CLIENT_BINDING_STATUS_SUB_ID,
    },
    client_binding_status::ClientBindingStatusInputV1,
    kind::{KIND_CLIENT_BINDING_STATUS, KIND_USER_TRUSTED_ASSERTION},
    CommunityId,
};
use client_binding_status_session::CurrentProjection;
use futures_util::{SinkExt, StreamExt};
use native_websocket::{WebSocketManager, WebSocketMessage};
use nostr::{Event, EventBuilder, JsonUtil, Keys, Kind, PublicKey, Tag, Timestamp};
use serde::Serialize;
use serde_json::json;
use tauri::ipc::{Channel, InvokeResponseBody};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_tungstenite::{accept_async, tungstenite::Message, WebSocketStream};
use uuid::Uuid;

const RECEIVE_TIMEOUT: Duration = Duration::from_secs(2);
const ORDINARY_SUB_ID: &str = "synthetic-ordinary-events";

type RelaySocket = WebSocketStream<tokio::net::TcpStream>;

#[derive(Serialize)]
struct ProjectionTrace {
    version: u64,
    steps: Vec<TraceStep>,
}

#[derive(Serialize)]
struct TraceStep {
    case: &'static str,
    // This is the production DTO, not a test-owned projection lookalike.
    projection: Option<CurrentProjection>,
}

impl ProjectionTrace {
    fn new() -> Self {
        Self {
            version: 1,
            steps: Vec::new(),
        }
    }

    async fn record(&mut self, case: &'static str, flow: &NativeFlow) {
        self.steps.push(TraceStep {
            case,
            projection: flow.projection().await,
        });
    }

    fn assert_contract_and_export_if_requested(&self) {
        let value = serde_json::to_value(self).expect("production projection trace serializes");
        let steps = value["steps"].as_array().expect("trace steps are an array");
        for step in steps {
            if let Some(projection) = step["projection"].as_object() {
                let mut keys = projection.keys().map(String::as_str).collect::<Vec<_>>();
                keys.sort_unstable();
                assert_eq!(
                    keys,
                    ["connectionEpoch", "eventAuthorPubkey", "freshUntil"],
                    "trace projection must remain the production current-only DTO",
                );
            }
        }

        let Some(raw_path) = env::var_os("BUZZ_J3C_PROJECTION_TRACE_OUT") else {
            return;
        };
        let path = PathBuf::from(raw_path);
        assert!(
            path.is_absolute(),
            "BUZZ_J3C_PROJECTION_TRACE_OUT must be an absolute test-artifact path"
        );
        let parent = path
            .parent()
            .expect("trace output must have a parent directory");
        std::fs::create_dir_all(parent).expect("create projection trace directory");
        let mut bytes = serde_json::to_vec_pretty(self).expect("serialize projection trace");
        bytes.push(b'\n');
        std::fs::write(&path, bytes).expect("write projection trace");
    }
}

struct NativeFlow {
    relay_socket: RelaySocket,
    manager: WebSocketManager,
    id: u32,
    raw_deliveries: Arc<AtomicUsize>,
    projection_deliveries: Arc<AtomicUsize>,
    epoch: ClientBindingEpoch,
}

impl NativeFlow {
    async fn connect(relay_keys: &Keys, author_keys: &Keys) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind synthetic relay to an OS-assigned loopback port");
        let address = listener.local_addr().expect("read synthetic relay address");
        assert!(address.ip().is_loopback());
        assert_ne!(address.port(), 0);

        let relay_pubkey = relay_keys.public_key();
        let challenge = Uuid::new_v4().to_string();
        let server_challenge = challenge.clone();
        let relay = tokio::spawn(async move {
            let (mut nip11, peer) = listener.accept().await.expect("accept NIP-11 client");
            assert!(peer.ip().is_loopback());
            let mut request = [0_u8; 4096];
            let read = nip11.read(&mut request).await.expect("read NIP-11 request");
            assert!(String::from_utf8_lossy(&request[..read]).starts_with("GET / HTTP/1.1"));
            let body = json!({ "self": relay_pubkey.to_hex() }).to_string();
            nip11
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: application/nostr+json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                        body.len(), body
                    )
                    .as_bytes(),
                )
                .await
                .expect("write NIP-11 identity");
            let (stream, peer) = listener.accept().await.expect("accept native WebSocket");
            assert!(peer.ip().is_loopback());
            let mut socket = accept_async(stream)
                .await
                .expect("accept WebSocket upgrade");
            socket
                .send(Message::Text(
                    json!(["AUTH", server_challenge]).to_string().into(),
                ))
                .await
                .expect("send NIP-42 challenge");
            socket
        });
        let relay_url = format!("ws://{address}/");
        let state = app_state::AppState::synthetic(author_keys.clone(), relay_url.clone());
        let manager = WebSocketManager::default();
        let raw_deliveries = Arc::new(AtomicUsize::new(0));
        let raw_for_channel = raw_deliveries.clone();
        let on_message = Channel::new(move |_: InvokeResponseBody| {
            raw_for_channel.fetch_add(1, Ordering::SeqCst);
            Ok(())
        });
        let projection_deliveries = Arc::new(AtomicUsize::new(0));
        let projection_for_channel = projection_deliveries.clone();
        let on_projection = Channel::new(move |_: InvokeResponseBody| {
            projection_for_channel.fetch_add(1, Ordering::SeqCst);
            Ok(())
        });
        let id = native_websocket::connect_status_for_test(
            &manager,
            &state,
            relay_url.clone(),
            on_message,
            on_projection,
        )
        .await
        .expect("open production status-capable WebSocket");
        let mut relay_socket = relay.await.expect("join synthetic relay accept task");

        let proof = tokio::time::timeout(RECEIVE_TIMEOUT, async {
            loop {
                if let Ok(proof) = manager
                    .status_auth_proof(id, &challenge, &relay_url, author_keys.public_key())
                    .await
                {
                    break proof;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("production socket records exact NIP-42 challenge");
        let epoch = proof.connection_epoch().clone();
        ClientBindingEpoch::parse(epoch.as_str()).expect("production epoch is canonical UUIDv4");
        let auth = EventBuilder::new(Kind::Custom(22242), "")
            .tags([
                Tag::parse(["relay", relay_url.as_str()]).expect("relay tag"),
                Tag::parse(["challenge", challenge.as_str()]).expect("challenge tag"),
                Tag::parse([
                    CLIENT_BINDING_SCOPE_TAG,
                    "1",
                    epoch.as_str(),
                    proof.relay_signer().to_hex().as_str(),
                ])
                .expect("binding scope tag"),
            ])
            .sign_with_keys(author_keys)
            .expect("sign scoped NIP-42 AUTH");
        let scope = ClientBindingScopeV1::from_verified_auth_event(&auth)
            .expect("relay accepts signed binding scope");
        assert_eq!(scope.connection_epoch(), &epoch);
        assert_eq!(scope.relay_signer(), relay_keys.public_key());
        manager
            .complete_status_auth(id, &proof)
            .await
            .expect("production manager activates authenticated projection owner");
        native_websocket::send_message(
            &manager,
            id,
            WebSocketMessage::Text(json!(["AUTH", auth]).to_string()),
        )
        .await
        .expect("send scoped AUTH through production native socket");
        let auth_frame = tokio::time::timeout(RECEIVE_TIMEOUT, relay_socket.next())
            .await
            .expect("relay receives AUTH")
            .expect("AUTH frame exists")
            .expect("AUTH frame valid");
        let Message::Text(auth_text) = auth_frame else {
            panic!("scoped AUTH must be text")
        };
        let auth_wire: serde_json::Value = serde_json::from_str(&auth_text).expect("AUTH JSON");
        let verified = Event::from_json(auth_wire[1].to_string()).expect("AUTH event parses");
        assert!(verified.verify_id() && verified.verify_signature());
        ClientBindingScopeV1::from_verified_auth_event(&verified)
            .expect("relay revalidates signed scope");

        Self {
            relay_socket,
            manager,
            id,
            raw_deliveries,
            projection_deliveries,
            epoch,
        }
    }

    async fn send_reserved_event(&mut self, sub_id: &str, event: &Event, now: u64) {
        assert!(
            matches!(
                sub_id,
                CLIENT_BINDING_BOOTSTRAP_SUB_ID | CLIENT_BINDING_STATUS_SUB_ID
            ),
            "reserved helper requires an exact native-owned subscription id"
        );
        let _ = now;
        self.send_event(sub_id, event, true).await;
    }

    async fn send_ordinary_event(&mut self, event: &Event, now: u64) {
        let _ = now;
        self.send_event(ORDINARY_SUB_ID, event, false).await;
    }

    async fn send_event(&mut self, sub_id: &str, event: &Event, reserved: bool) {
        let raw_before = self.raw_deliveries.load(Ordering::SeqCst);
        let event_json = serde_json::to_value(event).expect("serialize synthetic Nostr event");
        let frame = json!(["EVENT", sub_id, event_json]).to_string();
        self.relay_socket
            .send(Message::Text(frame.into()))
            .await
            .expect("send synthetic relay frame");
        self.relay_socket
            .send(Message::Text("projection-fold-barrier".into()))
            .await
            .expect("send ordered fold barrier");
        let expected = raw_before + if reserved { 1 } else { 2 };
        tokio::time::timeout(RECEIVE_TIMEOUT, async {
            while self.raw_deliveries.load(Ordering::SeqCst) < expected {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("production raw channel reaches ordered barrier");
        assert_eq!(
            self.raw_deliveries.load(Ordering::SeqCst),
            expected,
            "only exact reserved text frames are swallowed"
        );
    }

    async fn wait_for_expiry(&self) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while self.projection().await.is_some() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("production monotonic expiry clears projection");
    }

    async fn physical_disconnect(&mut self) {
        self.relay_socket
            .send(Message::Close(None))
            .await
            .expect("relay closes physical WebSocket");
        tokio::time::timeout(RECEIVE_TIMEOUT, async {
            while native_websocket::connection_present_for_test(&self.manager, self.id).await {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("production manager observes physical disconnect");
        assert!(self.projection().await.is_none());
    }

    async fn projection(&self) -> Option<CurrentProjection> {
        assert!(
            self.projection_deliveries.load(Ordering::SeqCst) > 0,
            "authenticated production projection channel is active"
        );
        native_websocket::current_projection_for_test(&self.manager).await
    }

    async fn logout(&self) {
        self.manager.suspend_projection().await;
        assert!(self.projection().await.is_none());
    }
}

fn random_epoch() -> ClientBindingEpoch {
    ClientBindingEpoch::new_v4()
}

fn random_domain() -> CommunityId {
    CommunityId::from_uuid(Uuid::new_v4())
}

fn bootstrap_event(
    relay: &Keys,
    domain: CommunityId,
    author: PublicKey,
    epoch: ClientBindingEpoch,
    issued_at: u64,
) -> Event {
    ClientBindingBootstrapInputV1::new(domain, author, epoch, issued_at)
        .expect("construct synthetic bootstrap")
        .sign_with_relay_keys(relay)
        .expect("sign synthetic bootstrap")
}

fn current_event(
    relay: &Keys,
    domain: CommunityId,
    author: PublicKey,
    revision: u64,
    issued_at: u64,
    fresh_until: u64,
) -> Event {
    ClientBindingStatusInputV1::current(
        domain,
        author,
        1,
        "policy.synthetic.example.invalid/v1",
        revision,
        issued_at,
        fresh_until,
        Some("Synthetic Example".to_string()),
    )
    .expect("construct synthetic current status")
    .sign_with_relay_keys(relay)
    .expect("sign synthetic current status")
}

fn withdrawal_event(
    relay: &Keys,
    domain: CommunityId,
    author: PublicKey,
    revision: u64,
    issued_at: u64,
    fresh_until: u64,
) -> Event {
    ClientBindingStatusInputV1::withdrawn(domain, author, revision, issued_at, fresh_until)
        .expect("construct synthetic withdrawal")
        .sign_with_relay_keys(relay)
        .expect("sign synthetic withdrawal")
}

fn raw_status_event(relay: &Keys, content: &str, issued_at: u64) -> Event {
    EventBuilder::new(
        Kind::Custom(KIND_CLIENT_BINDING_STATUS as u16),
        content.to_string(),
    )
    .tags([])
    .custom_created_at(Timestamp::from(issued_at))
    .sign_with_keys(relay)
    .expect("sign synthetic raw status")
}

async fn established_flow(
    relay: &Keys,
    author: &Keys,
    domain: CommunityId,
    now: u64,
) -> NativeFlow {
    let mut flow = NativeFlow::connect(relay, author).await;
    let bootstrap = bootstrap_event(relay, domain, author.public_key(), flow.epoch.clone(), now);
    flow.send_reserved_event(CLIENT_BINDING_BOOTSTRAP_SUB_ID, &bootstrap, now)
        .await;
    let current = current_event(relay, domain, author.public_key(), 1, now, now + 120);
    flow.send_reserved_event(CLIENT_BINDING_STATUS_SUB_ID, &current, now)
        .await;
    assert!(flow.projection().await.is_some());
    flow
}

#[tokio::test]
async fn loopback_relay_drives_production_projection_and_trace() {
    let now = native_websocket::unix_now_for_test();
    let relay = Keys::generate();
    let wrong_signer = Keys::generate();
    let author = Keys::generate();
    let other_author = Keys::generate();
    let profile_spoofer = Keys::generate();
    let domain = random_domain();
    let other_domain = random_domain();
    assert_ne!(domain, other_domain);

    let mut trace = ProjectionTrace::new();

    // One physical connection exercises the revision fold as a sequence, proving that
    // duplicate delivery retains current state while trusted-invalid evidence clears it.
    let mut flow = NativeFlow::connect(&relay, &author).await;
    let epoch = flow.epoch.clone();
    let bootstrap = bootstrap_event(&relay, domain, author.public_key(), epoch.clone(), now);
    flow.send_reserved_event(CLIENT_BINDING_BOOTSTRAP_SUB_ID, &bootstrap, now)
        .await;
    trace.record("bootstrap", &flow).await;

    let current = current_event(&relay, domain, author.public_key(), 10, now, now + 120);
    flow.send_reserved_event(CLIENT_BINDING_STATUS_SUB_ID, &current, now)
        .await;
    let first_projection = serde_json::to_value(flow.projection().await)
        .expect("serialize first production projection");
    let projected = flow.projection().await.expect("current status projects");
    assert_eq!(projected.event_author_pubkey, author.public_key().to_hex());
    assert_eq!(projected.fresh_until, now + 120);
    assert_eq!(projected.connection_epoch, epoch.as_str());
    trace.record("current", &flow).await;

    flow.send_reserved_event(CLIENT_BINDING_STATUS_SUB_ID, &current, now)
        .await;
    assert_eq!(
        serde_json::to_value(flow.projection().await).expect("serialize duplicate projection"),
        first_projection
    );
    trace.record("duplicate", &flow).await;

    let equal_conflict = current_event(&relay, domain, author.public_key(), 10, now, now + 121);
    flow.send_reserved_event(CLIENT_BINDING_STATUS_SUB_ID, &equal_conflict, now)
        .await;
    assert!(flow.projection().await.is_none());
    trace.record("equal-conflict", &flow).await;

    let rollback = current_event(&relay, domain, author.public_key(), 9, now, now + 120);
    flow.send_reserved_event(CLIENT_BINDING_STATUS_SUB_ID, &rollback, now)
        .await;
    assert!(flow.projection().await.is_none());
    trace.record("rollback", &flow).await;

    let newer = current_event(&relay, domain, author.public_key(), 11, now, now + 120);
    flow.send_reserved_event(CLIENT_BINDING_STATUS_SUB_ID, &newer, now)
        .await;
    assert!(flow.projection().await.is_some());
    trace.record("newer-restoration", &flow).await;

    let withdrawal = withdrawal_event(&relay, domain, author.public_key(), 12, now, now + 120);
    flow.send_reserved_event(CLIENT_BINDING_STATUS_SUB_ID, &withdrawal, now)
        .await;
    assert!(flow.projection().await.is_none());
    trace.record("withdrawal", &flow).await;

    let short_now = native_websocket::unix_now_for_test();
    let short_current = current_event(
        &relay,
        domain,
        author.public_key(),
        13,
        short_now,
        short_now + 2,
    );
    flow.send_reserved_event(CLIENT_BINDING_STATUS_SUB_ID, &short_current, short_now)
        .await;
    flow.wait_for_expiry().await;
    trace.record("passive-expiry", &flow).await;

    let disconnect_current = current_event(&relay, domain, author.public_key(), 14, now, now + 123);
    flow.send_reserved_event(CLIENT_BINDING_STATUS_SUB_ID, &disconnect_current, now)
        .await;
    assert!(flow.projection().await.is_some());
    flow.physical_disconnect().await;
    trace.record("disconnect", &flow).await;

    let reconnected = established_flow(&relay, &author, domain, now).await;
    trace.record("reconnect", &reconnected).await;
    reconnected.logout().await;
    trace.record("logout", &reconnected).await;

    let mut restarted = established_flow(&relay, &author, domain, now).await;
    restarted.physical_disconnect().await;
    trace.record("restart", &restarted).await;

    // A different physical relay connection starts empty even when the signer is reused.
    let relay_scope = NativeFlow::connect(&relay, &author).await;
    trace.record("relay-scope-change", &relay_scope).await;

    // Wrong-signer traffic is untrusted noise and cannot create or clear presentation.
    let mut signer_scope = NativeFlow::connect(&wrong_signer, &author).await;
    let old_signer_bootstrap = bootstrap_event(
        &relay,
        domain,
        author.public_key(),
        signer_scope.epoch.clone(),
        now,
    );
    signer_scope
        .send_reserved_event(CLIENT_BINDING_BOOTSTRAP_SUB_ID, &old_signer_bootstrap, now)
        .await;
    trace.record("signer-scope-change", &signer_scope).await;

    let mut author_scope = NativeFlow::connect(&relay, &other_author).await;
    let old_author_bootstrap = bootstrap_event(
        &relay,
        domain,
        author.public_key(),
        author_scope.epoch.clone(),
        now,
    );
    author_scope
        .send_reserved_event(CLIENT_BINDING_BOOTSTRAP_SUB_ID, &old_author_bootstrap, now)
        .await;
    trace.record("author-scope-change", &author_scope).await;

    let mut domain_scope = NativeFlow::connect(&relay, &author).await;
    let domain_bootstrap = bootstrap_event(
        &relay,
        other_domain,
        author.public_key(),
        domain_scope.epoch.clone(),
        now,
    );
    domain_scope
        .send_reserved_event(CLIENT_BINDING_BOOTSTRAP_SUB_ID, &domain_bootstrap, now)
        .await;
    let old_domain_status = current_event(&relay, domain, author.public_key(), 1, now, now + 152);
    domain_scope
        .send_reserved_event(CLIENT_BINDING_STATUS_SUB_ID, &old_domain_status, now)
        .await;
    trace.record("domain-scope-change", &domain_scope).await;

    let old_epoch = random_epoch();
    let mut epoch_scope = NativeFlow::connect(&relay, &author).await;
    assert_ne!(old_epoch, epoch_scope.epoch);
    let stale_epoch_bootstrap =
        bootstrap_event(&relay, domain, author.public_key(), old_epoch, now);
    epoch_scope
        .send_reserved_event(CLIENT_BINDING_BOOTSTRAP_SUB_ID, &stale_epoch_bootstrap, now)
        .await;
    trace.record("epoch-scope-change", &epoch_scope).await;

    let mut malformed = established_flow(&relay, &author, domain, now).await;
    let malformed_status = raw_status_event(&relay, r#"{"version":1,"domain":"broken"}"#, now);
    malformed
        .send_reserved_event(CLIENT_BINDING_STATUS_SUB_ID, &malformed_status, now)
        .await;
    trace.record("malformed-trusted", &malformed).await;

    let mut unsupported = established_flow(&relay, &author, domain, now).await;
    let unsupported_status = raw_status_event(&relay, r#"{"version":2}"#, now);
    unsupported
        .send_reserved_event(CLIENT_BINDING_STATUS_SUB_ID, &unsupported_status, now)
        .await;
    trace.record("unsupported-version", &unsupported).await;

    let mut mismatched_author = established_flow(&relay, &author, domain, now).await;
    let author_mismatch =
        current_event(&relay, domain, other_author.public_key(), 2, now, now + 162);
    mismatched_author
        .send_reserved_event(CLIENT_BINDING_STATUS_SUB_ID, &author_mismatch, now)
        .await;
    trace.record("author-mismatch", &mismatched_author).await;

    // Ordinary kind-0 and NIP-85 traffic crosses the same socket but never enters the
    // reserved production fold, so neither can manufacture a current projection.
    let mut legacy = NativeFlow::connect(&relay, &author).await;
    let spoofed_profile = EventBuilder::new(
        Kind::Metadata,
        r#"{"display_name":"Spoofed Verified User","nip05":"spoof@identity.example.invalid"}"#,
    )
    .sign_with_keys(&profile_spoofer)
    .expect("sign synthetic profile spoof");
    legacy.send_ordinary_event(&spoofed_profile, now + 50).await;
    trace.record("profile-spoof", &legacy).await;

    let subject = author.public_key().to_hex();
    let expiry = (now + 170).to_string();
    let nip85 = EventBuilder::new(
        Kind::Custom(KIND_USER_TRUSTED_ASSERTION as u16),
        String::new(),
    )
    .tags([
        Tag::parse(["d", subject.as_str()]).expect("synthetic d tag"),
        Tag::parse(["p", subject.as_str()]).expect("synthetic p tag"),
        Tag::parse(["verified", "relay"]).expect("synthetic verified tag"),
        Tag::parse(["active", "true"]).expect("synthetic active tag"),
        Tag::parse(["expiration", expiry.as_str()]).expect("synthetic expiration tag"),
        Tag::parse(["display_name", "Spoofed Legacy Assertion"])
            .expect("synthetic display-name tag"),
    ])
    .sign_with_keys(&relay)
    .expect("sign synthetic NIP-85 assertion");
    legacy.send_ordinary_event(&nip85, now + 50).await;
    trace.record("nip85-no-fallback", &legacy).await;

    let cases = trace.steps.iter().map(|step| step.case).collect::<Vec<_>>();
    assert_eq!(
        cases,
        [
            "bootstrap",
            "current",
            "duplicate",
            "equal-conflict",
            "rollback",
            "newer-restoration",
            "withdrawal",
            "passive-expiry",
            "disconnect",
            "reconnect",
            "logout",
            "restart",
            "relay-scope-change",
            "signer-scope-change",
            "author-scope-change",
            "domain-scope-change",
            "epoch-scope-change",
            "malformed-trusted",
            "unsupported-version",
            "author-mismatch",
            "profile-spoof",
            "nip85-no-fallback",
        ]
    );
    for step in &trace.steps {
        let expected_current = matches!(
            step.case,
            "current" | "duplicate" | "newer-restoration" | "reconnect"
        );
        assert_eq!(
            step.projection.is_some(),
            expected_current,
            "unexpected retained projection for {}",
            step.case
        );
    }

    trace.assert_contract_and_export_if_requested();
}
