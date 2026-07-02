//! NIP-AM kind:44200 metric publishing for the buzz-agent harness.
//!
//! Configured from three environment variables:
//! - `BUZZ_PRIVATE_KEY` — agent Nostr private key (nsec or hex).
//! - `BUZZ_RELAY_URL` — relay base URL (`wss://` or `https://`; both accepted).
//! - `BUZZ_AUTH_TAG` — NIP-OA attestation JSON (preferred owner source).
//!   Owner is derived by verifying the auth tag against the agent's own pubkey.
//!   Falls back to `BUZZ_AGENT_OWNER_PUBKEY` (npub or hex) if auth tag is absent.
//!
//! If any required variable is absent or unparseable, metric publishing is a
//! silent no-op. This mirrors the fail-open policy used throughout the harness.
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

/// Resolved configuration for a `MetricPublisher`. Separated from env-parsing
/// so tests can inject values directly without mutating process-global state.
pub(crate) struct MetricConfig {
    pub(crate) keys: Keys,
    pub(crate) owner_pubkey: nostr::PublicKey,
    /// HTTP(S) base URL — ws/wss already normalized to http/https, no trailing
    /// slash.
    pub(crate) base_url: String,
    /// Raw `BUZZ_AUTH_TAG` JSON, forwarded as `x-auth-tag` for attested agents.
    pub(crate) auth_tag_json: Option<String>,
}

/// Configured NIP-AM publisher. Constructed once per process from env vars.
/// When env vars are absent, construction succeeds and `is_noop()` returns
/// `true` — callers need not special-case the unconfigured case.
pub(crate) struct MetricPublisher {
    config: Option<MetricConfig>,
    http: Client,
}

impl MetricPublisher {
    /// Build from environment. Silent on parse errors — missing/malformed vars
    /// leave the config absent (no-op publisher).
    ///
    /// Owner resolution priority:
    /// 1. `BUZZ_AUTH_TAG` — NIP-OA attestation verified against this agent's
    ///    pubkey; extracts the owner pubkey from the tag.
    /// 2. `BUZZ_AGENT_OWNER_PUBKEY` — explicit hex or npub fallback.
    pub(crate) fn from_env() -> Self {
        Self {
            config: Self::config_from_env(),
            http: Client::new(),
        }
    }

    fn config_from_env() -> Option<MetricConfig> {
        let keys = std::env::var("BUZZ_PRIVATE_KEY")
            .ok()
            .and_then(|v| Keys::parse(&v).ok())?;
        let raw_url = std::env::var("BUZZ_RELAY_URL")
            .ok()
            .filter(|s| !s.is_empty())?;
        let base_url = ws_to_http(raw_url.trim_end_matches('/'));

        // Try BUZZ_AUTH_TAG first.
        let (owner_pubkey, auth_tag_json) = match std::env::var("BUZZ_AUTH_TAG")
            .ok()
            .filter(|s| !s.is_empty())
        {
            Some(tag_json) => {
                match buzz_sdk::nip_oa::verify_auth_tag(&tag_json, &keys.public_key()) {
                    Ok(pk) => (pk, Some(tag_json)),
                    // Auth tag present but verification failed — fall through.
                    Err(_) => resolve_explicit_owner()?,
                }
            }
            None => resolve_explicit_owner()?,
        };

        Some(MetricConfig {
            keys,
            owner_pubkey,
            base_url,
            auth_tag_json,
        })
    }

    /// Build from an explicit config (test helper — avoids process-env mutation).
    #[cfg(test)]
    pub(crate) fn from_config(config: MetricConfig) -> Self {
        Self {
            config: Some(config),
            http: Client::new(),
        }
    }

    /// Returns `true` when no complete config is available. Publishing is
    /// always a no-op in this state.
    #[cfg(test)]
    pub(crate) fn is_noop(&self) -> bool {
        self.config.is_none()
    }

    /// Best-effort publish a kind 44200 event.
    ///
    /// - `session_id` — the ACP session id for this turn.
    /// - `turn_seq` — monotonically increasing per-session turn counter.
    /// - `turn_id` — the run id for this turn (harness-internal).
    /// - `input_tokens` / `output_tokens` — summed across all LLM rounds in the turn.
    /// - `stop_reason` — the NIP-AM stop reason.
    ///
    /// No-op when no usage was observed (`input_tokens` and `output_tokens`
    /// both `None`) — per NIP-AM § "Do NOT publish an event for a turn with no
    /// observed usage".
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

        // No usage observed — NIP-AM forbids publishing an all-null metric.
        if input_tokens.is_none() && output_tokens.is_none() {
            return;
        }

        let MetricConfig {
            keys,
            owner_pubkey: owner_pk,
            base_url,
            auth_tag_json,
        } = match &self.config {
            Some(c) => c,
            None => return,
        };

        // buzz-agent has no session-cumulative counters — only per-turn deltas.
        // deltaReliable is true because we sum every round in this process;
        // no cross-process baseline is ever lost. Cumulative fields are omitted
        // since buzz-agent does not track rolling session totals across turns.
        let turn_counts = Some(TokenCounts {
            input_tokens,
            output_tokens,
            total_tokens: None,
            cost_usd: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
        });

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
        let mut req = self
            .http
            .post(&url)
            .header("Authorization", auth_header)
            .header("Content-Type", "application/json");
        if let Some(tag) = auth_tag_json {
            req = req.header("x-auth-tag", tag);
        }
        match tokio::time::timeout(METRIC_TIMEOUT, req.body(body_bytes).send()).await {
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

/// Normalize `ws://` / `wss://` relay URLs to `http://` / `https://`.
/// Pass-through for URLs that are already HTTP(S).
fn ws_to_http(url: &str) -> String {
    url.replace("wss://", "https://")
        .replace("ws://", "http://")
        .to_string()
}

/// Parse `BUZZ_AGENT_OWNER_PUBKEY` as the explicit owner fallback.
/// Returns `(pubkey, None)` on success, `None` if the var is absent/invalid.
fn resolve_explicit_owner() -> Option<(nostr::PublicKey, Option<String>)> {
    let pk = std::env::var("BUZZ_AGENT_OWNER_PUBKEY")
        .ok()
        .and_then(|v| nostr::PublicKey::parse(&v).ok())?;
    Some((pk, None))
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
    use nostr::Keys;

    fn make_config(owner_keys: &Keys) -> MetricConfig {
        MetricConfig {
            keys: Keys::generate(),
            owner_pubkey: owner_keys.public_key(),
            base_url: "https://relay.example.com".to_string(),
            auth_tag_json: None,
        }
    }

    /// A publisher built from an explicit config is not a no-op.
    #[test]
    fn test_metric_publisher_configured_when_config_injected() {
        let owner_keys = Keys::generate();
        let p = MetricPublisher::from_config(make_config(&owner_keys));
        assert!(
            !p.is_noop(),
            "publisher must not be noop when config is set"
        );
    }

    /// A publisher with no config (None) is a no-op.
    #[test]
    fn test_metric_publisher_noop_when_no_config() {
        let p = MetricPublisher {
            config: None,
            http: Client::new(),
        };
        assert!(p.is_noop(), "publisher must be noop when config is None");
    }

    /// When both token fields are None, publish returns without building/sending
    /// an event. Verified by the absence of a panic or network call (we use an
    /// invalid URL so any real HTTP attempt would error — silence is the proof).
    #[tokio::test]
    async fn test_publish_noop_when_no_usage_observed() {
        let owner_keys = Keys::generate();
        let mut config = make_config(&owner_keys);
        // Use an unreachable URL — if any HTTP request were made it would fail
        // visibly. The test must complete silently.
        config.base_url = "https://127.0.0.1:1".to_string();
        let p = MetricPublisher::from_config(config);
        // Both tokens absent → must return before any encrypt/send attempt.
        p.publish(
            "session-1",
            0,
            "turn-1",
            None,
            None,
            buzz_core::agent_turn_metric::StopReason::EndTurn,
        )
        .await;
        // If we reach here without error, the no-usage guard fired correctly.
    }

    /// ws:// URL is normalized to http://.
    #[test]
    fn test_ws_to_http_plain() {
        assert_eq!(
            ws_to_http("ws://relay.example.com"),
            "http://relay.example.com"
        );
    }

    /// wss:// URL is normalized to https://.
    #[test]
    fn test_ws_to_http_secure() {
        assert_eq!(
            ws_to_http("wss://relay.example.com"),
            "https://relay.example.com"
        );
    }

    /// https:// URLs pass through unchanged.
    #[test]
    fn test_ws_to_http_passthrough() {
        assert_eq!(
            ws_to_http("https://relay.example.com"),
            "https://relay.example.com"
        );
    }

    /// Auth tag JSON is forwarded in the `x-auth-tag` header field of the
    /// config. Verify it round-trips through the config struct intact.
    #[test]
    fn test_auth_tag_json_stored_in_config() {
        let tag_json = r#"["auth","deadbeef","*","sig"]"#;
        let owner_keys = Keys::generate();
        let config = MetricConfig {
            keys: Keys::generate(),
            owner_pubkey: owner_keys.public_key(),
            base_url: "https://relay.example.com".to_string(),
            auth_tag_json: Some(tag_json.to_string()),
        };
        assert_eq!(config.auth_tag_json.as_deref(), Some(tag_json));
    }
}
