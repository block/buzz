//! sprout-proxy binary — NIP-28 guest relay proxy for standard Nostr clients.

use std::sync::Arc;

use nostr::prelude::*;
use tokio::sync::{broadcast, mpsc};
use tracing::{error, info};

use sprout_proxy::channel_map::ChannelMap;
use sprout_proxy::guest_store::GuestStore;
use sprout_proxy::invite_store::InviteStore;
use sprout_proxy::server::{self, ProxyState};
use sprout_proxy::shadow_keys::ShadowKeyManager;
use sprout_proxy::translate::Translator;
use sprout_proxy::upstream::{UpstreamClient, UpstreamEvent};

// ── Env helpers ───────────────────────────────────────────────────────────────

fn env_required(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| {
        eprintln!("error: required environment variable {name} is not set");
        std::process::exit(1);
    })
}

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}

// ── Entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    // Init tracing — respects RUST_LOG; falls back to info for sprout_proxy and tower_http.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "sprout_proxy=info,tower_http=info".into()),
        )
        .init();

    // ── Parse env ─────────────────────────────────────────────────────────────

    let upstream_url = env_required("SPROUT_UPSTREAM_URL");
    let bind_addr = env_or("SPROUT_PROXY_BIND_ADDR", "0.0.0.0:4869");
    let server_key_hex = env_required("SPROUT_PROXY_SERVER_KEY");
    let salt_hex = env_required("SPROUT_PROXY_SALT");
    let api_token = env_required("SPROUT_PROXY_API_TOKEN");
    let relay_pubkey = env_required("SPROUT_RELAY_PUBKEY").to_lowercase();
    // Validate relay pubkey is well-formed 64-char hex at startup.
    // Input is lowercased above, so mixed-case is accepted.
    if relay_pubkey.len() != 64 || !relay_pubkey.chars().all(|c| c.is_ascii_hexdigit()) {
        eprintln!("error: SPROUT_RELAY_PUBKEY must be a 64-character hex string (32 bytes)");
        std::process::exit(1);
    }
    info!(relay_pubkey = %relay_pubkey, "relay pubkey configured for attribution trust");

    // ── Parse server keypair ──────────────────────────────────────────────────

    let server_secret = SecretKey::from_hex(&server_key_hex).unwrap_or_else(|e| {
        eprintln!("error: invalid SPROUT_PROXY_SERVER_KEY: {e}");
        std::process::exit(1);
    });
    let server_keys = Keys::new(server_secret);
    info!(pubkey = %server_keys.public_key(), "proxy server keypair loaded");

    // ── Parse salt ────────────────────────────────────────────────────────────

    let salt = hex::decode(&salt_hex).unwrap_or_else(|e| {
        eprintln!("error: invalid SPROUT_PROXY_SALT (must be hex): {e}");
        std::process::exit(1);
    });

    // ── Init shadow key manager ───────────────────────────────────────────────

    let shadow_keys = Arc::new(ShadowKeyManager::new(&salt).unwrap_or_else(|e| {
        eprintln!("error: shadow key manager init failed: {e}");
        std::process::exit(1);
    }));

    // ── Derive HTTP base URL from WS URL for REST API calls ───────────────────

    let api_base = upstream_url
        .replace("wss://", "https://")
        .replace("ws://", "http://");

    // ── Init channel map from REST API ────────────────────────────────────────

    info!("initializing channel map from {api_base}/api/channels ...");
    let channel_map = Arc::new(
        ChannelMap::init_from_rest(server_keys.clone(), &api_base, &api_token)
            .await
            .unwrap_or_else(|e| {
                eprintln!("error: failed to initialize channel map: {e}");
                std::process::exit(1);
            }),
    );
    info!(channels = channel_map.len(), "channel map ready");

    // ── Init translator ───────────────────────────────────────────────────────

    let translator = Arc::new(Translator::new(
        shadow_keys,
        channel_map.clone(),
        api_base.clone(),
        api_token.clone(),
        relay_pubkey,
    ));

    // ── Init guest store (empty — guests registered via POST /admin/guests) ────

    let guest_store = Arc::new(GuestStore::new());

    // ── Init invite store (empty — tokens created via POST /admin/invite) ─────

    let invite_store = Arc::new(InviteStore::new());

    // ── Init upstream client ──────────────────────────────────────────────────
    //
    // UpstreamClient owns its internal outbound channel. The server calls
    // upstream.send_event() / send_req() / send_close() directly via Arc.
    // Note: UpstreamClient generates a stable ephemeral keypair per process
    // lifetime for NIP-42 auth — consistent across reconnects.

    // Use server_keys for upstream NIP-42 auth so the auth event pubkey matches
    // the API token's owner_pubkey (the relay enforces this).
    let upstream = Arc::new(UpstreamClient::with_keys(
        upstream_url.clone(),
        api_token.clone(),
        server_keys.clone(),
    ));

    // ── upstream_events broadcast: UpstreamClient → all WebSocket sessions ────

    // upstream_events_tx: upstream → server (broadcast of inbound JSON strings)
    let (upstream_events_tx, _) = broadcast::channel::<String>(4096);

    // inbound_tx: UpstreamClient → bridge task (UpstreamEvent)
    let (inbound_tx, mut inbound_rx) = mpsc::channel::<UpstreamEvent>(256);

    // ── Bridge task: UpstreamEvent → broadcast String ─────────────────────────
    //
    // The server layer subscribes to `upstream_events_tx` as raw JSON strings.
    // The UpstreamClient emits typed `UpstreamEvent` values.  This task bridges
    // the two, serializing relay messages back to JSON for the server layer.

    let bridge_events_tx = upstream_events_tx.clone();
    tokio::spawn(async move {
        while let Some(event) = inbound_rx.recv().await {
            match event {
                UpstreamEvent::RelayMessage(json) => {
                    // Already raw JSON — forward directly to the broadcast channel.
                    let _ = bridge_events_tx.send(json);
                }
                UpstreamEvent::Connected => {
                    info!("upstream relay connected");
                }
                UpstreamEvent::Disconnected => {
                    info!("upstream relay disconnected — reconnecting");
                }
            }
        }
    });

    // ── Read admin secret from env (optional) ─────────────────────────────────

    let admin_secret = std::env::var("SPROUT_PROXY_ADMIN_SECRET").ok();
    if admin_secret.is_some() {
        info!("admin endpoint protected by SPROUT_PROXY_ADMIN_SECRET");
    } else {
        info!("admin endpoint running unauthenticated (dev mode) — set SPROUT_PROXY_ADMIN_SECRET to secure it");
    }

    // ── Build proxy state ─────────────────────────────────────────────────────

    // Relay URL for NIP-42 relay tag validation. Prefer explicit env var
    // (e.g. "wss://proxy.example.com") over the derived bind address fallback.
    let relay_url =
        std::env::var("SPROUT_PROXY_RELAY_URL").unwrap_or_else(|_| format!("ws://{}", bind_addr));

    let state = ProxyState {
        channel_map: channel_map.clone(),
        guest_store: guest_store.clone(),
        invite_store: invite_store.clone(),
        translator,
        upstream: upstream.clone(),
        upstream_events: upstream_events_tx.clone(),
        admin_secret,
        relay_url,
    };

    // ── Build router ──────────────────────────────────────────────────────────

    let app = server::router(state);

    // ── Bind listener ─────────────────────────────────────────────────────────

    info!("sprout-proxy starting on {bind_addr} → upstream {upstream_url}");

    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .unwrap_or_else(|e| {
            eprintln!("error: failed to bind {bind_addr}: {e}");
            std::process::exit(1);
        });

    // ── Run server + upstream concurrently ────────────────────────────────────

    tokio::select! {
        result = axum::serve(listener, app).with_graceful_shutdown(shutdown_signal()) => {
            if let Err(e) = result {
                error!("server error: {e}");
            }
        }
        _ = upstream.as_ref().clone().run(inbound_tx) => {
            error!("upstream client exited unexpectedly");
        }
    }

    info!("sprout-proxy shut down");
}

// ── Graceful shutdown ─────────────────────────────────────────────────────────

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to install CTRL+C handler");
    info!("shutdown signal received");
}
