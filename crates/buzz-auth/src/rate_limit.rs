//! Rate limiting types and interface.
//!
//! Defines the [`RateLimiter`] trait. The Redis-backed implementation lives in
//! `buzz-relay` / `buzz-pubsub`. Fixed-window counter algorithm.
//!
//! ⚠️ Fixed windows allow up to 2× burst at boundaries. Upgrade to sliding
//! window or token bucket for strict limiting.

use std::net::IpAddr;

use buzz_core::TenantContext;
use nostr::PublicKey;
use serde::{Deserialize, Serialize};

use crate::error::AuthError;

/// The outcome of a rate-limit check, including counter state for response headers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RateLimitResult {
    /// Whether the request is permitted (`true`) or should be rejected (`false`).
    pub allowed: bool,
    /// Current counter value after this increment.
    pub current: u64,
    /// The configured limit for this window.
    pub limit: u64,
    /// Seconds until the current window resets.
    pub reset_in_secs: u64,
}

impl RateLimitResult {
    /// Construct an **allowed** result.
    pub fn allowed(current: u64, limit: u64, reset_in_secs: u64) -> Self {
        Self {
            allowed: true,
            current,
            limit,
            reset_in_secs,
        }
    }

    /// Construct a **denied** result.
    pub fn denied(current: u64, limit: u64, reset_in_secs: u64) -> Self {
        Self {
            allowed: false,
            current,
            limit,
            reset_in_secs,
        }
    }
}

/// The category of operation being rate-limited.
///
/// Each variant maps to a distinct Redis key suffix so limits are tracked
/// independently per operation type.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LimitType {
    /// Nostr message events (kind:1 etc.) sent via WebSocket.
    Messages,
    /// HTTP REST API calls.
    ApiCalls,
    /// Relay-proxied GIF metadata searches.
    GifSearches,
    /// All WebSocket events (broader than `Messages`).
    WsEvents,
    /// Concurrent WebSocket connections from a single IP address.
    IpConnections,
}

impl LimitType {
    /// Short suffix used in Redis key construction (e.g. `"msg"`, `"api"`).
    pub fn key_suffix(&self) -> &'static str {
        match self {
            Self::Messages => "msg",
            Self::ApiCalls => "api",
            Self::GifSearches => "gif",
            Self::WsEvents => "ws",
            Self::IpConnections => "conn",
        }
    }
}

/// Per-tier rate limit thresholds.
///
/// All values are counts per the relevant time window (per-minute or per-second).
/// Loaded from the relay config file; sensible defaults are provided for all fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitConfig {
    /// Maximum messages per minute for human users. Default: 60.
    #[serde(default = "default_human_msg")]
    pub human_messages_per_min: u64,
    /// Maximum relay-proxied GIF searches per minute for each pubkey.
    /// Default: 30.
    #[serde(default = "default_gif_searches")]
    pub gif_searches_per_min: u64,
    /// Maximum HTTP API calls per minute for human users. Default: 300.
    #[serde(default = "default_human_api")]
    pub human_api_calls_per_min: u64,
    /// Maximum WebSocket events per second for human users. Default: 10.
    #[serde(default = "default_human_ws")]
    pub human_ws_events_per_sec: u64,
    /// Non-ephemeral events per minute that one owner's agents may publish
    /// into one channel, shared by every agent key that owner has. Events
    /// naming no channel share one more such budget. Default: 120.
    #[serde(
        default = "default_agent_channel_events",
        alias = "agent_standard_messages_per_min"
    )]
    pub agent_channel_events_per_min: u64,
    /// Non-ephemeral events per minute that one owner's agents may publish
    /// across every channel, shared by every agent key that owner has.
    /// Default: 600.
    #[serde(default = "default_agent_owner_events")]
    pub agent_owner_events_per_min: u64,
    /// Maximum HTTP API calls per minute for standard-tier agent tokens. Default: 600.
    #[serde(default = "default_agent_std_api")]
    pub agent_standard_api_calls_per_min: u64,
    /// Maximum messages per minute for elevated-tier agent tokens. Default: 300.
    #[serde(default = "default_agent_elev_msg")]
    pub agent_elevated_messages_per_min: u64,
    /// Maximum messages per minute for platform-tier agent tokens. Default: 600.
    #[serde(default = "default_agent_plat_msg")]
    pub agent_platform_messages_per_min: u64,
}

fn default_human_msg() -> u64 {
    60
}
fn default_gif_searches() -> u64 {
    30
}
fn default_human_api() -> u64 {
    300
}
fn default_human_ws() -> u64 {
    10
}
fn default_agent_channel_events() -> u64 {
    120
}
fn default_agent_owner_events() -> u64 {
    600
}
fn default_agent_std_api() -> u64 {
    600
}
fn default_agent_elev_msg() -> u64 {
    300
}
fn default_agent_plat_msg() -> u64 {
    600
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            human_messages_per_min: default_human_msg(),
            gif_searches_per_min: default_gif_searches(),
            human_api_calls_per_min: default_human_api(),
            human_ws_events_per_sec: default_human_ws(),
            agent_channel_events_per_min: default_agent_channel_events(),
            agent_owner_events_per_min: default_agent_owner_events(),
            agent_standard_api_calls_per_min: default_agent_std_api(),
            agent_elevated_messages_per_min: default_agent_elev_msg(),
            agent_platform_messages_per_min: default_agent_plat_msg(),
        }
    }
}

/// The two fixed-window budgets an agent's event is admitted against.
///
/// Both are keyed by the agent's owner, not the agent key, so every agent key
/// an owner has draws on the same budgets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentEventBudget {
    /// Window length in seconds, shared by both budgets.
    pub window_secs: u64,
    /// Events per window for one owner in one channel.
    pub channel_limit: u64,
    /// Events per window for one owner across all channels.
    pub owner_limit: u64,
}

/// The agent event budget that refused an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentBudgetScope {
    /// The owner's budget in the event's channel.
    Channel,
    /// The owner's budget across all channels.
    Owner,
}

/// Outcome of [`RateLimiter::check_agent_event`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentEventVerdict {
    /// Both budgets had room; both counters advanced.
    Admitted,
    /// A budget was full; neither counter advanced.
    Exceeded {
        /// The budget that refused the event. When both are full this is
        /// [`AgentBudgetScope::Owner`].
        scope: AgentBudgetScope,
        /// Seconds until every full budget's window resets.
        reset_in_secs: u64,
    },
}

/// Async rate-limiting interface.
///
/// The Redis-backed production implementation lives in `buzz-relay` / `buzz-pubsub`.
/// A no-op `AlwaysAllowRateLimiter` is provided for unit tests.
///
/// ## Tenant scoping
///
/// Pubkey-keyed limits ([`check_and_increment`]) take `&TenantContext` and the Redis
/// key is community-prefixed (`buzz:{community}:ratelimit:{pubkey}:{suffix}`). The
/// same pubkey active in two communities consumes two independent quotas — that is
/// the correct behavior under multi-tenant isolation (S1 cross-community fence).
///
/// Agent event budgets ([`check_agent_event`]) are keyed by the agent's owner,
/// also community-prefixed. Every agent key an owner has in a community draws
/// on the same budgets, so minting more agent keys does not add capacity.
///
/// IP-keyed limits ([`check_ip_connection`]) are **operator-global** by design. They
/// gate connection acceptance at the network edge, before host→community resolution
/// has completed (or, on resolve failure, instead of it). Threading `&TenantContext`
/// through the connection-rate fence would invert the order of operations. If
/// per-(community, IP) caps are ever needed as a tenant-fairness signal, that
/// belongs in an additive `LimitType` keyed on `(community, ip)`, not in this trait.
///
/// ⚠️ The fixed-window algorithm used by the Redis implementation allows up to 2×
/// burst at window boundaries. Upgrade to a sliding window or token bucket if strict
/// per-second limiting is required.
pub trait RateLimiter: Send + Sync {
    /// Increment the per-(community, pubkey) counter for `limit_type` and return
    /// whether the request is within `limit` for the given `window_secs`.
    ///
    /// `ctx` scopes the counter to the resolved community; the same pubkey in two
    /// communities is two independent quotas.
    fn check_and_increment(
        &self,
        ctx: &TenantContext,
        pubkey: &PublicKey,
        limit_type: LimitType,
        window_secs: u64,
        limit: u64,
    ) -> impl std::future::Future<Output = Result<RateLimitResult, AuthError>> + Send;

    /// Admit one agent event against its owner's budget in `channel` and its
    /// owner's budget across all channels, both in `ctx`'s community.
    ///
    /// The check is atomic across relay instances. Both counters advance
    /// together or not at all: a refused event consumes neither, so a channel
    /// that is out of budget cannot spend the owner's budget for its other
    /// channels. `channel` is `None` for an event that names no channel.
    fn check_agent_event(
        &self,
        ctx: &TenantContext,
        owner: &PublicKey,
        channel: Option<uuid::Uuid>,
        budget: AgentEventBudget,
    ) -> impl std::future::Future<Output = Result<AgentEventVerdict, AuthError>> + Send;

    /// Increment the per-IP connection counter and return whether the connection
    /// is within `limit` for the given `window_secs`.
    ///
    /// Operator-global — see trait docs. This fence runs before / outside of host
    /// resolution and intentionally does not take a `TenantContext`.
    fn check_ip_connection(
        &self,
        ip: &IpAddr,
        window_secs: u64,
        limit: u64,
    ) -> impl std::future::Future<Output = Result<RateLimitResult, AuthError>> + Send;
}

/// Redis key for pubkey-based rate limit:
/// `buzz:{community}:ratelimit:{pubkey_hex}:{suffix}`.
///
/// Community-prefixed: the same pubkey in two communities maps to two distinct
/// keys, so quotas don't bleed across the tenancy fence.
pub fn rate_limit_key(ctx: &TenantContext, pubkey: &PublicKey, limit_type: &LimitType) -> String {
    format!(
        "buzz:{}:ratelimit:{}:{}",
        ctx.community(),
        pubkey.to_hex(),
        limit_type.key_suffix()
    )
}

/// Redis key for an owner's agent events in one channel:
/// `buzz:{community}:ratelimit:agent-owner:{{owner_hex}}:channel:{channel}`,
/// where `{channel}` is `none` for events that name no channel.
///
/// The braces around the owner are a Redis Cluster hash tag. They put this key
/// and [`agent_owner_budget_key`] in one slot, as the two-key admission
/// script requires.
pub fn agent_channel_budget_key(
    ctx: &TenantContext,
    owner: &PublicKey,
    channel: Option<uuid::Uuid>,
) -> String {
    let channel = channel.map_or_else(|| "none".to_owned(), |channel| channel.to_string());
    format!(
        "buzz:{}:ratelimit:agent-owner:{{{}}}:channel:{channel}",
        ctx.community(),
        owner.to_hex()
    )
}

/// Redis key for an owner's agent events across all channels:
/// `buzz:{community}:ratelimit:agent-owner:{{owner_hex}}:all`.
pub fn agent_owner_budget_key(ctx: &TenantContext, owner: &PublicKey) -> String {
    format!(
        "buzz:{}:ratelimit:agent-owner:{{{}}}:all",
        ctx.community(),
        owner.to_hex()
    )
}

/// Redis key for IP-based rate limit: `buzz:ratelimit:ip:{ip}:conn`.
///
/// Operator-global by design — see [`RateLimiter`] docs.
pub fn ip_rate_limit_key(ip: &IpAddr) -> String {
    format!("buzz:ratelimit:ip:{}:conn", ip)
}

/// Always-allow rate limiter for unit tests.
#[cfg(any(test, feature = "test-utils"))]
pub struct AlwaysAllowRateLimiter;

#[cfg(any(test, feature = "test-utils"))]
impl RateLimiter for AlwaysAllowRateLimiter {
    async fn check_and_increment(
        &self,
        _ctx: &TenantContext,
        _pubkey: &PublicKey,
        _limit_type: LimitType,
        window_secs: u64,
        limit: u64,
    ) -> Result<RateLimitResult, AuthError> {
        Ok(RateLimitResult::allowed(1, limit, window_secs))
    }

    async fn check_agent_event(
        &self,
        _ctx: &TenantContext,
        _owner: &PublicKey,
        _channel: Option<uuid::Uuid>,
        _budget: AgentEventBudget,
    ) -> Result<AgentEventVerdict, AuthError> {
        Ok(AgentEventVerdict::Admitted)
    }

    async fn check_ip_connection(
        &self,
        _ip: &IpAddr,
        window_secs: u64,
        limit: u64,
    ) -> Result<RateLimitResult, AuthError> {
        Ok(RateLimitResult::allowed(1, limit, window_secs))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_core::CommunityId;
    use nostr::Keys;
    use sha2::Digest;
    use std::net::Ipv4Addr;
    use uuid::Uuid;

    fn fixture_ctx(host: &str) -> TenantContext {
        // Deterministic community id from host so test assertions can name the prefix.
        let bytes = sha2::Sha256::digest(host.as_bytes());
        let mut uuid_bytes = [0u8; 16];
        uuid_bytes.copy_from_slice(&bytes[..16]);
        let id = CommunityId::from_uuid(Uuid::from_bytes(uuid_bytes));
        TenantContext::resolved(id, host)
    }

    #[test]
    fn rate_limit_key_includes_community_prefix() {
        let ctx = fixture_ctx("relay-a.example");
        let keys = Keys::generate();
        let key = rate_limit_key(&ctx, &keys.public_key(), &LimitType::Messages);
        let expected_prefix = format!("buzz:{}:ratelimit:", ctx.community());
        assert!(
            key.starts_with(&expected_prefix),
            "key {key} should start with {expected_prefix}"
        );
        assert!(key.ends_with(":msg"));
    }

    #[test]
    fn gif_searches_have_an_independent_quota_key() {
        let ctx = fixture_ctx("relay-a.example");
        let keys = Keys::generate();
        let gif_key = rate_limit_key(&ctx, &keys.public_key(), &LimitType::GifSearches);
        let api_key = rate_limit_key(&ctx, &keys.public_key(), &LimitType::ApiCalls);

        assert!(gif_key.ends_with(":gif"));
        assert_ne!(gif_key, api_key);
    }

    #[test]
    fn rate_limit_key_isolates_communities_for_same_pubkey() {
        // The S1 cross-community isolation fence at the rate-limit key layer:
        // same pubkey, two communities -> two distinct Redis keys -> independent quotas.
        let keys = Keys::generate();
        let ctx_a = fixture_ctx("relay-a.example");
        let ctx_b = fixture_ctx("relay-b.example");
        let key_a = rate_limit_key(&ctx_a, &keys.public_key(), &LimitType::Messages);
        let key_b = rate_limit_key(&ctx_b, &keys.public_key(), &LimitType::Messages);
        assert_ne!(
            key_a, key_b,
            "same pubkey in two communities must not share a rate-limit key"
        );
    }

    #[test]
    fn rate_limit_key_components_are_lowercase() {
        // Stability/idempotence invariant: if pubkey hex or community Display
        // ever started emitting uppercase, the same (community, pubkey) would
        // produce two distinct Redis keys → effective 2× quota. Pin the
        // lowercase property here so the regression surfaces in unit tests,
        // not in production traffic.
        let ctx = fixture_ctx("relay-a.example");
        let keys = Keys::generate();
        let key = rate_limit_key(&ctx, &keys.public_key(), &LimitType::Messages);
        for c in key.chars() {
            assert!(
                !c.is_ascii_uppercase(),
                "rate-limit key {key} must be all-lowercase ASCII"
            );
        }
    }

    #[test]
    fn agent_budget_keys_are_scoped_by_community_owner_and_channel() {
        let owner = Keys::generate().public_key();
        let ctx_a = fixture_ctx("relay-a.example");
        let ctx_b = fixture_ctx("relay-b.example");
        let (one, two) = (Uuid::new_v4(), Uuid::new_v4());

        let channel_one = agent_channel_budget_key(&ctx_a, &owner, Some(one));
        assert_eq!(
            channel_one,
            format!(
                "buzz:{}:ratelimit:agent-owner:{{{}}}:channel:{one}",
                ctx_a.community(),
                owner.to_hex()
            )
        );
        assert_eq!(
            agent_owner_budget_key(&ctx_a, &owner),
            format!(
                "buzz:{}:ratelimit:agent-owner:{{{}}}:all",
                ctx_a.community(),
                owner.to_hex()
            )
        );
        let distinct = [
            channel_one,
            agent_channel_budget_key(&ctx_a, &owner, Some(two)),
            agent_channel_budget_key(&ctx_a, &owner, None),
            agent_owner_budget_key(&ctx_a, &owner),
            agent_channel_budget_key(&ctx_b, &owner, Some(one)),
            agent_owner_budget_key(&ctx_b, &owner),
            agent_owner_budget_key(&ctx_a, &Keys::generate().public_key()),
        ];
        let unique: std::collections::HashSet<_> = distinct.iter().collect();
        assert_eq!(unique.len(), distinct.len(), "{distinct:#?}");
    }

    #[test]
    fn agent_budget_keys_share_one_cluster_hash_tag() {
        // The admission script touches both keys, which Redis Cluster allows
        // only when they hash to one slot.
        fn hash_tag(key: &str) -> &str {
            let open = key.find('{').expect("hash tag opens");
            let close = key[open..].find('}').expect("hash tag closes") + open;
            &key[open + 1..close]
        }
        let ctx = fixture_ctx("relay-a.example");
        let owner = Keys::generate().public_key();
        let channel = agent_channel_budget_key(&ctx, &owner, Some(Uuid::new_v4()));
        let all = agent_owner_budget_key(&ctx, &owner);
        assert_eq!(hash_tag(&channel), owner.to_hex());
        assert_eq!(hash_tag(&all), owner.to_hex());
    }

    #[test]
    fn agent_budget_defaults_and_legacy_config_name() {
        let defaults = RateLimitConfig::default();
        assert_eq!(defaults.agent_channel_events_per_min, 120);
        assert_eq!(defaults.agent_owner_events_per_min, 600);
        // A config written for the per-agent-key budget keeps meaning one
        // agent's allowance in a channel.
        let legacy: RateLimitConfig =
            serde_json::from_str(r#"{"agent_standard_messages_per_min": 90}"#).unwrap();
        assert_eq!(legacy.agent_channel_events_per_min, 90);
        assert_eq!(legacy.agent_owner_events_per_min, 600);
    }

    #[test]
    fn ip_rate_limit_key_format() {
        // IP fence stays operator-global — no community in the key.
        let ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1));
        assert_eq!(ip_rate_limit_key(&ip), "buzz:ratelimit:ip:192.168.1.1:conn");
    }

    #[tokio::test]
    async fn always_allow_limiter() {
        let limiter = AlwaysAllowRateLimiter;
        let ctx = fixture_ctx("relay-a.example");
        let keys = Keys::generate();
        let result = limiter
            .check_and_increment(&ctx, &keys.public_key(), LimitType::Messages, 60, 60)
            .await
            .unwrap();
        assert!(result.allowed);
    }
}
