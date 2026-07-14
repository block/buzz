//! `POST /_mesh/demo/echo` — testbed-only join-side ingress for the mesh
//! reliable-stream smoke.
//!
//! This is the *client leg* of the `BUZZ_MESH_DEMO_ECHO` evidence run: the
//! owner-side echo consumer (see `mesh_boot::run_demo_echo`) validates and
//! echoes frames, but nothing in the product calls
//! [`ReliableStreamRouter::join`] yet — so cross-pod evidence needs a way to
//! drive a join from a chosen pod. This route is that way, and nothing more:
//!
//! - Gated on **both** `BUZZ_MESH_DEMO_ECHO=on` and mesh enabled; 404
//!   otherwise (the same strictness as the owner-side consumer — the route
//!   does not exist unless the operator opted the deployment into the demo).
//! - `Owned` result: this pod acquired the fenced lease. No renewer is
//!   spawned — this is a smoke, not a session — so the lease lives for its
//!   Redis TTL (30s default). Drive the owner pod first, then the forwarding
//!   pod within that window.
//! - `Forwarded` result: sends the payload to the owner over the mesh and
//!   waits (bounded) for the echoed frame, proving owner-side
//!   `recv_validated` (Redis fence included) and return-path delivery.
//!
//! Not a product flow. The real join-side consumer (goose/berd session
//! wiring) replaces this; the route stays demo-gated until it is deleted.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use buzz_core::CommunityId;
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::state::AppState;
use crate::tunnel::directory::SessionDirectory;
use crate::tunnel::reliable::{
    ReliableFrame, ReliableJoin, ReliableStreamError, ReliableStreamRouter,
};
use buzz_relay_mesh::RelayPeerTransport;

/// How long the forwarded leg waits for the owner's echo before failing the
/// smoke. Generous relative to an intra-cluster RTT; small enough that a
/// wedged owner fails the run instead of hanging the probe.
const ECHO_TIMEOUT: Duration = Duration::from_secs(10);

/// Request body for the demo echo probe.
#[derive(Debug, Deserialize)]
pub struct DemoEchoRequest {
    /// Community/tenant scope for the fenced session.
    pub community_id: Uuid,
    /// Session to join. The first pod to post a given id becomes the owner.
    pub session_id: Uuid,
    /// Opaque payload for the echo round-trip (UTF-8 for readable evidence).
    pub payload: String,
}

/// `POST /_mesh/demo/echo` handler. 404 unless the deployment opted in.
pub async fn demo_echo(
    State(state): State<Arc<AppState>>,
    Json(req): Json<DemoEchoRequest>,
) -> Response {
    // Same gate as the owner-side consumer: both flags, or the route does
    // not exist. 404 (not 403) so a non-demo deployment is indistinguishable
    // from one without the route.
    let Some(handle) = state.mesh() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if !state.config.mesh_demo_echo {
        return StatusCode::NOT_FOUND.into_response();
    }

    let router = ReliableStreamRouter::new(
        handle.directory.clone(),
        Arc::clone(&handle.transport),
        handle.local_runtime_id,
    );
    run_demo_join(&router, &handle.directory, req).await
}

/// Core of the probe, split from the handler so tests can drive it with a
/// directory + transport pair without standing up an `AppState`.
async fn run_demo_join<T>(
    router: &ReliableStreamRouter<T>,
    directory: &SessionDirectory,
    req: DemoEchoRequest,
) -> Response
where
    T: RelayPeerTransport + ?Sized,
{
    let community_id = CommunityId::from_uuid(req.community_id);
    match router.join(community_id, req.session_id).await {
        // This pod took (or already holds) the fenced lease. The lease is
        // deliberately not renewed: it expires with its Redis TTL.
        Ok(ReliableJoin::Owned { lease }) => Json(json!({
            "outcome": "owned",
            "generation": lease.generation,
            "owner_runtime_id": lease.owner_runtime_id.to_string(),
        }))
        .into_response(),

        // Another pod owns the session: send the payload over the mesh and
        // wait for the owner-side echo consumer to bounce it back.
        Ok(ReliableJoin::Forwarded { lease, mut stream }) => {
            if let Err(e) = stream
                .send_bytes(community_id, req.payload.as_bytes())
                .await
            {
                return echo_error(StatusCode::BAD_GATEWAY, "send failed", &e);
            }
            let echoed = tokio::time::timeout(ECHO_TIMEOUT, stream.recv_validated(directory)).await;
            match echoed {
                Ok(Ok(Some(ReliableFrame::Data(bytes)))) => Json(json!({
                    "outcome": "forwarded",
                    "generation": lease.generation,
                    "owner_runtime_id": lease.owner_runtime_id.to_string(),
                    "echoed_payload": String::from_utf8_lossy(&bytes),
                }))
                .into_response(),
                Ok(Ok(Some(ReliableFrame::Goodbye(reason)))) => (
                    StatusCode::BAD_GATEWAY,
                    Json(json!({"error": format!("owner sent goodbye: {reason:?}")})),
                )
                    .into_response(),
                Ok(Ok(None)) => (
                    StatusCode::BAD_GATEWAY,
                    Json(json!({"error": "stream closed before echo"})),
                )
                    .into_response(),
                Ok(Err(e)) => echo_error(StatusCode::BAD_GATEWAY, "recv failed", &e),
                Err(_) => (
                    StatusCode::GATEWAY_TIMEOUT,
                    Json(json!({"error": "timed out waiting for echo"})),
                )
                    .into_response(),
            }
        }

        Err(e) => echo_error(StatusCode::BAD_GATEWAY, "join failed", &e),
    }
}

fn echo_error(status: StatusCode, what: &str, e: &ReliableStreamError) -> Response {
    (status, Json(json!({"error": format!("{what}: {e}")}))).into_response()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use axum::body::to_bytes;
    use buzz_relay_mesh::endpoint::MeshEndpoint;
    use buzz_relay_mesh::{
        InboundHandler, MeshDatagram, MeshError, MeshStream, MeshStreamFrame, RuntimeId,
        StreamHello,
    };
    use uuid::Uuid;

    use super::*;
    use crate::tunnel::directory::SessionDirectory;

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
            Duration::from_secs(5),
        ))
    }

    struct NoopTransport;

    impl RelayPeerTransport for NoopTransport {
        fn send_datagram(&self, _to: RuntimeId, _dgram: MeshDatagram) -> Result<(), MeshError> {
            unreachable!("demo owned-arm test never sends datagrams")
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

    struct DirectTransport {
        peer: buzz_relay_mesh::peer::MeshPeer,
    }

    impl RelayPeerTransport for DirectTransport {
        fn send_datagram(&self, _to: RuntimeId, _dgram: MeshDatagram) -> Result<(), MeshError> {
            unreachable!("demo forwarded-arm test never sends datagrams")
        }

        fn open_session_stream(
            &self,
            _to: RuntimeId,
            hello: StreamHello,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<MeshStream, MeshError>> + Send + '_>,
        > {
            Box::pin(async move {
                let mut stream = self.peer.open_bi().await?;
                stream.send_frame(MeshStreamFrame::Hello(hello)).await?;
                Ok(stream)
            })
        }

        fn set_inbound(&self, _handler: Box<dyn InboundHandler>) {}
    }

    async fn body_json(resp: Response) -> serde_json::Value {
        let bytes = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    /// First post for a session acquires the fenced lease and reports `owned`.
    #[tokio::test]
    async fn demo_join_owned_arm_reports_generation() {
        let Some(directory) = redis_directory_if_available().await else {
            return;
        };
        let router = ReliableStreamRouter::new(
            directory.clone(),
            std::sync::Arc::new(NoopTransport),
            RuntimeId([7; 32]),
        );
        let resp = run_demo_join(
            &router,
            &directory,
            DemoEchoRequest {
                community_id: Uuid::new_v4(),
                session_id: Uuid::new_v4(),
                payload: "unused".into(),
            },
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_json(resp).await;
        assert_eq!(body["outcome"], "owned");
        assert!(body["generation"].as_u64().is_some());
    }

    /// Second runtime forwards to the owner and round-trips the payload
    /// through the owner-side echo consumer (`recv_validated` + `send_bytes`),
    /// end to end over a real mesh stream pair.
    #[tokio::test]
    async fn demo_join_forwarded_arm_round_trips_echo() {
        let Some(directory) = redis_directory_if_available().await else {
            return;
        };
        let community_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();

        let bind = || "127.0.0.1:0".parse().unwrap();
        let local_endpoint = MeshEndpoint::bind(bind()).await.unwrap();
        let owner_endpoint = MeshEndpoint::bind(bind()).await.unwrap();
        let owner_runtime = owner_endpoint.runtime_id();
        let owner_addr = owner_endpoint.addr();

        // Owner acquires the lease first (Mari's run order: podB first).
        let owner_router = ReliableStreamRouter::new(
            directory.clone(),
            std::sync::Arc::new(NoopTransport),
            owner_runtime,
        );
        let owned = run_demo_join(
            &owner_router,
            &directory,
            DemoEchoRequest {
                community_id,
                session_id,
                payload: "unused".into(),
            },
        )
        .await;
        assert_eq!(owned.status(), StatusCode::OK);
        assert_eq!(body_json(owned).await["outcome"], "owned");

        // Owner side: accept the inbound mesh stream and run the real demo
        // echo consumer against it.
        let accept_endpoint = owner_endpoint.clone();
        let echo_directory = directory.clone();
        let owner_task = tokio::spawn(async move {
            let peer = accept_endpoint.accept().await.unwrap().unwrap();
            let mut stream = peer.accept_bi().await.unwrap();
            let hello = match stream.recv_frame().await.unwrap().unwrap() {
                MeshStreamFrame::Hello(h) => h,
                other => panic!("expected hello, got {other:?}"),
            };
            let router = ReliableStreamRouter::new(
                echo_directory.clone(),
                std::sync::Arc::new(NoopTransport),
                owner_runtime,
            );
            let from = hello.sender;
            let inbound = router.accept_inbound(from, hello, stream).await.unwrap();
            crate::mesh_boot::run_demo_echo(
                echo_directory,
                inbound,
                std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            )
            .await;
        });

        // Forwarding side: join the same session through the demo core.
        let local_peer = local_endpoint.connect(owner_addr).await.unwrap();
        let local_router = ReliableStreamRouter::new(
            directory.clone(),
            std::sync::Arc::new(DirectTransport { peer: local_peer }),
            local_endpoint.runtime_id(),
        );
        let resp = run_demo_join(
            &local_router,
            &directory,
            DemoEchoRequest {
                community_id,
                session_id,
                payload: "mesh echo evidence".into(),
            },
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_json(resp).await;
        assert_eq!(body["outcome"], "forwarded");
        assert_eq!(body["echoed_payload"], "mesh echo evidence");
        owner_task.abort();
    }
}
