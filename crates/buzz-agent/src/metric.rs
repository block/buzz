//! NIP-AM kind:44200 metric publishing for the buzz-agent harness.
//!
//! Built from three environment variables:
//! - `BUZZ_PRIVATE_KEY` — agent Nostr private key (nsec or hex).
//! - `BUZZ_RELAY_URL` — relay base URL (e.g. `https://relay.example.com`).
//! - `BUZZ_AGENT_OWNER_PUBKEY` — owner npub or hex public key.
//!
//! If any variable is absent or unparseable, metric publishing is a silent
//! no-op. This mirrors the fail-open policy used throughout the agent harness.
//!
//! ## Turn tracking
//!
//! buzz-agent has no session-cumulative token counters. Each turn may span
//! multiple LLM rounds (tool calls); per-turn tokens are accumulated across
//! all rounds. `deltaReliable` is always `true` because buzz-agent tracks
//! every round within a turn in-process — no cross-process baseline is ever
//! lost. Session-level cumulative fields are omitted (`None`) because
//! buzz-agent does not maintain running totals across turns in a session.

use nostr::Keys;
use reqwest::Client;

/// Configured NIP-AM publisher. Constructed once per process from env vars.
/// When env vars are absent, construction succeeds and `is_noop()` returns
/// `true` — callers need not special-case the unconfigured case.
pub(crate) struct MetricPublisher {
    keys: Option<Keys>,
    owner_pubkey: Option<nostr::PublicKey>,
    base_url: Option<String>,
    http: Client,
}

impl MetricPublisher {
    /// Build from environment. Silent on parse errors — missing/malformed vars
    /// leave the corresponding field `None`.
    pub(crate) fn from_env() -> Self {
        let keys = std::env::var("BUZZ_PRIVATE_KEY")
            .ok()
            .and_then(|v| Keys::parse(&v).ok());
        let base_url = std::env::var("BUZZ_RELAY_URL")
            .ok()
            .filter(|s| !s.is_empty())
            .map(|s| s.trim_end_matches('/').to_string());
        let owner_pubkey = std::env::var("BUZZ_AGENT_OWNER_PUBKEY")
            .ok()
            .and_then(|v| nostr::PublicKey::parse(&v).ok());
        Self {
            keys,
            owner_pubkey,
            base_url,
            http: Client::new(),
        }
    }

    /// Returns `true` when no complete config is available. Publishing is
    /// always a no-op in this state.
    #[cfg(test)]
    pub(crate) fn is_noop(&self) -> bool {
        self.keys.is_none() || self.owner_pubkey.is_none() || self.base_url.is_none()
    }

    /// Best-effort publish a kind 44200 event.
    ///
    /// - `session_id` — the ACP session id for this turn.
    /// - `turn_seq` — monotonically increasing per-session turn counter.
    /// - `turn_id` — the run id for this turn (harness-internal).
    /// - `input_tokens` / `output_tokens` — summed across all LLM rounds in the turn.
    /// - `stop_reason` — the NIP-AM stop reason.
    ///
    /// Errors are logged at WARN and never propagated — a metric publish
    /// failure must never fail a turn.
    pub(crate) async fn publish(
        &self,
        session_id: &str,
        turn_seq: u64,
        turn_id: &str,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
        stop_reason: buzz_core::agent_turn_metric::StopReason,
    ) {
        use buzz_core::agent_turn_metric::{AgentTurnMetricPayload, TokenCounts};
        use nostr::{EventBuilder, Kind, Tag};

        let (keys, owner_pk, base_url) = match (&self.keys, &self.owner_pubkey, &self.base_url) {
            (Some(k), Some(pk), Some(url)) => (k, pk, url),
            _ => return,
        };

        // buzz-agent has no session-cumulative counters — only per-turn deltas.
        // deltaReliable is true because we sum every round in this process;
        // no cross-process baseline is ever lost. Cumulative fields are omitted
        // since buzz-agent does not track rolling session totals across turns.
        let turn_counts = if input_tokens.is_some() || output_tokens.is_some() {
            Some(TokenCounts {
                input_tokens,
                output_tokens,
                total_tokens: None,
                cost_usd: None,
                cache_read_tokens: None,
                cache_write_tokens: None,
            })
        } else {
            None
        };

        let timestamp = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let payload = AgentTurnMetricPayload {
            harness: "buzz-agent".to_string(),
            model: None,
            channel_id: None,
            session_id: Some(session_id.to_string()),
            turn_id: Some(turn_id.to_string()),
            turn_seq: Some(turn_seq),
            timestamp,
            turn: turn_counts,
            cumulative: None,
            delta_reliable: true,
            stop_reason: Some(stop_reason),
        };

        let ciphertext =
            match buzz_core::agent_turn_metric::encrypt_agent_turn_metric(keys, owner_pk, &payload)
            {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!(
                        target: "buzz_agent::metrics",
                        session_id,
                        turn_id,
                        "NIP-AM: encrypt failed: {e}"
                    );
                    return;
                }
            };

        let agent_hex = keys.public_key().to_hex();
        let owner_hex = owner_pk.to_hex();
        let event = match EventBuilder::new(
            Kind::Custom(buzz_core::kind::KIND_AGENT_TURN_METRIC as u16),
            ciphertext,
        )
        .tags([
            Tag::parse(["p", &owner_hex]).expect("p tag"),
            Tag::parse(["agent", &agent_hex]).expect("agent tag"),
        ])
        .sign_with_keys(keys)
        {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(
                    target: "buzz_agent::metrics",
                    session_id,
                    turn_id,
                    "NIP-AM: sign failed: {e}"
                );
                return;
            }
        };

        let body_bytes = match serde_json::to_vec(&event) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(
                    target: "buzz_agent::metrics",
                    session_id,
                    "NIP-AM: serialize failed: {e}"
                );
                return;
            }
        };

        let url = format!("{base_url}/events");
        let auth_header = match nip98_auth(keys, "POST", &url, Some(&body_bytes)) {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!(
                    target: "buzz_agent::metrics",
                    session_id,
                    "NIP-AM: NIP-98 auth failed: {e}"
                );
                return;
            }
        };

        const METRIC_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);
        match tokio::time::timeout(
            METRIC_TIMEOUT,
            self.http
                .post(&url)
                .header("Authorization", auth_header)
                .header("Content-Type", "application/json")
                .body(body_bytes)
                .send(),
        )
        .await
        {
            Ok(Ok(resp)) if resp.status().is_success() => {}
            Ok(Ok(resp)) => tracing::warn!(
                target: "buzz_agent::metrics",
                session_id,
                turn_id,
                "NIP-AM: publish HTTP {}", resp.status()
            ),
            Ok(Err(e)) => tracing::warn!(
                target: "buzz_agent::metrics",
                session_id,
                turn_id,
                "NIP-AM: publish failed: {e}"
            ),
            Err(_) => tracing::warn!(
                target: "buzz_agent::metrics",
                session_id,
                turn_id,
                "NIP-AM: publish timed out"
            ),
        }
    }
}

/// Build a NIP-98 HTTP Auth `Authorization` header value: `Nostr <base64(event_json)>`.
fn nip98_auth(keys: &Keys, method: &str, url: &str, body: Option<&[u8]>) -> Result<String, String> {
    use base64::Engine;
    use nostr::{EventBuilder, Kind, Tag};
    use sha2::{Digest, Sha256};

    let u_tag = Tag::parse(["u", url]).map_err(|e| e.to_string())?;
    let method_tag = Tag::parse(["method", method]).map_err(|e| e.to_string())?;
    let nonce_tag =
        Tag::parse(["nonce", &uuid::Uuid::new_v4().to_string()]).map_err(|e| e.to_string())?;
    let mut tags = vec![u_tag, method_tag, nonce_tag];
    if let Some(b) = body {
        let hash = hex::encode(Sha256::digest(b));
        let payload_tag = Tag::parse(["payload", &hash]).map_err(|e| e.to_string())?;
        tags.push(payload_tag);
    }
    let event = EventBuilder::new(Kind::HttpAuth, "")
        .tags(tags)
        .sign_with_keys(keys)
        .map_err(|e| e.to_string())?;
    let json = serde_json::to_string(&event).map_err(|e| e.to_string())?;
    Ok(format!(
        "Nostr {}",
        base64::engine::general_purpose::STANDARD.encode(json)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// When all three env vars are absent, `from_env` yields a no-op publisher.
    #[test]
    fn test_metric_publisher_noop_when_env_absent() {
        // Remove the vars if set in the test environment to avoid interference.
        std::env::remove_var("BUZZ_PRIVATE_KEY");
        std::env::remove_var("BUZZ_RELAY_URL");
        std::env::remove_var("BUZZ_AGENT_OWNER_PUBKEY");
        let p = MetricPublisher::from_env();
        assert!(p.is_noop(), "publisher must be noop when vars are absent");
    }

    /// A well-formed `BUZZ_PRIVATE_KEY` + `BUZZ_RELAY_URL` + `BUZZ_AGENT_OWNER_PUBKEY`
    /// makes the publisher non-noop.
    #[test]
    fn test_metric_publisher_configured_when_all_vars_present() {
        let agent_keys = Keys::generate();
        let owner_keys = Keys::generate();
        std::env::set_var("BUZZ_PRIVATE_KEY", agent_keys.secret_key().to_secret_hex());
        std::env::set_var("BUZZ_RELAY_URL", "https://relay.example.com");
        std::env::set_var("BUZZ_AGENT_OWNER_PUBKEY", owner_keys.public_key().to_hex());
        let p = MetricPublisher::from_env();
        assert!(
            !p.is_noop(),
            "publisher must not be noop when all vars are set"
        );
        // Restore env to a clean state.
        std::env::remove_var("BUZZ_PRIVATE_KEY");
        std::env::remove_var("BUZZ_RELAY_URL");
        std::env::remove_var("BUZZ_AGENT_OWNER_PUBKEY");
    }
}
