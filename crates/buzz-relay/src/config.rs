//! Relay configuration from environment variables.

use std::net::SocketAddr;
use std::time::Duration;

use sha2::{Digest, Sha256};
use thiserror::Error;
use tracing::warn;

/// Default maximum inbound WebSocket frame size in bytes.
///
/// Must comfortably exceed accepted event content sizes after Nostr JSON and
/// NIP-44 encryption overhead.
pub const DEFAULT_MAX_FRAME_BYTES: usize = 512 * 1024;

/// Errors that can occur while loading relay configuration.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// The `BUZZ_BIND_ADDR` environment variable could not be parsed as a socket address.
    #[error("invalid BUZZ_BIND_ADDR: {0}")]
    InvalidBindAddr(String),
    /// A configuration value failed validation.
    #[error("invalid config: {0}")]
    InvalidValue(String),
}

/// Authentication mode for the deployment-admin API.
///
/// Configured by `BUZZ_ADMIN_AUTH`. Exactly one variant is active; invalid
/// combinations (e.g. `BUZZ_ADMIN_TOKEN` set in `Nip98` mode) are startup
/// errors — they are not representable.
///
/// # Role resolution (nip98 mode only)
///
/// In `nip98` mode the authenticated pubkey is resolved to an
/// `AdminPrincipal` at request time via [`crate::api::admin::auth::resolve_admin_principal`]:
/// - `Operator/Config` if pubkey ∈ `RELAY_OPERATOR_PUBKEYS`
/// - `Operator/OwnerFallback` if pubkey == `RELAY_OWNER_PUBKEY` **and**
///   `RELAY_OPERATOR_PUBKEYS` is empty (evaluated from config, never runtime rows)
/// - `Moderator/Db` from the `relay_operators` table otherwise
/// - `None` → 403 (no fall-through role, ever)
///
/// `token`/`disabled` modes retain exactly the existing read surface; they
/// expose no role, no capabilities, and no mutation endpoints.
#[derive(Debug, Clone)]
pub enum AdminAuth {
    /// Operator bearer credential required on every request.
    /// Selected by `BUZZ_ADMIN_AUTH=token` (or leaving `BUZZ_ADMIN_AUTH` unset).
    /// Read-only surface only; mutations require `nip98` mode.
    Token(AdminToken),
    /// Bearer authentication disabled. The operator has explicitly asserted
    /// that the admin API is protected at the network layer (reverse proxy,
    /// VPN, firewall). `Host`/`Origin` checks remain active as defense-in-depth.
    /// Selected by `BUZZ_ADMIN_AUTH=disabled`.
    /// Read-only surface only; mutations require `nip98` mode.
    Disabled,
    /// NIP-98 HTTP Auth. Every request must carry an `Authorization: Nostr`
    /// header containing a signed kind-27235 event. The authenticated pubkey
    /// is resolved to an [`crate::api::admin::auth::AdminPrincipal`] at request
    /// time from config + DB. Selected by `BUZZ_ADMIN_AUTH=nip98`.
    /// Only mode that exposes mutation and staffing endpoints.
    Nip98,
}

/// Deny-by-default read-only deployment-admin configuration.
#[derive(Debug, Clone)]
pub struct AdminConfig {
    /// Exact admin HTTP authority.
    pub host: String,
    /// Authentication mode selected at startup.
    pub auth: AdminAuth,
    /// Optional admin SPA bundle directory.
    pub web_dir: Option<std::path::PathBuf>,
}

/// Operator bearer credential for the deployment-admin API, held decoded.
///
/// `Debug` is redacted by hand so the secret cannot escape through a derived
/// `Debug` on [`AdminConfig`] or [`Config`].
#[derive(Clone)]
pub struct AdminToken([u8; 32]);

impl AdminToken {
    /// Constant-time comparison against a presented credential.
    pub fn matches(&self, presented: &[u8; 32]) -> bool {
        use subtle::ConstantTimeEq;
        self.0.ct_eq(presented).into()
    }

    /// Build a token from raw bytes. Test-only: production tokens come from
    /// [`parse_admin_token`] so the operator-facing validation is never bypassed.
    #[cfg(test)]
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

impl std::fmt::Debug for AdminToken {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("AdminToken(redacted)")
    }
}

/// Guidance repeated in every `BUZZ_ADMIN_TOKEN` rejection.
const ADMIN_TOKEN_GUIDANCE: &str = "BUZZ_ADMIN_TOKEN must be 64 hexadecimal characters \
     (surrounding whitespace is trimmed); generate one with `openssl rand -hex 32`";

/// Read and validate the admin bearer credential. Fail-closed: the admin API is
/// only ever mounted alongside a token that passed this check.
fn parse_admin_token() -> Result<AdminToken, ConfigError> {
    let raw = match std::env::var("BUZZ_ADMIN_TOKEN") {
        Ok(value) => value,
        Err(std::env::VarError::NotPresent) => {
            return Err(ConfigError::InvalidValue(format!(
                "BUZZ_ADMIN_TOKEN is required whenever BUZZ_ADMIN_HOST is set — \
                 {ADMIN_TOKEN_GUIDANCE}"
            )))
        }
        Err(std::env::VarError::NotUnicode(_)) => {
            return Err(ConfigError::InvalidValue(ADMIN_TOKEN_GUIDANCE.to_string()))
        }
    };
    let mut token = [0u8; 32];
    hex::decode_to_slice(raw.trim(), &mut token)
        .map_err(|_| ConfigError::InvalidValue(ADMIN_TOKEN_GUIDANCE.to_string()))?;
    Ok(AdminToken(token))
}

/// Relay-hosted policy content presented on join surfaces.
#[derive(Debug, Clone)]
pub struct JoinPolicyConfig {
    /// Operator-provided Terms of Service document in Markdown.
    pub terms_markdown: Option<String>,
    /// Operator-provided Privacy Policy document in Markdown.
    pub privacy_markdown: Option<String>,
    /// Whether join surfaces must collect an 18+ attestation.
    pub age_attestation_required: bool,
    /// Content-derived identifier binding receipts to the exact policy revision.
    pub version: String,
}

/// Maximum configured jitter, leaving ten seconds of the hard-drain budget for
/// WebSocket close-frame delivery after the final delayed cancellation.
pub const MAX_DRAIN_JITTER_MS: u64 = 20_000;

/// Relay runtime configuration, loaded from environment variables.
#[derive(Debug, Clone)]
pub struct Config {
    /// Address the relay HTTP/WebSocket server binds to.
    pub bind_addr: SocketAddr,
    /// Postgres database connection URL.
    pub database_url: String,
    /// Optional read-replica connection URL (e.g. an Aurora `cluster-ro-`
    /// endpoint). Unset means all reads stay on the writer.
    pub read_database_url: Option<String>,
    /// Replica read budget `B` in milliseconds (`BUZZ_REPLICA_READ_MAX_AGE_MS`).
    /// `0` (the default) disables bounded-staleness replica routing; see
    /// [`buzz_db::DbConfig::replica_read_max_age_ms`].
    pub replica_read_max_age_ms: u64,

    /// Upper bound, in milliseconds, of the per-connection random delay applied
    /// when sending the `1012 Service Restart` close frame during graceful
    /// shutdown (`BUZZ_DRAIN_JITTER_MS`). Each live connection is closed after
    /// an independent delay drawn uniformly from `[1, drain_jitter_ms]` when
    /// jitter is enabled, which
    /// spreads client reconnects across the window instead of releasing the
    /// whole pod's sockets in one instant (the reconnect thundering herd that
    /// drives DB pool-timeout bursts on rolling deploys).
    ///
    /// Default `0` reproduces the previous all-at-once close. Values above
    /// [`MAX_DRAIN_JITTER_MS`] are capped, leaving headroom under the relay's
    /// 30-second hard-drain timeout for close-frame delivery.
    pub drain_jitter_ms: u64,
    /// Redis connection URL used by the pub/sub manager.
    pub redis_url: String,
    /// Maximum connections in the shared Redis pool. Defaults to 16.
    ///
    /// deadpool's own default is `CPU_COUNT * 2`, which on a 2-vCPU relay
    /// pod is only 4 — small enough that rate-limit checks, presence, and
    /// pub/sub publishes queue behind each other under load.
    pub redis_pool_size: usize,
    /// Maximum connections in the Postgres writer/reader pools. Defaults to 50.
    ///
    /// The `buzz-db` default of 20 was sized for a handful of pods against
    /// `max_connections=100`. Against Aurora (~5,000 connections) that cap
    /// is the binding constraint: a burst of concurrent handlers exhausts
    /// the per-pod pool and requests fail on acquire timeout while the
    /// database sits idle.
    pub db_pool_size: u32,
    /// Maximum connections in the Postgres read-replica pool
    /// (`BUZZ_DB_READ_POOL_SIZE`). Defaults to `db_pool_size`. Sized
    /// independently so reader capacity can be tuned against the replica's
    /// headroom without touching the writer pool.
    pub db_read_pool_size: Option<u32>,
    /// Public WebSocket URL of this relay, advertised in NIP-11.
    pub relay_url: String,
    /// Public WebSocket URL of the dedicated device-pairing relay, when configured.
    pub pairing_relay_url: Option<String>,
    /// Maximum number of concurrent WebSocket connections.
    pub max_connections: usize,
    /// Maximum number of concurrently executing message handlers.
    pub max_concurrent_handlers: usize,
    /// Per-connection outbound message buffer size (number of messages).
    pub send_buffer_size: usize,
    /// Maximum inbound WebSocket frame size in bytes.
    pub max_frame_bytes: usize,
    /// Number of consecutive buffer-full events tolerated before cancelling a slow client.
    pub slow_client_grace_limit: u8,
    /// Authentication provider configuration.
    pub auth: buzz_auth::AuthConfig,
    /// Whether REST API requests must present a valid token. Independent of
    /// WebSocket protocol auth, which is *always* required by REQ/EVENT/COUNT.
    pub require_auth_token: bool,
    /// Comma-separated list of allowed CORS origins.
    /// If empty, permissive CORS is used (dev mode).
    /// Example: "tauri://localhost,http://localhost:3000"
    pub cors_origins: Vec<String>,
    /// Optional hex-encoded private key for the relay's signing keypair.
    /// If absent, a fresh keypair is generated at startup.
    pub relay_private_key: Option<String>,
    /// Optional Unix Domain Socket path. When set, the relay also listens on this
    /// UDS for traffic (e.g. service mesh sidecar). Health probes still use TCP.
    pub uds_path: Option<String>,
    /// TCP port for the health-only router (`/_liveness`, `/_readiness`, `/_status`).
    /// Separate from the app router so K8s probes bypass Istio and auth middleware.
    pub health_port: u16,
    /// TCP port for the Prometheus metrics exporter (`GET /metrics`).
    pub metrics_port: u16,

    /// When true, NIP-42 pubkey-only authentication (no API token) is
    /// restricted to pubkeys in the `pubkey_allowlist` table. Users with valid
    /// API tokens bypass the allowlist entirely.
    /// Applies to all NIP-42 pubkey-only connections, regardless of `require_auth_token`.
    pub pubkey_allowlist_enabled: bool,

    /// When true, every authenticated request must also pass a relay-level
    /// membership check against the `relay_members` table.
    /// When false (default), the check is a no-op and all authenticated callers
    /// are permitted regardless of auth method (API token, NIP-42).
    pub require_relay_membership: bool,

    /// Whether this deployment can serve huddle (voice) audio.
    ///
    /// Huddle audio frames are relayed peer-to-peer *within a single pod*
    /// (`AudioRoomManager` is an in-process map; only huddle lifecycle events
    /// cross pods via Redis). Under horizontal scaling (any-pod-any-connection,
    /// plan §4 fork B) two peers in the same huddle can land on different pods
    /// and never hear each other. Rather than sticky-route huddles or ship a
    /// silent split-room (plan §5b, decided by Tyler), a horizontally-scaled
    /// deployment sets this `false` and the relay surfaces a clear, client-
    /// handleable "huddle audio unavailable" signal on join.
    ///
    /// Defaults to `true` so single-pod deployments (the N=1 case) keep today's
    /// behavior unchanged. Operators running multiple relay pods MUST set
    /// `BUZZ_HUDDLE_AUDIO_AVAILABLE=false` until the out-of-relay media/SFU
    /// service lands.
    pub huddle_audio_available: bool,

    /// Inter-relay mesh configuration (`BUZZ_MESH`, `BUZZ_MESH_BIND_ADDR`).
    /// Opt-in: mesh forms only when `BUZZ_MESH=on` is explicit. The default
    /// (absent/off) is exact single-instance behavior — no bind, no Redis
    /// registry write — so an image upgrade with untouched env is a strict
    /// no-regression rollout.
    pub mesh: buzz_relay_mesh::MeshConfig,

    /// Testbed-only reliable-stream echo consumer (`BUZZ_MESH_DEMO_ECHO`).
    /// When `on`, the owner side of an inbound reliable mesh stream echoes
    /// every validated `Data` frame back to the sender — a transport/
    /// session-routing smoke for cross-pod evidence runs, NOT a product flow.
    /// Same strict opt-in as `BUZZ_MESH`; default off means inbound reliable
    /// streams are accepted, logged, and closed (no session consumer yet).
    pub mesh_demo_echo: bool,

    /// Optional hex-encoded pubkey of the relay owner.
    /// When set, this pubkey is automatically bootstrapped into `relay_members`
    /// with the `owner` role on first startup.
    pub relay_owner_pubkey: Option<String>,

    /// Canonical HTTP origin of the deployment-global operator API.
    ///
    /// Every operator NIP-98 `u` tag is verified against this origin, independent
    /// of the inbound HTTP `Host` header and tenant registry. Required when
    /// `RELAY_OPERATOR_PUBKEYS` is non-empty. Set via `RELAY_OPERATOR_API_ORIGIN`
    /// as an `http://` or `https://` origin with no path, query, or fragment.
    pub relay_operator_api_origin: Option<String>,

    /// Deployment-level relay operator pubkeys allowed to use the
    /// `/operator/communities` management endpoints.
    ///
    /// Unlike `relay_owner_pubkey` (a role *within* the deployment community),
    /// operators span tenants: they may create new communities and bootstrap
    /// initial owners, but hold no implicit tenant membership row.
    /// Empty (the default) disables community provisioning entirely — fail closed.
    ///
    /// Set via `RELAY_OPERATOR_PUBKEYS` as a comma-separated list of 64-char
    /// hex pubkeys. Invalid entries are rejected at startup (config error), not
    /// skipped — a typo must not silently disable an operator.
    pub relay_operator_pubkeys: Vec<String>,

    /// Allow NIP-OA owner attestation for relay membership.
    ///
    /// When `true` and `require_relay_membership` is also `true`, agents
    /// bearing a valid NIP-OA `auth` tag can authenticate by proving their
    /// owner is a relay member. The agent gets session-scoped access.
    ///
    /// On open relays (`require_relay_membership = false`), NIP-OA owner
    /// extraction for agent→owner backfill happens unconditionally (the
    /// signature is cryptographically self-proving). This flag only controls
    /// whether NIP-OA can grant membership access on closed relays.
    ///
    /// Default: `false`. Set via `BUZZ_ALLOW_NIP_OA_AUTH=true`.
    pub allow_nip_oa_auth: bool,

    /// Media storage configuration (S3/MinIO).
    pub media: buzz_media::MediaConfig,
    /// Maximum concurrent media uploads handled by one relay process.
    pub media_max_concurrent_uploads: usize,
    /// Maximum concurrent media uploads accepted from one pubkey.
    pub media_max_concurrent_uploads_per_pubkey: u32,
    /// Maximum media upload starts accepted from one pubkey per minute.
    pub media_uploads_per_minute: u32,

    /// Whether tamper-evident event/media audit logging is enabled. Defaults to true.
    /// This does not control the separate `moderation_actions` audit trail.
    /// Set `BUZZ_AUDIT_ENABLED=false` for deployments that do not require it.
    pub audit_enabled: bool,

    /// Optional override for ephemeral channel TTL (in seconds).
    /// When set, any channel created with a TTL tag will use this value instead
    /// of the client-provided one. Useful for testing ephemeral expiry quickly.
    /// Example: `BUZZ_EPHEMERAL_TTL_OVERRIDE=60` → all ephemeral channels expire
    /// 60 seconds after the last message.
    pub ephemeral_ttl_override: Option<i32>,

    /// Root directory for the relay's local git scratch. No authoritative
    /// repository state lives here — runtime reads/writes hydrate ephemeral
    /// repos from object storage per request. Temporary workspaces, buffered
    /// subprocess output, and the disposable immutable pack cache live below
    /// this path.
    /// Repo-name uniqueness lives in Postgres (`git_repo_names`), not on disk,
    /// so this directory need not be persistent or shared across replicas.
    pub git_repo_path: std::path::PathBuf,
    /// Parent directory for process-isolated immutable pack cache sessions.
    pub git_pack_cache_path: std::path::PathBuf,
    /// Maximum pack file size for git push (bytes). Default: 500 MB.
    pub git_max_pack_bytes: u64,
    /// Maximum total bytes materialized for one git repo request. Default: 1 GB.
    ///
    /// This bounds clone/fetch hydration work across a repo's historical pack
    /// set rather than only bounding one incoming push body.
    pub git_max_repo_bytes: u64,
    /// Maximum bytes retained in the process-local immutable pack/index cache.
    /// Zero disables retention while preserving request-local hydration.
    pub git_pack_cache_max_bytes: u64,
    /// Maximum pack digests populated concurrently in one relay process.
    pub git_pack_cache_max_concurrent_populations: usize,
    /// Maximum number of repos per pubkey. Default: 100.
    pub git_max_repos_per_pubkey: u32,
    /// Maximum concurrent git subprocess operations. Default: 20.
    pub git_max_concurrent_ops: usize,
    /// HMAC secret for git pre-receive hook callbacks.
    /// Used to authenticate internal policy endpoint requests.
    pub git_hook_hmac_secret: String,

    /// Descriptor key identifier accepted in kind:30350 `exec` tags.
    pub push_executor_key_id: String,
    /// Exact HTTPS gateway endpoint used to submit client-authorized APNs delivery capabilities.
    /// Push lease support is disabled when unset.
    pub push_gateway_delivery_url: Option<url::Url>,
    /// Hard timeout for one gateway delivery request.
    pub push_gateway_timeout: Duration,

    /// Optional relay-hosted policy shown on join surfaces. Disabled when no
    /// documents or age attestation are configured.
    pub join_policy: Option<JoinPolicyConfig>,

    /// Deployment-admin API and SPA configuration. Absent means the surface is disabled.
    pub admin: Option<AdminConfig>,

    /// Optional path to the web UI `dist/` directory.
    /// When set, the relay serves the invite landing page and its static assets.
    /// When unset, no static file serving happens (relay behaves as before).
    pub web_dir: Option<std::path::PathBuf>,
    /// Whether the configured web bundle serves Git browser routes in addition
    /// to the public invite landing page. Defaults to false.
    pub serve_git_web_gui: bool,
}

fn parse_bind_addr(raw: &str) -> Result<SocketAddr, ConfigError> {
    raw.parse::<SocketAddr>()
        .map_err(|e| ConfigError::InvalidBindAddr(e.to_string()))
}

fn positive_u64_from_env(name: &str, default: u64) -> Result<u64, ConfigError> {
    match std::env::var(name) {
        Ok(raw) => raw
            .parse::<u64>()
            .ok()
            .filter(|value| *value > 0)
            .ok_or_else(|| ConfigError::InvalidValue(format!("{name} must be a positive integer"))),
        Err(std::env::VarError::NotPresent) => Ok(default),
        Err(std::env::VarError::NotUnicode(_)) => Err(ConfigError::InvalidValue(format!(
            "{name} must be valid Unicode"
        ))),
    }
}

fn rate_limit_config_from_env() -> Result<buzz_auth::RateLimitConfig, ConfigError> {
    let defaults = buzz_auth::RateLimitConfig::default();
    Ok(buzz_auth::RateLimitConfig {
        human_messages_per_min: positive_u64_from_env(
            "BUZZ_RATE_LIMIT_HUMAN_MESSAGES_PER_MIN",
            defaults.human_messages_per_min,
        )?,
        human_api_calls_per_min: positive_u64_from_env(
            "BUZZ_RATE_LIMIT_HUMAN_API_CALLS_PER_MIN",
            defaults.human_api_calls_per_min,
        )?,
        human_ws_events_per_sec: positive_u64_from_env(
            "BUZZ_RATE_LIMIT_HUMAN_WS_EVENTS_PER_SEC",
            defaults.human_ws_events_per_sec,
        )?,
        agent_standard_messages_per_min: positive_u64_from_env(
            "BUZZ_RATE_LIMIT_AGENT_STANDARD_MESSAGES_PER_MIN",
            defaults.agent_standard_messages_per_min,
        )?,
        agent_standard_api_calls_per_min: positive_u64_from_env(
            "BUZZ_RATE_LIMIT_AGENT_STANDARD_API_CALLS_PER_MIN",
            defaults.agent_standard_api_calls_per_min,
        )?,
        agent_elevated_messages_per_min: positive_u64_from_env(
            "BUZZ_RATE_LIMIT_AGENT_ELEVATED_MESSAGES_PER_MIN",
            defaults.agent_elevated_messages_per_min,
        )?,
        agent_platform_messages_per_min: positive_u64_from_env(
            "BUZZ_RATE_LIMIT_AGENT_PLATFORM_MESSAGES_PER_MIN",
            defaults.agent_platform_messages_per_min,
        )?,
    })
}

fn parse_operator_api_origin(raw: &str) -> Result<String, ConfigError> {
    let raw = raw.trim();
    let url = url::Url::parse(raw).map_err(|e| {
        ConfigError::InvalidValue(format!("RELAY_OPERATOR_API_ORIGIN is not a valid URL: {e}"))
    })?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(ConfigError::InvalidValue(
            "RELAY_OPERATOR_API_ORIGIN must be an http(s) origin with no credentials, path, query, or fragment"
                .to_string(),
        ));
    }
    Ok(raw.trim_end_matches('/').to_string())
}

const DEFAULT_PUSH_GATEWAY_DELIVERY_URL: &str = "https://push.buzz.xyz/v1/deliveries/apns";

fn parse_push_gateway_delivery_url(raw: &str) -> Result<url::Url, ConfigError> {
    let url = url::Url::parse(raw.trim()).map_err(|e| {
        ConfigError::InvalidValue(format!(
            "BUZZ_PUSH_GATEWAY_DELIVERY_URL is not a valid URL: {e}"
        ))
    })?;
    if url.scheme() != "https"
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/v1/deliveries/apns"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(ConfigError::InvalidValue(
            "BUZZ_PUSH_GATEWAY_DELIVERY_URL must be an exact HTTPS /v1/deliveries/apns URL without credentials, query, or fragment"
                .to_string(),
        ));
    }
    Ok(url)
}

fn parse_bool(name: &str, default: bool) -> Result<bool, ConfigError> {
    match std::env::var(name) {
        Err(std::env::VarError::NotPresent) => Ok(default),
        Err(error) => Err(ConfigError::InvalidValue(format!(
            "{name} must be valid UTF-8: {error}"
        ))),
        Ok(value) => match value.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "on" => Ok(true),
            "false" | "0" | "off" | "" => Ok(false),
            _ => Err(ConfigError::InvalidValue(format!(
                "{name} must be true or false"
            ))),
        },
    }
}

fn parse_optional_bool(name: &str) -> Result<bool, ConfigError> {
    parse_bool(name, false)
}

fn ensure_git_repo_path(
    raw: impl Into<std::path::PathBuf>,
) -> Result<std::path::PathBuf, ConfigError> {
    ensure_git_path("BUZZ_GIT_REPO_PATH", raw)
}

fn ensure_git_path(
    setting: &str,
    raw: impl Into<std::path::PathBuf>,
) -> Result<std::path::PathBuf, ConfigError> {
    let git_repo_path = raw.into();
    if let Err(e) = std::fs::create_dir_all(&git_repo_path) {
        return Err(ConfigError::InvalidValue(format!(
            "{setting}={} could not be created: {e}",
            git_repo_path.display()
        )));
    }
    Ok(git_repo_path)
}

/// Env vars that once gated authenticated media reads.
///
/// `BUZZ_REQUIRE_MEDIA_GET_AUTH` was the real flag; `BUZZ_REQUIRE_MEDIA_READ_AUTH`
/// was documented in `.env.example` as an accepted alias but was never read by
/// the relay. Media reads are now unconditionally authenticated, so both are
/// inert and an operator still setting either — especially to `false` — holds a
/// belief about their deployment that is no longer true.
const INERT_MEDIA_READ_AUTH_VARS: [&str; 2] = [
    "BUZZ_REQUIRE_MEDIA_GET_AUTH",
    "BUZZ_REQUIRE_MEDIA_READ_AUTH",
];

/// Which of `names` are present, so startup can warn that they do nothing.
///
/// `lookup` is injected rather than calling `std::env::var` directly: process
/// env is global mutable state, so a test that set real vars would race every
/// other test in the binary.
fn inert_env_vars<'a>(names: &[&'a str], lookup: impl Fn(&str) -> Option<String>) -> Vec<&'a str> {
    names
        .iter()
        .copied()
        .filter(|name| lookup(name).is_some())
        .collect()
}

impl Config {
    /// Loads configuration from environment variables, falling back to development defaults.
    pub fn from_env() -> Result<Self, ConfigError> {
        let bind_addr_raw =
            std::env::var("BUZZ_BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:3000".to_string());
        let bind_addr = parse_bind_addr(&bind_addr_raw)?;

        let database_url = std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://buzz:buzz_dev@localhost:5432/buzz".to_string()); // sadscan:disable np.postgres.1

        let read_database_url = std::env::var("READ_DATABASE_URL")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());

        // The old seconds-denominated name is a hard startup error, not an
        // alias: silently honouring it would mean 1000x the intended budget.
        if std::env::var("BUZZ_REPLICA_HEAD_MAX_AGE_SECS").is_ok() {
            return Err(ConfigError::InvalidValue(
                "BUZZ_REPLICA_HEAD_MAX_AGE_SECS was renamed to BUZZ_REPLICA_READ_MAX_AGE_MS \
                 (note: milliseconds, not seconds); refusing to start"
                    .to_string(),
            ));
        }

        // Replica read budget: 0 = off (the rollout default), so this is a
        // non-negative parse, unlike `positive_u64_from_env`.
        let replica_read_max_age_ms = match std::env::var("BUZZ_REPLICA_READ_MAX_AGE_MS") {
            Ok(raw) => raw.trim().parse::<u64>().map_err(|_| {
                ConfigError::InvalidValue(
                    "BUZZ_REPLICA_READ_MAX_AGE_MS must be a non-negative integer".to_string(),
                )
            })?,
            Err(_) => 0,
        };

        // Drain jitter: 0 = off (default). Clamp oversized values so every
        // delayed close is initiated with ten seconds left in the relay's
        // hard-drain budget. An empty/whitespace-only value is treated as unset
        // (jitter off), matching the sibling vars in this file — so setting the
        // var to "" is a valid kill switch, not a crashloop.
        let drain_jitter_ms = match std::env::var("BUZZ_DRAIN_JITTER_MS") {
            Ok(raw) if raw.trim().is_empty() => 0,
            Ok(raw) => raw
                .trim()
                .parse::<u64>()
                .map_err(|_| {
                    ConfigError::InvalidValue(
                        "BUZZ_DRAIN_JITTER_MS must be a non-negative integer".to_string(),
                    )
                })?
                .min(MAX_DRAIN_JITTER_MS),
            Err(_) => 0,
        };

        let redis_url =
            std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://localhost:6379".to_string());

        let redis_pool_size = std::env::var("BUZZ_REDIS_POOL_SIZE")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|&v| v > 0)
            .unwrap_or(16);

        let db_pool_size = std::env::var("BUZZ_DB_POOL_SIZE")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .filter(|&v| v > 0)
            .unwrap_or(50);

        let db_read_pool_size = std::env::var("BUZZ_DB_READ_POOL_SIZE")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .filter(|&v| v > 0);

        let relay_url =
            std::env::var("RELAY_URL").unwrap_or_else(|_| "ws://localhost:3000".to_string());

        let pairing_relay_url = std::env::var("BUZZ_PAIRING_RELAY_URL")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .map(|value| {
                let parsed = url::Url::parse(&value).map_err(|e| {
                    ConfigError::InvalidValue(format!(
                        "BUZZ_PAIRING_RELAY_URL must be a valid ws:// or wss:// URL: {e}"
                    ))
                })?;
                if !matches!(parsed.scheme(), "ws" | "wss") || parsed.host_str().is_none() {
                    return Err(ConfigError::InvalidValue(
                        "BUZZ_PAIRING_RELAY_URL must be a valid ws:// or wss:// URL".to_string(),
                    ));
                }
                Ok(value)
            })
            .transpose()?;

        let max_connections = std::env::var("BUZZ_MAX_CONNECTIONS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(10_000);

        let max_concurrent_handlers = std::env::var("BUZZ_MAX_CONCURRENT_HANDLERS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1024);

        let send_buffer_size = std::env::var("BUZZ_SEND_BUFFER")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1_000);

        let max_frame_bytes = std::env::var("BUZZ_MAX_FRAME_BYTES")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|&v| v > 0)
            .unwrap_or(DEFAULT_MAX_FRAME_BYTES);

        let slow_client_grace_limit = std::env::var("BUZZ_SLOW_CLIENT_GRACE_LIMIT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(15);

        let require_auth_token = std::env::var("BUZZ_REQUIRE_AUTH_TOKEN")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false);

        let pubkey_allowlist_enabled = std::env::var("BUZZ_PUBKEY_ALLOWLIST")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false);

        let require_relay_membership = std::env::var("BUZZ_REQUIRE_RELAY_MEMBERSHIP")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false);

        // Defaults true → single-pod (N=1) keeps today's huddle behavior. A
        // horizontally-scaled deployment sets this false; see the field doc.
        let huddle_audio_available = std::env::var("BUZZ_HUDDLE_AUDIO_AVAILABLE")
            .map(|v| !(v == "false" || v == "0"))
            .unwrap_or(true);

        // Mesh opt-in: default OFF. Strict rollout no-regression — an image
        // upgrade with untouched env must not bind a new UDP port or write a
        // new Redis key. Horizontally-scaled deployments explicitly set
        // `BUZZ_MESH=on`; anything else (absent, `off`, other values) keeps
        // exact single-instance behavior.
        let mesh_enabled = std::env::var("BUZZ_MESH")
            .map(|v| v.eq_ignore_ascii_case("on") || v == "true" || v == "1")
            .unwrap_or(false);
        let mesh_bind_addr = std::env::var("BUZZ_MESH_BIND_ADDR")
            .map(|raw| {
                raw.parse::<SocketAddr>().map_err(|e| {
                    ConfigError::InvalidValue(format!("invalid BUZZ_MESH_BIND_ADDR: {e}"))
                })
            })
            .unwrap_or_else(|_| Ok("0.0.0.0:3478".parse().expect("static default parses")))?;
        let mesh = buzz_relay_mesh::MeshConfig {
            enabled: mesh_enabled,
            bind_addr: mesh_bind_addr,
            registry_refresh: std::time::Duration::from_secs(15),
        };

        // Demo echo opt-in: same strict pattern as BUZZ_MESH — explicit
        // `on`/`true`/`1` only, anything else (absent, `off`, typos) is off.
        let mesh_demo_echo = std::env::var("BUZZ_MESH_DEMO_ECHO")
            .map(|v| v.eq_ignore_ascii_case("on") || v == "true" || v == "1")
            .unwrap_or(false);

        let allow_nip_oa_auth = std::env::var("BUZZ_ALLOW_NIP_OA_AUTH")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false);

        // Note: intentionally not prefixed with BUZZ_ — this is a relay-identity
        // config that may be shared across multiple services (e.g., ACP agent).
        let relay_owner_pubkey = std::env::var("RELAY_OWNER_PUBKEY")
            .ok()
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .map(|s| {
                // Must be exactly 64 lowercase hex characters (32-byte pubkey).
                // Fail closed — once RELAY_OWNER_PUBKEY can be the break-glass
                // operator root (owner-fallback B), silently discarding a malformed
                // value would be a lockout, not a graceful degradation.
                let valid = s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit());
                if valid {
                    Ok(s)
                } else {
                    Err(ConfigError::InvalidValue(format!(
                        "RELAY_OWNER_PUBKEY is not a valid 64-char hex pubkey — \
                         got: {s:?}. Fix or unset it; a malformed value is a startup error \
                         because this key can serve as the break-glass operator root when \
                         RELAY_OPERATOR_PUBKEYS is empty."
                    )))
                }
            })
            .transpose()?;

        // Note: intentionally not prefixed with BUZZ_ — same relay-identity
        // config family as RELAY_OWNER_PUBKEY. Comma-separated 64-char hex
        // pubkeys. Unlike RELAY_OWNER_PUBKEY (warn-and-ignore), an invalid
        // entry here is a hard config error: silently dropping an operator
        // pubkey would silently disable provisioning for that operator.
        let relay_operator_api_origin = std::env::var("RELAY_OPERATOR_API_ORIGIN")
            .ok()
            .filter(|raw| !raw.trim().is_empty())
            .map(|raw| parse_operator_api_origin(&raw))
            .transpose()?;

        let relay_operator_pubkeys = match std::env::var("RELAY_OPERATOR_PUBKEYS") {
            Ok(raw) => {
                let mut pubkeys = Vec::new();
                for entry in raw.split(',') {
                    let entry = entry.trim().to_lowercase();
                    if entry.is_empty() {
                        continue;
                    }
                    let valid = entry.len() == 64 && entry.chars().all(|c| c.is_ascii_hexdigit());
                    if !valid {
                        return Err(ConfigError::InvalidValue(format!(
                            "RELAY_OPERATOR_PUBKEYS entry is not a valid 64-char hex pubkey: {entry:?}"
                        )));
                    }
                    if !pubkeys.contains(&entry) {
                        pubkeys.push(entry);
                    }
                }
                pubkeys
            }
            Err(_) => Vec::new(),
        };
        if !relay_operator_pubkeys.is_empty() && relay_operator_api_origin.is_none() {
            return Err(ConfigError::InvalidValue(
                "RELAY_OPERATOR_API_ORIGIN is required when RELAY_OPERATOR_PUBKEYS is configured"
                    .to_string(),
            ));
        }

        let auth = buzz_auth::AuthConfig {
            rate_limits: rate_limit_config_from_env()?,
        };

        if !require_auth_token {
            warn!(
                "BUZZ_REQUIRE_AUTH_TOKEN is false — REST API requests bypass token auth. \
                 WebSocket protocol auth is unaffected. Set to true for production."
            );
        }

        let cors_origins = std::env::var("BUZZ_CORS_ORIGINS")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        let relay_private_key = std::env::var("BUZZ_RELAY_PRIVATE_KEY").ok();

        let uds_path = std::env::var("BUZZ_UDS_PATH")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let health_port = std::env::var("BUZZ_HEALTH_PORT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(8080);

        let metrics_port = std::env::var("BUZZ_METRICS_PORT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(9102);

        let s3_addressing_style = match std::env::var("BUZZ_S3_ADDRESSING_STYLE") {
            Ok(value) => value.parse().map_err(ConfigError::InvalidValue)?,
            Err(std::env::VarError::NotPresent) => buzz_media::config::S3AddressingStyle::default(),
            Err(std::env::VarError::NotUnicode(_)) => {
                return Err(ConfigError::InvalidValue(
                    "BUZZ_S3_ADDRESSING_STYLE must be valid Unicode and one of 'path' or 'virtual'"
                        .to_string(),
                ));
            }
        };
        let media = buzz_media::MediaConfig {
            s3_endpoint: std::env::var("BUZZ_S3_ENDPOINT")
                .unwrap_or_else(|_| "http://localhost:9000".to_string()),
            s3_access_key: std::env::var("BUZZ_S3_ACCESS_KEY")
                .unwrap_or_else(|_| "buzz_dev".to_string()),
            s3_secret_key: std::env::var("BUZZ_S3_SECRET_KEY")
                .unwrap_or_else(|_| "buzz_dev_secret".to_string()),
            s3_bucket: std::env::var("BUZZ_S3_BUCKET").unwrap_or_else(|_| "buzz-media".to_string()),
            s3_region: std::env::var("BUZZ_S3_REGION")
                .or_else(|_| std::env::var("AWS_REGION"))
                .unwrap_or_else(|_| "us-east-1".to_string()),
            s3_addressing_style,
            max_image_bytes: std::env::var("BUZZ_MAX_IMAGE_BYTES")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(50 * 1024 * 1024),
            max_gif_bytes: std::env::var("BUZZ_MAX_GIF_BYTES")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(10 * 1024 * 1024),
            max_video_bytes: std::env::var("BUZZ_MAX_VIDEO_BYTES")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(500 * 1024 * 1024),
            max_file_bytes: std::env::var("BUZZ_MAX_FILE_BYTES")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(100 * 1024 * 1024),
            public_base_url: std::env::var("BUZZ_MEDIA_BASE_URL")
                .unwrap_or_else(|_| "http://localhost:3000/media".to_string()),
            // Per-upload-event records (`_uploads/` moderation side channel).
            // Off by default; coherence between the three knobs is enforced in
            // MediaConfig::validate at startup.
            upload_records_enabled: std::env::var("BUZZ_MEDIA_UPLOAD_RECORDS")
                .map(|v| v == "true" || v == "1")
                .unwrap_or(false),
            upload_ip_header: std::env::var("BUZZ_MEDIA_UPLOAD_IP_HEADER")
                .ok()
                .map(|s| s.trim().to_lowercase())
                .filter(|s| !s.is_empty()),
            upload_port_header: std::env::var("BUZZ_MEDIA_UPLOAD_PORT_HEADER")
                .ok()
                .map(|s| s.trim().to_lowercase())
                .filter(|s| !s.is_empty()),
        };
        let media_max_concurrent_uploads: usize =
            std::env::var("BUZZ_MEDIA_MAX_CONCURRENT_UPLOADS")
                .ok()
                .and_then(|v| v.parse().ok())
                .filter(|&v| v > 0)
                .unwrap_or(8);
        let media_max_concurrent_uploads_per_pubkey: u32 =
            std::env::var("BUZZ_MEDIA_MAX_CONCURRENT_UPLOADS_PER_PUBKEY")
                .ok()
                .and_then(|v| v.parse().ok())
                .filter(|&v| v > 0)
                .unwrap_or(2)
                .min(u32::try_from(media_max_concurrent_uploads).unwrap_or(u32::MAX));
        let media_uploads_per_minute: u32 = std::env::var("BUZZ_MEDIA_UPLOADS_PER_MINUTE")
            .ok()
            .and_then(|v| v.parse().ok())
            .filter(|&v| v > 0)
            .unwrap_or(30);

        for name in inert_env_vars(&INERT_MEDIA_READ_AUTH_VARS, |n| std::env::var(n).ok()) {
            warn!(
                "{name} is set but is no longer read — GET/HEAD /media/* always require \
                 Blossom t=get auth plus relay membership. Remove it; a value of `false` \
                 does not re-open unauthenticated media reads."
            );
        }

        let ephemeral_ttl_override = std::env::var("BUZZ_EPHEMERAL_TTL_OVERRIDE")
            .ok()
            .and_then(|v| v.parse::<i32>().ok())
            .filter(|&v| v > 0);

        if let Some(ttl) = ephemeral_ttl_override {
            warn!(
                "BUZZ_EPHEMERAL_TTL_OVERRIDE={ttl}s — all ephemeral channels will use \
                 this TTL instead of the client-provided value."
            );
        }

        // Git server config
        let git_repo_path = ensure_git_repo_path(
            std::env::var("BUZZ_GIT_REPO_PATH").unwrap_or_else(|_| "./repos".to_string()),
        )?;
        let git_pack_cache_path = ensure_git_path(
            "BUZZ_GIT_PACK_CACHE_PATH",
            std::env::var("BUZZ_GIT_PACK_CACHE_PATH")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|_| git_repo_path.join(".pack-cache")),
        )?;
        let git_max_pack_bytes: u64 = std::env::var("BUZZ_GIT_MAX_PACK_BYTES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(500 * 1024 * 1024); // 500 MB
        let git_max_repo_bytes: u64 = std::env::var("BUZZ_GIT_MAX_REPO_BYTES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or_else(|| git_max_pack_bytes.saturating_mul(2)); // 1 GB at defaults
        let git_pack_cache_max_bytes: u64 = std::env::var("BUZZ_GIT_PACK_CACHE_MAX_BYTES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or_else(|| git_max_repo_bytes.saturating_mul(5)); // 5 GB at defaults
        let git_pack_cache_max_concurrent_populations: usize =
            std::env::var("BUZZ_GIT_PACK_CACHE_MAX_CONCURRENT_POPULATIONS")
                .ok()
                .and_then(|v| v.parse().ok())
                .filter(|value| *value > 0)
                .unwrap_or(2);
        let git_max_repos_per_pubkey: u32 = std::env::var("BUZZ_GIT_MAX_REPOS_PER_PUBKEY")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(100);
        let git_max_concurrent_ops: usize = std::env::var("BUZZ_GIT_MAX_CONCURRENT_OPS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(20);
        let git_hook_hmac_secret: String = std::env::var("BUZZ_GIT_HOOK_HMAC_SECRET")
            .unwrap_or_else(|_| {
                // Generate a random secret if not configured (dev mode).
                let secret: [u8; 32] = rand::random();
                hex::encode(secret)
            });
        let push_executor_key_id =
            std::env::var("BUZZ_PUSH_EXECUTOR_KEY_ID").unwrap_or_else(|_| "relay-v1".to_string());
        if push_executor_key_id.is_empty() || push_executor_key_id.len() > 64 {
            return Err(ConfigError::InvalidValue(
                "BUZZ_PUSH_EXECUTOR_KEY_ID must contain 1..=64 bytes".to_string(),
            ));
        }
        let push_gateway_delivery_url = match std::env::var("BUZZ_PUSH_GATEWAY_DELIVERY_URL") {
            Ok(raw) if raw.trim().is_empty() => None,
            Ok(raw) => Some(parse_push_gateway_delivery_url(&raw)?),
            Err(_) => Some(parse_push_gateway_delivery_url(
                DEFAULT_PUSH_GATEWAY_DELIVERY_URL,
            )?),
        };
        let push_gateway_timeout_millis = match std::env::var("BUZZ_PUSH_GATEWAY_TIMEOUT_MS") {
            Ok(raw) => raw
                .parse::<u64>()
                .ok()
                .filter(|millis| (100..=10_000).contains(millis))
                .ok_or_else(|| {
                    ConfigError::InvalidValue(
                        "BUZZ_PUSH_GATEWAY_TIMEOUT_MS must be an integer in 100..=10000"
                            .to_string(),
                    )
                })?,
            Err(_) => 2_000,
        };
        let push_gateway_timeout = Duration::from_millis(push_gateway_timeout_millis);

        const MAX_POLICY_MARKDOWN_BYTES: usize = 256 * 1024;
        let read_policy_markdown = |name: &str| -> Result<Option<String>, ConfigError> {
            let value = std::env::var(name)
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty());
            if value
                .as_ref()
                .is_some_and(|value| value.len() > MAX_POLICY_MARKDOWN_BYTES)
            {
                return Err(ConfigError::InvalidValue(format!(
                    "{name} must contain at most {MAX_POLICY_MARKDOWN_BYTES} bytes"
                )));
            }
            Ok(value)
        };
        let terms_markdown = read_policy_markdown("BUZZ_TERMS_OF_SERVICE_MARKDOWN")?;
        let privacy_markdown = read_policy_markdown("BUZZ_PRIVACY_POLICY_MARKDOWN")?;
        let age_attestation_required = parse_optional_bool("BUZZ_AGE_ATTESTATION_REQUIRED")?;
        let audit_enabled = parse_bool("BUZZ_AUDIT_ENABLED", true)?;
        let join_policy = if terms_markdown.is_none()
            && privacy_markdown.is_none()
            && !age_attestation_required
        {
            None
        } else {
            let mut hasher = Sha256::new();
            hasher.update(terms_markdown.as_deref().unwrap_or_default().as_bytes());
            hasher.update([0]);
            hasher.update(privacy_markdown.as_deref().unwrap_or_default().as_bytes());
            hasher.update([0, u8::from(age_attestation_required)]);
            Some(JoinPolicyConfig {
                terms_markdown,
                privacy_markdown,
                age_attestation_required,
                version: hex::encode(hasher.finalize()),
            })
        };

        // Read-only deployment-admin surface. The route is absent when the host is unset.
        let admin = match std::env::var("BUZZ_ADMIN_HOST")
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
        {
            None => {
                if std::env::var_os("BUZZ_ADMIN_TOKEN").is_some() {
                    tracing::warn!(
                        "BUZZ_ADMIN_TOKEN is set without BUZZ_ADMIN_HOST — the admin \
                         dashboard and API stay disabled and the token is ignored"
                    );
                }
                if std::env::var_os("BUZZ_ADMIN_AUTH").is_some() {
                    tracing::warn!(
                        "BUZZ_ADMIN_AUTH is set without BUZZ_ADMIN_HOST — \
                         the admin dashboard and API stay disabled and the value is ignored"
                    );
                }
                None
            }
            Some(host) => {
                if host.contains(['/', '\\', '@']) {
                    return Err(ConfigError::InvalidValue(
                        "BUZZ_ADMIN_HOST must be an exact authority".to_string(),
                    ));
                }

                // Parse BUZZ_ADMIN_AUTH. Accepted values: "token" (default when
                // unset), "disabled", "nip98". Any other non-empty value is a
                // startup error (typo-proofing).
                let auth_mode = match std::env::var("BUZZ_ADMIN_AUTH")
                    .ok()
                    .as_deref()
                    .map(str::trim)
                {
                    None | Some("") | Some("token") => "token",
                    Some("disabled") => "disabled",
                    Some("nip98") => "nip98",
                    Some(other) => {
                        return Err(ConfigError::InvalidValue(format!(
                            "BUZZ_ADMIN_AUTH must be \"token\", \"disabled\", or \"nip98\"; \
                             got \"{other}\""
                        )))
                    }
                };

                // BUZZ_ADMIN_TOKEN set in nip98 or disabled mode is ambiguous intent; fail
                // closed.
                if auth_mode != "token" && std::env::var_os("BUZZ_ADMIN_TOKEN").is_some() {
                    return Err(ConfigError::InvalidValue(format!(
                        "BUZZ_ADMIN_TOKEN is set but BUZZ_ADMIN_AUTH is \"{auth_mode}\" — \
                         BUZZ_ADMIN_TOKEN is only used in token mode; set one or the other"
                    )));
                }

                let auth = match auth_mode {
                    "disabled" => {
                        tracing::warn!(
                            "BUZZ_ADMIN_AUTH=disabled — the admin API is \
                             unauthenticated; the operator has asserted that access is \
                             controlled at the network layer (reverse proxy, VPN, firewall)"
                        );
                        AdminAuth::Disabled
                    }
                    "nip98" => AdminAuth::Nip98,
                    _ => AdminAuth::Token(parse_admin_token()?),
                };

                let web_dir = std::env::var("BUZZ_ADMIN_WEB_DIR")
                    .ok()
                    .map(|value| std::path::PathBuf::from(value.trim()))
                    .filter(|value| !value.as_os_str().is_empty());
                if let Some(ref dir) = web_dir {
                    if !dir.join("index.html").is_file() {
                        return Err(ConfigError::InvalidValue(format!(
                            "BUZZ_ADMIN_WEB_DIR={} does not contain index.html",
                            dir.display()
                        )));
                    }
                }
                Some(AdminConfig {
                    host,
                    auth,
                    web_dir,
                })
            }
        };

        // Web UI static file serving
        let web_dir = std::env::var("BUZZ_WEB_DIR")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .map(std::path::PathBuf::from);
        let serve_git_web_gui = std::env::var("BUZZ_SERVE_GIT_WEB_GUI")
            .map(|value| value == "true" || value == "1")
            .unwrap_or(false);

        if let Some(ref dir) = web_dir {
            if !dir.join("index.html").is_file() {
                return Err(ConfigError::InvalidValue(format!(
                    "BUZZ_WEB_DIR={} does not contain index.html",
                    dir.display()
                )));
            }
            tracing::info!("BUZZ_WEB_DIR={} — serving web UI from relay", dir.display());
        }

        // Reject explicitly-configured secrets that are too short.
        // The auto-generated fallback is always 64 hex chars (32 bytes), so this
        // only fires when someone sets BUZZ_GIT_HOOK_HMAC_SECRET to a weak value.
        if std::env::var("BUZZ_GIT_HOOK_HMAC_SECRET").is_ok() && git_hook_hmac_secret.len() < 32 {
            return Err(ConfigError::InvalidValue(
                "BUZZ_GIT_HOOK_HMAC_SECRET must be at least 32 characters (16 bytes hex)"
                    .to_string(),
            ));
        }

        Ok(Self {
            bind_addr,
            database_url,
            read_database_url,
            replica_read_max_age_ms,
            drain_jitter_ms,
            redis_url,
            redis_pool_size,
            db_pool_size,
            db_read_pool_size,
            relay_url,
            pairing_relay_url,
            max_connections,
            max_concurrent_handlers,
            send_buffer_size,
            max_frame_bytes,
            slow_client_grace_limit,
            auth,
            require_auth_token,
            cors_origins,
            relay_private_key,
            uds_path,
            health_port,
            metrics_port,
            pubkey_allowlist_enabled,
            require_relay_membership,
            huddle_audio_available,
            mesh,
            mesh_demo_echo,
            relay_owner_pubkey,
            relay_operator_api_origin,
            relay_operator_pubkeys,
            allow_nip_oa_auth,
            media,
            media_max_concurrent_uploads,
            media_max_concurrent_uploads_per_pubkey,
            media_uploads_per_minute,
            audit_enabled,
            ephemeral_ttl_override,
            git_repo_path,
            git_pack_cache_path,
            git_max_pack_bytes,
            git_max_repo_bytes,
            git_pack_cache_max_bytes,
            git_pack_cache_max_concurrent_populations,
            git_max_repos_per_pubkey,
            git_max_concurrent_ops,
            git_hook_hmac_secret,
            push_executor_key_id,
            push_gateway_delivery_url,
            push_gateway_timeout,
            join_policy,
            admin,
            web_dir,
            serve_git_web_gui,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Mutex to serialize tests that mutate environment variables.
    // Parallel env-var mutation causes `defaults_are_valid` to see the invalid
    // value set by `invalid_bind_addr_returns_error`, causing a flaky failure.
    static ENV_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Look up against a fixed set, standing in for process env.
    fn env_of<'a>(set: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + use<'a> {
        move |name| {
            set.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_string())
        }
    }

    /// The case that matters: an operator who pinned the old flag to `false`
    /// must be told it is inert, not left believing media reads are still open.
    #[test]
    fn inert_media_read_auth_vars_are_reported_even_when_false() {
        let found = inert_env_vars(
            &INERT_MEDIA_READ_AUTH_VARS,
            env_of(&[("BUZZ_REQUIRE_MEDIA_GET_AUTH", "false")]),
        );

        assert_eq!(found, vec!["BUZZ_REQUIRE_MEDIA_GET_AUTH"]);
    }

    /// `BUZZ_REQUIRE_MEDIA_READ_AUTH` was advertised in `.env.example` as an
    /// accepted alias but the relay never read it, so operators may hold it
    /// today. It warns too.
    #[test]
    fn inert_media_read_auth_vars_include_the_documented_alias() {
        let found = inert_env_vars(
            &INERT_MEDIA_READ_AUTH_VARS,
            env_of(&[
                ("BUZZ_REQUIRE_MEDIA_GET_AUTH", "true"),
                ("BUZZ_REQUIRE_MEDIA_READ_AUTH", "false"),
            ]),
        );

        assert_eq!(
            found,
            vec![
                "BUZZ_REQUIRE_MEDIA_GET_AUTH",
                "BUZZ_REQUIRE_MEDIA_READ_AUTH"
            ]
        );
    }

    #[test]
    fn inert_media_read_auth_vars_stay_quiet_when_unset() {
        let found = inert_env_vars(
            &INERT_MEDIA_READ_AUTH_VARS,
            env_of(&[("BUZZ_REQUIRE_RELAY_MEMBERSHIP", "true")]),
        );

        assert!(found.is_empty(), "unrelated vars must not warn: {found:?}");
    }

    #[test]
    fn defaults_are_valid() {
        let _guard = ENV_MUTEX.lock().unwrap();
        let config = Config::from_env().expect("default config");
        assert!(config.bind_addr.port() > 0);
        assert!(!config.database_url.is_empty());
        assert!(!config.redis_url.is_empty());
        assert_eq!(config.redis_pool_size, 16);
        assert_eq!(config.db_pool_size, 50);
        assert!(config.max_connections > 0);
        assert!(config.send_buffer_size > 0);
        assert_eq!(config.max_frame_bytes, DEFAULT_MAX_FRAME_BYTES);
        assert!(config.slow_client_grace_limit > 0);
        assert!(
            !config.pubkey_allowlist_enabled,
            "pubkey_allowlist_enabled should default to false"
        );
        assert!(
            !config.require_relay_membership,
            "require_relay_membership should default to false"
        );
        assert!(
            config.relay_owner_pubkey.is_none(),
            "relay_owner_pubkey should default to None"
        );
        assert!(
            config.relay_operator_pubkeys.is_empty(),
            "relay_operator_pubkeys should default empty (provisioning disabled)"
        );
        assert!(
            !config.allow_nip_oa_auth,
            "allow_nip_oa_auth should default to false"
        );
        assert!(
            !config.serve_git_web_gui,
            "serve_git_web_gui should default to false"
        );
        assert_eq!(
            config.media.s3_addressing_style,
            buzz_media::config::S3AddressingStyle::Path,
            "S3 addressing must default to path style for bundled MinIO compatibility"
        );
        assert!(
            config.join_policy.is_none(),
            "join_policy should default to None so policy prompts and acceptance receipts are opt-in"
        );
        assert!(
            config.huddle_audio_available,
            "huddle_audio_available should default to true so single-pod (N=1) keeps today's huddle behavior"
        );
    }

    /// Run `Config::from_env()` with the admin variables forced to `values`,
    /// restoring the ambient environment afterwards.
    fn config_with_admin_env(values: &[(&str, Option<&str>)]) -> Result<Config, ConfigError> {
        const KEYS: [&str; 3] = ["BUZZ_ADMIN_HOST", "BUZZ_ADMIN_TOKEN", "BUZZ_ADMIN_AUTH"];
        let previous: Vec<_> = KEYS
            .iter()
            .map(|key| (*key, std::env::var_os(key)))
            .collect();
        for key in KEYS {
            std::env::remove_var(key);
        }
        for (key, value) in values {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
        let config = Config::from_env();
        for (key, value) in previous {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
        config
    }

    const VALID_ADMIN_TOKEN: &str =
        "5f0e1d2c3b4a59687786958493a2b1c0decadebeefcafe0123456789abcdef01";

    #[test]
    fn admin_host_without_a_usable_token_fails_closed_at_startup() {
        let _guard = ENV_MUTEX.lock().unwrap();
        for token in [
            None,
            Some(""),
            Some("   "),
            Some("not-hex-not-hex-not-hex-not-hex-not-hex-not-hex-not-hex-not-hex1"),
            Some(&VALID_ADMIN_TOKEN[..62]),
            Some("5f0e1d2c3b4a59687786958493a2b1c0decadebeefcafe0123456789abcdef0100"),
        ] {
            let result = config_with_admin_env(&[
                ("BUZZ_ADMIN_HOST", Some("admin.example")),
                ("BUZZ_ADMIN_TOKEN", token),
            ]);
            assert!(
                matches!(
                    result,
                    Err(ConfigError::InvalidValue(ref message))
                        if message.contains("BUZZ_ADMIN_TOKEN")
                ),
                "{token:?} must be rejected"
            );
        }
    }

    #[test]
    fn admin_surface_activates_with_a_valid_token_and_tolerates_surrounding_whitespace() {
        let _guard = ENV_MUTEX.lock().unwrap();
        for token in [
            VALID_ADMIN_TOKEN.to_string(),
            format!("  {VALID_ADMIN_TOKEN}\n"),
        ] {
            let admin = config_with_admin_env(&[
                ("BUZZ_ADMIN_HOST", Some("admin.example")),
                ("BUZZ_ADMIN_TOKEN", Some(&token)),
            ])
            .expect("config with a valid admin token")
            .admin
            .expect("admin surface is configured");
            assert_eq!(admin.host, "admin.example");
            assert!(
                matches!(admin.auth, crate::config::AdminAuth::Token(ref t) if {
                    let mut expected = [0u8; 32];
                    hex::decode_to_slice(VALID_ADMIN_TOKEN, &mut expected).expect("hex fixture");
                    t.matches(&expected) && !t.matches(&[0u8; 32])
                })
            );
        }
    }

    #[test]
    fn admin_token_without_a_host_leaves_the_surface_disabled() {
        let _guard = ENV_MUTEX.lock().unwrap();
        let config = config_with_admin_env(&[
            ("BUZZ_ADMIN_HOST", None),
            ("BUZZ_ADMIN_TOKEN", Some(VALID_ADMIN_TOKEN)),
        ])
        .expect("a token alone is not a startup error");
        assert!(config.admin.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn admin_token_rejects_non_unicode_values() {
        use std::os::unix::ffi::OsStringExt;

        let _guard = ENV_MUTEX.lock().unwrap();
        let previous_host = std::env::var_os("BUZZ_ADMIN_HOST");
        let previous_token = std::env::var_os("BUZZ_ADMIN_TOKEN");
        std::env::set_var("BUZZ_ADMIN_HOST", "admin.example");
        std::env::set_var("BUZZ_ADMIN_TOKEN", std::ffi::OsString::from_vec(vec![0xff]));

        let invalid = Config::from_env();

        for (key, value) in [
            ("BUZZ_ADMIN_HOST", previous_host),
            ("BUZZ_ADMIN_TOKEN", previous_token),
        ] {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }

        assert!(matches!(
            invalid,
            Err(ConfigError::InvalidValue(ref message))
                if message.contains("BUZZ_ADMIN_TOKEN")
        ));
    }

    #[test]
    fn admin_token_debug_output_is_redacted() {
        let token = AdminToken::from_bytes([7u8; 32]);
        let rendered = format!("{token:?}");
        assert_eq!(rendered, "AdminToken(redacted)");
        assert!(!rendered.contains("07"));
    }

    #[test]
    fn disabled_mode_activates_without_a_token() {
        let _guard = ENV_MUTEX.lock().unwrap();
        let admin = config_with_admin_env(&[
            ("BUZZ_ADMIN_HOST", Some("admin.example")),
            ("BUZZ_ADMIN_TOKEN", None),
            ("BUZZ_ADMIN_AUTH", Some("disabled")),
        ])
        .expect("disabled mode without a token is valid")
        .admin
        .expect("admin surface is configured");
        assert_eq!(admin.host, "admin.example");
        assert!(matches!(admin.auth, crate::config::AdminAuth::Disabled));
    }

    #[test]
    fn disabled_mode_and_token_both_set_fails_closed() {
        let _guard = ENV_MUTEX.lock().unwrap();
        let result = config_with_admin_env(&[
            ("BUZZ_ADMIN_HOST", Some("admin.example")),
            ("BUZZ_ADMIN_TOKEN", Some(VALID_ADMIN_TOKEN)),
            ("BUZZ_ADMIN_AUTH", Some("disabled")),
        ]);
        assert!(
            matches!(
                result,
                Err(ConfigError::InvalidValue(ref message))
                    if message.contains("BUZZ_ADMIN_TOKEN")
                       && message.contains("disabled")
            ),
            "both set must be a startup error: {result:?}"
        );
    }

    #[test]
    fn admin_auth_junk_values_all_fail_closed() {
        let _guard = ENV_MUTEX.lock().unwrap();
        for junk in [
            "1",
            "yes",
            "TRUE",
            "True",
            "false",
            "0",
            "on",
            "insecure_no_auth",
        ] {
            let result = config_with_admin_env(&[
                ("BUZZ_ADMIN_HOST", Some("admin.example")),
                ("BUZZ_ADMIN_TOKEN", None),
                ("BUZZ_ADMIN_AUTH", Some(junk)),
            ]);
            assert!(
                matches!(
                    result,
                    Err(ConfigError::InvalidValue(ref message))
                        if message.contains("BUZZ_ADMIN_AUTH")
                ),
                "{junk:?} must be rejected: {result:?}"
            );
        }
    }

    #[test]
    fn admin_auth_empty_string_is_treated_as_token_mode() {
        // An empty value (e.g. `BUZZ_ADMIN_AUTH=`) is treated as unset → token mode.
        // Token mode without a token fails closed with a BUZZ_ADMIN_TOKEN error.
        let _guard = ENV_MUTEX.lock().unwrap();
        let result = config_with_admin_env(&[
            ("BUZZ_ADMIN_HOST", Some("admin.example")),
            ("BUZZ_ADMIN_TOKEN", None),
            ("BUZZ_ADMIN_AUTH", Some("")),
        ]);
        assert!(
            matches!(
                result,
                Err(ConfigError::InvalidValue(ref message))
                    if message.contains("BUZZ_ADMIN_TOKEN")
            ),
            "empty BUZZ_ADMIN_AUTH should fall through to the token requirement"
        );
    }

    #[test]
    fn admin_auth_explicit_token_mode_behaves_identically_to_unset() {
        let _guard = ENV_MUTEX.lock().unwrap();
        let admin = config_with_admin_env(&[
            ("BUZZ_ADMIN_HOST", Some("admin.example")),
            ("BUZZ_ADMIN_TOKEN", Some(VALID_ADMIN_TOKEN)),
            ("BUZZ_ADMIN_AUTH", Some("token")),
        ])
        .expect("explicit token mode with a valid token is valid")
        .admin
        .expect("admin surface is configured");
        assert!(matches!(admin.auth, crate::config::AdminAuth::Token(_)));
    }

    #[test]
    fn nip98_mode_parses_and_succeeds_without_pubkeys_env() {
        let _guard = ENV_MUTEX.lock().unwrap();
        let admin = config_with_admin_env(&[
            ("BUZZ_ADMIN_HOST", Some("admin.example")),
            ("BUZZ_ADMIN_AUTH", Some("nip98")),
        ])
        .expect(
            "nip98 mode succeeds without BUZZ_ADMIN_PUBKEYS (role resolution is at request time)",
        )
        .admin
        .expect("admin surface is configured");
        assert!(matches!(admin.auth, crate::config::AdminAuth::Nip98));
    }

    #[test]
    fn nip98_mode_and_token_both_set_fails_closed() {
        let _guard = ENV_MUTEX.lock().unwrap();
        let result = config_with_admin_env(&[
            ("BUZZ_ADMIN_HOST", Some("admin.example")),
            ("BUZZ_ADMIN_AUTH", Some("nip98")),
            ("BUZZ_ADMIN_TOKEN", Some(VALID_ADMIN_TOKEN)),
        ]);
        assert!(
            matches!(
                result,
                Err(ConfigError::InvalidValue(ref message))
                    if message.contains("BUZZ_ADMIN_TOKEN")
                       && message.contains("nip98")
            ),
            "nip98 + token must be a startup error: {result:?}"
        );
    }

    #[test]
    fn malformed_relay_owner_pubkey_is_a_startup_error_not_warn_and_ignore() {
        let _guard = ENV_MUTEX.lock().unwrap();
        let previous = std::env::var_os("RELAY_OWNER_PUBKEY");
        for bad in ["not-a-pubkey", &"a".repeat(63), &"z".repeat(64), "abcd"] {
            std::env::set_var("RELAY_OWNER_PUBKEY", bad);
            let result = Config::from_env();
            std::env::remove_var("RELAY_OWNER_PUBKEY");
            assert!(
                matches!(
                    result,
                    Err(ConfigError::InvalidValue(ref message))
                        if message.contains("RELAY_OWNER_PUBKEY")
                ),
                "malformed RELAY_OWNER_PUBKEY {bad:?} must be a startup error, got: {result:?}"
            );
        }
        // Restore.
        match previous {
            Some(v) => std::env::set_var("RELAY_OWNER_PUBKEY", v),
            None => std::env::remove_var("RELAY_OWNER_PUBKEY"),
        }
    }

    #[test]
    fn valid_relay_owner_pubkey_parses_correctly() {
        let _guard = ENV_MUTEX.lock().unwrap();
        let previous = std::env::var_os("RELAY_OWNER_PUBKEY");
        let valid = "a".repeat(64);
        std::env::set_var("RELAY_OWNER_PUBKEY", &valid);
        let config = Config::from_env().expect("valid RELAY_OWNER_PUBKEY parses");
        std::env::remove_var("RELAY_OWNER_PUBKEY");
        if let Some(v) = previous {
            std::env::set_var("RELAY_OWNER_PUBKEY", v);
        }
        assert_eq!(config.relay_owner_pubkey, Some(valid));
    }

    #[test]
    fn s3_addressing_style_env_accepts_virtual_and_rejects_invalid_values() {
        let _guard = ENV_MUTEX.lock().unwrap();
        let previous = std::env::var_os("BUZZ_S3_ADDRESSING_STYLE");

        std::env::set_var("BUZZ_S3_ADDRESSING_STYLE", "virtual");
        let configured = Config::from_env()
            .expect("virtual style config")
            .media
            .s3_addressing_style;

        std::env::set_var("BUZZ_S3_ADDRESSING_STYLE", "auto");
        let invalid = Config::from_env();

        if let Some(value) = previous {
            std::env::set_var("BUZZ_S3_ADDRESSING_STYLE", value);
        } else {
            std::env::remove_var("BUZZ_S3_ADDRESSING_STYLE");
        }

        assert_eq!(configured, buzz_media::config::S3AddressingStyle::Virtual);
        assert!(matches!(
            invalid,
            Err(ConfigError::InvalidValue(ref message))
                if message.contains("BUZZ_S3_ADDRESSING_STYLE must be 'path' or 'virtual'")
        ));
    }

    #[cfg(unix)]
    #[test]
    fn s3_addressing_style_env_rejects_non_unicode_values() {
        use std::os::unix::ffi::OsStringExt;

        let _guard = ENV_MUTEX.lock().unwrap();
        let previous = std::env::var_os("BUZZ_S3_ADDRESSING_STYLE");
        std::env::set_var(
            "BUZZ_S3_ADDRESSING_STYLE",
            std::ffi::OsString::from_vec(vec![0xff]),
        );

        let invalid = Config::from_env();

        if let Some(value) = previous {
            std::env::set_var("BUZZ_S3_ADDRESSING_STYLE", value);
        } else {
            std::env::remove_var("BUZZ_S3_ADDRESSING_STYLE");
        }

        assert!(matches!(
            invalid,
            Err(ConfigError::InvalidValue(ref message))
                if message.contains("must be valid Unicode")
        ));
    }

    #[test]
    fn redis_pool_size_env_override_and_invalid_fallback() {
        let _guard = ENV_MUTEX.lock().unwrap();
        let previous = std::env::var_os("BUZZ_REDIS_POOL_SIZE");

        std::env::set_var("BUZZ_REDIS_POOL_SIZE", "32");
        let overridden = Config::from_env().expect("config").redis_pool_size;

        std::env::set_var("BUZZ_REDIS_POOL_SIZE", "0");
        let zero = Config::from_env().expect("config").redis_pool_size;

        std::env::set_var("BUZZ_REDIS_POOL_SIZE", "not-a-number");
        let junk = Config::from_env().expect("config").redis_pool_size;

        if let Some(value) = previous {
            std::env::set_var("BUZZ_REDIS_POOL_SIZE", value);
        } else {
            std::env::remove_var("BUZZ_REDIS_POOL_SIZE");
        }

        assert_eq!(overridden, 32);
        assert_eq!(zero, 16, "zero must fall back to the default");
        assert_eq!(junk, 16, "unparsable value must fall back to the default");
    }

    #[test]
    fn db_pool_size_env_override_and_invalid_fallback() {
        let _guard = ENV_MUTEX.lock().unwrap();
        let previous = std::env::var_os("BUZZ_DB_POOL_SIZE");

        std::env::set_var("BUZZ_DB_POOL_SIZE", "80");
        let overridden = Config::from_env().expect("config").db_pool_size;

        std::env::set_var("BUZZ_DB_POOL_SIZE", "0");
        let zero = Config::from_env().expect("config").db_pool_size;

        std::env::set_var("BUZZ_DB_POOL_SIZE", "not-a-number");
        let junk = Config::from_env().expect("config").db_pool_size;

        if let Some(value) = previous {
            std::env::set_var("BUZZ_DB_POOL_SIZE", value);
        } else {
            std::env::remove_var("BUZZ_DB_POOL_SIZE");
        }

        assert_eq!(overridden, 80);
        assert_eq!(zero, 50, "zero must fall back to the default");
        assert_eq!(junk, 50, "unparsable value must fall back to the default");
    }

    #[test]
    fn db_read_pool_size_env_override_and_invalid_fallback() {
        let _guard = ENV_MUTEX.lock().unwrap();
        let previous = std::env::var_os("BUZZ_DB_READ_POOL_SIZE");

        std::env::remove_var("BUZZ_DB_READ_POOL_SIZE");
        let unset = Config::from_env().expect("config").db_read_pool_size;

        std::env::set_var("BUZZ_DB_READ_POOL_SIZE", "40");
        let overridden = Config::from_env().expect("config").db_read_pool_size;

        std::env::set_var("BUZZ_DB_READ_POOL_SIZE", "0");
        let zero = Config::from_env().expect("config").db_read_pool_size;

        std::env::set_var("BUZZ_DB_READ_POOL_SIZE", "not-a-number");
        let junk = Config::from_env().expect("config").db_read_pool_size;

        if let Some(value) = previous {
            std::env::set_var("BUZZ_DB_READ_POOL_SIZE", value);
        } else {
            std::env::remove_var("BUZZ_DB_READ_POOL_SIZE");
        }

        assert_eq!(unset, None, "unset must inherit the writer pool sizing");
        assert_eq!(overridden, Some(40));
        assert_eq!(zero, None, "zero must fall back to inheriting");
        assert_eq!(junk, None, "unparsable value must fall back to inheriting");
    }

    #[test]
    fn read_database_url_unset_or_blank_is_none() {
        let _guard = ENV_MUTEX.lock().unwrap();
        let previous = std::env::var_os("READ_DATABASE_URL");

        std::env::remove_var("READ_DATABASE_URL");
        let unset = Config::from_env().expect("config").read_database_url;

        std::env::set_var("READ_DATABASE_URL", "   ");
        let blank = Config::from_env().expect("config").read_database_url;

        std::env::set_var("READ_DATABASE_URL", "postgres://buzz:pw@replica:5432/buzz"); // sadscan:disable np.postgres.1
        let set = Config::from_env().expect("config").read_database_url;

        if let Some(value) = previous {
            std::env::set_var("READ_DATABASE_URL", value);
        } else {
            std::env::remove_var("READ_DATABASE_URL");
        }

        assert_eq!(unset, None, "unset READ_DATABASE_URL must disable routing");
        assert_eq!(blank, None, "blank READ_DATABASE_URL must disable routing");
        assert_eq!(
            set.as_deref(),
            Some("postgres://buzz:pw@replica:5432/buzz") // sadscan:disable np.postgres.1
        );
    }

    #[test]
    fn replica_read_max_age_defaults_off_and_rejects_junk() {
        let _guard = ENV_MUTEX.lock().unwrap();
        let previous = std::env::var_os("BUZZ_REPLICA_READ_MAX_AGE_MS");
        let previous_old = std::env::var_os("BUZZ_REPLICA_HEAD_MAX_AGE_SECS");
        std::env::remove_var("BUZZ_REPLICA_HEAD_MAX_AGE_SECS");

        std::env::remove_var("BUZZ_REPLICA_READ_MAX_AGE_MS");
        let unset = Config::from_env().expect("config").replica_read_max_age_ms;

        std::env::set_var("BUZZ_REPLICA_READ_MAX_AGE_MS", "1000");
        let set = Config::from_env().expect("config").replica_read_max_age_ms;

        std::env::set_var("BUZZ_REPLICA_READ_MAX_AGE_MS", "0");
        let zero = Config::from_env().expect("config").replica_read_max_age_ms;

        std::env::set_var("BUZZ_REPLICA_READ_MAX_AGE_MS", "soon");
        let junk = Config::from_env();

        // The retired seconds-denominated name must be a hard startup
        // error even alongside a valid new-name value: silently ignoring
        // it (or honouring it) would mean 1000x the intended budget.
        std::env::set_var("BUZZ_REPLICA_READ_MAX_AGE_MS", "1000");
        std::env::set_var("BUZZ_REPLICA_HEAD_MAX_AGE_SECS", "5");
        let old_name = Config::from_env();

        std::env::remove_var("BUZZ_REPLICA_HEAD_MAX_AGE_SECS");
        if let Some(value) = previous {
            std::env::set_var("BUZZ_REPLICA_READ_MAX_AGE_MS", value);
        } else {
            std::env::remove_var("BUZZ_REPLICA_READ_MAX_AGE_MS");
        }
        if let Some(value) = previous_old {
            std::env::set_var("BUZZ_REPLICA_HEAD_MAX_AGE_SECS", value);
        }

        assert_eq!(unset, 0, "replica read routing must default off");
        assert_eq!(set, 1000);
        assert_eq!(zero, 0, "explicit 0 is off");
        assert!(
            junk.is_err(),
            "an unparsable budget must fail loudly, not silently disable"
        );
        match old_name {
            Err(ConfigError::InvalidValue(message)) => assert!(
                message.contains("BUZZ_REPLICA_READ_MAX_AGE_MS"),
                "the error must name the replacement env var, got: {message}"
            ),
            other => panic!("old env name must hard-fail startup, got {other:?}"),
        }
    }

    #[test]
    fn drain_jitter_defaults_off_and_rejects_junk() {
        let _guard = ENV_MUTEX.lock().unwrap();
        let previous = std::env::var_os("BUZZ_DRAIN_JITTER_MS");

        std::env::remove_var("BUZZ_DRAIN_JITTER_MS");
        let unset = Config::from_env().expect("config").drain_jitter_ms;

        std::env::set_var("BUZZ_DRAIN_JITTER_MS", "20000");
        let set = Config::from_env().expect("config").drain_jitter_ms;

        std::env::set_var("BUZZ_DRAIN_JITTER_MS", "60000");
        let capped = Config::from_env().expect("config").drain_jitter_ms;

        std::env::set_var("BUZZ_DRAIN_JITTER_MS", "0");
        let zero = Config::from_env().expect("config").drain_jitter_ms;

        std::env::set_var("BUZZ_DRAIN_JITTER_MS", "soon");
        let junk = Config::from_env();

        std::env::set_var("BUZZ_DRAIN_JITTER_MS", "");
        let empty = Config::from_env()
            .expect("empty is a valid kill switch")
            .drain_jitter_ms;

        std::env::set_var("BUZZ_DRAIN_JITTER_MS", "   ");
        let blank = Config::from_env()
            .expect("whitespace-only is a valid kill switch")
            .drain_jitter_ms;

        if let Some(value) = previous {
            std::env::set_var("BUZZ_DRAIN_JITTER_MS", value);
        } else {
            std::env::remove_var("BUZZ_DRAIN_JITTER_MS");
        }

        assert_eq!(unset, 0, "drain jitter must default off");
        assert_eq!(set, MAX_DRAIN_JITTER_MS);
        assert_eq!(
            capped, MAX_DRAIN_JITTER_MS,
            "oversized jitter leaves close-frame flush headroom"
        );
        assert_eq!(zero, 0, "explicit 0 is off");
        assert!(
            junk.is_err(),
            "an unparsable jitter must fail loudly, not silently disable"
        );
        assert_eq!(
            empty, 0,
            "an empty value is treated as unset — a kill switch, not a crashloop"
        );
        assert_eq!(blank, 0, "a whitespace-only value is treated as unset");
    }

    #[test]
    fn audit_logging_defaults_on_and_accepts_explicit_off() {
        let _guard = ENV_MUTEX.lock().unwrap();
        let previous = std::env::var_os("BUZZ_AUDIT_ENABLED");
        std::env::remove_var("BUZZ_AUDIT_ENABLED");
        assert!(parse_bool("BUZZ_AUDIT_ENABLED", true).unwrap());
        std::env::set_var("BUZZ_AUDIT_ENABLED", "false");
        assert!(!parse_bool("BUZZ_AUDIT_ENABLED", true).unwrap());
        if let Some(value) = previous {
            std::env::set_var("BUZZ_AUDIT_ENABLED", value);
        } else {
            std::env::remove_var("BUZZ_AUDIT_ENABLED");
        }
    }

    #[test]
    fn audit_logging_rejects_invalid_boolean() {
        let _guard = ENV_MUTEX.lock().unwrap();
        let previous = std::env::var_os("BUZZ_AUDIT_ENABLED");
        std::env::set_var("BUZZ_AUDIT_ENABLED", "sometimes");
        let result = parse_bool("BUZZ_AUDIT_ENABLED", true);
        if let Some(value) = previous {
            std::env::set_var("BUZZ_AUDIT_ENABLED", value);
        } else {
            std::env::remove_var("BUZZ_AUDIT_ENABLED");
        }
        assert!(matches!(
            result,
            Err(ConfigError::InvalidValue(ref message))
                if message.contains("BUZZ_AUDIT_ENABLED")
        ));
    }

    #[test]
    fn join_policy_age_attestation_rejects_invalid_boolean() {
        let _guard = ENV_MUTEX.lock().unwrap();
        let previous = std::env::var_os("BUZZ_AGE_ATTESTATION_REQUIRED");
        std::env::set_var("BUZZ_AGE_ATTESTATION_REQUIRED", "sometimes");
        let result = parse_optional_bool("BUZZ_AGE_ATTESTATION_REQUIRED");
        if let Some(value) = previous {
            std::env::set_var("BUZZ_AGE_ATTESTATION_REQUIRED", value);
        } else {
            std::env::remove_var("BUZZ_AGE_ATTESTATION_REQUIRED");
        }
        assert!(matches!(
            result,
            Err(ConfigError::InvalidValue(ref message))
                if message.contains("BUZZ_AGE_ATTESTATION_REQUIRED")
        ));
    }

    #[test]
    fn rate_limits_can_be_overridden() {
        let _guard = ENV_MUTEX.lock().unwrap();
        std::env::set_var("BUZZ_RATE_LIMIT_HUMAN_MESSAGES_PER_MIN", "1001");
        std::env::set_var("BUZZ_RATE_LIMIT_HUMAN_API_CALLS_PER_MIN", "1002");
        std::env::set_var("BUZZ_RATE_LIMIT_HUMAN_WS_EVENTS_PER_SEC", "1003");

        let config = Config::from_env().expect("config");

        std::env::remove_var("BUZZ_RATE_LIMIT_HUMAN_MESSAGES_PER_MIN");
        std::env::remove_var("BUZZ_RATE_LIMIT_HUMAN_API_CALLS_PER_MIN");
        std::env::remove_var("BUZZ_RATE_LIMIT_HUMAN_WS_EVENTS_PER_SEC");
        assert_eq!(config.auth.rate_limits.human_messages_per_min, 1001);
        assert_eq!(config.auth.rate_limits.human_api_calls_per_min, 1002);
        assert_eq!(config.auth.rate_limits.human_ws_events_per_sec, 1003);
    }

    #[test]
    fn rate_limit_overrides_reject_zero() {
        let _guard = ENV_MUTEX.lock().unwrap();
        std::env::set_var("BUZZ_RATE_LIMIT_HUMAN_WS_EVENTS_PER_SEC", "0");
        let result = Config::from_env();
        std::env::remove_var("BUZZ_RATE_LIMIT_HUMAN_WS_EVENTS_PER_SEC");

        assert!(matches!(
            result,
            Err(ConfigError::InvalidValue(ref message))
                if message.contains("BUZZ_RATE_LIMIT_HUMAN_WS_EVENTS_PER_SEC")
        ));
    }

    #[test]
    fn relay_operator_pubkeys_parse_dedupe_and_normalize() {
        let _guard = ENV_MUTEX.lock().unwrap();
        std::env::set_var(
            "RELAY_OPERATOR_PUBKEYS",
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA,bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb,aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        );
        std::env::set_var(
            "RELAY_OPERATOR_API_ORIGIN",
            "http://buzz.mesh.bb-production.com",
        );
        let config = Config::from_env().expect("config");
        std::env::remove_var("RELAY_OPERATOR_PUBKEYS");
        std::env::remove_var("RELAY_OPERATOR_API_ORIGIN");

        assert_eq!(
            config.relay_operator_pubkeys,
            vec![
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
            ]
        );
    }

    #[test]
    fn relay_operator_pubkeys_invalid_entry_is_error() {
        let _guard = ENV_MUTEX.lock().unwrap();
        std::env::set_var("RELAY_OPERATOR_PUBKEYS", "not-a-pubkey");
        let result = Config::from_env();
        std::env::remove_var("RELAY_OPERATOR_PUBKEYS");

        assert!(matches!(
            result,
            Err(ConfigError::InvalidValue(ref msg)) if msg.contains("RELAY_OPERATOR_PUBKEYS")
        ));
    }

    #[test]
    fn relay_operator_pubkeys_require_api_origin() {
        let _guard = ENV_MUTEX.lock().unwrap();
        std::env::set_var(
            "RELAY_OPERATOR_PUBKEYS",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        );
        std::env::remove_var("RELAY_OPERATOR_API_ORIGIN");
        let result = Config::from_env();
        std::env::remove_var("RELAY_OPERATOR_PUBKEYS");

        assert!(matches!(
            result,
            Err(ConfigError::InvalidValue(ref msg)) if msg.contains("RELAY_OPERATOR_API_ORIGIN is required")
        ));
    }

    #[test]
    fn relay_operator_api_origin_rejects_paths() {
        let _guard = ENV_MUTEX.lock().unwrap();
        std::env::set_var("RELAY_OPERATOR_API_ORIGIN", "https://buzz.example/operator");
        let result = Config::from_env();
        std::env::remove_var("RELAY_OPERATOR_API_ORIGIN");

        assert!(matches!(
            result,
            Err(ConfigError::InvalidValue(ref msg)) if msg.contains("must be an http(s) origin")
        ));
    }

    #[test]
    fn push_gateway_defaults_to_buzz_and_can_be_disabled() {
        let _guard = ENV_MUTEX.lock().unwrap();
        let previous = std::env::var_os("BUZZ_PUSH_GATEWAY_DELIVERY_URL");
        std::env::remove_var("BUZZ_PUSH_GATEWAY_DELIVERY_URL");
        let config = Config::from_env().expect("default config");
        assert_eq!(
            config
                .push_gateway_delivery_url
                .as_ref()
                .map(url::Url::as_str),
            Some(DEFAULT_PUSH_GATEWAY_DELIVERY_URL)
        );

        std::env::set_var("BUZZ_PUSH_GATEWAY_DELIVERY_URL", "");
        let config = Config::from_env().expect("disabled push config");
        assert!(config.push_gateway_delivery_url.is_none());

        if let Some(value) = previous {
            std::env::set_var("BUZZ_PUSH_GATEWAY_DELIVERY_URL", value);
        } else {
            std::env::remove_var("BUZZ_PUSH_GATEWAY_DELIVERY_URL");
        }
    }

    #[test]
    fn push_gateway_url_is_exact_and_fail_closed() {
        assert!(parse_push_gateway_delivery_url("https://push.example/v1/deliveries/apns").is_ok());
        for invalid in [
            "http://push.example/v1/deliveries/apns",
            "https://push.example/v1/deliveries/apns/",
            "https://push.example/v1/deliveries/apns?token=x",
            "https://user@push.example/v1/deliveries/apns",
        ] {
            assert!(
                parse_push_gateway_delivery_url(invalid).is_err(),
                "{invalid}"
            );
        }
    }

    #[test]
    fn invalid_push_gateway_timeout_is_not_silently_defaulted() {
        let _guard = ENV_MUTEX.lock().unwrap();
        std::env::set_var("BUZZ_PUSH_GATEWAY_TIMEOUT_MS", "99");
        let result = Config::from_env();
        std::env::remove_var("BUZZ_PUSH_GATEWAY_TIMEOUT_MS");
        assert!(matches!(
            result,
            Err(ConfigError::InvalidValue(ref message))
                if message.contains("BUZZ_PUSH_GATEWAY_TIMEOUT_MS")
        ));
    }

    #[test]
    fn invalid_push_executor_key_id_is_rejected() {
        let _guard = ENV_MUTEX.lock().unwrap();
        std::env::set_var("BUZZ_PUSH_EXECUTOR_KEY_ID", "");
        let result = Config::from_env();
        std::env::remove_var("BUZZ_PUSH_EXECUTOR_KEY_ID");
        assert!(matches!(
            result,
            Err(ConfigError::InvalidValue(ref message))
                if message.contains("BUZZ_PUSH_EXECUTOR_KEY_ID")
        ));
    }

    #[test]
    fn huddle_audio_available_can_be_disabled_for_horizontal_scaling() {
        let _guard = ENV_MUTEX.lock().unwrap();
        std::env::set_var("BUZZ_HUDDLE_AUDIO_AVAILABLE", "false");
        let config = Config::from_env().expect("config");
        std::env::remove_var("BUZZ_HUDDLE_AUDIO_AVAILABLE");
        assert!(
            !config.huddle_audio_available,
            "BUZZ_HUDDLE_AUDIO_AVAILABLE=false must disable huddle audio (multi-pod deployments)"
        );
    }

    #[test]
    fn invalid_bind_addr_returns_error() {
        assert!(matches!(
            parse_bind_addr("not-an-addr"),
            Err(ConfigError::InvalidBindAddr(_))
        ));
    }

    #[test]
    fn pairing_relay_url_accepts_websocket_urls_and_rejects_http() {
        let _guard = ENV_MUTEX.lock().unwrap();
        std::env::set_var("BUZZ_PAIRING_RELAY_URL", "wss://pairing.buzz.xyz");
        let config = Config::from_env().expect("config");
        assert_eq!(
            config.pairing_relay_url.as_deref(),
            Some("wss://pairing.buzz.xyz")
        );

        std::env::set_var("BUZZ_PAIRING_RELAY_URL", "https://pairing.buzz.xyz");
        let result = Config::from_env();
        std::env::remove_var("BUZZ_PAIRING_RELAY_URL");
        assert!(matches!(
            result,
            Err(ConfigError::InvalidValue(ref msg)) if msg.contains("BUZZ_PAIRING_RELAY_URL")
        ));
    }

    #[test]
    fn max_frame_bytes_can_be_configured() {
        let _guard = ENV_MUTEX.lock().unwrap();
        std::env::set_var("BUZZ_MAX_FRAME_BYTES", "262144");
        let config = Config::from_env().expect("config");
        std::env::remove_var("BUZZ_MAX_FRAME_BYTES");
        assert_eq!(config.max_frame_bytes, 262_144);
    }

    #[test]
    fn git_repo_path_is_created_if_missing() {
        let _guard = ENV_MUTEX.lock().unwrap();
        // Pick a path under temp_dir that definitely doesn't exist yet.
        let base = std::env::temp_dir().join(format!(
            "buzz-test-git-repo-path-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let nested = base.join("nested").join("repos");
        assert!(!nested.exists(), "test precondition: path must not exist");

        std::env::set_var("BUZZ_GIT_REPO_PATH", &nested);
        let result = Config::from_env();
        std::env::remove_var("BUZZ_GIT_REPO_PATH");

        let config = result.expect("config should self-bootstrap missing git_repo_path");
        assert_eq!(config.git_repo_path, nested);
        assert!(
            nested.is_dir(),
            "git_repo_path should exist after config load"
        );

        // Cleanup.
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    #[cfg(unix)]
    fn git_repo_path_unwritable_returns_error() {
        // Try to create a path under a regular file — must fail.
        // Using /dev/null as the parent guarantees create_dir_all fails on unix.
        let bogus = std::path::PathBuf::from("/dev/null/cannot-create-here");
        let result = ensure_git_repo_path(&bogus);
        assert!(
            matches!(result, Err(ConfigError::InvalidValue(ref msg)) if msg.contains("BUZZ_GIT_REPO_PATH")),
            "expected InvalidValue mentioning BUZZ_GIT_REPO_PATH, got {result:?}"
        );
    }
}
