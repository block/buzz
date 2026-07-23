//! The live mesh runtime: warm peer manager, accept/dial loops, gossip
//! exchange, and the concrete [`RelayPeerTransport`] implementation.
//!
//! This is the piece that turns the lane modules into a running mesh:
//!
//! - [`MeshRuntime::start`] binds nothing itself — it takes an already-bound
//!   [`MeshEndpoint`] plus a [`MeshMembership`] table and spawns the loops.
//! - **Accept loop**: inbound connections are admitted only when the remote
//!   runtime id is present in the (attested) membership table; unknown ids get
//!   one registry rescan before rejection. Membership is a hint — admission
//!   here gates *dialability*, never session ownership.
//! - **Reconcile loop**: periodically rescans the Redis ready registry and
//!   dials every known, non-draining peer we are not yet connected to. This is
//!   what makes the mesh *warm*: failover is "next frame goes elsewhere," not
//!   "wait for a handshake."
//! - **Control stream**: exactly one per peer connection, opened by the
//!   dialer. Carries scuttlebutt gossip (`Digest` → `Delta`) both ways.
//! - **Simultaneous dial tie-break**: the connection dialed by the smaller
//!   runtime id wins; the loser is dropped. Deterministic on both ends.
//!
//! The fencing law holds here too: nothing in this file consults or mutates
//! session ownership. Transport moves fenced bytes; the session layer on both
//! ends validates the fence against Redis.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::endpoint::{direct_addr, MeshEndpoint};
use crate::gossip::{decode_message, encode_message, GossipMessage};
use crate::membership::MeshMembership;
use crate::peer::MeshPeer;
use crate::registry::{ReadyRecord, ReadyRegistry};
use crate::status::ConnectionState;
use crate::wire::{MeshStreamFrame, StreamHello, StreamRole};
use crate::{InboundHandler, MeshDatagram, MeshError, MeshStream, RelayPeerTransport, RuntimeId};

/// How often the reconcile loop rescans the registry and dials missing peers.
pub const DEFAULT_RECONCILE_INTERVAL: Duration = Duration::from_secs(5);
/// How often each side sends a gossip digest on every control stream.
pub const DEFAULT_GOSSIP_INTERVAL: Duration = Duration::from_secs(2);
/// Bound on queued control-stream frames per peer before backpressure.
const CONTROL_QUEUE_DEPTH: usize = 64;

struct PeerEntry {
    peer: MeshPeer,
    /// Writer queue for the peer's control stream. Present once the control
    /// stream is up (dialer opens it; acceptor receives it).
    control_tx: Option<mpsc::Sender<MeshStreamFrame>>,
    tasks: Vec<JoinHandle<()>>,
}

impl PeerEntry {
    fn abort(&self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

struct Inner {
    endpoint: MeshEndpoint,
    membership: MeshMembership,
    registry: Option<ReadyRegistry>,
    peers: RwLock<HashMap<RuntimeId, PeerEntry>>,
    handler: Mutex<Option<Arc<dyn InboundHandler>>>,
    gossip_interval: Duration,
    reconcile_interval: Duration,
}

/// Handle to the running mesh. Cheap to clone; dropping all clones does NOT
/// stop the loops — call [`MeshRuntime::shutdown`] for that.
#[derive(Clone)]
pub struct MeshRuntime {
    inner: Arc<Inner>,
    loops: Arc<Mutex<Vec<JoinHandle<()>>>>,
}

impl MeshRuntime {
    /// Spawn the mesh loops over an already-bound endpoint.
    ///
    /// `registry` is `None` in tests / single-instance shapes: the reconcile
    /// loop then dials from the membership table alone (seeded by gossip or
    /// test setup) and skips registry rescans.
    pub fn start(
        endpoint: MeshEndpoint,
        membership: MeshMembership,
        registry: Option<ReadyRegistry>,
    ) -> Self {
        Self::start_with_intervals(
            endpoint,
            membership,
            registry,
            DEFAULT_GOSSIP_INTERVAL,
            DEFAULT_RECONCILE_INTERVAL,
        )
    }

    pub fn start_with_intervals(
        endpoint: MeshEndpoint,
        membership: MeshMembership,
        registry: Option<ReadyRegistry>,
        gossip_interval: Duration,
        reconcile_interval: Duration,
    ) -> Self {
        let inner = Arc::new(Inner {
            endpoint,
            membership,
            registry,
            peers: RwLock::new(HashMap::new()),
            handler: Mutex::new(None),
            gossip_interval,
            reconcile_interval,
        });

        let accept = tokio::spawn(accept_loop(Arc::clone(&inner)));
        let reconcile = tokio::spawn(reconcile_loop(Arc::clone(&inner)));
        let gossip = tokio::spawn(gossip_tick_loop(Arc::clone(&inner)));

        Self {
            inner,
            loops: Arc::new(Mutex::new(vec![accept, reconcile, gossip])),
        }
    }

    pub fn membership(&self) -> &MeshMembership {
        &self.inner.membership
    }

    pub fn local_runtime_id(&self) -> RuntimeId {
        self.inner.endpoint.runtime_id()
    }

    /// Currently connected peer ids (either direction).
    pub fn connected_peers(&self) -> Vec<RuntimeId> {
        self.inner
            .peers
            .read()
            .expect("peer lock poisoned")
            .keys()
            .copied()
            .collect()
    }

    /// Force one reconcile pass right now (bootstrap fast-path: dial the seed
    /// records without waiting for the first interval tick).
    pub async fn reconcile_now(&self) {
        reconcile_once(&self.inner).await;
    }

    /// Stop all loops and drop all peer connections.
    pub fn shutdown(&self) {
        for task in self.loops.lock().expect("loop lock poisoned").drain(..) {
            task.abort();
        }
        let mut peers = self.inner.peers.write().expect("peer lock poisoned");
        for (_, entry) in peers.drain() {
            entry.abort();
        }
    }
}

impl RelayPeerTransport for MeshRuntime {
    fn send_datagram(&self, to: RuntimeId, dgram: MeshDatagram) -> Result<(), MeshError> {
        let peers = self.inner.peers.read().expect("peer lock poisoned");
        let entry = peers.get(&to).ok_or(MeshError::PeerNotConnected(to))?;
        entry.peer.send_datagram(&dgram)?;
        drop(peers);
        self.inner.membership.record_datagram_sent(to);
        Ok(())
    }

    fn open_session_stream(
        &self,
        to: RuntimeId,
        hello: StreamHello,
    ) -> crate::BoxFuture<'_, Result<MeshStream, MeshError>> {
        Box::pin(async move {
            let peer = {
                let peers = self.inner.peers.read().expect("peer lock poisoned");
                peers
                    .get(&to)
                    .map(|entry| entry.peer.clone())
                    .ok_or(MeshError::PeerNotConnected(to))?
            };
            let mut stream = peer.open_bi().await?;
            stream.send_frame(MeshStreamFrame::Hello(hello)).await?;
            self.inner.membership.record_stream_opened(to);
            Ok(stream)
        })
    }

    fn set_inbound(&self, handler: Box<dyn InboundHandler>) {
        *self.inner.handler.lock().expect("handler lock poisoned") = Some(Arc::from(handler));
    }
}

fn inbound_handler(inner: &Inner) -> Option<Arc<dyn InboundHandler>> {
    inner.handler.lock().expect("handler lock poisoned").clone()
}

/// Simultaneous-dial tie-break: the connection dialed by the smaller runtime
/// id wins. Returns true when the NEW connection should replace the existing.
fn new_connection_wins(local: RuntimeId, remote: RuntimeId, new_dialed_by_us: bool) -> bool {
    if local.0 < remote.0 {
        // We are the canonical dialer: our outbound connection wins.
        new_dialed_by_us
    } else {
        // The peer is the canonical dialer: their inbound connection wins.
        !new_dialed_by_us
    }
}

/// Install a connected peer, spawning its datagram + stream accept loops.
/// Returns false when an existing connection won the tie-break.
fn install_peer(inner: &Arc<Inner>, peer: MeshPeer, dialed_by_us: bool) -> bool {
    let remote = peer.runtime_id();
    let local = inner.endpoint.runtime_id();
    let mut peers = inner.peers.write().expect("peer lock poisoned");

    if let Some(existing) = peers.get(&remote) {
        if !new_connection_wins(local, remote, dialed_by_us) {
            tracing::debug!(peer = %remote, "mesh: kept existing connection (tie-break)");
            return false;
        }
        existing.abort();
    }

    let mut tasks = vec![
        tokio::spawn(datagram_recv_loop(Arc::clone(inner), peer.clone())),
        tokio::spawn(stream_accept_loop(Arc::clone(inner), peer.clone())),
    ];

    // Dialer opens the control stream for the connection.
    let control_tx = if dialed_by_us {
        let (tx, rx) = mpsc::channel(CONTROL_QUEUE_DEPTH);
        tasks.push(tokio::spawn(open_control_stream(
            Arc::clone(inner),
            peer.clone(),
            rx,
        )));
        Some(tx)
    } else {
        None
    };

    peers.insert(
        remote,
        PeerEntry {
            peer,
            control_tx,
            tasks,
        },
    );
    drop(peers);
    inner
        .membership
        .mark_connection_state(remote, ConnectionState::Connected);
    tracing::info!(peer = %remote, dialed_by_us, "mesh: peer connected");
    true
}

fn remove_peer(inner: &Inner, runtime_id: RuntimeId) {
    if let Some(entry) = inner
        .peers
        .write()
        .expect("peer lock poisoned")
        .remove(&runtime_id)
    {
        entry.abort();
        inner
            .membership
            .mark_connection_state(runtime_id, ConnectionState::Disconnected);
        tracing::info!(peer = %runtime_id, "mesh: peer disconnected");
    }
}

async fn accept_loop(inner: Arc<Inner>) {
    loop {
        match inner.endpoint.accept().await {
            Ok(Some(peer)) => {
                let remote = peer.runtime_id();
                if !is_known_peer(&inner, remote).await {
                    tracing::warn!(
                        peer = %remote,
                        "mesh: rejected inbound connection from unattested runtime id"
                    );
                    continue;
                }
                install_peer(&inner, peer, false);
            }
            Ok(None) => {
                tracing::info!("mesh: endpoint closed, accept loop exiting");
                return;
            }
            Err(err) => {
                tracing::warn!(%err, "mesh: inbound connection failed");
            }
        }
    }
}

/// Admission check for inbound connections: the runtime id must appear in the
/// attested membership table. Unknown ids get one registry rescan (covers the
/// bootstrap race where a fresh pod dials us before our next reconcile tick).
async fn is_known_peer(inner: &Arc<Inner>, runtime_id: RuntimeId) -> bool {
    if inner.membership.has_peer(runtime_id) {
        return true;
    }
    if let Some(registry) = &inner.registry {
        match registry.scan_ready().await {
            Ok(records) => inner.membership.apply_ready_records(records),
            Err(err) => tracing::warn!(%err, "mesh: registry rescan on inbound failed"),
        }
    }
    inner.membership.has_peer(runtime_id)
}

async fn reconcile_loop(inner: Arc<Inner>) {
    loop {
        reconcile_once(&inner).await;
        tokio::time::sleep(inner.reconcile_interval).await;
    }
}

async fn reconcile_once(inner: &Arc<Inner>) {
    if let Some(registry) = &inner.registry {
        match registry.scan_ready().await {
            Ok(records) => inner.membership.apply_ready_records(records),
            Err(err) => tracing::warn!(%err, "mesh: registry scan failed"),
        }
    }

    let local = inner.endpoint.runtime_id();
    let candidates: Vec<_> = inner
        .membership
        .records()
        .into_iter()
        .filter(|record| record.runtime_id != local && !record.draining)
        .collect();

    for record in candidates {
        let already_connected = inner
            .peers
            .read()
            .expect("peer lock poisoned")
            .contains_key(&record.runtime_id);
        if already_connected {
            continue;
        }
        dial_peer(inner, &record).await;
    }
}

async fn dial_peer(inner: &Arc<Inner>, record: &crate::gossip::GossipRecord) {
    for addr in &record.endpoint_addrs {
        let sock = match addr.parse() {
            Ok(sock) => sock,
            Err(err) => {
                tracing::warn!(peer = %record.runtime_id, addr, %err, "mesh: bad peer addr");
                continue;
            }
        };
        let endpoint_addr = match direct_addr(record.runtime_id, sock) {
            Ok(ea) => ea,
            Err(err) => {
                tracing::warn!(peer = %record.runtime_id, %err, "mesh: bad peer id");
                return;
            }
        };
        inner
            .membership
            .mark_connection_state(record.runtime_id, ConnectionState::Connecting);
        match inner.endpoint.connect(endpoint_addr).await {
            Ok(peer) => {
                install_peer(inner, peer, true);
                return;
            }
            Err(err) => {
                tracing::warn!(peer = %record.runtime_id, addr, %err, "mesh: dial failed");
            }
        }
    }
    inner
        .membership
        .mark_connection_state(record.runtime_id, ConnectionState::Disconnected);
}

async fn datagram_recv_loop(inner: Arc<Inner>, peer: MeshPeer) {
    let remote = peer.runtime_id();
    loop {
        match peer.recv_datagram().await {
            Ok(dgram) => {
                inner.membership.record_datagram_received(remote);
                if let Some(handler) = inbound_handler(&inner) {
                    handler.on_datagram(remote, dgram);
                }
            }
            Err(err) => {
                tracing::debug!(peer = %remote, %err, "mesh: datagram loop ended");
                remove_peer(&inner, remote);
                return;
            }
        }
    }
}

async fn stream_accept_loop(inner: Arc<Inner>, peer: MeshPeer) {
    let remote = peer.runtime_id();
    loop {
        let mut stream = match peer.accept_bi().await {
            Ok(stream) => stream,
            Err(err) => {
                tracing::debug!(peer = %remote, %err, "mesh: stream accept loop ended");
                remove_peer(&inner, remote);
                return;
            }
        };

        // The first frame on any stream MUST be Hello (wire contract).
        let hello = match stream.recv_frame().await {
            Ok(Some(MeshStreamFrame::Hello(hello))) => hello,
            Ok(other) => {
                tracing::warn!(peer = %remote, ?other, "mesh: stream without Hello — dropped");
                continue;
            }
            Err(err) => {
                tracing::warn!(peer = %remote, %err, "mesh: stream Hello read failed");
                continue;
            }
        };

        match hello.role {
            StreamRole::Control => {
                // Acceptor side of the per-connection control stream: register
                // a writer queue and start the gossip exchange.
                let (tx, rx) = mpsc::channel(CONTROL_QUEUE_DEPTH);
                if let Some(entry) = inner
                    .peers
                    .write()
                    .expect("peer lock poisoned")
                    .get_mut(&remote)
                {
                    entry.control_tx = Some(tx);
                }
                tokio::spawn(control_stream_exchange(
                    Arc::clone(&inner),
                    remote,
                    stream,
                    rx,
                ));
            }
            StreamRole::Session { .. } => {
                inner.membership.record_stream_received(remote);
                if let Some(handler) = inbound_handler(&inner) {
                    handler.on_session_stream(remote, hello, stream);
                } else {
                    tracing::warn!(
                        peer = %remote,
                        "mesh: session stream arrived before inbound handler was set — dropped"
                    );
                }
            }
        }
    }
}

/// Dialer side: open the control stream, send Hello{Control}, then exchange.
async fn open_control_stream(
    inner: Arc<Inner>,
    peer: MeshPeer,
    rx: mpsc::Receiver<MeshStreamFrame>,
) {
    let remote = peer.runtime_id();
    let mut stream = match peer.open_bi().await {
        Ok(stream) => stream,
        Err(err) => {
            tracing::warn!(peer = %remote, %err, "mesh: control stream open failed");
            return;
        }
    };
    let hello = MeshStreamFrame::Hello(StreamHello {
        sender: inner.endpoint.runtime_id(),
        role: StreamRole::Control,
    });
    if let Err(err) = stream.send_frame(hello).await {
        tracing::warn!(peer = %remote, %err, "mesh: control Hello send failed");
        return;
    }
    control_stream_exchange(inner, remote, stream, rx).await;
}

/// Both sides: pump queued outbound frames and dispatch inbound gossip.
///
/// Scuttlebutt: a received `Digest` is answered with a `Delta` of records the
/// digest is missing/behind on; a received `Delta` is applied to membership.
async fn control_stream_exchange(
    inner: Arc<Inner>,
    remote: RuntimeId,
    stream: MeshStream,
    mut rx: mpsc::Receiver<MeshStreamFrame>,
) {
    let MeshStream { mut send, mut recv } = stream;

    let send_inner = Arc::clone(&inner);
    let send_task = tokio::spawn(async move {
        while let Some(frame) = rx.recv().await {
            if let Err(err) = send.send_frame(frame).await {
                tracing::debug!(peer = %remote, %err, "mesh: control send ended");
                return;
            }
            send_inner.membership.record_gossip_frame_sent(remote);
        }
    });

    loop {
        match recv.recv_frame().await {
            Ok(Some(MeshStreamFrame::Gossip { payload })) => {
                inner.membership.record_gossip_frame_received(remote);
                match decode_message(&payload) {
                    Ok(GossipMessage::Digest { entries, .. }) => {
                        let delta = inner.membership.delta_for(&entries);
                        if let Ok(payload) = encode_message(&delta) {
                            send_control_frame(&inner, remote, MeshStreamFrame::Gossip { payload });
                        }
                    }
                    Ok(GossipMessage::Delta { records, .. }) => {
                        for record in records {
                            inner.membership.apply_gossip_record(record);
                        }
                    }
                    Err(err) => {
                        tracing::warn!(peer = %remote, %err, "mesh: bad gossip payload");
                    }
                }
            }
            Ok(Some(other)) => {
                tracing::warn!(peer = %remote, ?other, "mesh: non-gossip frame on control stream");
            }
            Ok(None) | Err(_) => {
                tracing::debug!(peer = %remote, "mesh: control stream closed");
                send_task.abort();
                return;
            }
        }
    }
}

fn send_control_frame(inner: &Inner, remote: RuntimeId, frame: MeshStreamFrame) {
    let peers = inner.peers.read().expect("peer lock poisoned");
    if let Some(tx) = peers.get(&remote).and_then(|e| e.control_tx.as_ref()) {
        // try_send: gossip is periodic and idempotent — dropping a frame under
        // backpressure is strictly better than blocking a recv loop.
        let _ = tx.try_send(frame);
    }
}

/// Periodic gossip: refresh the local heartbeat and send a digest on every
/// control stream. Deltas flow back per the exchange loop.
async fn gossip_tick_loop(inner: Arc<Inner>) {
    loop {
        tokio::time::sleep(inner.gossip_interval).await;
        // Heartbeat: bump the local record so peers' phi accrual sees life.
        inner.membership.update_local(|_| {});
        let digest = inner.membership.digest();
        let Ok(payload) = encode_message(&digest) else {
            continue;
        };
        let targets: Vec<RuntimeId> = {
            let peers = inner.peers.read().expect("peer lock poisoned");
            peers
                .iter()
                .filter(|(_, e)| e.control_tx.is_some())
                .map(|(id, _)| *id)
                .collect()
        };
        for remote in targets {
            send_control_frame(
                &inner,
                remote,
                MeshStreamFrame::Gossip {
                    payload: payload.clone(),
                },
            );
        }
    }
}

/// Readiness-gated registry heartbeat loop, spawned by the relay after boot.
/// `ready` is the relay-owned readiness predicate (shutdown flag et al.).
pub fn spawn_registry_heartbeat(
    registry: ReadyRegistry,
    record: ReadyRecord,
    ready: Arc<dyn Fn() -> bool + Send + Sync>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut heartbeat = registry.heartbeat(record);
        let interval = registry.refresh_interval();
        loop {
            if let Err(err) = heartbeat.tick(ready()).await {
                tracing::warn!(%err, "mesh: registry heartbeat tick failed");
            }
            tokio::time::sleep(interval).await;
        }
    })
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::sync::Mutex as StdMutex;
    use std::time::Duration;

    use iroh::SecretKey;
    use tokio::time::timeout;
    use uuid::Uuid;

    use super::*;
    use crate::gossip::GossipRecord;
    use crate::wire::{FencedHeader, Profile};

    fn loopback_any() -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0)
    }

    async fn runtime(key_byte: u8) -> (MeshRuntime, Vec<String>) {
        let endpoint = MeshEndpoint::bind_with_secret_key(
            SecretKey::from_bytes(&[key_byte; 32]),
            loopback_any(),
        )
        .await
        .unwrap();
        let addrs: Vec<String> = endpoint
            .addr()
            .addrs
            .iter()
            .filter_map(|ta| match ta {
                iroh::TransportAddr::Ip(sock) if sock.ip().is_loopback() => Some(sock.to_string()),
                _ => None,
            })
            .collect();
        assert!(!addrs.is_empty(), "endpoint must expose a loopback addr");
        let record = GossipRecord::new(endpoint.runtime_id(), addrs.clone(), 1);
        let membership = MeshMembership::new(record);
        let rt = MeshRuntime::start_with_intervals(
            endpoint,
            membership,
            None,
            Duration::from_millis(100),
            Duration::from_millis(200),
        );
        (rt, addrs)
    }

    /// Seed b's record into a's membership so a dials b.
    fn seed(a: &MeshRuntime, b: &MeshRuntime, b_addrs: &[String]) {
        a.membership().apply_gossip_record(GossipRecord::new(
            b.local_runtime_id(),
            b_addrs.to_vec(),
            1,
        ));
    }

    async fn connected_pair() -> (MeshRuntime, MeshRuntime) {
        let (a, a_addrs) = runtime(1).await;
        let (b, b_addrs) = runtime(2).await;
        // Both directions: with no registry to rescan, the acceptor's
        // admission gate requires the dialer to already be in its membership
        // table (production gets this from the attested ready registry).
        seed(&a, &b, &b_addrs);
        seed(&b, &a, &a_addrs);
        a.reconcile_now().await;
        // Wait for both sides to see the connection.
        timeout(Duration::from_secs(5), async {
            loop {
                if a.connected_peers().contains(&b.local_runtime_id())
                    && b.connected_peers().contains(&a.local_runtime_id())
                {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("mesh pair should connect");
        (a, b)
    }

    struct RecordingHandler {
        datagrams: StdMutex<Vec<(RuntimeId, MeshDatagram)>>,
        streams: StdMutex<Vec<(RuntimeId, StreamHello)>>,
    }

    impl RecordingHandler {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                datagrams: StdMutex::new(Vec::new()),
                streams: StdMutex::new(Vec::new()),
            })
        }
    }

    impl InboundHandler for Arc<RecordingHandler> {
        fn on_datagram(&self, from: RuntimeId, dgram: MeshDatagram) {
            self.datagrams.lock().unwrap().push((from, dgram));
        }
        fn on_session_stream(&self, from: RuntimeId, hello: StreamHello, _stream: MeshStream) {
            self.streams.lock().unwrap().push((from, hello));
        }
    }

    fn fenced(owner: RuntimeId) -> FencedHeader {
        FencedHeader {
            session_id: Uuid::from_u128(0xFEED),
            generation: 3,
            owner_runtime_id: owner,
        }
    }

    #[tokio::test]
    async fn warm_pair_connects_and_gossips_membership() {
        let (a, b) = connected_pair().await;
        // Gossip heartbeats should keep flowing; wait for a to see a gossiped
        // (version > 1) record from b.
        timeout(Duration::from_secs(5), async {
            loop {
                let seen = a
                    .membership()
                    .records()
                    .into_iter()
                    .find(|r| r.runtime_id == b.local_runtime_id());
                if seen.is_some_and(|r| r.version > 1) {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("gossip should refresh b's record on a");
        a.shutdown();
        b.shutdown();
    }

    #[tokio::test]
    async fn transport_datagram_reaches_inbound_handler() {
        let (a, b) = connected_pair().await;
        let handler = RecordingHandler::new();
        b.set_inbound(Box::new(Arc::clone(&handler)));

        let dgram = MeshDatagram {
            fenced: fenced(b.local_runtime_id()),
            seq: 7,
            payload: vec![1, 2, 3],
        };
        a.send_datagram(b.local_runtime_id(), dgram.clone())
            .unwrap();

        timeout(Duration::from_secs(5), async {
            loop {
                if !handler.datagrams.lock().unwrap().is_empty() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("datagram should arrive");
        let got = handler.datagrams.lock().unwrap();
        assert_eq!(got[0].0, a.local_runtime_id());
        assert_eq!(got[0].1, dgram);
        drop(got);
        a.shutdown();
        b.shutdown();
    }

    #[tokio::test]
    async fn transport_session_stream_reaches_inbound_handler() {
        let (a, b) = connected_pair().await;
        let handler = RecordingHandler::new();
        b.set_inbound(Box::new(Arc::clone(&handler)));

        let hello = StreamHello {
            sender: a.local_runtime_id(),
            role: StreamRole::Session {
                fenced: fenced(b.local_runtime_id()),
                profile: Profile::ReliableStream,
            },
        };
        let mut stream = a
            .open_session_stream(b.local_runtime_id(), hello.clone())
            .await
            .unwrap();
        stream
            .send_frame(MeshStreamFrame::Data {
                fenced: fenced(b.local_runtime_id()),
                payload: b"tunnel".to_vec(),
            })
            .await
            .unwrap();

        timeout(Duration::from_secs(5), async {
            loop {
                if !handler.streams.lock().unwrap().is_empty() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("session stream should arrive");
        let got = handler.streams.lock().unwrap();
        assert_eq!(got[0].0, a.local_runtime_id());
        assert_eq!(got[0].1, hello);
        drop(got);
        a.shutdown();
        b.shutdown();
    }

    #[tokio::test]
    async fn send_to_unconnected_peer_is_typed_error() {
        let (a, _addrs) = runtime(9).await;
        let ghost = RuntimeId([42u8; 32]);
        let err = a
            .send_datagram(
                ghost,
                MeshDatagram {
                    fenced: fenced(ghost),
                    seq: 0,
                    payload: vec![],
                },
            )
            .unwrap_err();
        assert!(matches!(err, MeshError::PeerNotConnected(id) if id == ghost));
        a.shutdown();
    }

    #[tokio::test]
    async fn simultaneous_dial_converges_to_one_connection() {
        let (a, a_addrs) = runtime(3).await;
        let (b, b_addrs) = runtime(4).await;
        seed(&a, &b, &b_addrs);
        seed(&b, &a, &a_addrs);
        // Both dial at once.
        tokio::join!(a.reconcile_now(), b.reconcile_now());

        timeout(Duration::from_secs(5), async {
            loop {
                if a.connected_peers().contains(&b.local_runtime_id())
                    && b.connected_peers().contains(&a.local_runtime_id())
                {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("simultaneous dial should converge");
        // Datagrams still flow after the tie-break.
        let handler = RecordingHandler::new();
        b.set_inbound(Box::new(Arc::clone(&handler)));
        // The surviving connection may need a beat to settle.
        timeout(Duration::from_secs(5), async {
            loop {
                let dgram = MeshDatagram {
                    fenced: fenced(b.local_runtime_id()),
                    seq: 1,
                    payload: vec![9],
                };
                let _ = a.send_datagram(b.local_runtime_id(), dgram);
                if !handler.datagrams.lock().unwrap().is_empty() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("datagram should flow after tie-break");
        a.shutdown();
        b.shutdown();
    }

    #[test]
    fn tie_break_is_symmetric() {
        let small = RuntimeId([1u8; 32]);
        let large = RuntimeId([2u8; 32]);
        // small dials large: small's outbound wins, large's inbound wins.
        assert!(new_connection_wins(small, large, true));
        assert!(new_connection_wins(large, small, false));
        // large dials small: loses on both ends.
        assert!(!new_connection_wins(large, small, true));
        assert!(!new_connection_wins(small, large, false));
    }
}
