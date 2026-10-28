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
        })
    }

    /// Opens a WebSocket to `community` and authenticates as `keys`.
    pub async fn connect(
        &self,
        community: &Community,
        keys: &Keys,
        auth_tag: Option<&Tag>,
    ) -> anyhow::Result<NostrWsConnection> {
        let mut conn = match (&self.connect_addr, &community.host) {
            (Some(addr), Some(_)) => {
                NostrWsConnection::connect_to_addr(&community.ws_url, addr).await?
            }
            _ => NostrWsConnection::connect(&community.ws_url).await?,
        };
        conn.authenticate(keys, auth_tag).await?;
        Ok(conn)
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
            anyhow::bail!("HTTP {status}: {}", truncate(&body));
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
