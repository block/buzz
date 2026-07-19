use std::{collections::HashMap, sync::Arc, time::Duration};

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tauri::{ipc::Channel, plugin::TauriPlugin, Manager, Runtime};
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio_tungstenite::{
    connect_async,
    tungstenite::protocol::{frame::coding::CloseCode, CloseFrame, Message},
};
use tokio_util::sync::CancellationToken;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_millis(250);
const SEND_QUEUE_CAPACITY: usize = 64;

pub(crate) fn install_crypto_provider() {
    // Dependencies enable both rustls providers; choose one before TLS setup.
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
}

type Id = u32;

#[derive(Debug, Deserialize)]
#[serde(tag = "type", content = "data")]
enum WebSocketMessage {
    Text(String),
    Binary(Vec<u8>),
    Ping(Vec<u8>),
    Pong(Vec<u8>),
    Close(Option<CloseFramePayload>),
}

#[derive(Debug, Deserialize)]
struct CloseFramePayload {
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
}

#[derive(Clone)]
struct WebSocketManager {
    connections: Arc<Mutex<HashMap<Id, Arc<ConnectionHandle>>>>,
    connect_cancel: Arc<Mutex<CancellationToken>>,
}

impl Default for WebSocketManager {
    fn default() -> Self {
        Self {
            connections: Arc::default(),
            connect_cancel: Arc::new(Mutex::new(CancellationToken::new())),
        }
    }
}

impl WebSocketManager {
    async fn remove(&self, id: Id) -> Option<Arc<ConnectionHandle>> {
        self.connections.lock().await.remove(&id)
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
            Self::disconnect_handle(handle).await;
        }
    }
}

async fn open_connection(
    manager: &WebSocketManager,
    url: &str,
    on_message: Channel<serde_json::Value>,
) -> Result<Id, String> {
    let connect_cancel = manager.connect_cancel.lock().await.clone();
    let (socket, _) = tokio::select! {
        _ = connect_cancel.cancelled() => return Err("WebSocket connection cancelled".to_string()),
        result = tokio::time::timeout(CONNECT_TIMEOUT, connect_async(url)) => result
            .map_err(|_| "WebSocket connection timed out".to_string())?
            .map_err(|error| error.to_string())?,
    };

    // Serialize registration with disconnect_all so a reload cannot miss a
    // connection that finished its handshake concurrently with teardown.
    let current_connect_cancel = manager.connect_cancel.lock().await;
    if connect_cancel.is_cancelled() {
        return Err("WebSocket connection cancelled".to_string());
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
    });
    let mut task_slot = handle.task.lock().await;
    manager.connections.lock().await.insert(id, handle.clone());

    let task_manager = manager.clone();
    let task = tauri::async_runtime::spawn(run_connection(
        id,
        socket,
        receiver,
        cancel,
        on_message,
        task_manager,
    ));
    *task_slot = Some(task);
    drop(task_slot);
    drop(current_connect_cancel);
    Ok(id)
}

#[tauri::command]
async fn connect(
    manager: tauri::State<'_, WebSocketManager>,
    url: String,
    on_message: Channel<serde_json::Value>,
    _config: Option<serde_json::Value>,
) -> Result<Id, String> {
    open_connection(manager.inner(), &url, on_message).await
}

async fn send_message(
    manager: &WebSocketManager,
    id: Id,
    message: WebSocketMessage,
) -> Result<(), String> {
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
    let mut connect_cancel = manager.connect_cancel.lock().await;
    connect_cancel.cancel();
    *connect_cancel = CancellationToken::new();
    let handles = {
        let mut connections = manager.connections.lock().await;
        connections
            .drain()
            .map(|(_, handle)| handle)
            .collect::<Vec<_>>()
    };
    futures_util::future::join_all(handles.into_iter().map(WebSocketManager::disconnect_handle))
        .await;
    Ok(())
}

async fn run_connection<S>(
    id: Id,
    mut socket: tokio_tungstenite::WebSocketStream<S>,
    mut receiver: mpsc::Receiver<SendRequest>,
    cancel: CancellationToken,
    on_message: Channel<serde_json::Value>,
    manager: WebSocketManager,
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
                    Some(Ok(message)) => outbound_message(message),
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
    manager.remove(id).await;
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
mod tests {
    use super::*;
    use futures_util::FutureExt;
    use std::sync::atomic::{AtomicBool, Ordering};

    use tauri::ipc::InvokeResponseBody;
    use tokio::io::duplex;
    use tokio_tungstenite::{tungstenite::protocol::Role, WebSocketStream};

    fn silent_channel() -> Channel<serde_json::Value> {
        Channel::new(|_: InvokeResponseBody| Ok(()))
    }

    #[tokio::test]
    async fn secure_websocket_reaches_tls_without_panicking() {
        install_crypto_provider();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (_stream, _) = listener.accept().await.unwrap();
            tokio::time::sleep(Duration::from_millis(100)).await;
        });
        let result = std::panic::AssertUnwindSafe(tokio_tungstenite::connect_async(format!(
            "wss://{address}"
        )))
        .catch_unwind()
        .await;

        assert!(result.is_ok(), "TLS setup must not panic");
        server.await.unwrap();
    }

    #[tokio::test]
    async fn live_tcp_server_connect_send_and_disconnect() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (received_tx, received_rx) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let message = socket.next().await.unwrap().unwrap();
            received_tx.send(message).unwrap();
            while let Some(message) = socket.next().await {
                if matches!(message, Ok(Message::Close(_))) {
                    break;
                }
            }
        });

        let manager = WebSocketManager::default();
        let id = open_connection(&manager, &format!("ws://{address}"), silent_channel())
            .await
            .unwrap();
        send_message(&manager, id, WebSocketMessage::Text("live-probe".into()))
            .await
            .unwrap();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), received_rx)
                .await
                .unwrap()
                .unwrap(),
            Message::Text("live-probe".into())
        );

        manager.disconnect(id).await;
        assert!(!manager.connections.lock().await.contains_key(&id));
        tokio::time::timeout(Duration::from_secs(1), server)
            .await
            .expect("live server should observe native socket shutdown")
            .unwrap();
    }

    #[tokio::test]
    async fn eof_removes_connection() {
        let manager = WebSocketManager::default();
        let (client_io, server_io) = duplex(1024);
        let (client, server) = tokio::join!(
            WebSocketStream::from_raw_socket(client_io, Role::Client, None),
            WebSocketStream::from_raw_socket(server_io, Role::Server, None),
        );
        let (sender, receiver) = mpsc::channel(SEND_QUEUE_CAPACITY);
        let handle = Arc::new(ConnectionHandle {
            sender,
            cancel: CancellationToken::new(),
            task: Mutex::new(None),
        });
        manager.connections.lock().await.insert(1, handle.clone());
        let task = tauri::async_runtime::spawn(run_connection(
            1,
            client,
            receiver,
            handle.cancel.clone(),
            silent_channel(),
            manager.clone(),
        ));
        *handle.task.lock().await = Some(task);

        drop(server);
        tokio::time::timeout(Duration::from_secs(1), async {
            while manager.connections.lock().await.contains_key(&1) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("EOF should clean up its native connection ID");
    }

    #[tokio::test]
    async fn disconnect_removes_and_drops_task_before_returning() {
        struct DropGuard(Arc<AtomicBool>);
        impl Drop for DropGuard {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }

        let manager = WebSocketManager::default();
        let dropped = Arc::new(AtomicBool::new(false));
        let task_dropped = dropped.clone();
        let (ready_tx, ready_rx) = oneshot::channel();
        let (sender, _receiver) = mpsc::channel(SEND_QUEUE_CAPACITY);
        let handle = Arc::new(ConnectionHandle {
            sender,
            cancel: CancellationToken::new(),
            task: Mutex::new(Some(tauri::async_runtime::spawn(async move {
                let _guard = DropGuard(task_dropped);
                ready_tx.send(()).unwrap();
                std::future::pending::<()>().await;
            }))),
        });
        manager.connections.lock().await.insert(7, handle);
        ready_rx.await.unwrap();

        tokio::time::timeout(Duration::from_secs(1), manager.disconnect(7))
            .await
            .expect("disconnect should abort an unresponsive task");
        assert!(!manager.connections.lock().await.contains_key(&7));
        assert!(dropped.load(Ordering::SeqCst));

        // Repeated teardown is intentionally a no-op.
        manager.disconnect(7).await;
    }

    #[tokio::test]
    async fn teardown_gate_stays_closed_until_tasks_stop() {
        let manager = WebSocketManager::default();
        let gate = manager.connect_cancel.lock().await;
        let (sender, _receiver) = mpsc::channel(SEND_QUEUE_CAPACITY);
        let handle = Arc::new(ConnectionHandle {
            sender,
            cancel: CancellationToken::new(),
            task: Mutex::new(Some(tauri::async_runtime::spawn(async {
                std::future::pending::<()>().await;
            }))),
        });
        manager.connections.lock().await.insert(1, handle);
        gate.cancel();
        let handles = {
            let mut connections = manager.connections.lock().await;
            connections
                .drain()
                .map(|(_, handle)| handle)
                .collect::<Vec<_>>()
        };

        let shutdown = futures_util::future::join_all(
            handles.into_iter().map(WebSocketManager::disconnect_handle),
        );
        assert!(manager.connect_cancel.try_lock().is_err());
        shutdown.await;
        drop(gate);
        assert!(manager.connect_cancel.try_lock().is_ok());
    }

    #[tokio::test]
    async fn one_connection_does_not_block_another_send_queue() {
        let manager = WebSocketManager::default();
        let (blocked_sender, blocked_receiver) = mpsc::channel(1);
        blocked_sender
            .send(SendRequest {
                message: Message::Text("blocked".into()),
                result: oneshot::channel().0,
            })
            .await
            .unwrap();
        let blocked = Arc::new(ConnectionHandle {
            sender: blocked_sender,
            cancel: CancellationToken::new(),
            task: Mutex::new(None),
        });
        manager.connections.lock().await.insert(1, blocked);

        let (healthy_sender, mut healthy_receiver) = mpsc::channel(1);
        let healthy = Arc::new(ConnectionHandle {
            sender: healthy_sender.clone(),
            cancel: CancellationToken::new(),
            task: Mutex::new(None),
        });
        manager.connections.lock().await.insert(2, healthy);

        let (result, _) = oneshot::channel();
        tokio::time::timeout(
            Duration::from_millis(50),
            healthy_sender.send(SendRequest {
                message: Message::Text("healthy".into()),
                result,
            }),
        )
        .await
        .expect("a full queue on one connection must not block another")
        .unwrap();
        assert!(healthy_receiver.recv().await.is_some());
        drop(blocked_receiver);
    }
}
