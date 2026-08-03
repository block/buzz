//! Bounded, capability-authenticated loopback control server.

use std::{
    future::Future,
    io,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    pin::Pin,
    sync::Arc,
    time::Duration,
};
use subtle::ConstantTimeEq;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{OwnedSemaphorePermit, Semaphore},
    time::timeout,
};
use uuid::Uuid;

use crate::protocol::{
    AuthorizedCapability, ControlError, ControlOperation, ControlPayload, ControlRequest,
    ControlResponse, HelloResponse, SecretToken, CONTROL_DEADLINE_SECS, CONTROL_PROTOCOL_VERSION,
    MAX_ASSIGNMENT_TEXT_BYTES, MAX_CONTROL_REQUEST_BYTES, MAX_CONTROL_RESPONSE_BYTES,
};

// A stalled unauthenticated peer can hold at most one request-sized buffer. Keep
// this small and acquire before spawning so task, descriptor, and buffer growth
// are all bounded by the same budget.
const MAX_PRE_AUTH_CONNECTIONS: usize = 16;

/// Boxed asynchronous handler result used by the control-server seam.
pub type HandlerFuture<'a> =
    Pin<Box<dyn Future<Output = Result<ControlPayload, ControlError>> + Send + 'a>>;

/// Operation handler implemented by the privileged runtime supervisor.
pub trait ControlHandler: Send + Sync + 'static {
    /// Handles one already-authenticated and capability-authorized operation.
    fn handle(
        &self,
        capability: AuthorizedCapability,
        operation: ControlOperation,
    ) -> HandlerFuture<'_>;
}

/// Closure adapter for handlers that return an owned future.
pub struct ControlHandlerFn<F>(pub F);
impl<F, Fut> ControlHandler for ControlHandlerFn<F>
where
    F: Fn(AuthorizedCapability, ControlOperation) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<ControlPayload, ControlError>> + Send + 'static,
{
    fn handle(
        &self,
        capability: AuthorizedCapability,
        operation: ControlOperation,
    ) -> HandlerFuture<'_> {
        Box::pin((self.0)(capability, operation))
    }
}

/// Immutable control listener configuration for one runtime generation.
#[derive(Clone)]
pub struct ControlServerConfig {
    pub bind_addr: SocketAddr,
    pub runtime_id: String,
    pub generation: Uuid,
    pub controller_token: SecretToken,
    pub model_token: SecretToken,
}
impl std::fmt::Debug for ControlServerConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ControlServerConfig")
            .field("bind_addr", &self.bind_addr)
            .field("runtime_id", &self.runtime_id)
            .field("generation", &self.generation)
            .field("controller_token", &self.controller_token)
            .field("model_token", &self.model_token)
            .finish()
    }
}
impl ControlServerConfig {
    /// Creates a loopback-only configuration with independently generated capabilities.
    pub fn new(runtime_id: String, generation: Uuid) -> Self {
        Self {
            bind_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
            runtime_id,
            generation,
            controller_token: SecretToken::generate(),
            model_token: SecretToken::generate(),
        }
    }
}

/// Bound control listener. Calling `serve` consumes it and accepts until cancelled.
pub struct RuntimeServer {
    listener: TcpListener,
    config: Arc<ControlServerConfig>,
    pre_auth: Arc<Semaphore>,
}
impl RuntimeServer {
    /// Binds the configured address, rejecting non-loopback addresses before IO.
    pub async fn bind(config: ControlServerConfig) -> Result<Self, ServerError> {
        if !config.bind_addr.ip().is_loopback() {
            return Err(ServerError::NonLoopback);
        }
        let listener = TcpListener::bind(config.bind_addr).await?;
        Ok(Self {
            listener,
            config: Arc::new(config),
            pre_auth: Arc::new(Semaphore::new(MAX_PRE_AUTH_CONNECTIONS)),
        })
    }
    /// Returns the actual loopback address, including an assigned ephemeral port.
    pub fn local_addr(&self) -> Result<SocketAddr, ServerError> {
        Ok(self.listener.local_addr()?)
    }
    /// Returns generation and redacted-capability configuration for receipt construction.
    pub fn config(&self) -> &ControlServerConfig {
        &self.config
    }
    /// Serves one-request/one-response connections until the task is cancelled.
    pub async fn serve(self, handler: Arc<dyn ControlHandler>) -> Result<(), ServerError> {
        loop {
            let (stream, peer) = self.listener.accept().await?;
            if !peer.ip().is_loopback() {
                continue;
            }
            let permit = match Arc::clone(&self.pre_auth).try_acquire_owned() {
                Ok(permit) => permit,
                Err(_) => continue,
            };
            let config = Arc::clone(&self.config);
            let handler = Arc::clone(&handler);
            tokio::spawn(async move {
                let _ = serve_connection(stream, config, handler, permit).await;
            });
        }
    }
}

/// Frame and server failure.
#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error("control IO failed: {0}")]
    Io(#[from] io::Error),
    #[error("control frame length {announced} exceeds {maximum} bytes")]
    FrameTooLarge { announced: usize, maximum: usize },
    #[error("control JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("control operation timed out")]
    Timeout,
    #[error("control server address is not loopback")]
    NonLoopback,
}

/// Reads a four-byte big-endian length and rejects oversize before allocating its payload.
pub async fn read_bounded_frame<R: AsyncRead + Unpin>(
    reader: &mut R,
    maximum: usize,
) -> Result<Vec<u8>, ServerError> {
    let mut header = [0_u8; 4];
    reader.read_exact(&mut header).await?;
    let announced = u32::from_be_bytes(header) as usize;
    if announced > maximum {
        return Err(ServerError::FrameTooLarge { announced, maximum });
    }
    let mut payload = vec![0_u8; announced];
    reader.read_exact(&mut payload).await?;
    Ok(payload)
}

/// Writes one four-byte big-endian frame after checking the complete payload bound.
pub async fn write_bounded_frame<W: AsyncWrite + Unpin>(
    writer: &mut W,
    payload: &[u8],
    maximum: usize,
) -> Result<(), ServerError> {
    if payload.len() > maximum || payload.len() > u32::MAX as usize {
        return Err(ServerError::FrameTooLarge {
            announced: payload.len(),
            maximum,
        });
    }
    writer
        .write_all(&(payload.len() as u32).to_be_bytes())
        .await?;
    writer.write_all(payload).await?;
    writer.flush().await?;
    Ok(())
}

enum Handshake {
    Authenticated(ControlRequest, AuthorizedCapability),
    Unauthorized,
}

async fn read_handshake(
    stream: &mut TcpStream,
    config: &ControlServerConfig,
) -> Result<Handshake, ServerError> {
    let payload = read_bounded_frame(stream, MAX_CONTROL_REQUEST_BYTES).await?;
    let Ok(request) = serde_json::from_slice::<ControlRequest>(&payload) else {
        return Ok(Handshake::Unauthorized);
    };
    Ok(match authenticate(&request, config) {
        Some(capability) => Handshake::Authenticated(request, capability),
        None => Handshake::Unauthorized,
    })
}

async fn serve_connection(
    mut stream: TcpStream,
    config: Arc<ControlServerConfig>,
    handler: Arc<dyn ControlHandler>,
    pre_auth_permit: OwnedSemaphorePermit,
) -> Result<(), ServerError> {
    let deadline = Duration::from_secs(CONTROL_DEADLINE_SECS);
    let handshake = timeout(deadline, read_handshake(&mut stream, &config))
        .await
        .map_err(|_| ServerError::Timeout)??;
    let response = match handshake {
        Handshake::Authenticated(request, capability) => {
            // Authentication is the boundary: privileged operations may outlive
            // the handshake without starving new clients of pre-auth capacity.
            drop(pre_auth_permit);
            dispatch_authenticated(request, capability, &config, handler.as_ref()).await
        }
        Handshake::Unauthorized => {
            let _pre_auth_permit = pre_auth_permit;
            let response = ControlResponse::failure(ControlError::unauthorized());
            let bytes = serde_json::to_vec(&response)?;
            timeout(
                deadline,
                write_bounded_frame(&mut stream, &bytes, MAX_CONTROL_RESPONSE_BYTES),
            )
            .await
            .map_err(|_| ServerError::Timeout)??;
            return Ok(());
        }
    };
    let bytes = serde_json::to_vec(&response)?;
    timeout(
        deadline,
        write_bounded_frame(&mut stream, &bytes, MAX_CONTROL_RESPONSE_BYTES),
    )
    .await
    .map_err(|_| ServerError::Timeout)??;
    Ok(())
}

async fn dispatch_authenticated(
    request: ControlRequest,
    capability: AuthorizedCapability,
    config: &ControlServerConfig,
    handler: &dyn ControlHandler,
) -> ControlResponse {
    if capability == AuthorizedCapability::Model && !request.operation.model_allowed() {
        return ControlResponse::failure(ControlError::unauthorized());
    }
    if request.operation == ControlOperation::Hello {
        let name = if capability == AuthorizedCapability::Controller {
            "controller"
        } else {
            "model"
        };
        return ControlResponse::success(ControlPayload::Hello(HelloResponse {
            runtime_id: config.runtime_id.clone(),
            generation: config.generation,
            capability: name.into(),
        }));
    }
    if let ControlOperation::JobsStart(start) = &request.operation {
        if let Err(error) = start.validate() {
            return ControlResponse::failure(ControlError::new(
                "invalid_request",
                error.to_string(),
            ));
        }
    }
    if let ControlOperation::AssignmentSetState {
        assignment_id,
        request,
    } = &request.operation
    {
        if assignment_id.trim().is_empty() || assignment_id.len() > MAX_ASSIGNMENT_TEXT_BYTES {
            return ControlResponse::failure(ControlError::new(
                "invalid_request",
                "assignment id is required",
            ));
        }
        if let Err(error) = request.validate() {
            return ControlResponse::failure(ControlError::new(
                "invalid_request",
                error.to_string(),
            ));
        }
    }
    match handler.handle(capability, request.operation).await {
        Ok(payload) => ControlResponse::success(payload),
        Err(error) => ControlResponse::failure(error),
    }
}

fn authenticate(
    request: &ControlRequest,
    config: &ControlServerConfig,
) -> Option<AuthorizedCapability> {
    if request.protocol_version != CONTROL_PROTOCOL_VERSION
        || request.generation != config.generation
    {
        return None;
    }
    if token_eq(&request.control_token, &config.controller_token) {
        return Some(AuthorizedCapability::Controller);
    }
    if token_eq(&request.control_token, &config.model_token) {
        return Some(AuthorizedCapability::Model);
    }
    None
}
fn token_eq(left: &SecretToken, right: &SecretToken) -> bool {
    let left = left.expose_secret().as_bytes();
    let right = right.expose_secret().as_bytes();
    left.len() == right.len() && bool::from(left.ct_eq(right))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::AsyncReadExt;

    fn test_config() -> ControlServerConfig {
        ControlServerConfig {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            runtime_id: "runtime".into(),
            generation: Uuid::new_v4(),
            controller_token: SecretToken::new("controller"),
            model_token: SecretToken::new("model"),
        }
    }

    async fn wait_for_permits(semaphore: &Semaphore, expected: usize, wait: Duration) {
        timeout(wait, async {
            while semaphore.available_permits() != expected {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }

    async fn wait_for_owners(semaphore: &Arc<Semaphore>, expected: usize, wait: Duration) {
        timeout(wait, async {
            while Arc::strong_count(semaphore) != expected {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }
    async fn send_request(
        address: SocketAddr,
        control_request: ControlRequest,
    ) -> Result<ControlResponse, ServerError> {
        let mut stream = TcpStream::connect(address).await?;
        let bytes = serde_json::to_vec(&control_request)?;
        write_bounded_frame(&mut stream, &bytes, MAX_CONTROL_REQUEST_BYTES).await?;
        let response = read_bounded_frame(&mut stream, MAX_CONTROL_RESPONSE_BYTES).await?;
        Ok(serde_json::from_slice(&response)?)
    }

    #[tokio::test]
    async fn unauthenticated_connection_stress_stays_within_pre_auth_budget() {
        let config = test_config();
        let valid_request = ControlRequest {
            protocol_version: CONTROL_PROTOCOL_VERSION,
            generation: config.generation,
            control_token: config.controller_token.clone(),
            operation: ControlOperation::Hello,
        };
        let server = RuntimeServer::bind(config).await.unwrap();
        let address = server.local_addr().unwrap();
        let pre_auth = Arc::clone(&server.pre_auth);
        let server_task = tokio::spawn(server.serve(Arc::new(ControlHandlerFn(
            |_capability, _operation| async { Ok::<_, ControlError>(ControlPayload::Ack) },
        ))));

        // Every admitted peer stalls at a different handshake point. The
        // request-sized cases exercise the maximum allocation per permit.
        let mut admitted = Vec::with_capacity(MAX_PRE_AUTH_CONNECTIONS);
        for index in 0..MAX_PRE_AUTH_CONNECTIONS {
            let mut stream = TcpStream::connect(address).await.unwrap();
            match index % 3 {
                0 => {}
                1 => stream.write_all(&[0]).await.unwrap(),
                _ => {
                    stream
                        .write_all(&(MAX_CONTROL_REQUEST_BYTES as u32).to_be_bytes())
                        .await
                        .unwrap();
                    stream.write_all(b"{").await.unwrap();
                }
            }
            admitted.push(stream);
        }
        wait_for_permits(&pre_auth, 0, Duration::from_secs(2)).await;
        assert_eq!(
            Arc::strong_count(&pre_auth),
            MAX_PRE_AUTH_CONNECTIONS + 2,
            "only the server, observer, and bounded permit holders own the semaphore"
        );

        // Saturated peers are accepted and dropped before a task is spawned or
        // their header is read, regardless of how much they announce.
        let mut rejected = Vec::with_capacity(MAX_PRE_AUTH_CONNECTIONS * 4);
        for index in 0..MAX_PRE_AUTH_CONNECTIONS * 4 {
            let mut stream = TcpStream::connect(address).await.unwrap();
            let write = match index % 3 {
                0 => Ok(()),
                1 => stream.write_all(&[0, 0]).await,
                _ => {
                    stream
                        .write_all(&((MAX_CONTROL_REQUEST_BYTES as u32) + 1).to_be_bytes())
                        .await
                }
            };
            let _ = write;
            rejected.push(stream);
        }
        for mut stream in rejected {
            let mut byte = [0_u8; 1];
            match timeout(Duration::from_secs(2), stream.read(&mut byte)).await {
                Ok(Ok(0)) | Ok(Err(_)) => {}
                other => panic!("saturated connection was not closed immediately: {other:?}"),
            }
        }
        assert_eq!(pre_auth.available_permits(), 0);
        assert_eq!(Arc::strong_count(&pre_auth), MAX_PRE_AUTH_CONNECTIONS + 2);

        // The single handshake deadline releases every stalled slot without
        // requiring the clients to disconnect. A valid retry then succeeds.
        wait_for_permits(
            &pre_auth,
            MAX_PRE_AUTH_CONNECTIONS,
            Duration::from_secs(CONTROL_DEADLINE_SECS + 2),
        )
        .await;
        wait_for_owners(&pre_auth, 2, Duration::from_secs(2)).await;
        assert_eq!(Arc::strong_count(&pre_auth), 2);
        let response = send_request(address, valid_request).await.unwrap();
        assert!(matches!(response.result, Some(ControlPayload::Hello(_))));
        assert!(response.error.is_none());
        drop(admitted);
        server_task.abort();
    }

    #[tokio::test]
    async fn authenticated_long_operations_release_pre_auth_budget() {
        let config = test_config();
        let operation_count = MAX_PRE_AUTH_CONNECTIONS * 2;
        let entered = Arc::new(AtomicUsize::new(0));
        let release = Arc::new(Semaphore::new(0));
        let handler_entered = Arc::clone(&entered);
        let handler_release = Arc::clone(&release);
        let server = RuntimeServer::bind(config.clone()).await.unwrap();
        let address = server.local_addr().unwrap();
        let pre_auth = Arc::clone(&server.pre_auth);
        let server_task = tokio::spawn(server.serve(Arc::new(ControlHandlerFn(
            move |_capability, _operation| {
                let entered = Arc::clone(&handler_entered);
                let release = Arc::clone(&handler_release);
                async move {
                    entered.fetch_add(1, Ordering::SeqCst);
                    release.acquire_owned().await.unwrap().forget();
                    Ok::<_, ControlError>(ControlPayload::Ack)
                }
            },
        ))));

        let mut clients = Vec::with_capacity(operation_count);
        for expected in 1..=operation_count {
            let control_request = ControlRequest {
                protocol_version: CONTROL_PROTOCOL_VERSION,
                generation: config.generation,
                control_token: config.controller_token.clone(),
                operation: ControlOperation::Status,
            };
            clients.push(tokio::spawn(send_request(address, control_request)));
            timeout(Duration::from_secs(2), async {
                while entered.load(Ordering::SeqCst) != expected {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            assert_eq!(pre_auth.available_permits(), MAX_PRE_AUTH_CONNECTIONS);
        }
        assert_eq!(
            pre_auth.available_permits(),
            MAX_PRE_AUTH_CONNECTIONS,
            "authenticated handlers must not retain pre-auth permits"
        );
        assert_eq!(Arc::strong_count(&pre_auth), 2);

        release.add_permits(operation_count);
        for client in clients {
            let response = client.await.unwrap().unwrap();
            assert_eq!(response.result, Some(ControlPayload::Ack));
            assert!(response.error.is_none());
        }
        server_task.abort();
    }
}
