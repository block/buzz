//! Reliable-stream tunnel routing for berd ↔ goose-server sessions.
//!
//! This module owns the relay-side session decision for `Profile::ReliableStream`:
//! first join acquires the Redis fenced lease and becomes the owner runtime;
//! later joins on the owner stay local, while later joins on other runtimes open
//! a fenced mesh bi-stream to the owner. The caller remains responsible for the
//! client-facing WebSocket/bridge bytes; this module supplies the routing and
//! mesh-frame discipline shared by that handler.

use std::sync::Arc;
use std::time::Duration;

use buzz_core::CommunityId;
use buzz_relay_mesh::{
    FencedHeader, GoodbyeReason, MeshError, MeshStream, MeshStreamFrame, Profile,
    RelayPeerTransport, RuntimeId, StreamHello, StreamRole,
};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::directory::{
    AcquireResult, DirectoryError, ReleaseResult, RenewResult, SessionDirectory, SessionLease,
};

/// Maximum reliable-stream application chunk per mesh frame.
///
/// The wire cap is 16 MiB (`buzz_relay_mesh::wire::MAX_STREAM_FRAME`), but using
/// 1 MiB chunks keeps per-frame allocations bounded and comfortably supports
/// goose's 50 MiB HTTP bodies by streaming multiple ordered frames.
pub const MAX_RELIABLE_PAYLOAD_BYTES: usize = 1024 * 1024;

/// Renewal cadence for the default 30s session lease.
const DEFAULT_RENEW_INTERVAL: Duration = Duration::from_secs(10);

/// Relay-side router for reliable tunnel joins.
#[derive(Clone)]
pub struct ReliableStreamRouter<T: ?Sized> {
    directory: SessionDirectory,
    transport: Arc<T>,
    local_runtime_id: RuntimeId,
}

impl<T> ReliableStreamRouter<T>
where
    T: RelayPeerTransport + ?Sized,
{
    /// Create a reliable-stream router from the fenced directory, mesh transport,
    /// and this process's boot-unique runtime id.
    pub fn new(
        directory: SessionDirectory,
        transport: Arc<T>,
        local_runtime_id: RuntimeId,
    ) -> Self {
        Self {
            directory,
            transport,
            local_runtime_id,
        }
    }

    /// Return the fenced session directory used by this router.
    pub fn directory(&self) -> &SessionDirectory {
        &self.directory
    }

    /// Return this process's local mesh runtime id.
    pub fn local_runtime_id(&self) -> RuntimeId {
        self.local_runtime_id
    }

    /// Join a reliable-stream session from a client connected to this runtime.
    ///
    /// If no lease exists, this runtime becomes the owner. If the owner is this
    /// runtime, the caller should pair the client locally with the owner-side
    /// session hub. If another runtime owns the session, this opens a reliable
    /// mesh bi-stream to that owner and sends the required `Hello` first.
    pub async fn join(
        &self,
        community_id: CommunityId,
        session_id: Uuid,
    ) -> Result<ReliableJoin, ReliableStreamError> {
        let lease = match self
            .directory
            .acquire(
                community_id,
                session_id,
                self.local_runtime_id,
                Profile::ReliableStream,
            )
            .await?
        {
            AcquireResult::Acquired(lease) => return Ok(ReliableJoin::Owned { lease }),
            AcquireResult::Exists(lease) => lease,
        };

        if lease.profile != Profile::ReliableStream {
            return Err(ReliableStreamError::ProfileMismatch {
                session_id,
                expected: Profile::ReliableStream,
                actual: lease.profile,
            });
        }

        if lease.owner_runtime_id == self.local_runtime_id {
            return Ok(ReliableJoin::Owned { lease });
        }

        let fenced = lease.fenced_header();
        let hello = StreamHello {
            sender: self.local_runtime_id,
            role: StreamRole::Session {
                fenced,
                profile: Profile::ReliableStream,
            },
        };
        let stream = self
            .transport
            .open_session_stream(lease.owner_runtime_id, hello)
            .await?;

        Ok(ReliableJoin::Forwarded {
            lease,
            stream: ReliableMeshStream::new(fenced, stream),
        })
    }

    /// Accept an inbound mesh session stream opened by a non-owner runtime.
    ///
    /// The transport layer has already decoded the stream's first `Hello`; this
    /// validates that it is a reliable-stream session, that the claimed sender
    /// matches the authenticated peer, and that the fenced owner is local. Redis
    /// validation starts on the first stateful reliable frame, which carries the
    /// tenant community and is checked before payload delivery.
    pub async fn accept_inbound(
        &self,
        from: RuntimeId,
        hello: StreamHello,
        stream: MeshStream,
    ) -> Result<ReliableInbound, ReliableStreamError> {
        if hello.sender != from {
            return Err(ReliableStreamError::SenderMismatch {
                peer: from,
                hello_sender: hello.sender,
            });
        }

        let StreamRole::Session { fenced, profile } = hello.role else {
            return Err(ReliableStreamError::UnexpectedStreamRole);
        };
        if profile != Profile::ReliableStream {
            return Err(ReliableStreamError::ProfileMismatch {
                session_id: fenced.session_id,
                expected: Profile::ReliableStream,
                actual: profile,
            });
        }

        if fenced.owner_runtime_id != self.local_runtime_id {
            return Err(ReliableStreamError::OwnerIsNotLocal {
                session_id: fenced.session_id,
                owner_runtime_id: fenced.owner_runtime_id,
                local_runtime_id: self.local_runtime_id,
            });
        }

        Ok(ReliableInbound {
            fenced,
            from,
            stream: ReliableMeshStream::new_inbound(fenced, stream),
        })
    }

    /// Start background lease renewal for an owner-side session.
    ///
    /// Losing the fenced lease is fail-loud: the worker exits after logging; the
    /// caller should also validate/write through the fenced stream boundary and
    /// close clients when the session layer observes loss.
    pub fn spawn_renewer(&self, lease: SessionLease, cancel: CancellationToken) -> JoinHandle<()> {
        self.spawn_observable_renewer(lease, cancel).task
    }

    /// Start background lease renewal and return a loss signal consumers can
    /// observe.
    ///
    /// `lost` is cancelled when this runtime loses ownership or Redis renewal
    /// fails, so session consumers can tear down local state (`Room`, generation
    /// floors, client bridges). Caller-initiated `cancel` is treated as normal
    /// shutdown and does not trip the loss signal.
    pub fn spawn_observable_renewer(
        &self,
        lease: SessionLease,
        cancel: CancellationToken,
    ) -> ReliableLeaseRenewer {
        spawn_lease_renewer(self.directory.clone(), lease, cancel)
    }
}

/// Background lease renewer plus an observable ownership-loss signal.
pub struct ReliableLeaseRenewer {
    /// Worker task. Await during teardown if the caller needs release completion.
    pub task: JoinHandle<()>,
    /// Cancelled when renewal observes loss/NotOwner or a renewal error.
    pub lost: CancellationToken,
}

/// Result of a local client joining a reliable tunnel session.
pub enum ReliableJoin {
    /// This runtime owns the fenced session. Pair the client locally and renew
    /// `lease` for the life of the owner-side session.
    Owned {
        /// Fenced ownership lease acquired by this runtime.
        lease: SessionLease,
    },
    /// Another runtime owns the session. Pump client bytes through `stream`.
    Forwarded {
        /// Current owner lease read from Redis.
        lease: SessionLease,
        /// Reliable mesh stream opened to the owner runtime.
        stream: ReliableMeshStream,
    },
}

/// Inbound non-owner stream accepted by the owner runtime.
pub struct ReliableInbound {
    /// Fenced session tuple validated for this inbound stream.
    pub fenced: FencedHeader,
    /// Authenticated peer runtime that opened the stream.
    pub from: RuntimeId,
    /// Reliable stream wrapper pinned to `fenced`.
    pub stream: ReliableMeshStream,
}

/// A reliable mesh stream pinned to one fenced session.
pub struct ReliableMeshStream {
    fenced: FencedHeader,
    stream: MeshStream,
    community_id: Option<CommunityId>,
}

impl ReliableMeshStream {
    /// Wrap a raw mesh stream for one fenced reliable session.
    pub fn new(fenced: FencedHeader, stream: MeshStream) -> Self {
        Self {
            fenced,
            stream,
            community_id: None,
        }
    }

    /// Wrap a raw inbound mesh stream. The community is latched from the first
    /// stateful reliable frame before any payload is delivered.
    pub fn new_inbound(fenced: FencedHeader, stream: MeshStream) -> Self {
        Self::new(fenced, stream)
    }

    /// Return the community/tenant latched from the first stateful frame.
    pub fn community_id(&self) -> Option<CommunityId> {
        self.community_id
    }

    /// Pin this outbound stream to a community before sending payload frames.
    pub fn with_community(mut self, community_id: CommunityId) -> Self {
        self.community_id = Some(community_id);
        self
    }

    /// Return the fenced tuple pinned to this stream.
    pub fn fenced(&self) -> FencedHeader {
        self.fenced
    }

    /// Send bytes as one or more ordered mesh `Data` frames.
    pub async fn send_bytes(
        &mut self,
        community_id: CommunityId,
        bytes: &[u8],
    ) -> Result<(), ReliableStreamError> {
        self.ensure_outbound_community(community_id)?;
        for chunk in bytes.chunks(MAX_RELIABLE_PAYLOAD_BYTES) {
            let payload = ReliableWireFrame::Data {
                community_id,
                payload: chunk.to_vec(),
            }
            .encode();
            self.stream
                .send_frame(MeshStreamFrame::Data {
                    fenced: self.fenced,
                    payload,
                })
                .await?;
        }
        Ok(())
    }

    /// Finish the send half without fabricating a session frame. Used when an
    /// owner-side stream is draining before the first validated frame has
    /// latched its community id.
    pub fn finish(&mut self) -> Result<(), ReliableStreamError> {
        self.stream.finish()?;
        Ok(())
    }

    /// Send a clean reliable-session close frame and finish the send half.
    pub async fn send_goodbye(
        &mut self,
        community_id: CommunityId,
        reason: GoodbyeReason,
    ) -> Result<(), ReliableStreamError> {
        self.ensure_outbound_community(community_id)?;
        let payload = ReliableWireFrame::Goodbye {
            community_id,
            reason,
        }
        .encode();
        self.stream
            .send_frame(MeshStreamFrame::Data {
                fenced: self.fenced,
                payload,
            })
            .await?;
        self.stream.finish()?;
        Ok(())
    }

    /// Receive and validate the next session frame.
    ///
    /// Every incoming `Data`/`Goodbye` frame is checked against both the stream's
    /// pinned fenced tuple and the Redis directory. This is the reliable-stream
    /// equivalent of Dawn's hot-path media floor, but authoritative: stale or
    /// mismatched frames fail the session rather than being dropped silently.
    pub async fn recv_validated(
        &mut self,
        directory: &SessionDirectory,
    ) -> Result<Option<ReliableFrame>, ReliableStreamError> {
        let Some(frame) = self.stream.recv_frame().await? else {
            return Ok(None);
        };

        match frame {
            MeshStreamFrame::Data { fenced, payload } => {
                let frame = ReliableWireFrame::decode(&payload)?;
                let community_id = frame.community_id();
                self.validate_frame_fence(directory, community_id, &fenced)
                    .await?;
                match frame {
                    ReliableWireFrame::Data { payload, .. } => {
                        Ok(Some(ReliableFrame::Data(payload)))
                    }
                    ReliableWireFrame::Goodbye { reason, .. } => {
                        Ok(Some(ReliableFrame::Goodbye(reason)))
                    }
                }
            }
            MeshStreamFrame::Goodbye { .. } => Err(ReliableStreamError::UnexpectedFrame("goodbye")),
            MeshStreamFrame::Hello(_) => Err(ReliableStreamError::UnexpectedFrame("hello")),
            MeshStreamFrame::Gossip { .. } => Err(ReliableStreamError::UnexpectedFrame("gossip")),
        }
    }

    async fn validate_frame_fence(
        &mut self,
        directory: &SessionDirectory,
        community_id: CommunityId,
        fenced: &FencedHeader,
    ) -> Result<(), ReliableStreamError> {
        if *fenced != self.fenced {
            return Err(ReliableStreamError::FrameFenceMismatch {
                expected: self.fenced,
                actual: *fenced,
            });
        }
        match self.community_id {
            Some(expected) if expected != community_id => {
                return Err(ReliableStreamError::CommunityMismatch {
                    expected,
                    actual: community_id,
                });
            }
            Some(_) | None => {}
        }
        directory
            .validate_fenced_header(community_id, fenced)
            .await?;
        if self.community_id.is_none() {
            self.community_id = Some(community_id);
        }
        Ok(())
    }

    fn ensure_outbound_community(
        &mut self,
        community_id: CommunityId,
    ) -> Result<(), ReliableStreamError> {
        match self.community_id {
            Some(expected) if expected != community_id => {
                Err(ReliableStreamError::CommunityMismatch {
                    expected,
                    actual: community_id,
                })
            }
            Some(_) => Ok(()),
            None => {
                self.community_id = Some(community_id);
                Ok(())
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ReliableWireFrame {
    Data {
        community_id: CommunityId,
        payload: Vec<u8>,
    },
    Goodbye {
        community_id: CommunityId,
        reason: GoodbyeReason,
    },
}

impl ReliableWireFrame {
    const VERSION: u8 = 1;
    const DATA: u8 = 1;
    const GOODBYE: u8 = 2;

    fn community_id(&self) -> CommunityId {
        match self {
            Self::Data { community_id, .. } | Self::Goodbye { community_id, .. } => *community_id,
        }
    }

    fn encode(&self) -> Vec<u8> {
        match self {
            Self::Data {
                community_id,
                payload,
            } => {
                let mut encoded = Vec::with_capacity(18 + payload.len());
                encoded.push(Self::VERSION);
                encoded.push(Self::DATA);
                encoded.extend_from_slice(community_id.as_uuid().as_bytes());
                encoded.extend_from_slice(payload);
                encoded
            }
            Self::Goodbye {
                community_id,
                reason,
            } => {
                let mut encoded = Vec::with_capacity(19);
                encoded.push(Self::VERSION);
                encoded.push(Self::GOODBYE);
                encoded.extend_from_slice(community_id.as_uuid().as_bytes());
                encoded.push(reason.to_wire_byte());
                encoded
            }
        }
    }

    fn decode(bytes: &[u8]) -> Result<Self, ReliableStreamError> {
        if bytes.len() < 18 {
            return Err(ReliableStreamError::MalformedReliableFrame("too short"));
        }
        if bytes[0] != Self::VERSION {
            return Err(ReliableStreamError::MalformedReliableFrame(
                "unknown version",
            ));
        }
        let community_id = CommunityId::from_uuid(Uuid::from_bytes(
            bytes[2..18].try_into().expect("16 byte community id slice"),
        ));
        match bytes[1] {
            Self::DATA => Ok(Self::Data {
                community_id,
                payload: bytes[18..].to_vec(),
            }),
            Self::GOODBYE => {
                if bytes.len() != 19 {
                    return Err(ReliableStreamError::MalformedReliableFrame(
                        "bad goodbye length",
                    ));
                }
                let reason = <GoodbyeReason as GoodbyeReasonWireExt>::from_wire_byte(bytes[18])?;
                Ok(Self::Goodbye {
                    community_id,
                    reason,
                })
            }
            _ => Err(ReliableStreamError::MalformedReliableFrame("unknown kind")),
        }
    }
}

trait GoodbyeReasonWireExt {
    fn to_wire_byte(self) -> u8;
    fn from_wire_byte(byte: u8) -> Result<GoodbyeReason, ReliableStreamError>;
}

impl GoodbyeReasonWireExt for GoodbyeReason {
    fn to_wire_byte(self) -> u8 {
        match self {
            GoodbyeReason::SessionEnded => 1,
            GoodbyeReason::Draining => 2,
            GoodbyeReason::StaleGeneration => 3,
        }
    }

    fn from_wire_byte(byte: u8) -> Result<GoodbyeReason, ReliableStreamError> {
        match byte {
            1 => Ok(GoodbyeReason::SessionEnded),
            2 => Ok(GoodbyeReason::Draining),
            3 => Ok(GoodbyeReason::StaleGeneration),
            _ => Err(ReliableStreamError::MalformedReliableFrame(
                "unknown goodbye reason",
            )),
        }
    }
}

/// Validated frame from a reliable mesh stream.
#[derive(Debug, PartialEq, Eq)]
pub enum ReliableFrame {
    /// Ordered opaque bytes from the remote tunnel endpoint.
    Data(Vec<u8>),
    /// Clean session close with a typed reason.
    Goodbye(GoodbyeReason),
}

#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum ReliableStreamError {
    #[error("session directory: {0}")]
    Directory(#[from] DirectoryError),
    #[error(transparent)]
    Mesh(#[from] MeshError),
    #[error("profile mismatch for session {session_id}: expected {expected:?}, got {actual:?}")]
    ProfileMismatch {
        session_id: Uuid,
        expected: Profile,
        actual: Profile,
    },
    #[error("stream hello sender {hello_sender} does not match authenticated peer {peer}")]
    SenderMismatch {
        peer: RuntimeId,
        hello_sender: RuntimeId,
    },
    #[error("unexpected non-session stream role")]
    UnexpectedStreamRole,
    #[error("reliable stream owner for session {session_id} is {owner_runtime_id}, not local runtime {local_runtime_id}")]
    OwnerIsNotLocal {
        session_id: Uuid,
        owner_runtime_id: RuntimeId,
        local_runtime_id: RuntimeId,
    },
    #[error("unexpected {0} frame on reliable session stream")]
    UnexpectedFrame(&'static str),
    #[error("frame fence mismatch: expected {expected:?}, got {actual:?}")]
    FrameFenceMismatch {
        expected: FencedHeader,
        actual: FencedHeader,
    },
    #[error("community mismatch on reliable stream: expected {expected}, got {actual}")]
    CommunityMismatch {
        expected: CommunityId,
        actual: CommunityId,
    },
    #[error("malformed reliable frame: {0}")]
    MalformedReliableFrame(&'static str),
}

fn spawn_lease_renewer(
    directory: SessionDirectory,
    lease: SessionLease,
    cancel: CancellationToken,
) -> ReliableLeaseRenewer {
    spawn_lease_renewer_with_interval(directory, lease, cancel, DEFAULT_RENEW_INTERVAL)
}

fn spawn_lease_renewer_with_interval(
    directory: SessionDirectory,
    lease: SessionLease,
    cancel: CancellationToken,
    renew_interval: Duration,
) -> ReliableLeaseRenewer {
    let lost = CancellationToken::new();
    let lost_for_task = lost.clone();
    let task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(renew_interval);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let caller_cancelled = loop {
            tokio::select! {
                _ = cancel.cancelled() => break true,
                _ = interval.tick() => {
                    match directory.renew(&lease).await {
                        Ok(RenewResult::Renewed(_)) => {}
                        Ok(RenewResult::Lost { current, known_generation }) => {
                            tracing::warn!(
                                session_id = %lease.session_id,
                                owner_runtime_id = %lease.owner_runtime_id,
                                generation = lease.generation,
                                ?current,
                                ?known_generation,
                                "reliable tunnel lease renewal lost"
                            );
                            lost_for_task.cancel();
                            break false;
                        }
                        Err(err) => {
                            tracing::warn!(
                                session_id = %lease.session_id,
                                owner_runtime_id = %lease.owner_runtime_id,
                                generation = lease.generation,
                                error = %err,
                                "reliable tunnel lease renewal failed"
                            );
                            lost_for_task.cancel();
                            break false;
                        }
                    }
                }
            }
        };

        match directory.release(&lease).await {
            Ok(ReleaseResult::Released(_)) => {}
            Ok(ReleaseResult::NotOwner {
                current,
                known_generation,
            }) => {
                tracing::warn!(
                    session_id = %lease.session_id,
                    owner_runtime_id = %lease.owner_runtime_id,
                    generation = lease.generation,
                    ?current,
                    ?known_generation,
                    "reliable tunnel lease release found non-owner"
                );
                lost_for_task.cancel();
            }
            Err(err) => {
                tracing::warn!(
                    session_id = %lease.session_id,
                    owner_runtime_id = %lease.owner_runtime_id,
                    generation = lease.generation,
                    error = %err,
                    "reliable tunnel lease release failed"
                );
                if !caller_cancelled {
                    lost_for_task.cancel();
                }
            }
        }
    });

    ReliableLeaseRenewer { task, lost }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use buzz_relay_mesh::endpoint::MeshEndpoint;
    use buzz_relay_mesh::{InboundHandler, MeshDatagram, PeerInfo};

    use super::*;

    fn community() -> CommunityId {
        CommunityId::from_uuid(Uuid::from_u128(0xCAFE))
    }

    fn runtime(byte: u8) -> RuntimeId {
        RuntimeId([byte; 32])
    }

    fn pool() -> deadpool_redis::Pool {
        let url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".into());
        deadpool_redis::Config::from_url(url)
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .expect("create redis pool")
    }

    async fn redis_directory_if_available() -> Option<SessionDirectory> {
        let pool = pool();
        let mut conn = pool.get().await.ok()?;
        redis::cmd("PING")
            .query_async::<String>(&mut *conn)
            .await
            .ok()?;
        Some(SessionDirectory::with_lease_ttl(
            pool,
            Duration::from_millis(500),
        ))
    }

    async fn clear_keys(directory: &SessionDirectory, community_id: CommunityId, session_id: Uuid) {
        let base = format!("buzz:{}:tunnel:{}", community_id, session_id);
        let _ = directory
            .release(&SessionLease {
                community_id,
                session_id,
                owner_runtime_id: runtime(1),
                generation: 1,
                profile: Profile::ReliableStream,
            })
            .await;
        let mut conn = pool().get().await.expect("redis conn");
        let _: () = redis::cmd("DEL")
            .arg(format!("{base}:lease"))
            .arg(format!("{base}:generation"))
            .query_async(&mut *conn)
            .await
            .expect("clear keys");
    }

    struct NoopTransport;

    impl RelayPeerTransport for NoopTransport {
        fn send_datagram(&self, _to: RuntimeId, _dgram: MeshDatagram) -> Result<(), MeshError> {
            unreachable!("reliable tests do not send datagrams")
        }

        fn open_session_stream(
            &self,
            _to: RuntimeId,
            _hello: StreamHello,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<MeshStream, MeshError>> + Send + '_>,
        > {
            Box::pin(async { Err(MeshError::Transport("unexpected open".into())) })
        }

        fn set_inbound(&self, _handler: Box<dyn InboundHandler>) {}
    }

    #[tokio::test]
    async fn first_join_acquires_local_ownership() {
        let Some(directory) = redis_directory_if_available().await else {
            return;
        };
        let community_id = community();
        let session_id = Uuid::new_v4();
        clear_keys(&directory, community_id, session_id).await;

        let router = ReliableStreamRouter::new(directory, Arc::new(NoopTransport), runtime(1));
        let join = router.join(community_id, session_id).await.unwrap();
        let ReliableJoin::Owned { lease } = join else {
            panic!("first join owns locally")
        };
        assert_eq!(lease.owner_runtime_id, runtime(1));
        assert_eq!(lease.profile, Profile::ReliableStream);
    }

    struct DirectTransport {
        peer: buzz_relay_mesh::peer::MeshPeer,
        opened: Mutex<Vec<(RuntimeId, StreamHello)>>,
    }

    impl RelayPeerTransport for DirectTransport {
        fn send_datagram(&self, _to: RuntimeId, _dgram: MeshDatagram) -> Result<(), MeshError> {
            unreachable!("reliable tests do not send datagrams")
        }

        fn open_session_stream(
            &self,
            to: RuntimeId,
            hello: StreamHello,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<MeshStream, MeshError>> + Send + '_>,
        > {
            Box::pin(async move {
                self.opened.lock().unwrap().push((to, hello.clone()));
                let mut stream = self.peer.open_bi().await?;
                stream.send_frame(MeshStreamFrame::Hello(hello)).await?;
                Ok(stream)
            })
        }

        fn set_inbound(&self, _handler: Box<dyn InboundHandler>) {}
    }

    async fn endpoint_pair() -> (MeshEndpoint, MeshEndpoint) {
        let bind = || "127.0.0.1:0".parse().unwrap();
        let a = MeshEndpoint::bind(bind()).await.unwrap();
        let b = MeshEndpoint::bind(bind()).await.unwrap();
        (a, b)
    }

    #[tokio::test]
    async fn later_join_routes_to_remote_owner_with_reliable_hello() {
        let Some(directory) = redis_directory_if_available().await else {
            return;
        };
        let community_id = community();
        let session_id = Uuid::new_v4();
        clear_keys(&directory, community_id, session_id).await;

        let (local_endpoint, owner_endpoint) = endpoint_pair().await;
        let local_runtime = local_endpoint.runtime_id();
        let owner_runtime = owner_endpoint.runtime_id();
        let owner_addr = owner_endpoint.addr();
        let accept_endpoint = owner_endpoint.clone();
        let accept = tokio::spawn(async move { accept_endpoint.accept().await.unwrap().unwrap() });
        let local_peer = local_endpoint.connect(owner_addr).await.unwrap();
        let owner_peer = accept.await.unwrap();

        let owner_lease = match directory
            .acquire(
                community_id,
                session_id,
                owner_runtime,
                Profile::ReliableStream,
            )
            .await
            .unwrap()
        {
            AcquireResult::Acquired(lease) => lease,
            AcquireResult::Exists(_) => panic!("fresh session should acquire"),
        };

        let transport = Arc::new(DirectTransport {
            peer: local_peer,
            opened: Mutex::new(Vec::new()),
        });
        let router = ReliableStreamRouter::new(directory.clone(), transport, local_runtime);

        let recv_hello = tokio::spawn(async move {
            let mut stream = owner_peer.accept_bi().await.unwrap();
            stream.recv_frame().await.unwrap().unwrap()
        });

        let join = router.join(community_id, session_id).await.unwrap();
        let ReliableJoin::Forwarded { lease, .. } = join else {
            panic!("second runtime should forward")
        };
        assert_eq!(lease, owner_lease);

        let got = tokio::time::timeout(Duration::from_secs(5), recv_hello)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            got,
            MeshStreamFrame::Hello(StreamHello {
                sender: local_runtime,
                role: StreamRole::Session {
                    fenced: owner_lease.fenced_header(),
                    profile: Profile::ReliableStream,
                },
            })
        );
    }

    #[tokio::test]
    async fn observable_renewer_signals_loss_when_lease_disappears() {
        let Some(directory) = redis_directory_if_available().await else {
            return;
        };
        let community_id = community();
        let session_id = Uuid::new_v4();
        clear_keys(&directory, community_id, session_id).await;
        let lease = match directory
            .acquire(
                community_id,
                session_id,
                runtime(1),
                Profile::ReliableStream,
            )
            .await
            .unwrap()
        {
            AcquireResult::Acquired(lease) => lease,
            AcquireResult::Exists(_) => panic!("fresh session should acquire"),
        };

        let cancel = CancellationToken::new();
        let renewer = spawn_lease_renewer_with_interval(
            directory.clone(),
            lease.clone(),
            cancel,
            Duration::from_millis(10),
        );
        directory.release(&lease).await.unwrap();

        tokio::time::timeout(Duration::from_secs(2), renewer.lost.cancelled())
            .await
            .expect("lost token is cancelled after ownership loss");
        renewer.task.await.unwrap();
    }

    #[tokio::test]
    async fn observable_renewer_normal_cancel_does_not_signal_loss() {
        let Some(directory) = redis_directory_if_available().await else {
            return;
        };
        let community_id = community();
        let session_id = Uuid::new_v4();
        clear_keys(&directory, community_id, session_id).await;
        let lease = match directory
            .acquire(
                community_id,
                session_id,
                runtime(1),
                Profile::ReliableStream,
            )
            .await
            .unwrap()
        {
            AcquireResult::Acquired(lease) => lease,
            AcquireResult::Exists(_) => panic!("fresh session should acquire"),
        };

        let cancel = CancellationToken::new();
        let renewer = spawn_lease_renewer_with_interval(
            directory,
            lease,
            cancel.clone(),
            Duration::from_millis(10),
        );
        cancel.cancel();
        renewer.task.await.unwrap();
        assert!(
            !renewer.lost.is_cancelled(),
            "caller-initiated shutdown is not ownership loss"
        );
    }

    #[test]
    fn reliable_wire_frame_carries_community_without_plain_payload_changes() {
        let frame = ReliableWireFrame::Data {
            community_id: community(),
            payload: b"goose bytes".to_vec(),
        };
        let encoded = frame.encode();
        let decoded = ReliableWireFrame::decode(&encoded).unwrap();
        assert_eq!(decoded, frame);
        assert_eq!(decoded.community_id(), community());
    }

    #[test]
    fn payload_chunking_covers_goose_sized_bodies() {
        let fifty_mib: usize = 50 * 1024 * 1024;
        let chunks = fifty_mib.div_ceil(MAX_RELIABLE_PAYLOAD_BYTES);
        assert_eq!(chunks, 50);
        assert!(MAX_RELIABLE_PAYLOAD_BYTES < buzz_relay_mesh::wire::MAX_STREAM_FRAME as usize);
    }

    #[allow(dead_code)]
    fn _peer_info_is_not_an_owner_signal(_peer: PeerInfo) {}
}
