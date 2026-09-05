//! Consumer-boundary tests: real reliable framing/Redis validation, controlled
//! transport halves. Redis tests are explicit (never silently skipped):
//! REDIS_URL=... cargo test -p buzz-relay mesh_boot::echo_tests -- --include-ignored

use super::*;
use crate::tunnel::{
    directory::{AcquireResult, SessionLease},
    reliable::ReliableMeshStream,
};
use buzz_relay_mesh::{
    BoxFuture, FencedHeader, MeshError, MeshStreamFrame, StreamRecvHalf, StreamSendHalf,
};
use futures_util::poll;
use std::{future::Future, pin::Pin, sync::atomic::AtomicUsize, time::Duration};
use tokio::sync::mpsc;

#[derive(Default)]
struct Observed {
    receives: AtomicUsize,
    consumed: AtomicUsize,
    finishes: AtomicUsize,
}

struct SendHalf(mpsc::UnboundedSender<MeshStreamFrame>, Arc<Observed>);
impl StreamSendHalf for SendHalf {
    fn send_frame(&mut self, frame: MeshStreamFrame) -> BoxFuture<'_, Result<(), MeshError>> {
        Box::pin(async move {
            self.0.send(frame).unwrap();
            Ok(())
        })
    }
    fn finish(&mut self) -> Result<(), MeshError> {
        self.1.finishes.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

struct RecvHalf(mpsc::UnboundedReceiver<MeshStreamFrame>, Arc<Observed>);
impl StreamRecvHalf for RecvHalf {
    fn recv_frame(&mut self) -> BoxFuture<'_, Result<Option<MeshStreamFrame>, MeshError>> {
        self.1.receives.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            let frame = self.0.recv().await;
            if frame.is_some() {
                self.1.consumed.fetch_add(1, Ordering::SeqCst);
            }
            Ok(frame)
        })
    }
}

fn streams(fenced: FencedHeader) -> (ReliableInbound, ReliableMeshStream, Arc<Observed>) {
    let (input_tx, input_rx) = mpsc::unbounded_channel();
    let (output_tx, output_rx) = mpsc::unbounded_channel();
    let owner = Arc::new(Observed::default());
    let peer = Arc::new(Observed::default());
    let inbound = ReliableInbound {
        fenced,
        from: RuntimeId([18; 32]),
        stream: ReliableMeshStream::new_inbound(
            fenced,
            MeshStream::new(
                Box::new(SendHalf(output_tx, owner.clone())),
                Box::new(RecvHalf(input_rx, owner.clone())),
            ),
        ),
    };
    let peer_stream = ReliableMeshStream::new_inbound(
        fenced,
        MeshStream::new(
            Box::new(SendHalf(input_tx, peer.clone())),
            Box::new(RecvHalf(output_rx, peer)),
        ),
    );
    (inbound, peer_stream, owner)
}

fn pool(url: String) -> deadpool_redis::Pool {
    let mut config = deadpool_redis::Config::from_url(url);
    config.pool = Some(deadpool_redis::PoolConfig::new(1));
    config
        .create_pool(Some(deadpool_redis::Runtime::Tokio1))
        .unwrap()
}

struct Fixture {
    pool: deadpool_redis::Pool,
    directory: SessionDirectory,
    lease: SessionLease,
}
impl Fixture {
    async fn new() -> Self {
        let pool = pool(std::env::var("REDIS_URL").expect("explicit test Redis required"));
        let directory = SessionDirectory::with_lease_ttl(pool.clone(), Duration::from_secs(5));
        let community = buzz_core::CommunityId::from_uuid(uuid::Uuid::new_v4());
        let lease = match directory
            .acquire(
                community,
                uuid::Uuid::new_v4(),
                RuntimeId([17; 32]),
                Profile::ReliableStream,
            )
            .await
            .unwrap()
        {
            AcquireResult::Acquired(lease) => lease,
            _ => panic!("UUID collision"),
        };
        Self {
            pool,
            directory,
            lease,
        }
    }

    async fn cleanup(self) {
        self.directory.release(&self.lease).await.unwrap();
        let mut conn = self.pool.get().await.unwrap();
        let community = self.lease.community_id;
        let session = self.lease.session_id;
        let _: () = redis::cmd("DEL")
            .arg(format!("buzz:{community}:tunnel:{session}:generation"))
            .query_async(&mut *conn)
            .await
            .unwrap();
    }
}

// Drive the actual consumer ourselves: a held private pool slot guarantees
// validation is pending, and each poll after 110ms must service a 100ms tick.
// No spawned-task scheduling assumption decides whether the frame was consumed
// or the consumer crossed a tick. The Redis server/other pools are never blocked.
async fn housekeeping(consumer: &mut Pin<&mut impl Future<Output = ()>>) {
    tokio::time::sleep(Duration::from_millis(110)).await;
    assert!(poll!(consumer.as_mut()).is_pending());
}

#[tokio::test]
#[ignore = "requires explicit REDIS_URL"]
async fn consumed_frame_survives_housekeeping_during_validation() {
    let f = Fixture::new().await;
    let (inbound, mut peer, seen) = streams(f.lease.fenced_header());
    let held = f.pool.get().await.unwrap();
    let shutdown = Arc::new(AtomicBool::new(false));
    let consumer = run_demo_echo(f.directory.clone(), inbound, shutdown);
    tokio::pin!(consumer);
    assert!(poll!(&mut consumer).is_pending());
    peer.send_bytes(f.lease.community_id, b"first")
        .await
        .unwrap();
    assert!(poll!(&mut consumer).is_pending());
    assert_eq!(seen.consumed.load(Ordering::SeqCst), 1);
    for _ in 0..3 {
        housekeeping(&mut consumer).await;
    }
    assert_eq!(
        seen.receives.load(Ordering::SeqCst),
        1,
        "pending receive was recreated"
    );
    drop(held);
    tokio::time::timeout(Duration::from_secs(2), async {
        let receive = async {
            assert!(matches!(peer.recv_validated(&f.directory).await.unwrap(),
                Some(ReliableFrame::Data(bytes)) if bytes == b"first"));
            peer.send_bytes(f.lease.community_id, b"second")
                .await
                .unwrap();
            assert!(matches!(peer.recv_validated(&f.directory).await.unwrap(),
                Some(ReliableFrame::Data(bytes)) if bytes == b"second"));
            peer.send_goodbye(f.lease.community_id, GoodbyeReason::Draining)
                .await
                .unwrap();
        };
        tokio::join!(&mut consumer, receive);
    })
    .await
    .expect("echoes and peer Goodbye must terminate");
    assert_eq!(seen.consumed.load(Ordering::SeqCst), 3);
    assert!(
        peer.recv_validated(&f.directory).await.unwrap().is_none(),
        "no duplicate echo"
    );
    f.cleanup().await;
}

async fn drain_during_validation(latched: bool) {
    let f = Fixture::new().await;
    let (inbound, mut peer, seen) = streams(f.lease.fenced_header());
    let shutdown = Arc::new(AtomicBool::new(false));
    let consumer = run_demo_echo(f.directory.clone(), inbound, shutdown.clone());
    tokio::pin!(consumer);
    if latched {
        peer.send_bytes(f.lease.community_id, b"latch")
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            tokio::select! {
                _ = &mut consumer => panic!("premature termination"),
                frame = peer.recv_validated(&f.directory) => assert!(matches!(frame.unwrap(),
                    Some(ReliableFrame::Data(bytes)) if bytes == b"latch")),
            }
        })
        .await
        .unwrap();
    }
    let held = f.pool.get().await.unwrap();
    peer.send_bytes(f.lease.community_id, b"pending")
        .await
        .unwrap();
    assert!(poll!(&mut consumer).is_pending());
    let count = if latched { 2 } else { 1 };
    assert_eq!(seen.consumed.load(Ordering::SeqCst), count);
    housekeeping(&mut consumer).await;
    shutdown.store(true, Ordering::Relaxed);
    tokio::time::timeout(Duration::from_secs(1), &mut consumer)
        .await
        .unwrap();
    // Shutdown cannot wait for validation, and must not start another receive.
    assert_eq!(seen.receives.load(Ordering::SeqCst), count);
    assert_eq!(seen.finishes.load(Ordering::SeqCst), 1);
    drop(held);
    if latched {
        assert!(matches!(
            peer.recv_validated(&f.directory).await.unwrap(),
            Some(ReliableFrame::Goodbye(GoodbyeReason::Draining))
        ));
    }
    assert!(
        peer.recv_validated(&f.directory).await.unwrap().is_none(),
        "unvalidated Data must not echo"
    );
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires explicit REDIS_URL"]
async fn drain_before_latch_cancels_validation_and_finishes() {
    drain_during_validation(false).await;
}

#[tokio::test]
#[ignore = "requires explicit REDIS_URL"]
async fn drain_after_latch_cancels_validation_and_sends_goodbye() {
    drain_during_validation(true).await;
}

#[tokio::test]
#[ignore = "requires explicit REDIS_URL"]
async fn retained_receive_still_rejects_released_fence() {
    let f = Fixture::new().await;
    let (inbound, mut peer, seen) = streams(f.lease.fenced_header());
    let mut held = f.pool.get().await.unwrap();
    let consumer = run_demo_echo(
        f.directory.clone(),
        inbound,
        Arc::new(AtomicBool::new(false)),
    );
    tokio::pin!(consumer);
    peer.send_bytes(f.lease.community_id, b"stale")
        .await
        .unwrap();
    assert!(poll!(&mut consumer).is_pending());
    housekeeping(&mut consumer).await;
    // Remove only this test's lease while validation is still waiting for the
    // pool slot. Keep its generation floor, just like a normal release.
    let _: () = redis::cmd("DEL")
        .arg(format!(
            "buzz:{}:tunnel:{}:lease",
            f.lease.community_id, f.lease.session_id
        ))
        .query_async(&mut *held)
        .await
        .unwrap();
    drop(held);
    tokio::time::timeout(Duration::from_secs(2), &mut consumer)
        .await
        .unwrap();
    assert_eq!(seen.consumed.load(Ordering::SeqCst), 1);
    assert!(peer.recv_validated(&f.directory).await.unwrap().is_none());
    f.cleanup().await;
}

#[tokio::test]
async fn drain_idle_before_latch_and_eof_terminate_without_redis() {
    for draining in [true, false] {
        let directory = SessionDirectory::new(pool("redis://127.0.0.1:1".into()));
        let fenced = FencedHeader {
            session_id: uuid::Uuid::new_v4(),
            generation: 1,
            owner_runtime_id: RuntimeId([17; 32]),
        };
        let (inbound, peer, seen) = streams(fenced);
        let peer = if draining {
            Some(peer)
        } else {
            drop(peer);
            None
        };
        tokio::time::timeout(
            Duration::from_secs(1),
            run_demo_echo(directory, inbound, Arc::new(AtomicBool::new(draining))),
        )
        .await
        .unwrap();
        assert_eq!(seen.finishes.load(Ordering::SeqCst), usize::from(draining));
        assert!(seen.receives.load(Ordering::SeqCst) <= 1);
        drop(peer);
    }
}
