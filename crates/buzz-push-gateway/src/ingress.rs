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
    hash::BuildHasher,
    net::{IpAddr, SocketAddr},
    sync::{Arc, Mutex},
    time::Instant,
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

struct Sources {
    last_sweep: Instant,
    entries: HashMap<u16, (Instant, u32, Arc<Semaphore>)>,
}
impl Sources {
    fn bucket(&self, source: IpAddr) -> u16 {
        let source = match source.to_canonical() {
            IpAddr::V6(ip) => IpAddr::V6((ip.to_bits() & (u128::MAX << 64)).into()),
            ip => ip,
        };
        // Randomized, fixed buckets bound memory without rejecting every new
        // source when full. Collisions share limits; IPv6 hosts share a /64.
        (self.entries.hasher().hash_one(source) % 4096) as u16
    }
}

// Shared with the handler so committed delivery work can outlive its request.
pub(crate) struct WorkPermit {
    _work: OwnedSemaphorePermit,
    _source: OwnedSemaphorePermit,
}

#[derive(Clone)]
pub(crate) struct Lane {
    work: Arc<Semaphore>,
    sources: Arc<Mutex<Sources>>,
    per_minute: u32,
    per_source_concurrent: usize,
    trusted_proxy_cidrs: Vec<(IpAddr, u8)>,
}
impl Lane {
    pub(crate) fn new(
        concurrent: usize,
        per_source_concurrent: usize,
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
            per_source_concurrent,
            trusted_proxy_cidrs,
        }
    }
    pub(crate) async fn admit(self, mut request: Request, next: Next) -> Response {
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
        let source_permit = match self.admit_source(source) {
            Ok(permit) => permit,
            Err(retry_after) => return super::http::throttled(retry_after),
        };
        let Ok(work) = self.work.clone().try_acquire_owned() else {
            return super::http::throttled(1);
        };
        let permit = Arc::new(WorkPermit {
            _work: work,
            _source: source_permit,
        });
        request.extensions_mut().insert(permit.clone());
        let response = next.run(request).await;
        drop(permit);
        response
    }
    fn admit_source(&self, source: IpAddr) -> Result<OwnedSemaphorePermit, i64> {
        let mut sources = self.sources.lock().map_err(|_| 60)?;
        let source = sources.bucket(source);
        let now = Instant::now();
        if now.duration_since(sources.last_sweep).as_secs() >= 60 {
            sources.entries.retain(|_, (start, _, work)| {
                now.duration_since(*start).as_secs() < 60
                    || work.available_permits() < self.per_source_concurrent
            });
            sources.last_sweep = now;
        }
        let (start, used, work) = sources
            .entries
            .entry(source)
            .or_insert_with(|| (now, 0, Arc::new(Semaphore::new(self.per_source_concurrent))));
        if now.duration_since(*start).as_secs() >= 60 {
            *start = now;
            *used = 0;
        }
        if *used >= self.per_minute {
            return Err(60);
        }
        *used += 1;
        work.clone().try_acquire_owned().map_err(|_| 1)
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

    // Pick distinct buckets explicitly so randomized hashing cannot flake tests.
    fn unused_source(lane: &Lane, source: IpAddr) -> IpAddr {
        let sources = lane.sources.lock().unwrap();
        (0..65536u128)
            .map(|offset| match source {
                IpAddr::V4(ip) => IpAddr::V4((ip.to_bits() + offset as u32).into()),
                IpAddr::V6(ip) => IpAddr::V6((ip.to_bits() + (offset << 64)).into()),
            })
            .find(|source| !sources.entries.contains_key(&sources.bucket(*source)))
            .unwrap()
    }

    #[tokio::test]
    async fn trusted_proxy_clients_have_separate_source_budgets() {
        let lane = Lane::new(
            4,
            2,
            1,
            vec![
                ("10.0.0.0".parse().unwrap(), 24),
                ("2001:db8::".parse().unwrap(), 64),
            ],
        );
        let app = app(lane.clone());
        for (peer, client) in [
            ("10.0.0.2:123", "192.0.2.1"),
            ("10.0.0.2:123", "192.0.2.2"),
            ("[::ffff:10.0.0.2]:123", "192.0.2.3"),
            ("[::ffff:10.0.0.2]:123", "192.0.2.4"),
            ("[2001:db8::1]:123", "2001:db8:1::1"),
        ] {
            let client = unused_source(&lane, client.parse().unwrap()).to_string();
            assert_eq!(
                app.clone()
                    .oneshot(request(peer, &[("x-buzz-client-ip", &client)]))
                    .await
                    .unwrap()
                    .status(),
                StatusCode::NO_CONTENT
            );
            let response = app
                .clone()
                .oneshot(request(peer, &[("x-buzz-client-ip", &client)]))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
            assert_eq!(response.headers()["retry-after"], "60");
        }
    }

    #[tokio::test]
    async fn untrusted_clients_cannot_spoof_a_fresh_budget() {
        let app = app(Lane::new(4, 2, 1, vec![("10.0.0.2".parse().unwrap(), 32)]));
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
    async fn ipv6_hosts_share_a_prefix_budget_directly_and_through_a_proxy() {
        for proxied in [false, true] {
            let app = app(Lane::new(4, 2, 1, vec![("10.0.0.2".parse().unwrap(), 32)]));
            for (client, expected) in [
                ("2001:db8:1::1", StatusCode::NO_CONTENT),
                ("2001:db8:1::2", StatusCode::TOO_MANY_REQUESTS),
            ] {
                let request = if proxied {
                    request("10.0.0.2:123", &[("x-buzz-client-ip", client)])
                } else {
                    request(&format!("[{client}]:123"), &[])
                };
                assert_eq!(
                    app.clone().oneshot(request).await.unwrap().status(),
                    expected
                );
            }
        }
    }

    #[tokio::test]
    async fn source_churn_does_not_fill_the_table_and_block_new_clients() {
        let lane = Lane::new(4, 2, u32::MAX, Vec::new());
        let app = app(lane.clone());
        for n in 0..4097 {
            let peer = format!("198.18.{}.{}:123", n >> 8, n & 255);
            assert_eq!(
                app.clone()
                    .oneshot(request(&peer, &[]))
                    .await
                    .unwrap()
                    .status(),
                StatusCode::NO_CONTENT
            );
        }
        assert_eq!(
            app.oneshot(request("203.0.113.1:123", &[]))
                .await
                .unwrap()
                .status(),
            StatusCode::NO_CONTENT
        );
        assert!(lane.sources.lock().unwrap().entries.len() <= 4096);
    }

    #[tokio::test]
    async fn trusted_proxy_requires_one_bare_ip_without_consuming_client_capacity() {
        let lane = Lane::new(4, 2, 1, vec![("10.0.0.2".parse().unwrap(), 32)]);
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
    fn source_budgets_are_independent_between_lanes() {
        let delivery = Lane::new(8, 8, 1, Vec::new());
        let revocation = Lane::new(4, 2, 1, Vec::new());
        let source = "192.0.2.1".parse().unwrap();
        assert!(delivery.admit_source(source).is_ok());
        assert!(delivery.admit_source(source).is_err());
        assert!(revocation.admit_source(source).is_ok());
        assert!(revocation.admit_source(source).is_err());
    }
    #[test]
    fn active_source_capacity_survives_rate_reset_and_sweep() {
        let lane = Lane::new(4, 2, 120, Vec::new());
        let source = "192.0.2.1".parse().unwrap();
        let collision = {
            let sources = lane.sources.lock().unwrap();
            (1..1_000_000u32)
                .map(|n| IpAddr::V4(n.into()))
                .find(|ip| *ip != source && sources.bucket(*ip) == sources.bucket(source))
                .unwrap()
        };
        let a = lane.admit_source(source).unwrap();
        let b = lane.admit_source(collision).unwrap();
        let other = unused_source(&lane, "192.0.2.2".parse().unwrap());
        assert_eq!(lane.admit_source(source).unwrap_err(), 1);
        // Expire the rate window while work is still active. Neither a reset
        // nor a sweep may replace the semaphore owned by those requests.
        let age_source = || {
            let mut sources = lane.sources.lock().unwrap();
            let bucket = sources.bucket(source);
            sources.last_sweep = Instant::now() - std::time::Duration::from_secs(61);
            sources.entries.get_mut(&bucket).unwrap().0 = sources.last_sweep;
        };
        age_source();
        drop(lane.admit_source(other).unwrap()); // Sweep from a different source.
        assert_eq!(lane.admit_source(source).unwrap_err(), 1); // Rate reset.
        assert_eq!(lane.admit_source(collision).unwrap_err(), 1);
        drop(a);
        let c = lane.admit_source(source).unwrap();
        assert_eq!(lane.admit_source(source).unwrap_err(), 1);
        drop((b, c));
        age_source();
        drop(lane.admit_source(other).unwrap());
        {
            let sources = lane.sources.lock().unwrap();
            assert!(!sources.entries.contains_key(&sources.bucket(source)));
        }
        let _a = lane.admit_source(source).unwrap();
        let _b = lane.admit_source(collision).unwrap();
        assert_eq!(lane.admit_source(source).unwrap_err(), 1);
    }

    #[tokio::test]
    async fn delivery_cannot_acquire_reserved_revocation_work() {
        let delivery = Lane::new(8, 8, 10, Vec::new());
        let revocation = Lane::new(4, 2, 10, Vec::new());
        let _busy = delivery.work.acquire_many(8).await.unwrap();
        assert!(delivery.work.try_acquire().is_err());
        assert!(revocation.work.try_acquire().is_ok());
    }
}
