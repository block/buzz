//! `buzz-migrate` — the operator claim-service binary.
//!
//! Loads the Slack export roster, holds the operator's admin key, and serves
//! the claim HTTP surface. Its Slack OIDC path admits verified workspace
//! members and automates the owner/admin attestation half of a two-party import
//! identity binding. See the crate docs for the model.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use buzz_migrate::oidc::OidcConfig;
use buzz_migrate::roster::Roster;
use buzz_migrate::server::{router, AppState, Inner, Mailer};
use buzz_migrate::token::ConsumedNonces;
use clap::Parser;
use nostr::Keys;

/// Operator claim-service for Slack→Buzz identity migration.
#[derive(Parser, Debug)]
#[command(name = "buzz-migrate", version, about)]
struct Args {
    /// Relay base URL (http/https/ws/wss). The admin key must be a community
    /// owner or admin on this relay.
    #[arg(long, env = "BUZZ_RELAY_URL", default_value = "http://localhost:3000")]
    relay_url: String,

    /// Operator admin private key (hex or nsec). Used only to sign attestations.
    #[arg(long, env = "BUZZ_PRIVATE_KEY")]
    admin_key: String,

    /// NIP-OA auth tag JSON (community membership delegation), if the relay
    /// requires one.
    #[arg(long, env = "BUZZ_AUTH_TAG")]
    auth_tag: Option<String>,

    /// Unzipped Slack export directory (must contain users.json).
    #[arg(long)]
    export_dir: PathBuf,

    /// Address to bind the HTTP service to.
    #[arg(long, default_value = "127.0.0.1:8787")]
    bind: String,

    /// Public base URL of this service, used in email and OIDC callbacks.
    /// Defaults to `http://<bind>`.
    #[arg(long)]
    base_url: Option<String>,

    /// Hex secret (>=32 bytes recommended) that signs magic-link tokens. If
    /// omitted, a random one is generated — fine for a single run, but tokens
    /// minted before a restart stop verifying. Set it to survive restarts.
    #[arg(long, env = "BUZZ_MIGRATE_TOKEN_SECRET")]
    token_secret: Option<String>,

    /// Magic-link token lifetime in seconds (default 72h).
    #[arg(long, default_value_t = 72 * 3600)]
    token_ttl_secs: u64,

    /// Slack OIDC client id (enables the Sign-in-with-Slack channel).
    #[arg(long, env = "SLACK_CLIENT_ID")]
    slack_client_id: Option<String>,

    /// Slack OIDC client secret.
    #[arg(long, env = "SLACK_CLIENT_SECRET")]
    slack_client_secret: Option<String>,

    /// Slack workspace id (team id, e.g. T0266FRGM) whose users may claim
    /// imported identities. Namespaces every `slack:<team>:<user>` subject so
    /// ids can't collide across workspaces — required on both channels.
    #[arg(long, env = "SLACK_TEAM_ID")]
    slack_team_id: String,

    /// OIDC redirect URI registered on the Slack app. Defaults to
    /// `<base_url>/oidc/callback`.
    #[arg(long)]
    oidc_redirect_uri: Option<String>,

    /// Enable dev-only routes (e.g. /oidc/dev-complete) for local testing
    /// without a real Slack app. Never set this in production.
    #[arg(long)]
    dev: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "buzz_migrate=info,tower_http=info".into()),
        )
        .init();

    let args = Args::parse();

    let admin = Keys::parse(&args.admin_key)
        .map_err(|e| format!("invalid --admin-key (hex or nsec): {e}"))?;

    let auth_tag = match args.auth_tag.as_deref() {
        Some(json) => Some(
            buzz_sdk::nip_oa::parse_auth_tag(json)
                .map_err(|e| format!("invalid BUZZ_AUTH_TAG: {e}"))?,
        ),
        None => None,
    };

    let users_path = args.export_dir.join("users.json");
    let users_bytes = std::fs::read(&users_path)
        .map_err(|e| format!("could not read {}: {e}", users_path.display()))?;
    let roster = Roster::from_users_json(&users_bytes, &args.slack_team_id)
        .map_err(|e| format!("could not parse {}: {e}", users_path.display()))?;
    tracing::info!(
        mailable = roster.mailable_count(),
        "loaded Slack export roster"
    );

    let token_secret = match args.token_secret {
        Some(hex_secret) => {
            hex::decode(hex_secret.trim()).map_err(|_| "--token-secret must be hex")?
        }
        None => {
            let s = rand::random::<[u8; 32]>().to_vec();
            tracing::warn!(
                "no --token-secret set: generated an ephemeral one; links minted now will \
                 stop verifying after a restart"
            );
            s
        }
    };
    if token_secret.len() < 32 {
        return Err("--token-secret must contain at least 32 bytes".into());
    }
    if args.token_ttl_secs == 0 {
        return Err("--token-ttl-secs must be greater than zero".into());
    }

    let base_url = normalize_public_base_url(
        &args
            .base_url
            .unwrap_or_else(|| format!("http://{}", args.bind)),
    )?;

    let oidc = match (args.slack_client_id, args.slack_client_secret) {
        (Some(client_id), Some(client_secret)) => {
            let redirect_uri = args
                .oidc_redirect_uri
                .unwrap_or_else(|| format!("{base_url}/oidc/callback"));
            tracing::info!(%redirect_uri, "OIDC channel enabled (Sign in with Slack)");
            Some(OidcConfig {
                client_id,
                client_secret,
                redirect_uri,
                team_id: args.slack_team_id.clone(),
            })
        }
        (None, None) => {
            tracing::info!("OIDC channel disabled (no Slack OIDC credentials)");
            None
        }
        _ => {
            return Err(
                "set SLACK_CLIENT_ID and SLACK_CLIENT_SECRET together, or omit both".into(),
            );
        }
    };

    if args.dev {
        tracing::warn!("--dev enabled: /oidc/dev-complete is active; do NOT use in production");
    }

    let inner = Inner {
        roster,
        token_secret,
        consumed: Mutex::new(ConsumedNonces::new()),
        admin,
        team_id: args.slack_team_id,
        relay_url: to_ws_url(&args.relay_url),
        auth_tag,
        base_url,
        token_ttl_secs: args.token_ttl_secs,
        mailer: if args.dev {
            Mailer::Dev
        } else {
            Mailer::Disabled
        },
        http: reqwest::Client::new(),
        oidc,
        oidc_states: Mutex::new(HashMap::new()),
        oidc_codes: Mutex::new(HashMap::new()),
        dev: args.dev,
    };
    let state = AppState(Arc::new(inner));

    let listener = tokio::net::TcpListener::bind(&args.bind).await?;
    tracing::info!(bind = %args.bind, "buzz-migrate claim-service listening");
    axum::serve(listener, router(state)).await?;
    Ok(())
}

/// Convert an http(s) relay URL to its ws(s) equivalent for event publishing.
/// ws/wss URLs pass through unchanged.
fn to_ws_url(url: &str) -> String {
    if let Some(rest) = url.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = url.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        url.to_string()
    }
}

fn normalize_public_base_url(value: &str) -> Result<String, String> {
    let parsed = url::Url::parse(value).map_err(|error| format!("invalid --base-url: {error}"))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err("--base-url must be an http(s) URL with a host".into());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("--base-url must not include credentials".into());
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err("--base-url must not include a query or fragment".into());
    }
    Ok(value.trim_end_matches('/').to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_urls_become_ws() {
        assert_eq!(to_ws_url("http://localhost:3000"), "ws://localhost:3000");
        assert_eq!(to_ws_url("https://relay.example"), "wss://relay.example");
        assert_eq!(to_ws_url("ws://x:1"), "ws://x:1");
        assert_eq!(to_ws_url("wss://x"), "wss://x");
    }

    #[test]
    fn public_base_url_is_validated_and_normalized() {
        assert_eq!(
            normalize_public_base_url("https://migrate.example/").unwrap(),
            "https://migrate.example"
        );
        assert!(normalize_public_base_url("file:///tmp/migrate").is_err());
        assert!(normalize_public_base_url("https://user@migrate.example").is_err());
        assert!(normalize_public_base_url("https://migrate.example?next=evil").is_err());
        assert!(normalize_public_base_url("https://migrate.example#fragment").is_err());
    }
}
