use futures_util::{SinkExt, StreamExt};
use nostr::PublicKey;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc, time::Duration};
use tauri::{ipc::Channel, plugin::TauriPlugin, Manager, Runtime};
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{
        client::IntoClientRequest,
        protocol::{frame::coding::CloseCode, CloseFrame, Message},
    },
};
use tokio_util::sync::CancellationToken;

use crate::{
    app_state::AppState,
    client_binding_status_session::{
        is_reserved_text, ClientBindingStatusSession, ProjectionUpdate,
    },
};
#[cfg(test)]
use buzz_core_pkg::client_binding_bootstrap::ClientBindingEpoch;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_millis(250);
const SEND_QUEUE_CAPACITY: usize = 64;
#[path = "native_websocket_status.rs"]
mod native_websocket_status;

pub(crate) use native_websocket_status::StatusAuthProof;
#[cfg(test)]
use native_websocket_status::{monotonic_deadline_after, status_expiry_sleep, ProjectionOwner};
use native_websocket_status::{
    prepare_status_session, unix_now, PreparedStatus, ProjectionState, StatusScope,
};
pub(crate) fn install_crypto_provider() {
    // Dependencies enable both rustls providers; choose one before TLS setup.
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
}

type Id = u32;

#[derive(Debug, Deserialize)]
#[serde(tag = "type", content = "data")]
pub(crate) enum WebSocketMessage {
    Text(String),
    Binary(Vec<u8>),
    Ping(Vec<u8>),
    Pong(Vec<u8>),
    Close(Option<CloseFramePayload>),
}

#[derive(Debug, Deserialize)]
pub(crate) struct CloseFramePayload {
    code: u16,
    reason: String,
}

impl From<WebSocketMessage> for Message {
    fn from(message: WebSocketMessage) -> Self {
        match message {
            WebSocketMessage::Text(value) => Message::Text(value.into()),
            WebSocketMessage::Binary(value) => Message::Binary(value.into()),
            WebSocketMessage::Ping(value) => Message::Ping(value.into()),
            WebSocketMessage::Pong(value) => Message::Pong(value.into()),
            WebSocketMessage::Close(frame) => Message::Close(frame.map(|frame| CloseFrame {
                code: CloseCode::from(frame.code),
                reason: frame.reason.into(),
            })),
        }
    }
}

#[derive(Serialize)]
#[serde(tag = "type", content = "data")]
enum OutboundMessage {
    Text(String),
    Binary(Vec<u8>),
    Ping(Vec<u8>),
    Pong(Vec<u8>),
    Close(Option<CloseFramePayloadOut>),
    Error(String),
}

#[derive(Serialize)]
struct CloseFramePayloadOut {
    code: u16,
    reason: String,
}

struct SendRequest {
    message: Message,
    result: oneshot::Sender<Result<(), String>>,
}

struct ConnectionHandle {
    sender: mpsc::Sender<SendRequest>,
    cancel: CancellationToken,
    task: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    status_scope: Mutex<Option<StatusScope>>,
    #[cfg(test)]
    fold_pause: Option<Arc<TestFoldPause>>,
}

#[cfg(test)]
struct TestFoldPause {
    entered: std::sync::atomic::AtomicBool,
    released: std::sync::Mutex<bool>,
    released_cv: std::sync::Condvar,
}

#[cfg(test)]
impl TestFoldPause {
    fn new() -> Self {
        Self {
            entered: std::sync::atomic::AtomicBool::new(false),
            released: std::sync::Mutex::new(false),
            released_cv: std::sync::Condvar::new(),
        }
    }

    fn block(&self) {
        self.entered
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let mut released = self.released.lock().expect("fold-pause lock");
        while !*released {
            released = self.released_cv.wait(released).expect("fold-pause wait");
        }
    }

    fn release(&self) {
        *self.released.lock().expect("fold-pause lock") = true;
        self.released_cv.notify_all();
    }
}

#[derive(Clone)]
pub(crate) struct WebSocketManager {
    connections: Arc<Mutex<HashMap<Id, Arc<ConnectionHandle>>>>,
    connect_cancel: Arc<Mutex<CancellationToken>>,
    projection: Arc<Mutex<ProjectionState>>,
}
impl Default for WebSocketManager {
    fn default() -> Self {
        Self {
            connections: Arc::default(),
            connect_cancel: Arc::new(Mutex::new(CancellationToken::new())),
            projection: Arc::default(),
        }
    }
}

pub(crate) struct ScopeMutationGuard {
    manager: Option<WebSocketManager>,
    token: u64,
}

impl ScopeMutationGuard {
    pub(crate) async fn finish(mut self) {
        if let Some(manager) = self.manager.take() {
            manager.finish_scope_mutation(self.token).await;
        }
    }
}

impl Drop for ScopeMutationGuard {
    fn drop(&mut self) {
        let Some(manager) = self.manager.take() else {
            return;
        };
        let token = self.token;
        std::mem::drop(tauri::async_runtime::spawn(async move {
            manager.finish_scope_mutation(token).await;
        }));
    }
}

impl WebSocketManager {
    async fn remove(&self, id: Id) -> Option<Arc<ConnectionHandle>> {
        self.connections.lock().await.remove(&id)
    }

    async fn remove_if_current(&self, id: Id, handle: &Arc<ConnectionHandle>) {
        let mut connections = self.connections.lock().await;
        if connections
            .get(&id)
            .is_some_and(|current| Arc::ptr_eq(current, handle))
        {
            connections.remove(&id);
        }
    }

    async fn current_connect_cancel(&self) -> CancellationToken {
        self.connect_cancel.lock().await.clone()
    }

    /// Invalidate all browser-visible status and revoke the current socket's
    /// ownership. Late work from that socket is rejected by the owner fence.
    pub(crate) async fn invalidate_projection(&self) {
        {
            let mut projection = self.projection.lock().await;
            Self::advance_generation(&mut projection);
            if let Some(owner) = projection.owner.take() {
                let _ = owner.channel.send(serde_json::Value::Null);
            }
            projection.current = None;
        }
        self.cancel_status_connections().await;
    }

    /// Enter a fail-closed workspace or identity mutation interval.
    /// Overlapping mutations keep status disabled until the last one exits.
    pub(crate) async fn begin_scope_mutation(&self) -> Result<ScopeMutationGuard, String> {
        let token = {
            let mut projection = self.projection.lock().await;
            match projection.mutation_head.checked_add(1) {
                None => {
                    projection.suspended = true;
                    if let Some(owner) = projection.owner.take() {
                        let _ = owner.channel.send(serde_json::Value::Null);
                    }
                    projection.current = None;
                    Err("status scope mutation token exhausted")
                }
                Some(token) => {
                    projection.mutation_head = token;
                    projection.active_mutations.insert(token);
                    if !Self::advance_generation(&mut projection) {
                        projection.active_mutations.remove(&token);
                        Err("status projection generation exhausted")
                    } else {
                        if let Some(owner) = projection.owner.take() {
                            let _ = owner.channel.send(serde_json::Value::Null);
                        }
                        projection.current = None;
                        Ok(token)
                    }
                }
            }
        };
        let token = match token {
            Ok(token) => token,
            Err(error) => {
                self.cancel_status_connections().await;
                return Err(error.to_string());
            }
        };
        let guard = ScopeMutationGuard {
            manager: Some(self.clone()),
            token,
        };
        self.cancel_status_connections().await;
        Ok(guard)
    }

    /// Exit one workspace or identity mutation interval and fence all work
    /// that raced the mutation, including failed or panicked blocking work.
    async fn finish_scope_mutation(&self, token: u64) {
        {
            let mut projection = self.projection.lock().await;
            if !projection.active_mutations.remove(&token) {
                return;
            }
            Self::advance_generation(&mut projection);
            if let Some(owner) = projection.owner.take() {
                let _ = owner.channel.send(serde_json::Value::Null);
            }
            projection.current = None;
        }
        self.cancel_status_connections().await;
    }

    /// Permanently disable status projection for the remainder of this process.
    /// Sign-out uses this before starting restart so no racing webview request
    /// can regain presentation ownership with the retiring identity.
    pub(crate) async fn suspend_projection(&self) {
        {
            let mut projection = self.projection.lock().await;
            projection.suspended = true;
            Self::advance_generation(&mut projection);
            if let Some(owner) = projection.owner.take() {
                let _ = owner.channel.send(serde_json::Value::Null);
            }
            projection.current = None;
        }
        self.cancel_status_connections().await;
    }

    async fn cancel_status_connections(&self) {
        let handles = self
            .connections
            .lock()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for handle in handles {
            if handle.status_scope.lock().await.take().is_some() {
                handle.cancel.cancel();
            }
        }
    }

    async fn record_status_challenge(
        &self,
        id: Id,
        handle: &Arc<ConnectionHandle>,
        challenge: &str,
    ) {
        if !self
            .connections
            .lock()
            .await
            .get(&id)
            .is_some_and(|current| Arc::ptr_eq(current, handle))
        {
            return;
        }
        let revoke_epoch = {
            let mut status_scope = handle.status_scope.lock().await;
            let Some(scope) = status_scope.as_mut() else {
                return;
            };
            if scope.auth_proven {
                let epoch = scope.epoch.clone();
                status_scope.take();
                Some(epoch)
            } else {
                match scope.challenge.as_deref() {
                    None => scope.challenge = Some(challenge.to_owned()),
                    Some(existing) if existing == challenge => {}
                    Some(_) => {
                        status_scope.take();
                    }
                }
                None
            }
        };
        if let Some(epoch) = revoke_epoch {
            self.clear_projection_if_owner(id, handle, &epoch).await;
        }
    }

    pub(crate) async fn status_auth_proof(
        &self,
        id: Id,
        challenge: &str,
        relay_url: &str,
        expected_author: PublicKey,
    ) -> Result<StatusAuthProof, String> {
        let handle = self
            .connections
            .lock()
            .await
            .get(&id)
            .cloned()
            .ok_or_else(|| "native WebSocket is not current".to_string())?;
        let current_head = self
            .status_head()
            .await
            .ok_or_else(|| "native WebSocket status is suspended".to_string())?;
        let scope = handle.status_scope.lock().await;
        let scope = scope
            .as_ref()
            .ok_or_else(|| "native WebSocket is not status-capable".to_string())?;
        if scope.auth_proven
            || scope.challenge.as_deref() != Some(challenge)
            || scope.relay_url != relay_url
            || scope.expected_author != expected_author
            || (scope.generation, scope.attempt) != current_head
        {
            return Err("native WebSocket status scope does not match".to_string());
        }
        Ok(StatusAuthProof {
            handle: Arc::clone(&handle),
            challenge: challenge.to_owned(),
            relay_url: scope.relay_url.clone(),
            relay_signer: scope.relay_signer,
            expected_author: scope.expected_author,
            epoch: scope.epoch.clone(),
            generation: scope.generation,
            attempt: scope.attempt,
        })
    }

    pub(crate) async fn complete_status_auth(
        &self,
        id: Id,
        proof: &StatusAuthProof,
    ) -> Result<(), String> {
        if self.activate_projection_after_auth(id, proof).await {
            Ok(())
        } else {
            Err("native WebSocket status scope changed while signing".to_string())
        }
    }

    async fn disconnect_handle(handle: Arc<ConnectionHandle>) {
        handle.cancel.cancel();
        if let Some(mut task) = handle.task.lock().await.take() {
            if tokio::time::timeout(SHUTDOWN_TIMEOUT, &mut task)
                .await
                .is_err()
            {
                task.abort();
                let _ = task.await;
            }
        }
    }

    async fn disconnect(&self, id: Id) {
        if let Some(handle) = self.remove(id).await {
            let owner_epoch = self
                .projection
                .lock()
                .await
                .owner
                .as_ref()
                .filter(|owner| owner.id == id && Arc::ptr_eq(&owner.handle, &handle))
                .map(|owner| owner.epoch.clone());
            if let Some(epoch) = owner_epoch {
                self.clear_projection_if_owner(id, &handle, &epoch).await;
            }
            Self::disconnect_handle(handle).await;
        }
    }

    async fn disconnect_all(&self) {
        self.invalidate_projection().await;
        let mut connect_cancel = self.connect_cancel.lock().await;
        connect_cancel.cancel();
        *connect_cancel = CancellationToken::new();
        let handles = {
            let mut connections = self.connections.lock().await;
            connections
                .drain()
                .map(|(_, handle)| handle)
                .collect::<Vec<_>>()
        };
        futures_util::future::join_all(handles.into_iter().map(Self::disconnect_handle)).await;
    }
}

#[cfg(test)]
async fn open_connection(
    manager: &WebSocketManager,
    url: &str,
    on_message: Channel<serde_json::Value>,
) -> Result<Id, String> {
    let connect_cancel = manager.current_connect_cancel().await;
    open_connection_with_projection(manager, url, on_message, None, connect_cancel).await
}

async fn open_connection_with_projection(
    manager: &WebSocketManager,
    url: &str,
    on_message: Channel<serde_json::Value>,
    prepared_status: Option<PreparedStatus>,
    connect_cancel: CancellationToken,
) -> Result<Id, String> {
    let request = url
        .into_client_request()
        .map_err(|error| error.to_string())?;
    let (socket, _) = tokio::select! {
        _ = connect_cancel.cancelled() => return Err("WebSocket connection cancelled".to_string()),
        result = tokio::time::timeout(CONNECT_TIMEOUT, connect_async(request)) => result
            .map_err(|_| "WebSocket connection timed out".to_string())?
            .map_err(|error| error.to_string())?,
    };

    // Serialize registration with disconnect_all so a reload cannot miss a
    // connection that finished its handshake concurrently with teardown.
    let current_connect_cancel = manager.connect_cancel.lock().await;
    if connect_cancel.is_cancelled() {
        return Err("WebSocket connection cancelled".to_string());
    }
    let current_head = manager.status_head().await;
    if prepared_status.as_ref().is_some_and(|prepared| {
        Some((prepared.scope.generation, prepared.scope.attempt)) != current_head
    }) {
        return Err("WebSocket connection scope changed".to_string());
    }

    let id = loop {
        let candidate = uuid::Uuid::new_v4().as_u128() as u32;
        if !manager.connections.lock().await.contains_key(&candidate) {
            break candidate;
        }
    };
    let (sender, receiver) = mpsc::channel(SEND_QUEUE_CAPACITY);
    let cancel = CancellationToken::new();
    let handle = Arc::new(ConnectionHandle {
        sender,
        cancel: cancel.clone(),
        task: Mutex::new(None),
        status_scope: Mutex::new(prepared_status.as_ref().map(|prepared| StatusScope {
            relay_url: prepared.scope.relay_url.clone(),
            relay_signer: prepared.scope.relay_signer,
            expected_author: prepared.scope.expected_author,
            epoch: prepared.scope.epoch.clone(),
            projection_channel: prepared.scope.projection_channel.clone(),
            generation: prepared.scope.generation,
            attempt: prepared.scope.attempt,
            challenge: None,
            auth_proven: false,
        })),
        #[cfg(test)]
        fold_pause: None,
    });
    let mut task_slot = handle.task.lock().await;
    manager.connections.lock().await.insert(id, handle.clone());

    let registered_head = manager.status_head().await;
    if prepared_status.as_ref().is_some_and(|prepared| {
        Some((prepared.scope.generation, prepared.scope.attempt)) != registered_head
    }) {
        manager.remove_if_current(id, &handle).await;
        handle.cancel.cancel();
        return Err("WebSocket connection scope changed".to_string());
    }

    let status_session = prepared_status.map(|prepared| prepared.session);

    let task_manager = manager.clone();
    let task = tauri::async_runtime::spawn(run_connection_inner(
        id,
        socket,
        receiver,
        cancel,
        on_message,
        task_manager,
        handle.clone(),
        status_session,
    ));
    *task_slot = Some(task);
    drop(task_slot);
    drop(current_connect_cancel);
    Ok(id)
}

#[tauri::command]
async fn connect(
    manager: tauri::State<'_, WebSocketManager>,
    state: tauri::State<'_, AppState>,
    url: String,
    on_message: Channel<serde_json::Value>,
    _config: Option<serde_json::Value>,
) -> Result<Id, String> {
    connect_internal(manager.inner(), state.inner(), url, on_message, None).await
}

#[tauri::command]
/// Open a native socket with an optional connection-local status projection.
///
/// Status setup fails closed; ordinary WebSocket traffic remains available
/// when the candidate is ineligible or cannot be prepared.
async fn connect_with_status(
    manager: tauri::State<'_, WebSocketManager>,
    state: tauri::State<'_, AppState>,
    url: String,
    on_message: Channel<serde_json::Value>,
    on_projection: Channel<serde_json::Value>,
    _config: Option<serde_json::Value>,
) -> Result<Id, String> {
    connect_internal(
        manager.inner(),
        state.inner(),
        url,
        on_message,
        Some(on_projection),
    )
    .await
}

async fn connect_internal(
    manager: &WebSocketManager,
    state: &AppState,
    url: String,
    on_message: Channel<serde_json::Value>,
    on_projection: Option<Channel<serde_json::Value>>,
) -> Result<Id, String> {
    let connect_cancel = manager.current_connect_cancel().await;
    let status_candidate = on_projection.is_some()
        && url == crate::relay::relay_ws_url_with_override(state)
        && state.signing_keys().is_ok();
    let status_head = if status_candidate {
        manager.begin_status_attempt().await
    } else {
        None
    };
    let prepared_status = match (on_projection, status_head) {
        (Some(channel), Some((generation, attempt))) => tokio::select! {
            _ = connect_cancel.cancelled() => {
                return Err("WebSocket connection cancelled".to_string());
            }
            prepared = prepare_status_session(state, &url, channel, generation, attempt) => prepared,
        },
        _ => None,
    };
    if prepared_status.is_some()
        && manager.status_head().await
            != prepared_status
                .as_ref()
                .map(|prepared| (prepared.scope.generation, prepared.scope.attempt))
    {
        return Err("WebSocket connection scope changed".to_string());
    }
    open_connection_with_projection(manager, &url, on_message, prepared_status, connect_cancel)
        .await
}

pub(crate) async fn send_message(
    manager: &WebSocketManager,
    id: Id,
    message: WebSocketMessage,
) -> Result<(), String> {
    // Egress guard: the NIP-49 local key backup must never reach a relay.
    // This is the single choke point for all webview-originated websocket
    // frames (see `crate::egress_guard`).
    match &message {
        WebSocketMessage::Text(text) => {
            crate::egress_guard::assert_no_key_backup(text, "websocket text frame")?
        }
        WebSocketMessage::Binary(bytes) => {
            crate::egress_guard::assert_no_key_backup_bytes(bytes, "websocket binary frame")?
        }
        _ => {}
    }
    let handle = manager
        .connections
        .lock()
        .await
        .get(&id)
        .cloned()
        .ok_or_else(|| format!("WebSocket connection {id} not found"))?;
    let (result_tx, result_rx) = oneshot::channel();
    tokio::time::timeout(
        WRITE_TIMEOUT,
        handle.sender.send(SendRequest {
            message: message.into(),
            result: result_tx,
        }),
    )
    .await
    .map_err(|_| "WebSocket send queue timed out".to_string())?
    .map_err(|_| "WebSocket connection closed".to_string())?;

    tokio::time::timeout(WRITE_TIMEOUT, result_rx)
        .await
        .map_err(|_| "WebSocket send timed out".to_string())?
        .map_err(|_| "WebSocket connection closed".to_string())?
}

#[tauri::command]
async fn send(
    manager: tauri::State<'_, WebSocketManager>,
    id: Id,
    message: WebSocketMessage,
) -> Result<(), String> {
    send_message(manager.inner(), id, message).await
}

#[tauri::command]
async fn disconnect(manager: tauri::State<'_, WebSocketManager>, id: Id) -> Result<(), String> {
    manager.disconnect(id).await;
    Ok(())
}

#[tauri::command]
async fn disconnect_all(manager: tauri::State<'_, WebSocketManager>) -> Result<(), String> {
    manager.disconnect_all().await;
    Ok(())
}

#[cfg(test)]
async fn run_connection<S>(
    id: Id,
    socket: tokio_tungstenite::WebSocketStream<S>,
    receiver: mpsc::Receiver<SendRequest>,
    cancel: CancellationToken,
    on_message: Channel<serde_json::Value>,
    manager: WebSocketManager,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let Some(handle) = manager.connections.lock().await.get(&id).cloned() else {
        return;
    };
    run_connection_inner(
        id, socket, receiver, cancel, on_message, manager, handle, None,
    )
    .await;
}

#[allow(clippy::too_many_arguments)]
async fn run_connection_inner<S>(
    id: Id,
    mut socket: tokio_tungstenite::WebSocketStream<S>,
    mut receiver: mpsc::Receiver<SendRequest>,
    cancel: CancellationToken,
    on_message: Channel<serde_json::Value>,
    manager: WebSocketManager,
    handle: Arc<ConnectionHandle>,
    mut status_session: Option<ClientBindingStatusSession>,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                let _ = tokio::time::timeout(
                    SHUTDOWN_TIMEOUT,
                    socket.send(Message::Close(Some(CloseFrame {
                        code: CloseCode::Normal,
                        reason: "disconnect".into(),
                    }))),
                ).await;
                break;
            }
            request = receiver.recv() => {
                let Some(request) = request else { break };
                let result = tokio::time::timeout(WRITE_TIMEOUT, socket.send(request.message))
                    .await
                    .map_err(|_| "WebSocket send timed out".to_string())
                    .and_then(|result| result.map_err(|error| error.to_string()));
                let failed = result.is_err();
                let _ = request.result.send(result);
                if failed { break; }
            }
            incoming = socket.next() => {
                let message = match incoming {
                    Some(Ok(message)) => {
                        if let Message::Text(text) = &message {
                            if let Some(challenge) = nip42_challenge(text) {
                                manager
                                    .record_status_challenge(id, &handle, &challenge)
                                    .await;
                            }
                        }
                        let reserved_text = reserved_text_message(&message);
                        if let Some(text) = reserved_text {
                            if let Some(mut session) = status_session.take() {
                                let epoch = session.connection_epoch().clone();
                                #[cfg(test)]
                                let fold_pause = handle.fold_pause.clone();
                                let folded = tauri::async_runtime::spawn_blocking(move || {
                                    #[cfg(test)]
                                    if let Some(fold_pause) = fold_pause {
                                        fold_pause.block();
                                    }
                                    let update = session.consume_text(&text, unix_now());
                                    (session, update)
                                })
                                .await;
                                match folded {
                                    Ok((mut returned_session, update)) => {
                                        let update = if matches!(
                                            returned_session.expire(unix_now()),
                                            ProjectionUpdate::Clear
                                        ) {
                                            Some(ProjectionUpdate::Clear)
                                        } else {
                                            update
                                        };
                                        status_session = Some(returned_session);
                                        if let Some(update) = update {
                                            manager
                                                .apply_projection_update(
                                                    id, &handle, &epoch, update,
                                                )
                                                .await;
                                        }
                                    }
                                    Err(_) => {
                                        manager
                                            .clear_projection_if_owner(id, &handle, &epoch)
                                            .await;
                                    }
                                }
                            }
                            continue;
                        }
                        outbound_message(message)
                    }
                    Some(Err(error)) => OutboundMessage::Error(error.to_string()),
                    None => OutboundMessage::Close(None),
                };
                let terminal = matches!(message, OutboundMessage::Close(_) | OutboundMessage::Error(_));
                if let Ok(value) = serde_json::to_value(message) {
                    let _ = on_message.send(value);
                }
                if terminal { break; }
            }
        }
    }
    if let Some(session) = status_session.as_mut() {
        let epoch = session.connection_epoch().clone();
        let update = session.disconnect();
        manager
            .apply_projection_update(id, &handle, &epoch, update)
            .await;
        manager.clear_projection_if_owner(id, &handle, &epoch).await;
    }
    manager.remove_if_current(id, &handle).await;
}

fn reserved_text_message(message: &Message) -> Option<String> {
    match message {
        Message::Text(value) if is_reserved_text(value) => Some(value.to_string()),
        _ => None,
    }
}

fn nip42_challenge(text: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let values = value.as_array()?;
    if values.len() != 2 || values.first().and_then(serde_json::Value::as_str) != Some("AUTH") {
        return None;
    }
    values.get(1)?.as_str().map(str::to_owned)
}

fn outbound_message(message: Message) -> OutboundMessage {
    match message {
        Message::Text(value) => OutboundMessage::Text(value.to_string()),
        Message::Binary(value) => OutboundMessage::Binary(value.to_vec()),
        Message::Ping(value) => OutboundMessage::Ping(value.to_vec()),
        Message::Pong(value) => OutboundMessage::Pong(value.to_vec()),
        Message::Close(frame) => OutboundMessage::Close(frame.map(|frame| CloseFramePayloadOut {
            code: frame.code.into(),
            reason: frame.reason.to_string(),
        })),
        Message::Frame(_) => OutboundMessage::Error("unexpected raw WebSocket frame".to_string()),
    }
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    install_crypto_provider();
    tauri::plugin::Builder::new("websocket")
        .invoke_handler(tauri::generate_handler![
            connect,
            connect_with_status,
            send,
            disconnect,
            disconnect_all
        ])
        .setup(|app, _api| {
            app.manage(WebSocketManager::default());
            Ok(())
        })
        .build()
}

#[cfg(test)]
#[path = "native_websocket_tests.rs"]
mod tests;
