//! Git hosting — Smart HTTP transport, permission hooks, and policy engine.
//!
//! # Module structure
//!
//! - `transport` — Smart HTTP protocol (info/refs, upload-pack, receive-pack)
//! - `hook` — Pre-receive hook script and injection
//! - `policy` — Internal policy endpoint (HMAC-authenticated callback from hook)

use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::post,
    Router,
};
use tower_http::limit::RequestBodyLimitLayer;

use crate::state::AppState;

pub mod cas_publish;
pub mod hook;
pub mod hydrate;
pub mod manifest;
pub mod manifest_event;
pub mod policy;
pub mod store;
pub mod transport;

pub use transport::git_router;

/// Connect-info marker for requests that arrived over the relay's Unix-domain
/// socket listener (`BUZZ_UDS_PATH`).
///
/// The UDS listener is bound to a filesystem path inside the pod and is only
/// reachable by processes in the same pod (the pre-receive hook is one). There
/// is no peer IP for a unix socket, so axum can't synthesize a loopback
/// `ConnectInfo<SocketAddr>` — instead we attach this marker and treat its
/// presence as equivalent to "came from localhost" in `require_localhost`.
#[derive(Clone, Debug)]
pub struct UdsConnectInfo;

#[cfg(unix)]
impl
    axum::extract::connect_info::Connected<
        axum::serve::IncomingStream<'_, tokio::net::UnixListener>,
    > for UdsConnectInfo
{
    fn connect_info(_stream: axum::serve::IncomingStream<'_, tokio::net::UnixListener>) -> Self {
        Self
    }
}

/// Middleware that rejects requests from non-loopback addresses.
///
/// Defense-in-depth: the internal policy endpoint should only be reachable
/// from localhost (the pre-receive hook runs on the same host as the relay).
///
/// Two trusted transports satisfy "localhost":
/// - a loopback TCP peer (`ConnectInfo<SocketAddr>` with a loopback IP), or
/// - the relay's own Unix-domain socket (`ConnectInfo<UdsConnectInfo>`), which
///   is bound to an in-pod path and not reachable off-host.
///
/// Fail-closed: if neither connect-info is present, reject. In particular the
/// TCP listener still requires a loopback `SocketAddr`, so this does not weaken
/// the existing TCP guard.
async fn require_localhost(req: Request<Body>, next: Next) -> Response {
    let from_loopback_tcp = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0.ip().is_loopback())
        .unwrap_or(false);

    let from_uds = req
        .extensions()
        .get::<ConnectInfo<UdsConnectInfo>>()
        .is_some();

    if !from_loopback_tcp && !from_uds {
        return (StatusCode::FORBIDDEN, "internal endpoint: localhost only").into_response();
    }

    next.run(req).await
}

/// Build the internal git policy router.
///
/// Mounted at `/internal/git/policy` — only accessible from localhost.
/// The pre-receive hook calls this to authorize pushes.
/// Body limit: 1 MB (500 refs × ~200 bytes each = ~100 KB typical; 1 MB is generous).
pub fn git_policy_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/internal/git/policy", post(policy::hook_policy_check))
        .layer(RequestBodyLimitLayer::new(1024 * 1024)) // 1 MB
        .layer(middleware::from_fn(require_localhost))
        .with_state(state)
}

#[cfg(test)]
mod require_localhost_tests {
    use super::*;
    use axum::{body::Body, http::Request, routing::get};
    use std::net::{Ipv4Addr, SocketAddr};
    use tower::ServiceExt; // for `oneshot`

    /// A trivial router guarded by `require_localhost`, returning 200 when the
    /// guard lets the request through. The handler itself never rejects, so any
    /// 403 we observe is the guard's doing.
    fn guarded_router() -> Router {
        Router::new()
            .route("/x", get(|| async { StatusCode::OK }))
            .layer(middleware::from_fn(require_localhost))
    }

    async fn status_with<F>(install_connect_info: F) -> StatusCode
    where
        F: FnOnce(Request<Body>) -> Request<Body>,
    {
        let req = install_connect_info(Request::builder().uri("/x").body(Body::empty()).unwrap());
        guarded_router().oneshot(req).await.unwrap().status()
    }

    #[tokio::test]
    async fn rejects_when_no_connect_info() {
        // Fail-closed: neither TCP nor UDS connect-info present.
        assert_eq!(status_with(|req| req).await, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn accepts_loopback_tcp() {
        let status = status_with(|mut req| {
            req.extensions_mut()
                .insert(ConnectInfo(SocketAddr::from((Ipv4Addr::LOCALHOST, 12345))));
            req
        })
        .await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn rejects_non_loopback_tcp() {
        let status = status_with(|mut req| {
            req.extensions_mut().insert(ConnectInfo(SocketAddr::from((
                Ipv4Addr::new(10, 0, 0, 5),
                12345,
            ))));
            req
        })
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn accepts_uds_marker() {
        let status = status_with(|mut req| {
            req.extensions_mut().insert(ConnectInfo(UdsConnectInfo));
            req
        })
        .await;
        assert_eq!(status, StatusCode::OK);
    }
}
