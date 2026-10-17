//! Screen guards are exercised through the actual ephemeral ingest handler.

use super::*;
use buzz_core::{CommunityId, TenantContext};
use nostr::{EventBuilder, Keys, Kind, Tag};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

fn connection(keys: &Keys) -> Arc<ConnectionState> {
    let cancel = CancellationToken::new();
    Arc::new(ConnectionState {
        conn_id: Uuid::new_v4(),
        tenant: TenantContext::resolved(CommunityId::from_uuid(Uuid::new_v4()), "screen.test"),
        remote_addr: "127.0.0.1:1234".parse().unwrap(),
        auth_state: std::sync::Mutex::new(AuthState::Authenticated(buzz_auth::AuthContext {
            pubkey: keys.public_key(),
            scopes: vec![],
            channel_ids: None,
            auth_method: buzz_auth::AuthMethod::Nip42,
            agent_owner_pubkey: None,
        })),
        subscriptions: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
        send_tx: tokio::sync::mpsc::channel(8).0,
        ctrl_tx: tokio::sync::mpsc::channel(8).0,
        terminal_ctrl_tx: tokio::sync::mpsc::channel(1).0,
        cancel: cancel.clone(),
        backpressure_count: Arc::new(std::sync::atomic::AtomicU8::new(0)),
        grace_limit: 3,
        nip_fi_assertion: None,
        session_deadline: None,
        nip_fi_gate: crate::nip_fi_gate::SessionAdmissionGate::off_mode(cancel.clone()),
        community_control: crate::state::CommunityConnectionControl::new(cancel),
    })
}

fn stop_event(keys: &Keys, channels: &[String]) -> Event {
    EventBuilder::new(
        Kind::Custom(buzz_core::kind::KIND_HUDDLE_SCREEN as u16),
        serde_json::json!({
            "type": "stopped",
            "share": {
                "session_id": Uuid::new_v4(),
                "started_at": nostr::Timestamp::now().as_secs() * 1000,
                "label": "Screen"
            }
        })
        .to_string(),
    )
    .tags(
        channels
            .iter()
            .map(|channel| Tag::parse(["h", channel.as_str()]).unwrap()),
    )
    .sign_with_keys(keys)
    .unwrap()
}

async fn deliver(event: Event, keys: &Keys, state: Arc<AppState>) -> Result<(), IngestError> {
    let conn = connection(keys);
    super::handle_ephemeral_event(
        event,
        conn.conn_id,
        keys.public_key().to_bytes().to_vec(),
        keys.public_key(),
        conn,
        state,
    )
    .await
}

#[tokio::test]
async fn screen_handler_rejects_global_and_ambiguous_scope_before_any_fanout() {
    let state = crate::state::tests::test_state().await;
    state.db.pool().close().await;
    let keys = Keys::generate();
    let channel = Uuid::new_v4().to_string();
    for channels in [vec![], vec![channel.clone(), channel.clone()]] {
        let result = deliver(stop_event(&keys, &channels), &keys, state.clone()).await;
        assert!(matches!(
            result,
            Err(IngestError::Rejected(ref message))
                if message == "invalid: screen events require exactly one h tag"
        ));
    }
}

#[tokio::test]
async fn screen_handler_propagates_channel_lookup_outage_as_error() {
    let state = crate::state::tests::test_state().await;
    state.db.pool().close().await;
    let keys = Keys::generate();
    let event = stop_event(&keys, &[Uuid::new_v4().to_string()]);
    let result = deliver(event, &keys, state).await;
    assert!(matches!(
        result,
        Err(IngestError::Internal(ref message)) if message == "error: screen huddle lookup failed"
    ));
}
