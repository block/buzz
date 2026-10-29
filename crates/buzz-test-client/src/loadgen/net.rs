//! Connection helpers that honor the scenario's `connect_addr` override.

use std::net::SocketAddr;
use std::time::Duration;

use buzz_ws_client::NostrWsConnection;
use nostr::{Event, JsonUtil, Keys, Tag};

use super::identity;
use super::scenario::{Community, Scenario};

/// Shared endpoints for one shard: an HTTP client whose community hosts
/// resolve to `connect_addr` when it is set.
#[derive(Clone)]
pub struct Net {
    pub http: reqwest::Client,
    connect_addr: Option<String>,
    connect_timeout: Duration,
}

impl Net {
    pub async fn new(scenario: &Scenario) -> anyhow::Result<Self> {
        let mut builder = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .pool_max_idle_per_host(1024);
        if let Some(addr) = &scenario.target.connect_addr {
            let resolved: SocketAddr = tokio::net::lookup_host(addr)
                .await?
                .next()
                .ok_or_else(|| anyhow::anyhow!("connect_addr {addr} did not resolve"))?;
            for community in scenario.communities() {
                if let Some(host) = &community.host {
                    builder = builder.resolve(host, resolved);
                }
            }
        }
        Ok(Self {
            http: builder.build()?,
            connect_addr: scenario.target.connect_addr.clone(),
            connect_timeout: Duration::from_secs_f64(scenario.target.connect_timeout_secs),
        })
    }

    /// Opens a WebSocket to `community` and authenticates as `keys`, failing
    /// once the whole attempt exceeds `target.connect_timeout_secs`.
    pub async fn connect(
        &self,
        community: &Community,
        keys: &Keys,
        auth_tag: Option<&Tag>,
    ) -> anyhow::Result<NostrWsConnection> {
        let attempt = async {
            let mut conn = match (&self.connect_addr, &community.host) {
                (Some(addr), Some(_)) => {
                    NostrWsConnection::connect_to_addr(&community.ws_url, addr).await?
                }
                _ => NostrWsConnection::connect(&community.ws_url).await?,
            };
            conn.authenticate(keys, auth_tag).await?;
            anyhow::Ok(conn)
        };
        tokio::time::timeout(self.connect_timeout, attempt)
            .await
            .map_err(|_| anyhow::anyhow!("connect timeout after {:?}", self.connect_timeout))?
    }

    /// `POST {base}{path}` with a NIP-98 header; returns status and body.
    pub async fn post(
        &self,
        keys: &Keys,
        base: &str,
        path: &str,
        body: Vec<u8>,
        auth_tag: Option<&Tag>,
    ) -> anyhow::Result<(reqwest::StatusCode, String)> {
        let url = format!("{base}{path}");
        let mut request = self
            .http
            .post(&url)
            .header(
                "Authorization",
                identity::nip98_header(keys, "POST", &url, &body),
            )
            .header("Content-Type", "application/json");
        if let Some(tag) = auth_tag {
            request = request.header("x-auth-tag", serde_json::to_string(tag.as_slice())?);
        }
        let response = request.body(body).send().await?;
        let status = response.status();
        Ok((status, response.text().await?))
    }

    /// `GET {url}` with a NIP-98 header; returns status and body.
    pub async fn get(
        &self,
        keys: &Keys,
        url: &str,
    ) -> anyhow::Result<(reqwest::StatusCode, String)> {
        let response = self
            .http
            .get(url)
            .header(
                "Authorization",
                identity::nip98_header(keys, "GET", url, &[]),
            )
            .send()
            .await?;
        let status = response.status();
        Ok((status, response.text().await?))
    }

    /// Submits `event` through HTTP `/events`; errors unless the relay accepted it.
    pub async fn submit(
        &self,
        keys: &Keys,
        community: &Community,
        event: &Event,
        auth_tag: Option<&Tag>,
    ) -> anyhow::Result<()> {
        let (status, body) = self
            .post(
                keys,
                &community.http_url,
                "/events",
                event.as_json().into_bytes(),
                auth_tag,
            )
            .await?;
        if !status.is_success() {
            anyhow::bail!("{}", http_reason(status, &body));
        }
        let value: serde_json::Value = serde_json::from_str(&body)?;
        if value["accepted"].as_bool() != Some(true) {
            anyhow::bail!("rejected: {}", value["message"].as_str().unwrap_or(&body));
        }
        Ok(())
    }
}

pub fn truncate(s: &str) -> String {
    s.chars().take(200).collect()
}

/// Failure text for a non-success HTTP response. It starts `HTTP <status>`,
/// which `stats` reads to tell a 429 from every other failure.
pub fn http_reason(status: reqwest::StatusCode, body: &str) -> String {
    format!("HTTP {status}: {}", truncate(body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stalled_upgrade_fails_at_the_connect_deadline() {
        // A peer that accepts TCP but never answers the WebSocket upgrade.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let held = tokio::spawn(async move {
            let mut open = Vec::new();
            while let Ok((socket, _)) = listener.accept().await {
                open.push(socket);
            }
        });
        let scenario = Scenario::parse(&format!(
            r#"
run_id = "t"
[target]
ws_url = "ws://{addr}"
http_url = "http://{addr}"
connect_timeout_secs = 0.3
[shape]
communities = 1
channels_per_community = 1
[population]
humans = 1
agents = 0
channels_per_member = 1
[[ramp]]
users = 1.0
hold_secs = 1
"#
        ))
        .unwrap();
        let net = Net::new(&scenario).await.unwrap();
        let community = &scenario.communities()[0];
        let started = std::time::Instant::now();
        let err = tokio::time::timeout(
            Duration::from_secs(5),
            net.connect(community, &Keys::generate(), None),
        )
        .await
        .expect("connect returns at its own deadline")
        .err()
        .expect("a silent peer cannot complete the upgrade");
        assert!(err.to_string().contains("connect timeout"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(2));
        held.abort();
    }
}
