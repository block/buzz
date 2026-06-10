//! Relay configuration from environment variables.

use std::net::SocketAddr;

use thiserror::Error;
use tracing::warn;

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
    /// Typesense search server URL.
    pub typesense_url: String,
    /// Typesense API key.
    pub typesense_key: String,
    /// Public WebSocket URL of this relay, advertised in NIP-11.
    pub relay_url: String,
    /// Maximum number of concurrent WebSocket connections.
    pub max_connections: usize,
    /// Maximum number of concurrently executing message handlers.
    pub max_concurrent_handlers: usize,
    /// Per-connection outbound message buffer size (number of messages).
    pub send_buffer_size: usize,
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

    /// Optional hex-encoded pubkey of the relay owner.
    /// When set, this pubkey is automatically bootstrapped into `relay_members`
    /// with the `owner` role on first startup.
    pub relay_owner_pubkey: Option<String>,

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

    /// Optional override for ephemeral channel TTL (in seconds).
    /// When set, any channel created with a TTL tag will use this value instead
    /// of the client-provided one. Useful for testing ephemeral expiry quickly.
    /// Example: `BUZZ_EPHEMERAL_TTL_OVERRIDE=60` → all ephemeral channels expire
    /// 60 seconds after the last message.
    pub ephemeral_ttl_override: Option<i32>,

    // ── Git server configuration ─────────────────────────────────────────────
    /// Root directory for the relay's local git state. No per-repo bare repos
    /// live here — runtime reads/writes hydrate ephemeral repos from object
    /// storage. Holds only the name-reservation index at `{git_repo_path}/.names/`.
    pub git_repo_path: std::path::PathBuf,
    /// Maximum pack file size for git push (bytes). Default: 500 MB.
    pub git_max_pack_bytes: u64,
    /// Maximum number of repos per pubkey. Default: 100.
    pub git_max_repos_per_pubkey: u32,
    /// Maximum concurrent git subprocess operations. Default: 20.
    pub git_max_concurrent_ops: usize,
    /// HMAC secret for git pre-receive hook callbacks.
    /// Used to authenticate internal policy endpoint requests.
    pub git_hook_hmac_secret: String,

    // ── Web UI serving ────────────────────────────────────────────────────────
    /// Optional path to the web UI `dist/` directory.
    /// When set, the relay serves the SPA from this directory for browser requests.
    /// When unset, no static file serving happens (relay behaves as before).
    pub web_dir: Option<std::path::PathBuf>,
}

impl Config {
    /// Loads configuration from environment variables, falling back to development defaults.
    pub fn from_env() -> Result<Self, ConfigError> {
        let bind_addr = std::env::var("BUZZ_BIND_ADDR")
            .unwrap_or_else(|_| "0.0.0.0:3000".to_string())
            .parse::<SocketAddr>()
            .map_err(|e| ConfigError::InvalidBindAddr(e.to_string()))?;

        let database_url = std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://buzz:buzz_dev@localhost:5432/buzz".to_string());

        let redis_url =
            std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://localhost:6379".to_string());

        let typesense_url =
            std::env::var("TYPESENSE_URL").unwrap_or_else(|_| "http://localhost:8108".to_string());

        let typesense_key =
            std::env::var("TYPESENSE_API_KEY").unwrap_or_else(|_| "buzz_dev_key".to_string());

        let relay_url =
            std::env::var("RELAY_URL").unwrap_or_else(|_| "ws://localhost:3000".to_string());

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

        let require_auth_token = std::env::var("BUZZ_REQUIRE_AUTH_TOKEN")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false);

        let pubkey_allowlist_enabled = std::env::var("BUZZ_PUBKEY_ALLOWLIST")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false);

        let require_relay_membership = std::env::var("BUZZ_REQUIRE_RELAY_MEMBERSHIP")
            .map(|v| v == "true" || v == "1")
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
            server_domain: std::env::var("BUZZ_MEDIA_SERVER_DOMAIN")
                .ok()
                .filter(|s| !s.is_empty())
                .or_else(|| {
                    // Auto-derive from RELAY_URL so desktop uploads work out-of-the-box
                    // without requiring an extra env var in dev mode.
                    url::Url::parse(
                        &relay_url
                            .replace("ws://", "http://")
                            .replace("wss://", "https://"),
                    )
                    .ok()
                    .and_then(|u| {
                        let host = u.host_str()?.to_string();
                        match u.port() {
                            Some(p) => Some(format!("{host}:{p}")),
                            None => Some(host),
                        }
                    })
                }),
        };

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
        let git_repo_path: std::path::PathBuf = std::env::var("BUZZ_GIT_REPO_PATH")
            .unwrap_or_else(|_| "./repos".to_string())
            .into();
        // Ensure the git repo root exists. The smart-HTTP transport and the
        // kind:30617 side-effect handler both canonicalize this path; if it's
        // missing, all git operations 500 with "git service misconfigured" and
        // repo announcements silently fail to create their bare repo on disk.
        // Bootstrapping here makes the relay self-provision its own data dir
        // (matches how we treat other relay-owned paths) rather than requiring
        // ops to mkdir it out of band.
        if let Err(e) = std::fs::create_dir_all(&git_repo_path) {
            return Err(ConfigError::InvalidValue(format!(
                "BUZZ_GIT_REPO_PATH={} could not be created: {e}",
                git_repo_path.display()
            )));
        }
        let git_max_pack_bytes: u64 = std::env::var("BUZZ_GIT_MAX_PACK_BYTES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(500 * 1024 * 1024); // 500 MB
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
            typesense_url,
            typesense_key,
            relay_url,
            max_connections,
            max_concurrent_handlers,
            send_buffer_size,
            auth,
            require_auth_token,
            cors_origins,
            relay_private_key,
            uds_path,
            health_port,
            metrics_port,
            pubkey_allowlist_enabled,
            require_relay_membership,
            relay_owner_pubkey,
            allow_nip_oa_auth,
            media,
            ephemeral_ttl_override,
            git_repo_path,
            git_max_pack_bytes,
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
            !config.allow_nip_oa_auth,
            "allow_nip_oa_auth should default to false"
        );
    }

    #[test]
    fn invalid_bind_addr_returns_error() {
        let _guard = ENV_MUTEX.lock().unwrap();
        std::env::set_var("BUZZ_BIND_ADDR", "not-an-addr");
        let result = Config::from_env();
        std::env::remove_var("BUZZ_BIND_ADDR");
        assert!(matches!(result, Err(ConfigError::InvalidBindAddr(_))));
    }

    #[test]
    fn server_domain_auto_derived_from_relay_url() {
        let _guard = ENV_MUTEX.lock().unwrap();
        // Clear explicit override so auto-derive kicks in
        std::env::remove_var("BUZZ_MEDIA_SERVER_DOMAIN");
        std::env::set_var("RELAY_URL", "ws://localhost:3000");
        let config = Config::from_env().expect("config");
        std::env::remove_var("RELAY_URL");
        assert_eq!(
            config.media.server_domain.as_deref(),
            Some("localhost:3000")
        );
    }

    #[test]
    fn server_domain_auto_derived_default_port() {
        let _guard = ENV_MUTEX.lock().unwrap();
        std::env::remove_var("BUZZ_MEDIA_SERVER_DOMAIN");
        std::env::set_var("RELAY_URL", "wss://relay.example.com");
        let config = Config::from_env().expect("config");
        std::env::remove_var("RELAY_URL");
        assert_eq!(
            config.media.server_domain.as_deref(),
            Some("relay.example.com")
        );
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
        let _guard = ENV_MUTEX.lock().unwrap();
        // Try to create a path under a regular file — must fail.
        // Using /dev/null as the parent guarantees create_dir_all fails on unix.
        let bogus = std::path::PathBuf::from("/dev/null/cannot-create-here");
        std::env::set_var("BUZZ_GIT_REPO_PATH", &bogus);
        let result = Config::from_env();
        std::env::remove_var("BUZZ_GIT_REPO_PATH");
        assert!(
            matches!(result, Err(ConfigError::InvalidValue(ref msg)) if msg.contains("BUZZ_GIT_REPO_PATH")),
            "expected InvalidValue mentioning BUZZ_GIT_REPO_PATH, got {result:?}"
        );
    }

    #[test]
    fn server_domain_explicit_override_wins() {
        let _guard = ENV_MUTEX.lock().unwrap();
        std::env::set_var("BUZZ_MEDIA_SERVER_DOMAIN", "custom.example.com");
        std::env::set_var("RELAY_URL", "ws://localhost:3000");
        let config = Config::from_env().expect("config");
        std::env::remove_var("BUZZ_MEDIA_SERVER_DOMAIN");
        std::env::remove_var("RELAY_URL");
        assert_eq!(
            config.media.server_domain.as_deref(),
            Some("custom.example.com")
        );
    }
}
