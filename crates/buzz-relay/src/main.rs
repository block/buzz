use std::sync::atomic::Ordering;
use std::sync::Arc;

use tracing::{error, info, warn};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

use buzz_audit::AuditService;
use buzz_auth::AuthService;
use buzz_db::{Db, DbConfig};
use buzz_pubsub::PubSubManager;
use buzz_search::{SearchConfig, SearchService};

use buzz_relay::config::Config;
use buzz_relay::metrics as relay_metrics;
use buzz_relay::router::{build_health_router, build_router};
use buzz_relay::state::AppState;
use buzz_workflow::WorkflowEngine;

fn buzz_auto_migrate_enabled(value: Option<&str>) -> bool {
    value.map(str::trim).is_some_and(|value| {
        matches!(
            value.to_ascii_lowercase().as_str(),
            "true" | "1" | "yes" | "on"
        )
    })
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // JSON-only structured logs — simple, machine-parseable, CAKE-compatible.
    tracing_subscriber::registry()
        .with(fmt::layer().json().flatten_event(true))
        .with(EnvFilter::from_default_env().add_directive("buzz_relay=info".parse()?))
        .init();

    info!("Starting buzz-relay");

    let config = Config::from_env().map_err(|e| {
        error!("Invalid configuration: {e}");
        anyhow::anyhow!("Configuration error: {e}")
    })?;
    info!(
        bind_addr = %config.bind_addr,
        relay_url = %config.relay_url,
        health_port = config.health_port,
        metrics_port = config.metrics_port,
        max_frame_bytes = config.max_frame_bytes,
        "Config loaded"
    );

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

    let auto_migrate =
        buzz_auto_migrate_enabled(std::env::var("BUZZ_AUTO_MIGRATE").ok().as_deref());
    if auto_migrate {
        db.migrate().await.map_err(|e| {
            error!("Failed to run database migrations: {e}");
            anyhow::anyhow!("Database migration failed: {e}")
        })?;
        info!("Database migrations complete");
    } else {
        info!("Skipping database migrations because BUZZ_AUTO_MIGRATE is not enabled");
    }

    if let Err(e) = db.ensure_future_partitions(3).await {
        error!("Failed to ensure partitions: {e}");
    }

    // NIP-43: if membership enforcement is on, a valid owner pubkey is required.
    // config.rs already strips invalid values with a warning; catch the resulting
    // None here so we fail fast with a clear message rather than starting a relay
    // that no one can administer.
    if config.require_relay_membership && config.relay_owner_pubkey.is_none() {
        error!(
            "BUZZ_REQUIRE_RELAY_MEMBERSHIP=true but RELAY_OWNER_PUBKEY is not set or invalid. \
             Set RELAY_OWNER_PUBKEY to a valid 64-char hex pubkey."
        );
        return Err(anyhow::anyhow!(
            "RELAY_OWNER_PUBKEY required when BUZZ_REQUIRE_RELAY_MEMBERSHIP=true"
        ));
    }

    // NIP-43: relay membership requires a stable signing key.
    // Check this before any DB mutations so we fail fast — no point backfilling
    // or bootstrapping if we'll reject the config anyway.
    if config.require_relay_membership && config.relay_private_key.is_none() {
        return Err(anyhow::anyhow!(
            "BUZZ_RELAY_PRIVATE_KEY is required when BUZZ_REQUIRE_RELAY_MEMBERSHIP=true. \
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
                    "Failed to backfill pubkey_allowlist (required when BUZZ_REQUIRE_RELAY_MEMBERSHIP=true): {e}"
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
                        "Failed to bootstrap relay owner (required when BUZZ_REQUIRE_RELAY_MEMBERSHIP=true): {e}"
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

    // Spawn Redis pub/sub subscriber for cross-pod cache-key invalidation.
    // Membership / visibility changes on other pods are received here and the
    // matching local moka caches are dropped (via the consumer loop below).
    let pubsub_for_cache = Arc::clone(&pubsub);
    tokio::spawn(async move { pubsub_for_cache.run_cache_invalidation_subscriber().await });

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

    let workflow_config = buzz_workflow::WorkflowConfig::default();
    let workflow_engine = Arc::new(WorkflowEngine::new(db.clone(), workflow_config));

    let relay_keypair = if let Some(hex) = &config.relay_private_key {
        nostr::Keys::parse(hex)
            .map_err(|e| anyhow::anyhow!("invalid BUZZ_RELAY_PRIVATE_KEY: {e}"))?
    } else if !config.require_auth_token {
        // Dev mode: use a deterministic keypair so addressable events (kind:39000/39001/39002)
        // replace correctly across restarts. Without this, each restart generates a new pubkey
        // and replace_addressable_event inserts duplicates instead of replacing.
        const DEV_RELAY_PRIVKEY: &str =
            "0000000000000000000000000000000000000000000000000000000000000001";
        let keys = nostr::Keys::parse(DEV_RELAY_PRIVKEY).expect("hardcoded dev key is valid");
        tracing::warn!(
            pubkey = %keys.public_key().to_hex(),
            "Using hardcoded dev relay keypair (BUZZ_REQUIRE_AUTH_TOKEN=false). \
             Set BUZZ_RELAY_PRIVATE_KEY for production."
        );
        keys
    } else {
        panic!(
            "BUZZ_RELAY_PRIVATE_KEY must be set when BUZZ_REQUIRE_AUTH_TOKEN=true. \
             A stable relay identity is required for production."
        );
    };

    config
        .media
        .validate()
        .map_err(|e| anyhow::anyhow!("invalid media config: {e}"))?;
    let media_storage = buzz_media::MediaStorage::new(&config.media)
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
    if std::env::var("BUZZ_GIT_CONFORMANCE_PROBE")
        .map(|v| v != "false")
        .unwrap_or(true)
    {
        let race_width = std::env::var("BUZZ_GIT_PROBE_WRITERS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(32);
        let race_rounds = std::env::var("BUZZ_GIT_PROBE_ROUNDS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3);
        let cfg = buzz_relay::api::git::store::ProbeConfig {
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
                buzz_relay::handlers::side_effects::publish_nip43_membership_list(&startup_state)
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
    // Only runs when BUZZ_RECONCILE_CHANNELS=true (dev/CI environments).
    // Production relays create channels through the event pipeline and don't need this.
    if std::env::var("BUZZ_RECONCILE_CHANNELS").is_ok() {
        let reconcile_state = Arc::clone(&state);
        tokio::spawn(async move {
            // Try immediately, then retry every 5s for up to 2 minutes.
            // Handles CI pattern: relay starts → seed script inserts data → reconciliation.
            for attempt in 0..24u32 {
                if attempt > 0 {
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                }
                match buzz_relay::handlers::side_effects::reconcile_channel_events(&reconcile_state)
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
    let action_sink = Arc::new(buzz_relay::workflow_sink::RelayActionSink::new(&state));
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
        let reaper_interval_secs: u64 = std::env::var("BUZZ_REAPER_INTERVAL_SECS")
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
                    if let Err(e) = buzz_relay::handlers::side_effects::emit_system_message(
                        &reaper_state,
                        *channel_id,
                        serde_json::json!({ "type": "channel_auto_archived" }),
                    )
                    .await
                    {
                        error!(channel = %channel_id, "reaper system message failed: {e}");
                    }

                    // Update NIP-29 discovery events so clients see the archived state.
                    if let Err(e) = buzz_relay::handlers::side_effects::emit_group_discovery_events(
                        &reaper_state,
                        *channel_id,
                    )
                    .await
                    {
                        error!(channel = %channel_id, "reaper discovery update failed: {e}");
                    }

                    // Close live subscriptions so connected clients drop the
                    // archived channel immediately (CLOSED is in the client's
                    // drop-set → no reconnect storm). Offline clients are caught
                    // by the archived=true skip in discover_channels on reconnect.
                    buzz_relay::handlers::side_effects::evict_all_channel_subscriptions(
                        &reaper_state,
                        *channel_id,
                    )
                    .await;
                }
            }
        });
    }

    // NIP-ER reminder scheduler — polls for due reminders and publishes them
    // to Redis pub/sub for cross-pod fan-out. Each pod's existing
    // subscribe_local consumer picks them up and applies the author-only gate.
    // Mirrors the channel reaper pattern. Cross-pod dedup via `delivered_at`
    // column: only the pod that wins the atomic claim publishes.
    {
        let scheduler_state = Arc::clone(&state);
        let scheduler_interval_secs: u64 = std::env::var("SPROUT_REMINDER_SCHEDULER_INTERVAL_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(10);
        let scheduler_batch_limit: i64 = std::env::var("SPROUT_REMINDER_SCHEDULER_BATCH_LIMIT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(100);
        tokio::spawn(async move {
            info!(
                interval_secs = scheduler_interval_secs,
                batch_limit = scheduler_batch_limit,
                "NIP-ER reminder scheduler started"
            );
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(scheduler_interval_secs)).await;

                let now_secs = chrono::Utc::now().timestamp();
                let due = match scheduler_state
                    .db
                    .query_due_reminders(now_secs, scheduler_batch_limit)
                    .await
                {
                    Ok(reminders) => reminders,
                    Err(e) => {
                        error!("Reminder scheduler tick failed: {e}");
                        continue;
                    }
                };

                if due.is_empty() {
                    continue;
                }

                info!(count = due.len(), "Reminder scheduler: due reminders found");

                for reminder in due {
                    // Publish first, then claim. If publish fails the reminder
                    // stays unclaimed and will be retried next tick. If claim
                    // fails after a successful publish, duplicate fan-out on the
                    // next tick is harmless (subscribers dedup by event ID).
                    if let Err(e) = scheduler_state
                        .pubsub
                        .publish_event(uuid::Uuid::nil(), &reminder_to_event(&reminder))
                        .await
                    {
                        error!(
                            event_id = hex::encode(&reminder.id),
                            "Reminder scheduler: Redis publish failed, skipping claim: {e}"
                        );
                        continue;
                    }

                    // Atomic cross-pod claim — only the winner marks it delivered.
                    match scheduler_state
                        .db
                        .claim_due_reminder(&reminder.id, reminder.created_at)
                        .await
                    {
                        Ok(true) => {}
                        Ok(false) => {} // Another pod claimed it; duplicate publish is harmless.
                        Err(e) => {
                            warn!(
                                event_id = hex::encode(&reminder.id),
                                "Reminder scheduler: claim failed after publish (duplicate delivery possible): {e}"
                            );
                        }
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
                        buzz_relay::handlers::event::fan_out_pubsub_event(
                            &state_for_sub,
                            channel_event,
                        )
                        .await;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        metrics::counter!("buzz_multinode_fanout_lag_total").increment(n);
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

    // Cross-pod cache-invalidation consumer: receive cache-key drops from Redis
    // pub/sub (published by other relay instances when membership/visibility
    // changes) and apply the matching local moka drop. Uses the `*_local` drop
    // variants so a received drop is never re-published.
    {
        let state_for_cache = Arc::clone(&state);
        let mut rx = state_for_cache.pubsub.subscribe_cache_invalidations();
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(invalidation) => {
                        state_for_cache.apply_cache_invalidation(invalidation);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        metrics::counter!("buzz_cache_invalidation_lag_total").increment(n);
                        tracing::warn!("Cache-invalidation consumer lagged by {n} messages");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        tracing::error!("Cache-invalidation broadcast channel closed");
                        break;
                    }
                }
            }
        });
    }

    let router = build_router(Arc::clone(&state));
    let health_router = build_health_router(Arc::clone(&state));

    serve(router, health_router, Arc::clone(&state)).await?;

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
/// │  Listener 1: TCP BUZZ_BIND_ADDR:3000  (app router)   │
/// │  Listener 2: UDS BUZZ_UDS_PATH        (app, optional)│
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

    let health_listener = tokio::net::TcpListener::bind(("0.0.0.0", config.health_port))
        .await
        .map_err(|e| anyhow::anyhow!("Failed to bind health port {}: {e}", config.health_port))?;
    info!(port = config.health_port, "Health probe listener started");
    tokio::spawn(async move {
        axum::serve(health_listener, health_router).await.ok();
    });

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

    let tcp_listener = tokio::net::TcpListener::bind(&config.bind_addr)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to bind {}: {e}", config.bind_addr))?;
    info!(addr = %config.bind_addr, "buzz-relay TCP listening");

    #[cfg(unix)]
    if let Some(ref uds_path) = config.uds_path {
        use std::os::unix::fs::FileTypeExt as _;
        match std::fs::symlink_metadata(uds_path) {
            Ok(meta) if meta.file_type().is_socket() => {
                let _ = std::fs::remove_file(uds_path);
            }
            Ok(_) => {
                return Err(anyhow::anyhow!(
                    "BUZZ_UDS_PATH {uds_path} exists but is not a socket"
                ));
            }
            Err(_) => {}
        }
        let uds_listener = tokio::net::UnixListener::bind(uds_path)
            .map_err(|e| anyhow::anyhow!("Failed to bind UDS {uds_path}: {e}"))?;
        info!(path = %uds_path, "buzz-relay UDS listening");

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
        tracing::warn!("BUZZ_UDS_PATH set but UDS not supported on this platform");
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
/// Reconstruct a `nostr::Event` from a [`DueReminder`] row for Redis pub/sub.
fn reminder_to_event(reminder: &buzz_db::event::DueReminder) -> nostr::Event {
    let event_json = serde_json::json!({
        "id": hex::encode(&reminder.id),
        "pubkey": hex::encode(&reminder.pubkey),
        "created_at": reminder.created_at.timestamp(),
        "kind": reminder.kind as u16,
        "tags": reminder.tags,
        "content": reminder.content,
        "sig": hex::encode(&reminder.sig),
    });

    serde_json::from_value(event_json).expect("valid event JSON from DB row")
}

#[cfg(test)]
mod tests {
    use super::buzz_auto_migrate_enabled;

    #[test]
    fn buzz_auto_migrate_is_opt_in() {
        assert!(!buzz_auto_migrate_enabled(None));
        assert!(!buzz_auto_migrate_enabled(Some("")));
        assert!(!buzz_auto_migrate_enabled(Some("false")));
        assert!(!buzz_auto_migrate_enabled(Some("0")));
        assert!(!buzz_auto_migrate_enabled(Some("no")));

        assert!(buzz_auto_migrate_enabled(Some("true")));
        assert!(buzz_auto_migrate_enabled(Some("TRUE")));
        assert!(buzz_auto_migrate_enabled(Some(" 1 ")));
        assert!(buzz_auto_migrate_enabled(Some("yes")));
        assert!(buzz_auto_migrate_enabled(Some("on")));
    }
}
