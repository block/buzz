//! Exact-byte loopback proof for the test-only J3C client-status composition.

#[path = "../../../../desktop/src-tauri/src/client_binding_status_session.rs"]
mod client_binding_status_session;

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::atomic::AtomicU8;
use std::sync::Arc;
use std::task::{Context, Poll};

use axum::extract::ws::Message as AxumMessage;
use buzz_core::client_binding_bootstrap::{
    ClientBindingBootstrapInputV1, ClientBindingEpoch, CLIENT_BINDING_BOOTSTRAP_SUB_ID,
    CLIENT_BINDING_STATUS_SUB_ID,
};
use buzz_core::client_binding_status::ClientBindingStatusInputV1;
use buzz_core::CommunityId;
use futures_util::{Sink, StreamExt};
use nostr::{Keys, Timestamp};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, Mutex};
use tokio::time::{timeout, Duration};
use tokio_tungstenite::tungstenite::{
    protocol::{frame::coding::CloseCode, CloseFrame},
    Error as TungsteniteError, Message as TungsteniteMessage,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::{send_loop_inner, OutboundData};
use crate::protocol::RelayMessage;
use crate::state::ConnectionManager;
use client_binding_status_session::{
    ClientBindingStatusSession, CurrentProjection, ProjectionUpdate,
};

struct TungsteniteSink<S>(S);

impl<S> Sink<AxumMessage> for TungsteniteSink<S>
where
    S: Sink<TungsteniteMessage, Error = TungsteniteError> + Unpin,
{
    type Error = TungsteniteError;

    fn poll_ready(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.0).poll_ready(context)
    }

    fn start_send(mut self: Pin<&mut Self>, item: AxumMessage) -> Result<(), Self::Error> {
        let item = match item {
            AxumMessage::Text(text) => TungsteniteMessage::Text(text.to_string().into()),
            AxumMessage::Binary(bytes) => TungsteniteMessage::Binary(bytes),
            AxumMessage::Ping(bytes) => TungsteniteMessage::Ping(bytes),
            AxumMessage::Pong(bytes) => TungsteniteMessage::Pong(bytes),
            AxumMessage::Close(frame) => TungsteniteMessage::Close(frame.map(|frame| CloseFrame {
                code: CloseCode::from(frame.code),
                reason: frame.reason.to_string().into(),
            })),
        };
        Pin::new(&mut self.0).start_send(item)
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.0).poll_flush(context)
    }

    fn poll_close(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.0).poll_close(context)
    }
}

async fn receive_exact(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    expected: &str,
) -> String {
    let message = timeout(Duration::from_secs(2), socket.next())
        .await
        .expect("production socket writer must not time out")
        .expect("loopback socket remains connected")
        .expect("production socket writer emits a valid frame");
    let TungsteniteMessage::Text(text) = message else {
        panic!("client-status transport must emit text");
    };
    assert_eq!(text.as_str().as_bytes(), expected.as_bytes());
    text.to_string()
}

fn assert_current(
    update: Option<ProjectionUpdate>,
    author: &Keys,
    epoch: &ClientBindingEpoch,
    fresh_until: u64,
) {
    let Some(ProjectionUpdate::Current(CurrentProjection {
        event_author_pubkey,
        fresh_until: projected_fresh_until,
        connection_epoch,
    })) = update
    else {
        panic!("exact production bytes must project current status");
    };
    assert_eq!(event_author_pubkey, author.public_key().to_hex());
    assert_eq!(projected_fresh_until, fresh_until);
    assert_eq!(connection_epoch, epoch.as_str());
}

#[tokio::test]
async fn production_outbound_bytes_cross_loopback_into_native_status_session() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("ephemeral loopback listener binds");
    let address = listener.local_addr().expect("loopback address resolves");
    assert_ne!(address.port(), 0);

    let relay = Keys::generate();
    let author = Keys::generate();
    let domain = CommunityId::from_uuid(Uuid::new_v4());
    let epoch = ClientBindingEpoch::new_v4();
    let connection_id = Uuid::new_v4();
    let now = Timestamp::now().as_secs();
    let fresh_until = now + 120;

    let connections = Arc::new(ConnectionManager::new());
    let (data_tx, data_rx) = mpsc::channel::<OutboundData>(8);
    let (ctrl_tx, ctrl_rx) = mpsc::channel(2);
    let cancel = CancellationToken::new();
    connections.register(
        connection_id,
        data_tx,
        ctrl_tx,
        cancel.clone(),
        domain,
        Arc::new(AtomicU8::new(0)),
        Arc::new(Mutex::new(HashMap::new())),
        3,
    );
    connections.set_authenticated_pubkey(connection_id, author.public_key().to_bytes().to_vec());

    let writer_cancel = cancel.clone();
    let server = tokio::spawn(async move {
        let (tcp, peer) = listener.accept().await.expect("loopback client connects");
        assert!(peer.ip().is_loopback());
        let socket = tokio_tungstenite::accept_async(tcp)
            .await
            .expect("loopback WebSocket upgrades");
        let (sink, _stream) = socket.split();
        send_loop_inner(TungsteniteSink(sink), data_rx, ctrl_rx, writer_cancel).await;
    });

    let (mut socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}"))
        .await
        .expect("loopback WebSocket client connects");
    let mut session =
        ClientBindingStatusSession::new(relay.public_key(), author.public_key(), epoch.clone());
    assert_eq!(session.connection_epoch(), &epoch);

    let bootstrap =
        ClientBindingBootstrapInputV1::new(domain, author.public_key(), epoch.clone(), now)
            .expect("connection bootstrap input is valid")
            .sign_with_relay_keys(&relay)
            .expect("ephemeral relay signs bootstrap");
    let bootstrap_frame = RelayMessage::event(CLIENT_BINDING_BOOTSTRAP_SUB_ID, &bootstrap);
    assert!(connections.send_to(connection_id, bootstrap_frame.clone()));
    let bootstrap_text = receive_exact(&mut socket, &bootstrap_frame).await;
    assert!(matches!(
        session.consume_text(&bootstrap_text, now),
        Some(ProjectionUpdate::Unchanged)
    ));

    let current = ClientBindingStatusInputV1::current(
        domain,
        author.public_key(),
        7,
        "opaque-current",
        10,
        now,
        fresh_until,
        None,
    )
    .expect("current status input is valid")
    .sign_with_relay_keys(&relay)
    .expect("ephemeral relay signs current status");
    let current_frame = RelayMessage::event(CLIENT_BINDING_STATUS_SUB_ID, &current);
    assert!(connections.send_to(connection_id, current_frame.clone()));
    let current_text = receive_exact(&mut socket, &current_frame).await;
    assert_current(
        session.consume_text(&current_text, now),
        &author,
        &epoch,
        fresh_until,
    );

    let trusted_invalid = ClientBindingStatusInputV1::current(
        domain,
        author.public_key(),
        8,
        "opaque-trusted-invalid",
        11,
        now,
        fresh_until,
        None,
    )
    .expect("trusted-invalid status input is valid")
    .sign_with_relay_keys(&relay)
    .expect("ephemeral relay signs trusted-invalid status");
    let malformed_outer = serde_json::json!([
        "EVENT",
        CLIENT_BINDING_STATUS_SUB_ID,
        trusted_invalid,
        "unexpected"
    ])
    .to_string();
    assert!(connections.send_to(connection_id, malformed_outer.clone()));
    let malformed_text = receive_exact(&mut socket, &malformed_outer).await;
    assert!(matches!(
        session.consume_text(&malformed_text, now),
        Some(ProjectionUpdate::Clear)
    ));

    let replay_frame = RelayMessage::event(CLIENT_BINDING_STATUS_SUB_ID, &trusted_invalid);
    assert!(connections.send_to(connection_id, replay_frame.clone()));
    let replay_text = receive_exact(&mut socket, &replay_frame).await;
    assert!(matches!(
        session.consume_text(&replay_text, now),
        Some(ProjectionUpdate::Unchanged)
    ));
    assert_eq!(session.projected_fresh_until(), None);

    let newer = ClientBindingStatusInputV1::current(
        domain,
        author.public_key(),
        9,
        "opaque-newer-restoration",
        12,
        now,
        fresh_until,
        None,
    )
    .expect("newer status input is valid")
    .sign_with_relay_keys(&relay)
    .expect("ephemeral relay signs newer status");
    let newer_frame = RelayMessage::event(CLIENT_BINDING_STATUS_SUB_ID, &newer);
    assert!(connections.send_to(connection_id, newer_frame.clone()));
    let newer_text = receive_exact(&mut socket, &newer_frame).await;
    assert_current(
        session.consume_text(&newer_text, now),
        &author,
        &epoch,
        fresh_until,
    );
    assert!(matches!(session.disconnect(), ProjectionUpdate::Clear));
    assert_eq!(session.projected_fresh_until(), None);

    cancel.cancel();
    timeout(Duration::from_secs(2), server)
        .await
        .expect("production socket writer stops after cancellation")
        .expect("production socket writer task does not panic");
}
