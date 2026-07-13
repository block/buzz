//! Relay configuration from environment variables.

use std::net::SocketAddr;

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

/// Relay runtime configuration, loaded from environment variables.
#[derive(Debug, Clone)]
pub struct Config {
    /// Address the relay HTTP/WebSocket server binds to.
    pub bind_addr: SocketAddr,
    /// Postgres database connection URL.
    pub database_url: String,
    /// Redis connection URL used by the pub/sub manager.
    pub redis_url: String,
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

    /// Optional override for ephemeral channel TTL (in seconds).
    /// When set, any channel created with a TTL tag will use this value instead
    /// of the client-provided one. Useful for testing ephemeral expiry quickly.
    /// Example: `BUZZ_EPHEMERAL_TTL_OVERRIDE=60` → all ephemeral channels expire
    /// 60 seconds after the last message.
    pub ephemeral_ttl_override: Option<i32>,

    /// Root directory for the relay's local git scratch. No per-repo bare repos
    /// or persistent git state live here — runtime reads/writes hydrate
    /// ephemeral repos from object storage per request, and repo-name
    /// uniqueness now lives in Postgres (`git_repo_names`), not on disk. Retained
    /// for ephemeral working space and env compatibility; the relay no longer
    /// depends on this path being persistent or shared across replicas, so it
    /// needs no ReadWriteMany volume. (Removing the field entirely is a
    /// follow-up cleanup once the deploy chart drops the git PVC mount.)
    pub git_repo_path: std::path::PathBuf,
    /// Maximum pack file size for git push (bytes). Default: 500 MB.
    pub git_max_pack_bytes: u64,
    /// Maximum total bytes materialized for one git repo request. Default: 1 GB.
    ///
    /// This bounds clone/fetch hydration work across a repo's historical pack
    /// set rather than only bounding one incoming push body.
    pub git_max_repo_bytes: u64,
    /// Maximum number of repos per pubkey. Default: 100.
    pub git_max_repos_per_pubkey: u32,
    /// Maximum concurrent git subprocess operations. Default: 20.
    pub git_max_concurrent_ops: usize,
    /// HMAC secret for git pre-receive hook callbacks.
    /// Used to authenticate internal policy endpoint requests.
    pub git_hook_hmac_secret: String,

    /// Optional path to the web UI `dist/` directory.
    /// When set, the relay serves the SPA from this directory for browser requests.
    /// When unset, no static file serving happens (relay behaves as before).
    pub web_dir: Option<std::path::PathBuf>,
}

fn parse_bind_addr(raw: &str) -> Result<SocketAddr, ConfigError> {
    raw.parse::<SocketAddr>()
        .map_err(|e| ConfigError::InvalidBindAddr(e.to_string()))
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

fn ensure_git_repo_path(
    raw: impl Into<std::path::PathBuf>,
) -> Result<std::path::PathBuf, ConfigError> {
    let git_repo_path = raw.into();
    if let Err(e) = std::fs::create_dir_all(&git_repo_path) {
        return Err(ConfigError::InvalidValue(format!(
            "BUZZ_GIT_REPO_PATH={} could not be created: {e}",
            git_repo_path.display()
        )));
    }
    Ok(git_repo_path)
}

impl Config {
    /// Loads configuration from environment variables, falling back to development defaults.
    pub fn from_env() -> Result<Self, ConfigError> {
        let bind_addr_raw =
            std::env::var("BUZZ_BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:3000".to_string());
        let bind_addr = parse_bind_addr(&bind_addr_raw)?;

        let database_url = std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://buzz:buzz_dev@localhost:5432/buzz".to_string()); // sadscan:disable np.postgres.1

        let redis_url =
            std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://localhost:6379".to_string());

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

        let allow_nip_oa_auth = std::env::var("BUZZ_ALLOW_NIP_OA_AUTH")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false);

        // Note: intentionally not prefixed with BUZZ_ — this is a relay-identity
        // config that may be shared across multiple services (e.g., ACP agent).
        let relay_owner_pubkey = std::env::var("RELAY_OWNER_PUBKEY")
            .ok()
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .and_then(|s| {
                // Must be exactly 64 lowercase hex characters (32-byte pubkey).
                let valid = s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit());
                if valid {
                    Some(s)
                } else {
                    warn!(
                        "RELAY_OWNER_PUBKEY is not a valid 64-char hex pubkey — ignoring. \
                         Got: {s:?}"
                    );
                    None
                }
            });

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

        let auth = buzz_auth::AuthConfig::default();

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
        let git_max_pack_bytes: u64 = std::env::var("BUZZ_GIT_MAX_PACK_BYTES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(500 * 1024 * 1024); // 500 MB
        let git_max_repo_bytes: u64 = std::env::var("BUZZ_GIT_MAX_REPO_BYTES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or_else(|| git_max_pack_bytes.saturating_mul(2)); // 1 GB at defaults
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
        // Web UI static file serving
        let web_dir = std::env::var("BUZZ_WEB_DIR")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .map(std::path::PathBuf::from);

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
            redis_url,
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
            relay_owner_pubkey,
            relay_operator_api_origin,
            relay_operator_pubkeys,
            allow_nip_oa_auth,
            media,
            media_max_concurrent_uploads,
            media_max_concurrent_uploads_per_pubkey,
            media_uploads_per_minute,
            ephemeral_ttl_override,
            git_repo_path,
            git_max_pack_bytes,
            git_max_repo_bytes,
            git_max_repos_per_pubkey,
            git_max_concurrent_ops,
            git_hook_hmac_secret,
            web_dir,
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

    #[test]
    fn defaults_are_valid() {
        let _guard = ENV_MUTEX.lock().unwrap();
        let config = Config::from_env().expect("default config");
        assert!(config.bind_addr.port() > 0);
        assert!(!config.database_url.is_empty());
        assert!(!config.redis_url.is_empty());
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
            config.huddle_audio_available,
            "huddle_audio_available should default to true so single-pod (N=1) keeps today's huddle behavior"
        );
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
