use std::sync::atomic::Ordering;
use std::sync::Arc;

use tracing::{error, info};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

use sprout_audit::AuditService;
use sprout_auth::AuthService;
use sprout_db::{Db, DbConfig};
use sprout_pubsub::PubSubManager;
use sprout_search::{SearchConfig, SearchService};

use sprout_relay::config::Config;
use sprout_relay::metrics as relay_metrics;
use sprout_relay::router::{build_health_router, build_router};
use sprout_relay::state::AppState;
use sprout_workflow::WorkflowEngine;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // JSON-only structured logs — simple, machine-parseable, CAKE-compatible.
    tracing_subscriber::registry()
        .with(fmt::layer().json().flatten_event(true))
        .with(EnvFilter::from_default_env().add_directive("sprout_relay=info".parse()?))
        .init();

    info!("Starting sprout-relay");

    let config = Config::from_env().map_err(|e| {
        error!("Invalid configuration: {e}");
        anyhow::anyhow!("Configuration error: {e}")
    })?;
    info!(
        bind_addr = %config.bind_addr,
        relay_url = %config.relay_url,
        health_port = config.health_port,
        metrics_port = config.metrics_port,
        "Config loaded"
    );

    // ── Metrics recorder (Prometheus exporter on :9102) ──────────────────────
    relay_metrics::install(config.metrics_port);
    info!(
        port = config.metrics_port,
        "Prometheus metrics exporter started"
    );

    let db_config = DbConfig {
        database_url: config.database_url.clone(),
        ..DbConfig::default()
    };
    let db = Db::new(&db_config).await.map_err(|e| {
        error!("Failed to connect to Postgres: {e}");
        anyhow::anyhow!("DB connection failed: {e}")
    })?;
    info!("Postgres connected");

    if let Err(e) = db.ensure_future_partitions(3).await {
        error!("Failed to ensure partitions: {e}");
    }

    // NIP-43: if membership enforcement is on, a valid owner pubkey is required.
    // config.rs already strips invalid values with a warning; catch the resulting
    // None here so we fail fast with a clear message rather than starting a relay
    // that no one can administer.
    if config.require_relay_membership && config.relay_owner_pubkey.is_none() {
        error!(
            "SPROUT_REQUIRE_RELAY_MEMBERSHIP=true but RELAY_OWNER_PUBKEY is not set or invalid. \
             Set RELAY_OWNER_PUBKEY to a valid 64-char hex pubkey."
        );
        return Err(anyhow::anyhow!(
            "RELAY_OWNER_PUBKEY required when SPROUT_REQUIRE_RELAY_MEMBERSHIP=true"
        ));
    }

    // NIP-43: relay membership requires a stable signing key.
    // Check this before any DB mutations so we fail fast — no point backfilling
    // or bootstrapping if we'll reject the config anyway.
    if config.require_relay_membership && config.relay_private_key.is_none() {
        return Err(anyhow::anyhow!(
            "SPROUT_RELAY_PRIVATE_KEY is required when SPROUT_REQUIRE_RELAY_MEMBERSHIP=true. \
             NIP-43 events signed with an ephemeral key become unverifiable after restart."
        ));
    }

    // NIP-43: migrate any existing pubkey_allowlist entries to relay_members.
    // Idempotent — safe to run every startup. Must run before bootstrap_owner
    // so that existing allowlist users become relay members before the owner
    // is promoted (otherwise enabling membership locks everyone out).
    match db.backfill_from_allowlist().await {
        Ok(0) => {}
        Ok(n) => info!("Backfilled {n} pubkey_allowlist entries into relay_members"),
        Err(e) => {
            if config.require_relay_membership {
                error!(
                    "Fatal: failed to backfill allowlist with membership enforcement enabled: {e}"
                );
                return Err(anyhow::anyhow!(
                    "Failed to backfill pubkey_allowlist (required when SPROUT_REQUIRE_RELAY_MEMBERSHIP=true): {e}"
                ));
            } else {
                error!("Failed to backfill pubkey_allowlist (non-fatal): {e}");
            }
        }
    }

    // NIP-43: ensure the configured relay owner always holds the owner role.
    if let Some(ref owner_pubkey) = config.relay_owner_pubkey {
        match db.bootstrap_owner(owner_pubkey).await {
            Ok(()) => info!(pubkey = %owner_pubkey, "Relay owner bootstrapped"),
            Err(e) => {
                if config.require_relay_membership {
                    // Membership enforcement is on — a missing owner means no one
                    // can administer the relay. Fail fast rather than silently start
                    // in a broken state.
                    error!("Fatal: failed to bootstrap relay owner with membership enforcement enabled: {e}");
                    return Err(anyhow::anyhow!(
                        "Failed to bootstrap relay owner (required when SPROUT_REQUIRE_RELAY_MEMBERSHIP=true): {e}"
                    ));
                } else {
                    error!(
                        "Failed to bootstrap relay owner (non-fatal, membership not required): {e}"
                    );
                }
            }
        }
    }

    // NIP-33: backfill d_tag for any existing parameterized replaceable events
    // that predate the column addition. Idempotent — no-ops when fully populated.
    match db.backfill_d_tags().await {
        Ok(0) => {}
        Ok(n) => info!("Backfilled d_tag for {n} NIP-33 events"),
        Err(e) => error!("Failed to backfill d_tags: {e}"),
    }

    let audit_pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .min_connections(1)
        .connect(&config.database_url)
        .await
        .map_err(|e| anyhow::anyhow!("Audit DB connection failed: {e}"))?;
    let audit = AuditService::new(audit_pool);
    if let Err(e) = audit.ensure_schema().await {
        error!("Failed to ensure audit schema: {e}");
    }
    info!("Audit service ready");

    let redis_pool = {
        let cfg = deadpool_redis::Config::from_url(&config.redis_url);
        cfg.create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .map_err(|e| anyhow::anyhow!("Redis pool creation failed: {e}"))?
    };
    let redis_health_pool = redis_pool.clone(); // cheap Arc clone — shared with readiness handler
    let pubsub = Arc::new(
        PubSubManager::new(&config.redis_url, redis_pool)
            .await
            .map_err(|e| anyhow::anyhow!("PubSub init failed: {e}"))?,
    );
    info!("Redis pub/sub connected");

    // Spawn Redis pub/sub subscriber for multi-node fan-out.
    // Events published by other relay instances are received here and
    // fanned out to local WebSocket subscribers.
    let pubsub_for_sub = Arc::clone(&pubsub);
    tokio::spawn(async move { pubsub_for_sub.run_subscriber().await });

    let auth = AuthService::new(config.auth.clone());

    let search_config = SearchConfig {
        url: config.typesense_url.clone(),
        api_key: config.typesense_key.clone(),
        collection: std::env::var("TYPESENSE_COLLECTION").unwrap_or_else(|_| "events".to_string()),
    };
    let search = SearchService::new(search_config);
    if let Err(e) = search.ensure_collection().await {
        error!("Typesense collection setup failed (non-fatal): {e}");
    }

    let workflow_config = sprout_workflow::WorkflowConfig::default();
    let workflow_engine = Arc::new(WorkflowEngine::new(db.clone(), workflow_config));

    let relay_keypair = if let Some(hex) = &config.relay_private_key {
        nostr::Keys::parse(hex)
            .map_err(|e| anyhow::anyhow!("invalid SPROUT_RELAY_PRIVATE_KEY: {e}"))?
    } else if !config.require_auth_token {
        // Dev mode: use a deterministic keypair so addressable events (kind:39000/39001/39002)
        // replace correctly across restarts. Without this, each restart generates a new pubkey
        // and replace_addressable_event inserts duplicates instead of replacing.
        const DEV_RELAY_PRIVKEY: &str =
            "0000000000000000000000000000000000000000000000000000000000000001";
        let keys = nostr::Keys::parse(DEV_RELAY_PRIVKEY).expect("hardcoded dev key is valid");
        tracing::warn!(
            pubkey = %keys.public_key().to_hex(),
            "Using hardcoded dev relay keypair (SPROUT_REQUIRE_AUTH_TOKEN=false). \
             Set SPROUT_RELAY_PRIVATE_KEY for production."
        );
        keys
    } else {
        panic!(
            "SPROUT_RELAY_PRIVATE_KEY must be set when SPROUT_REQUIRE_AUTH_TOKEN=true. \
             A stable relay identity is required for production."
        );
    };

    config
        .media
        .validate()
        .map_err(|e| anyhow::anyhow!("invalid media config: {e}"))?;
    let media_storage = sprout_media::MediaStorage::new(&config.media)
        .map_err(|e| anyhow::anyhow!("failed to initialize media storage: {e}"))?;
    info!("Media storage connected");

    let (app_state, audit_shutdown) = AppState::new(
        config.clone(),
        db,
        redis_health_pool,
        audit,
        pubsub,
        auth,
        search,
        Arc::clone(&workflow_engine),
        relay_keypair,
        media_storage,
    );
    let state = Arc::new(app_state);

    // Git-on-object-storage: admit the configured S3/MinIO backend against the
    // linearizable conditional-write axiom (A3) before serving git traffic.
    // Failure is fatal: a backend that cannot satisfy pointer CAS invalidates
    // the manifest-pointer protocol. This is a deployment gate, not a proof.
    if std::env::var("SPROUT_GIT_CONFORMANCE_PROBE")
        .map(|v| v != "false")
        .unwrap_or(true)
    {
        let race_width = std::env::var("SPROUT_GIT_PROBE_WRITERS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(32);
        let race_rounds = std::env::var("SPROUT_GIT_PROBE_ROUNDS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3);
        let cfg = sprout_relay::api::git::store::ProbeConfig {
            race_width,
            race_rounds,
        };
        tracing::info!(
            race_width,
            race_rounds,
            "running git object-store conformance probe (A3 gate)"
        );
        let report = state
            .git_store
            .run_conformance_probe(cfg)
            .await
            .map_err(|e| anyhow::anyhow!("git conformance probe failed: {e}"))?;
        tracing::info!(
            race_width = report.race_width,
            race_rounds = report.race_rounds,
            transport_drops = report.transport_drops,
            "git object-store backend admitted: A3 conformance probe passed"
        );
    }

    // NIP-43: publish the initial membership list on startup so clients can
    // REQ kind:13534 immediately without waiting for the next membership change.
    if config.require_relay_membership {
        let startup_state = Arc::clone(&state);
        tokio::spawn(async move {
            if let Err(e) =
                sprout_relay::handlers::side_effects::publish_nip43_membership_list(&startup_state)
                    .await
            {
                tracing::warn!(error = %e, "failed to publish initial NIP-43 membership list on startup");
            } else {
                tracing::info!("NIP-43 membership list published on startup");
            }
        });
    }

    // Emit kind:39000/39002 discovery events for channels that exist in the DB
    // but don't have corresponding events (e.g. seeded via direct SQL inserts).
    // Only runs when SPROUT_RECONCILE_CHANNELS=true (dev/CI environments).
    // Production relays create channels through the event pipeline and don't need this.
    if std::env::var("SPROUT_RECONCILE_CHANNELS").is_ok() {
        let reconcile_state = Arc::clone(&state);
        tokio::spawn(async move {
            // Try immediately, then retry every 5s for up to 2 minutes.
            // Handles CI pattern: relay starts → seed script inserts data → reconciliation.
            for attempt in 0..24u32 {
                if attempt > 0 {
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                }
                match sprout_relay::handlers::side_effects::reconcile_channel_events(
                    &reconcile_state,
                )
                .await
                {
                    Ok(()) => {}
                    Err(e) => {
                        tracing::warn!(error = %e, "channel reconciliation attempt failed");
                    }
                }
            }
        });
    }

    // Wire the action sink — must happen after AppState (which creates
    // sub_registry, conn_manager) and before the cron loop starts.
    let action_sink = Arc::new(sprout_relay::workflow_sink::RelayActionSink::new(&state));
    workflow_engine.set_action_sink(action_sink);

    // Start the cron loop AFTER the action sink is wired.
    let wf_cron = Arc::clone(&workflow_engine);
    tokio::spawn(async move { wf_cron.run().await });

    // Ephemeral channel reaper — archives channels whose TTL deadline has passed.
    // Runs every 60s, matching the workflow cron loop pattern. The SQL UPDATE
    // uses `archived_at IS NULL` as a guard, so concurrent runs from multiple
    // pods are harmless (at worst, duplicate system messages — same trade-off
    // as the workflow cron loop). Will be upgraded to use pg_advisory_lock
    // together with the workflow engine in a future multi-pod coordination pass.
    {
        let reaper_state = Arc::clone(&state);
        let reaper_interval_secs: u64 = std::env::var("SPROUT_REAPER_INTERVAL_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(60);
        tokio::spawn(async move {
            info!(
                interval_secs = reaper_interval_secs,
                "Ephemeral channel reaper started"
            );
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(reaper_interval_secs)).await;

                let expired = match reaper_state.db.reap_expired_ephemeral_channels().await {
                    Ok(ids) => ids,
                    Err(e) => {
                        error!("Ephemeral reaper tick failed: {e}");
                        continue;
                    }
                };

                if expired.is_empty() {
                    continue;
                }

                info!(count = expired.len(), "Ephemeral reaper archived channels");

                for channel_id in &expired {
                    // Emit a system message so members see why the channel was archived.
                    if let Err(e) = sprout_relay::handlers::side_effects::emit_system_message(
                        &reaper_state,
                        *channel_id,
                        serde_json::json!({ "type": "channel_auto_archived" }),
                    )
                    .await
                    {
                        error!(channel = %channel_id, "reaper system message failed: {e}");
                    }

                    // Update NIP-29 discovery events so clients see the archived state.
                    if let Err(e) =
                        sprout_relay::handlers::side_effects::emit_group_discovery_events(
                            &reaper_state,
                            *channel_id,
                        )
                        .await
                    {
                        error!(channel = %channel_id, "reaper discovery update failed: {e}");
                    }
                }
            }
        });
    }

    // Multi-node fan-out consumer: receive events from Redis pub/sub
    // (published by other relay instances) and fan out to local WS subscribers.
    {
        let state_for_sub = Arc::clone(&state);
        let mut rx = state_for_sub.pubsub.subscribe_local();
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(channel_event) => {
                        // Nil UUID is the sentinel for channel-less global events
                        // (see event.rs `else` branch). Convert back to None so
                        // fan_out() uses the global subscriber index instead of
                        // looking up subscribers under Some(Uuid::nil()), which
                        // would find nothing and silently drop every cross-node
                        // global event.
                        let channel_id = if channel_event.channel_id.is_nil() {
                            None
                        } else {
                            Some(channel_event.channel_id)
                        };
                        let stored = sprout_core::StoredEvent::new(channel_event.event, channel_id);

                        // Skip events that were already fanned out in-process (local echo).
                        // The cache has TTL-based eviction (60s) so entries are bounded
                        // regardless of subscriber health.
                        let event_id_bytes = stored.event.id.to_bytes();
                        if state_for_sub.local_event_ids.get(&event_id_bytes).is_some() {
                            state_for_sub.local_event_ids.invalidate(&event_id_bytes);
                            continue;
                        }

                        let matches = state_for_sub.sub_registry.fan_out(&stored);
                        let matches = sprout_relay::handlers::event::filter_fanout_by_access(
                            &state_for_sub,
                            &stored,
                            matches,
                        )
                        .await;
                        metrics::counter!("sprout_multinode_fanout_total").increment(1);
                        if matches.is_empty() {
                            continue;
                        }

                        let event_json = match serde_json::to_string(&stored.event) {
                            Ok(json) => json,
                            Err(e) => {
                                tracing::error!(
                                    "Failed to serialize event for multi-node fan-out: {e}"
                                );
                                continue;
                            }
                        };
                        let mut drop_count = 0u32;
                        for (conn_id, sub_id) in &matches {
                            let msg = format!(r#"["EVENT","{}",{}]"#, sub_id, event_json);
                            if !state_for_sub.conn_manager.send_to(*conn_id, msg) {
                                drop_count += 1;
                            }
                        }
                        if drop_count > 0 {
                            tracing::warn!(
                                event_id = %stored.event.id.to_hex(),
                                drop_count,
                                "multi-node fan-out: {drop_count} connection(s) dropped"
                            );
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        metrics::counter!("sprout_multinode_fanout_lag_total").increment(n);
                        tracing::warn!("Multi-node fan-out lagged by {n} messages");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        tracing::error!("Multi-node fan-out broadcast channel closed");
                        break;
                    }
                }
            }
        });
    }

    let router = build_router(Arc::clone(&state));
    let health_router = build_health_router(Arc::clone(&state));

    serve(router, health_router, Arc::clone(&state)).await?;

    // ── Drain audit queue ────────────────────────────────────────────────────
    // Signal the audit worker to stop accepting, flush buffered entries, and
    // exit. Uses a CancellationToken so it works regardless of how many
    // Arc<AppState> clones are still alive in background tasks.
    audit_shutdown
        .drain(std::time::Duration::from_secs(5))
        .await;

    Ok(())
}

/// Bind all listeners and run with graceful shutdown.
///
/// ```text
/// ┌─────────────────────────────────────────────────────────┐
/// │  Listener 1: TCP SPROUT_BIND_ADDR:3000  (app router)   │
/// │  Listener 2: UDS SPROUT_UDS_PATH        (app, optional)│
/// │  Listener 3: TCP 0.0.0.0:8080           (health only)  │
/// │  Listener 4: TCP 0.0.0.0:9102           (metrics, via  │
/// │              PrometheusBuilder — already bound)         │
/// │                                                         │
/// │  SIGTERM → shutting_down=true → readiness 503           │
/// │         → graceful drain (30s) → exit                   │
/// └─────────────────────────────────────────────────────────┘
/// ```
async fn serve(
    router: axum::Router,
    health_router: axum::Router,
    state: Arc<AppState>,
) -> anyhow::Result<()> {
    let config = &state.config;

    // ── Health listener (port 8080) ──────────────────────────────────────────
    let health_listener = tokio::net::TcpListener::bind(("0.0.0.0", config.health_port))
        .await
        .map_err(|e| anyhow::anyhow!("Failed to bind health port {}: {e}", config.health_port))?;
    info!(port = config.health_port, "Health probe listener started");
    tokio::spawn(async move {
        axum::serve(health_listener, health_router).await.ok();
    });

    // ── Shutdown coordination ────────────────────────────────────────────────
    let (shutdown_tx, _) = tokio::sync::watch::channel(false);
    let shutdown_flag = Arc::clone(&state.shutting_down);
    let tx = shutdown_tx.clone();
    tokio::spawn(async move {
        shutdown_signal().await;
        shutdown_flag.store(true, Ordering::Relaxed);
        info!("Shutdown signal received — readiness now returns 503");
        // 5s grace: let K8s stop routing new traffic before we close listeners.
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        info!("Starting graceful drain (30s timeout)");
        let _ = tx.send(true);
        // Hard timeout: force exit if connections don't drain within 30s.
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        tracing::error!("Drain timeout exceeded — forcing exit");
        std::process::exit(1);
    });

    // ── App listener (TCP) ───────────────────────────────────────────────────
    let tcp_listener = tokio::net::TcpListener::bind(&config.bind_addr)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to bind {}: {e}", config.bind_addr))?;
    info!(addr = %config.bind_addr, "sprout-relay TCP listening");

    // ── App listener (UDS, optional) ─────────────────────────────────────────
    #[cfg(unix)]
    if let Some(ref uds_path) = config.uds_path {
        use std::os::unix::fs::FileTypeExt as _;
        match std::fs::symlink_metadata(uds_path) {
            Ok(meta) if meta.file_type().is_socket() => {
                let _ = std::fs::remove_file(uds_path);
            }
            Ok(_) => {
                return Err(anyhow::anyhow!(
                    "SPROUT_UDS_PATH {uds_path} exists but is not a socket"
                ));
            }
            Err(_) => {}
        }
        let uds_listener = tokio::net::UnixListener::bind(uds_path)
            .map_err(|e| anyhow::anyhow!("Failed to bind UDS {uds_path}: {e}"))?;
        info!(path = %uds_path, "sprout-relay UDS listening");

        let router_uds = router.clone();
        let mut uds_rx = shutdown_tx.subscribe();
        let uds_handle = tokio::spawn(async move {
            axum::serve(uds_listener, router_uds.into_make_service())
                .with_graceful_shutdown(async move {
                    uds_rx.changed().await.ok();
                })
                .await
                .ok();
        });

        let mut tcp_rx = shutdown_tx.subscribe();
        axum::serve(
            tcp_listener,
            router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .with_graceful_shutdown(async move {
            tcp_rx.changed().await.ok();
        })
        .await
        .map_err(|e| anyhow::anyhow!("TCP server error: {e}"))?;

        uds_handle.abort();
        return Ok(());
    }

    #[cfg(not(unix))]
    if config.uds_path.is_some() {
        tracing::warn!("SPROUT_UDS_PATH set but UDS not supported on this platform");
    }

    // TCP-only path.
    let mut tcp_rx = shutdown_tx.subscribe();
    axum::serve(
        tcp_listener,
        router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        tcp_rx.changed().await.ok();
    })
    .await
    .map_err(|e| anyhow::anyhow!("Server error: {e}"))?;

    Ok(())
}

/// Wait for SIGTERM (Unix) or Ctrl+C.
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigterm = signal(SignalKind::terminate()).expect("install SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {},
            _ = sigterm.recv() => {},
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await.ok();
    }
}
