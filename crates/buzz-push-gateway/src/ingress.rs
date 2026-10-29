//! Independent, bounded ingress lanes keep anonymous delivery/enrollment work
//! from occupying management and revocation capacity. Proxy headers require
//! explicitly trusted TCP peers and a single, proxy-overwritten client address.
use axum::{
    extract::{ConnectInfo, Request},
    middleware::Next,
    response::Response,
};
use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    sync::{Arc, Mutex},
    time::Instant,
};
use tokio::sync::Semaphore;

struct Sources {
    last_sweep: Instant,
    entries: HashMap<IpAddr, (Instant, u32)>,
}

#[derive(Clone)]
pub(crate) struct Lane {
    work: Arc<Semaphore>,
    sources: Arc<Mutex<Sources>>,
    per_minute: u32,
    trusted_proxy_cidrs: Vec<(IpAddr, u8)>,
}
impl Lane {
    pub(crate) fn new(
        concurrent: usize,
        per_minute: u32,
        trusted_proxy_cidrs: Vec<(IpAddr, u8)>,
    ) -> Self {
        Self {
            work: Arc::new(Semaphore::new(concurrent)),
            sources: Arc::new(Mutex::new(Sources {
                last_sweep: Instant::now(),
                entries: HashMap::new(),
            })),
            per_minute,
            trusted_proxy_cidrs,
        }
    }
    pub(crate) async fn admit(self, request: Request, next: Next) -> Response {
        let source = request
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .map(|v| v.0.ip().to_canonical());
        #[cfg(test)]
        let source = source.or(Some(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)));
        let Some(source) = source else {
            return super::http::throttled(60);
        };
        let source =
            if self
                .trusted_proxy_cidrs
                .iter()
                .any(|(network, prefix)| match (source, *network) {
                    (IpAddr::V4(ip), IpAddr::V4(net)) if (1..=32).contains(prefix) => {
                        ip.to_bits() >> (32 - prefix) == net.to_bits() >> (32 - prefix)
                    }
                    (IpAddr::V6(ip), IpAddr::V6(net)) if (1..=128).contains(prefix) => {
                        ip.to_bits() >> (128 - prefix) == net.to_bits() >> (128 - prefix)
                    }
                    _ => false,
                })
            {
                let mut values = request.headers().get_all("x-buzz-client-ip").iter();
                let address = values
                    .next()
                    .and_then(|v| v.to_str().ok())
                    .filter(|v| v.len() <= 45)
                    .and_then(|v| v.parse::<IpAddr>().ok());
                let Some(address) = address.filter(|_| values.next().is_none()) else {
                    return super::http::throttled(60);
                };
                address
            } else {
                source
            }
            .to_canonical();
        if !self.allow_source(source) {
            return super::http::throttled(60);
        }
        let Ok(_permit) = self.work.try_acquire() else {
            return super::http::throttled(1);
        };
        next.run(request).await
    }
    fn allow_source(&self, source: IpAddr) -> bool {
        let Ok(mut sources) = self.sources.lock() else {
            return false;
        };
        let now = Instant::now();
        if now.duration_since(sources.last_sweep).as_secs() >= 60 {
            sources
                .entries
                .retain(|_, (start, _)| now.duration_since(*start).as_secs() < 60);
            sources.last_sweep = now;
        }
        if sources.entries.len() >= 4096 && !sources.entries.contains_key(&source) {
            return false;
        }
        let (start, used) = sources.entries.entry(source).or_insert((now, 0));
        if now.duration_since(*start).as_secs() >= 60 {
            *start = now;
            *used = 0;
        }
        if *used >= self.per_minute {
            return false;
        }
        *used += 1;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::StatusCode, routing::post, Router};
    use tower::ServiceExt;

    fn app(lane: Lane) -> Router {
        Router::new()
            .route(
                "/v1/delegations/revoke",
                post(|| async { StatusCode::NO_CONTENT }),
            )
            .layer(axum::middleware::from_fn(move |request, next| {
                lane.clone().admit(request, next)
            }))
    }

    fn request(peer: &str, headers: &[(&str, &str)]) -> Request {
        let mut request = Request::builder()
            .method("POST")
            .uri("/v1/delegations/revoke")
            .body(Body::empty())
            .unwrap();
        request
            .extensions_mut()
            .insert(ConnectInfo(peer.parse::<SocketAddr>().unwrap()));
        for (name, value) in headers {
            request.headers_mut().append(
                axum::http::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                value.parse().unwrap(),
            );
        }
        request
    }

    #[tokio::test]
    async fn trusted_proxy_clients_have_separate_source_budgets() {
        let app = app(Lane::new(
            4,
            1,
            vec![
                ("10.0.0.0".parse().unwrap(), 24),
                ("2001:db8::".parse().unwrap(), 64),
            ],
        ));
        for (peer, client) in [
            ("10.0.0.2:123", "192.0.2.1"),
            ("10.0.0.2:123", "192.0.2.2"),
            ("[::ffff:10.0.0.2]:123", "192.0.2.3"),
            ("[::ffff:10.0.0.2]:123", "192.0.2.4"),
            ("[2001:db8::1]:123", "2001:db8:1::1"),
        ] {
            assert_eq!(
                app.clone()
                    .oneshot(request(peer, &[("x-buzz-client-ip", client)]))
                    .await
                    .unwrap()
                    .status(),
                StatusCode::NO_CONTENT
            );
            let response = app
                .clone()
                .oneshot(request(peer, &[("x-buzz-client-ip", client)]))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
            assert_eq!(response.headers()["retry-after"], "60");
        }
    }

    #[tokio::test]
    async fn untrusted_clients_cannot_spoof_a_fresh_budget() {
        let app = app(Lane::new(4, 1, vec![("10.0.0.2".parse().unwrap(), 32)]));
        for (n, expected) in [
            (1, StatusCode::NO_CONTENT),
            (2, StatusCode::TOO_MANY_REQUESTS),
        ] {
            let claimed = format!("192.0.2.{n}");
            assert_eq!(
                app.clone()
                    .oneshot(request(
                        "10.0.1.2:123",
                        &[
                            ("x-buzz-client-ip", &claimed),
                            ("x-forwarded-for", &claimed),
                            ("forwarded", &claimed)
                        ]
                    ))
                    .await
                    .unwrap()
                    .status(),
                expected
            );
        }
    }

    #[tokio::test]
    async fn trusted_proxy_requires_one_bare_ip_without_consuming_client_capacity() {
        let lane = Lane::new(4, 1, vec![("10.0.0.2".parse().unwrap(), 32)]);
        let app = app(lane.clone());
        for headers in [
            vec![],
            vec![("x-buzz-client-ip", "192.0.2.1,192.0.2.2")],
            vec![("x-buzz-client-ip", "192.0.2.1:80")],
            vec![("x-buzz-client-ip", "unknown")],
            vec![("x-buzz-client-ip", " 192.0.2.1")],
            vec![("x-buzz-client-ip", "[::1]")],
            vec![
                ("x-buzz-client-ip", "192.0.2.1"),
                ("x-buzz-client-ip", "192.0.2.1"),
            ],
        ] {
            assert_eq!(
                app.clone()
                    .oneshot(request("10.0.0.2:123", &headers))
                    .await
                    .unwrap()
                    .status(),
                StatusCode::TOO_MANY_REQUESTS
            );
        }
        assert!(lane.sources.lock().unwrap().entries.is_empty());
        assert_eq!(lane.work.available_permits(), 4);
        assert_eq!(
            app.oneshot(request(
                "10.0.0.2:123",
                &[("x-buzz-client-ip", "192.0.2.1")]
            ))
            .await
            .unwrap()
            .status(),
            StatusCode::NO_CONTENT
        );
    }

    #[test]
    fn bookkeeping_and_sources_are_bounded_and_lanes_are_independent() {
        let delivery = Lane::new(8, 1, Vec::new());
        let revocation = Lane::new(4, 1, Vec::new());
        for n in 0..4096u32 {
            assert!(delivery.allow_source(IpAddr::V4(n.into())));
        }
        let other = IpAddr::V4(5000u32.into());
        assert!(!delivery.allow_source(other));
        assert!(revocation.allow_source(other));
        assert!(!revocation.allow_source(other));
        assert_eq!(delivery.sources.lock().unwrap().entries.len(), 4096);
    }
    #[tokio::test]
    async fn delivery_cannot_acquire_reserved_revocation_work() {
        let delivery = Lane::new(8, 10, Vec::new());
        let revocation = Lane::new(4, 10, Vec::new());
        let _busy = delivery.work.acquire_many(8).await.unwrap();
        assert!(delivery.work.try_acquire().is_err());
        assert!(revocation.work.try_acquire().is_ok());
    }
}
