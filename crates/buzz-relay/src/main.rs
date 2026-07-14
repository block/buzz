use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use tracing::{error, info, warn};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};
use uuid::Uuid;

use buzz_audit::AuditService;
use buzz_auth::AuthService;
use buzz_core::CommunityId;
use buzz_db::{Db, DbConfig};
use buzz_pubsub::PubSubManager;
use buzz_search::SearchService;

use buzz_relay::config::Config;
use buzz_relay::metrics as relay_metrics;
use buzz_relay::router::{build_health_router, build_router};
use buzz_relay::state::AppState;
use buzz_relay::telemetry;
use buzz_workflow::WorkflowEngine;
use tokio_util::sync::CancellationToken;

fn buzz_auto_migrate_enabled(value: Option<&str>) -> bool {
    value.map(str::trim).is_some_and(|value| {
        matches!(
            value.to_ascii_lowercase().as_str(),
            "true" | "1" | "yes" | "on"
        )
    })
}

/// Controls how many per-community gauge series the usage poller emits.
///
/// Datadog cost is proportional to the number of unique time-series.  With ~25
/// gauge label combinations per community, a relay hosting thousands of
/// communities would incur five-figure monthly costs if every community always
/// gets a full set of series.  This knob is the cost lever.
///
/// Fleet-wide totals (`buzz_total_*`) always emit regardless of mode.
///
/// Set via `BUZZ_USAGE_METRICS_PER_COMMUNITY`:
///   - `all` — emit per-community series for every community (default)
///   - `off` — suppress all per-community series; fleet totals only
///
/// A `top:<k>` mode (per-community series for the k most-active communities)
/// is planned as a fast-follow once the series-lifecycle (gauge idle-timeout
/// and stable tie-breaking across pods) is fully designed.
#[derive(Debug, Clone)]
enum PerCommunityMode {
    All,
    Off,
}

impl PerCommunityMode {
    fn from_env() -> Self {
        let raw = std::env::var("BUZZ_USAGE_METRICS_PER_COMMUNITY")
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        match raw.as_str() {
            "" | "all" => PerCommunityMode::All,
            "off" => PerCommunityMode::Off,
            other => {
                warn!(
                    value = other,
                    "BUZZ_USAGE_METRICS_PER_COMMUNITY: unknown value — defaulting to all"
                );
                PerCommunityMode::All
            }
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Install the ring CryptoProvider for rustls. Required before any rustls
    // TLS connection (rediss:// to ElastiCache, wss://, S3 over TLS): both
    // aws-lc-rs and ring are compiled in transitively, so rustls can't
    // auto-select a provider and would panic at first use without this.
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("failed to install rustls crypto provider");

    // JSON-only structured logs — simple, machine-parseable, CAKE-compatible.
    // If OTEL_EXPORTER_OTLP_ENDPOINT is set, also attach an OpenTelemetry tracing
    // layer that exports spans via OTLP gRPC alongside the JSON stdout logs.
    //
    // Build a single shared Resource (service.name=buzz-relay by default, overridable
    // via OTEL_SERVICE_NAME) for the trace provider so that Datadog can identify
    // spans under the correct service identity.
    let resource = telemetry::service_resource();
    let tracer_init = telemetry::try_init_tracer(resource.clone());
    let otel_layer = match &tracer_init {
        telemetry::TracerInit::Enabled(p) => {
            use opentelemetry::trace::TracerProvider as _;
            Some(tracing_opentelemetry::layer().with_tracer(p.tracer("buzz-relay")))
        }
        _ => None,
    };

    tracing_subscriber::registry()
        .with(fmt::layer().json().flatten_event(true))
        .with(EnvFilter::from_default_env().add_directive("buzz_relay=info".parse()?))
        .with(otel_layer)
        .init();

    // Log any exporter-build failure now that the subscriber is installed.
    if let telemetry::TracerInit::ExporterBuildFailed(ref e) = tracer_init {
        warn!(error = %e, "Failed to build OTLP trace exporter; distributed tracing disabled");
    }

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

    // NIP-43 / multi-tenant: seed the deployment's *own* community before any
    // membership backfill or owner bootstrap, so those writes are scoped to a
    // real `(community_id, pubkey)` and not a global pubkey. The host is derived
    // from `relay_url` with the *same* normalization request resolution uses
    // (`relay_url_authority` → `normalize_host`), so the bootstrapped owner lands
    // in exactly the community that live requests for this host will resolve to.
    //
    // `ensure_configured_community` is idempotent, so this is safe to run every
    // startup. An empty authority (unparseable `relay_url`)
    // is a misconfiguration — fail fast when membership is enforced rather than
    // seeding an empty-host community that no request can ever resolve to.
    let deployment_community = {
        let host = buzz_relay::tenant::relay_url_authority(&config.relay_url);
        if host.is_empty() {
            if config.require_relay_membership {
                return Err(anyhow::anyhow!(
                    "Cannot derive a community host from BUZZ_RELAY_URL ({:?}); a resolvable host is required when BUZZ_REQUIRE_RELAY_MEMBERSHIP=true",
                    config.relay_url
                ));
            }
            error!(
                relay_url = %config.relay_url,
                "Could not derive a community host from relay_url; skipping membership backfill/bootstrap (non-fatal, membership not required)"
            );
            None
        } else {
            match db.ensure_configured_community(&host).await {
                Ok(record) => {
                    info!(host = %record.host, community = %record.id, "Deployment community ensured");
                    Some(record.id)
                }
                Err(e) => {
                    if config.require_relay_membership {
                        error!("Fatal: failed to ensure deployment community with membership enforcement enabled: {e}");
                        return Err(anyhow::anyhow!(
                            "Failed to ensure deployment community (required when BUZZ_REQUIRE_RELAY_MEMBERSHIP=true): {e}"
                        ));
                    }
                    error!("Failed to ensure deployment community (non-fatal, membership not required): {e}");
                    None
                }
            }
        }
    };

    // NIP-43: migrate any existing pubkey_allowlist entries to relay_members.
    // Idempotent — safe to run every startup. Must run before bootstrap_owner
    // so that existing allowlist users become relay members before the owner
    // is promoted (otherwise enabling membership locks everyone out).
    if let Some(community) = deployment_community {
        match db.backfill_from_allowlist(community).await {
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
    }

    // NIP-43: ensure the configured relay owner always holds the owner role
    // within the deployment community.
    if let (Some(community), Some(owner_pubkey)) =
        (deployment_community, config.relay_owner_pubkey.as_ref())
    {
        match db.bootstrap_owner(community, owner_pubkey).await {
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
    // Audit schema is provisioned by the sqlx migrations at startup (the
    // `audit_log` DDL is part of the migrated schema), so there is no runtime
    // schema-ensure step.
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

    // Spawn Redis pub/sub subscriber for cross-pod connection-control commands.
    // Bans recorded on other pods are received here and applied to any local
    // sockets (via the consumer loop below), enforcing live disconnect fan-out.
    let pubsub_for_conn_ctrl = Arc::clone(&pubsub);
    tokio::spawn(async move { pubsub_for_conn_ctrl.run_conn_control_subscriber().await });

    let auth = AuthService::new(config.auth.clone());

    // Postgres FTS: the searchable row IS the persisted event row (its
    // `tsvector` column is populated by the `insert_event` write), so there is
    // no external collection to provision — the search service just queries the
    // same Postgres over its own pool.
    let search_pool = sqlx::postgres::PgPoolOptions::new()
        .connect(&config.database_url)
        .await
        .map_err(|e| anyhow::anyhow!("Search DB connection failed: {e}"))?;
    let search = SearchService::new(search_pool);
    info!("Search service ready (Postgres FTS)");

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
            // Resolve the deployment's community from the configured relay URL
            // host (single-community per deployment), failing closed if the host
            // isn't mapped — the membership list is community-scoped now, so the
            // relay-signed startup publish must carry a resolved tenant.
            let tenant = match buzz_relay::tenant::bind_deployment_community(
                &startup_state.db,
                &startup_state.config.relay_url,
            )
            .await
            {
                Ok(ctx) => ctx,
                Err(e) => {
                    tracing::warn!(
                        error = ?e,
                        "initial NIP-43 membership list skipped: relay host is not mapped to a community"
                    );
                    return;
                }
            };
            if let Err(e) = buzz_relay::handlers::side_effects::publish_nip43_membership_list(
                &tenant,
                &startup_state,
            )
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
            // Resolve the deployment's community from the configured relay URL
            // host (dev/CI runs single-community), failing closed if the host
            // isn't mapped — the reconciler is community-scoped now, so there is
            // no global "all channels" sweep.
            let tenant = match buzz_relay::tenant::bind_deployment_community(
                &reconcile_state.db,
                &reconcile_state.config.relay_url,
            )
            .await
            {
                Ok(ctx) => ctx,
                Err(e) => {
                    tracing::warn!(
                        error = ?e,
                        "channel reconciliation skipped: relay host is not mapped to a community"
                    );
                    return;
                }
            };
            // Try immediately, then retry every 5s for up to 2 minutes.
            // Handles CI pattern: relay starts → seed script inserts data → reconciliation.
            for attempt in 0..24u32 {
                if attempt > 0 {
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                }
                match buzz_relay::handlers::side_effects::reconcile_channel_events(
                    &tenant,
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

                for channel in &expired {
                    // Per-row tenant: the reaper crosses communities, so each
                    // archived channel carries its own server-resolved
                    // `(community, host)` from the DB RETURNING. Build the
                    // `TenantContext` from that row — never a default tenant.
                    let tenant = buzz_core::tenant::TenantContext::resolved(
                        channel.community_id,
                        channel.host.clone(),
                    );
                    let channel_id = channel.channel_id;
                    // Emit a system message so members see why the channel was archived.
                    if let Err(e) = buzz_relay::handlers::side_effects::emit_system_message(
                        &tenant,
                        &reaper_state,
                        channel_id,
                        serde_json::json!({ "type": "channel_auto_archived" }),
                    )
                    .await
                    {
                        error!(channel = %channel_id, "reaper system message failed: {e}");
                    }

                    // Update NIP-29 discovery events so clients see the archived state.
                    if let Err(e) = buzz_relay::handlers::side_effects::emit_group_discovery_events(
                        &tenant,
                        &reaper_state,
                        channel_id,
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
                        &tenant,
                        &reaper_state,
                        channel_id,
                    )
                    .await;
                }
            }
        });
    }

    // NIP-PL matcher and worker are enabled as one unit. Lease acceptance is
    // already disabled without the exact gateway URL, so discovery and runtime
    // cannot advertise or accumulate work for an undeliverable configuration.
    if state.config.push_gateway_delivery_url.is_some() {
        tokio::spawn(buzz_relay::push_runtime::run_matcher(Arc::clone(&state)));
        tokio::spawn(buzz_relay::push_runtime::run_delivery_worker(Arc::clone(
            &state,
        )));
        info!("NIP-PL push matcher and delivery worker started");
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
            // The scheduler is a background sweep with no inbound connection,
            // so it cannot use a request Host header as tenant provenance. Each
            // DueReminder row carries `(community_id, host)` from the DB row's
            // community join (mirroring the ephemeral-channel reaper); publish
            // each reminder to that row's community-global topic.
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
                    // Claim before side effect (§5c: claim-before-publish). A
                    // unique per-attempt stamp lets a failed publish roll back
                    // exactly this pod's claim via compare-and-clear, without a
                    // racing pod's later claim being clobbered. `delivered_at`
                    // is only ever read as a NULL/non-NULL sentinel (the
                    // due-reminder query guard and the partial index), never as
                    // a wall-clock value, so an opaque stamp is safe to store.
                    let reminder_tenant = buzz_core::tenant::TenantContext::resolved(
                        reminder.community_id,
                        reminder.host.clone(),
                    );
                    let delivery_stamp = chrono::Utc::now()
                        .timestamp_nanos_opt()
                        .unwrap_or_else(|| chrono::Utc::now().timestamp())
                        ^ rand::random::<i64>();

                    match scheduler_state
                        .db
                        .claim_due_reminder_with_stamp(
                            reminder.community_id,
                            &reminder.id,
                            reminder.created_at,
                            delivery_stamp,
                        )
                        .await
                    {
                        Ok(true) => {}         // We won the claim — proceed to publish.
                        Ok(false) => continue, // Another pod claimed it; no side effect here.
                        Err(e) => {
                            warn!(
                                event_id = hex::encode(&reminder.id),
                                "Reminder scheduler: claim failed, skipping publish: {e}"
                            );
                            continue;
                        }
                    }

                    // Publish the single side effect. On failure, release our
                    // claim so the next tick (this pod or another) can retry —
                    // the stamp guard ensures we only clear our own claim.
                    if let Err(e) = scheduler_state
                        .pubsub
                        .publish_event(
                            &reminder_tenant,
                            buzz_pubsub::EventTopic::Global,
                            &reminder_to_event(&reminder),
                        )
                        .await
                    {
                        error!(
                            event_id = hex::encode(&reminder.id),
                            "Reminder scheduler: Redis publish failed after claim, releasing: {e}"
                        );
                        if let Err(release_err) = scheduler_state
                            .db
                            .release_due_reminder(
                                reminder.community_id,
                                &reminder.id,
                                reminder.created_at,
                                delivery_stamp,
                            )
                            .await
                        {
                            warn!(
                                event_id = hex::encode(&reminder.id),
                                "Reminder scheduler: release after failed publish errored \
                                 (reminder stays claimed, will not retry): {release_err}"
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
                    Ok(scoped) => {
                        // The Redis topic carries the originating community,
                        // and the local moka keys carry that same label. Apply
                        // only the matching tenant-local drop; a mutation in A
                        // must not flush B's derived state.
                        state_for_cache
                            .apply_cache_invalidation(scoped.community_id, scoped.invalidation);
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

    // Durable lifecycle backstop: Redis pub/sub cannot deliver to a pod that was
    // offline. Periodically revalidate only communities with local live sockets
    // so missed archive commands still converge without a global DB scan.
    {
        let lifecycle_state = Arc::clone(&state);
        let interval_secs = std::env::var("BUZZ_COMMUNITY_REVALIDATE_INTERVAL_SECS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(30)
            .clamp(1, 300);
        let cancel = lifecycle_state.community_revalidator_cancel.clone();
        tokio::spawn(run_community_revalidator(
            lifecycle_state,
            std::time::Duration::from_secs(interval_secs),
            cancel,
        ));
    }

    // Cross-pod connection-control consumer: receive disconnect commands from
    // Redis pub/sub (published by the pod that recorded a ban) and close any
    // matching local sockets. A member's live connections may land on any pod,
    // so this is how a ban reaches sockets the banning pod does not hold. The DB
    // ban row is the durable backstop; even a dropped command still refuses the
    // banned member's next auth attempt at the auth seam.
    {
        let state_for_conn_ctrl = Arc::clone(&state);
        let mut rx = state_for_conn_ctrl.pubsub.subscribe_conn_control();
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(scoped) => match scoped.command {
                        buzz_pubsub::conn_control::ConnControl::DisconnectCommunity => {
                            state_for_conn_ctrl
                                .community_connections
                                .disconnect_community(scoped.community_id);
                        }
                        buzz_pubsub::conn_control::ConnControl::DisconnectPubkey {
                            pubkey,
                            event_id,
                            reason,
                        } => {
                            state_for_conn_ctrl.conn_manager.disconnect_pubkey(
                                scoped.community_id,
                                &pubkey,
                                &event_id,
                                &reason,
                            );
                        }
                    },
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        metrics::counter!("buzz_conn_control_lag_total").increment(n);
                        tracing::warn!("Connection-control consumer lagged by {n} messages");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        tracing::error!("Connection-control broadcast channel closed");
                        break;
                    }
                }
            }
        });
    }

    let router = build_router(Arc::clone(&state));
    let health_router = build_health_router(Arc::clone(&state));

    // Pool metrics: periodic background task polling DB + Redis pool stats.
    {
        let pool_state = Arc::clone(&state);
        let interval_secs = std::env::var("BUZZ_POOL_METRICS_INTERVAL_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(10)
            .max(1); // tokio::time::interval panics on Duration::ZERO
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
            loop {
                interval.tick().await;
                let db_stats = pool_state.db.pool_stats();
                let active = db_stats.size.saturating_sub(db_stats.idle);
                metrics::gauge!("buzz_db_pool_size").set(db_stats.size as f64);
                metrics::gauge!("buzz_db_pool_idle").set(db_stats.idle as f64);
                metrics::gauge!("buzz_db_pool_active").set(active as f64);
                metrics::gauge!("buzz_db_pool_max").set(db_stats.max as f64);

                let rs = pool_state.redis_pool.status();
                metrics::gauge!("buzz_redis_pool_available").set(rs.available as f64);
                metrics::gauge!("buzz_redis_pool_size").set(rs.size as f64);
                metrics::gauge!("buzz_redis_pool_max").set(rs.max_size as f64);
                metrics::gauge!("buzz_redis_pool_waiting").set(rs.waiting as f64);
            }
        });
    }

    // Usage metrics: periodic background task polling per-community stats.
    //
    // DB-derived gauges (users, channels, messages, members, workflows, git
    // repos, active users/channels) are SET from GROUP BY queries — one per
    // tick. In-memory gauges (ws_connections, subscriptions, users_online)
    // are snapshotted from live in-memory state. Both avoid inc/dec drift.
    //
    // Multi-pod semantics:
    //   DB-derived: all pods export the same value → dashboard uses max()
    //   In-memory:  each pod exports its partition → dashboard uses sum()
    {
        let usage_state = Arc::clone(&state);
        let per_community_mode = PerCommunityMode::from_env();
        let interval_secs = std::env::var("BUZZ_USAGE_METRICS_INTERVAL_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(300) // 300s default: adoption stocks don't need minute freshness;
            // if event-table rollups (message counts, DAU/WAU/MAU) become
            // slow at scale, move them to a maintained rollup table and drop
            // the interval back to 60s.
            .max(5); // 5s minimum: cheaper than pool poller's 1s minimum
        tokio::spawn(async move {
            // Jitter the first tick by a random fraction of the interval so
            // that a rolling deploy with N pods doesn't hammer the DB
            // simultaneously at boot. Each pod picks a start delay in
            // [0, interval_secs) using true per-process randomness (PID-derived
            // seeds are unsafe in containers where the relay is typically PID 1
            // in every pod, which would make all pods compute the same delay).
            let jitter_secs = rand::random::<u64>() % interval_secs;
            tokio::time::sleep(std::time::Duration::from_secs(jitter_secs)).await;

            let mut interval = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
            // Skip a tick rather than scheduling a burst of catch-up ticks if
            // the system falls behind (e.g. the previous tick took > interval).
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                if let Err(e) = run_usage_metrics_tick(&usage_state, &per_community_mode).await {
                    error!(error = %e, "Usage metrics tick failed — skipping");
                }
            }
        });
    }

    serve(router, health_router, Arc::clone(&state)).await?;
    state.community_revalidator_cancel.cancel();

    // Signal the audit worker to stop accepting, flush buffered entries, and
    // exit. Uses a CancellationToken so it works regardless of how many
    // Arc<AppState> clones are still alive in background tasks.
    audit_shutdown
        .drain(std::time::Duration::from_secs(5))
        .await;

    // Flush pending OTEL spans before exit.
    if let telemetry::TracerInit::Enabled(tp) = tracer_init {
        if let Err(e) = tp.shutdown() {
            tracing::warn!(error = %e, "OTEL tracer provider shutdown error");
        }
    }

    Ok(())
}

async fn run_community_revalidator(
    state: Arc<AppState>,
    period: std::time::Duration,
    cancel: CancellationToken,
) {
    run_periodic_until_cancelled(period, cancel, || async {
        let closed = state.revalidate_live_communities().await;
        if closed > 0 {
            tracing::info!(
                closed,
                "closed sockets for inactive communities during lifecycle revalidation"
            );
        }
    })
    .await;
}

async fn run_periodic_until_cancelled<Tick, TickFuture>(
    period: std::time::Duration,
    cancel: CancellationToken,
    mut tick: Tick,
) where
    Tick: FnMut() -> TickFuture,
    TickFuture: std::future::Future<Output = ()>,
{
    let mut interval = tokio::time::interval(period);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => break,
            _ = interval.tick() => tick().await,
        }
    }
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

/// Run one tick of the usage metrics poller.
///
/// Queries the DB for per-community stock counts and snapshots in-memory
/// state for connection/subscription gauges. On any DB error, returns `Err`
/// so the caller can log and skip the tick without crashing.
///
/// All DB-derived gauges use absolute SET (not increment), so they self-heal
/// after a missed tick or a pod restart.
async fn run_usage_metrics_tick(
    state: &AppState,
    per_community_mode: &PerCommunityMode,
) -> anyhow::Result<()> {
    // --- community id → host label map (one query, cached for this tick) ---
    let hosts = state.db.usage_community_hosts().await?;
    let host_map: HashMap<Uuid, String> = hosts.into_iter().map(|c| (c.id, c.host)).collect();

    // --- Collect all DB results before emitting any metrics (C4) ---
    //
    // All `.await?` calls happen here. If any query fails the function returns
    // early — no metrics are emitted for this tick — preventing a mixed
    // fresh/stale snapshot where later gauges retain their last value while
    // earlier ones are updated.

    let community_total = state.db.usage_community_count().await?;
    let user_rows = state.db.usage_user_counts().await?;
    let channel_rows = state.db.usage_channel_counts().await?;
    let message_rows = state.db.usage_message_counts().await?;
    let relay_member_rows = state.db.usage_relay_member_counts().await?;
    let workflow_rows = state.db.usage_workflow_counts().await?;
    let git_repo_rows = state.db.usage_git_repo_counts().await?;
    let active_users_1d = state.db.usage_active_user_counts("1 day").await?;
    let active_users_7d = state.db.usage_active_user_counts("7 days").await?;
    let active_users_30d = state.db.usage_active_user_counts("30 days").await?;
    let active_channels_1d = state.db.usage_active_channel_counts("1 day").await?;
    let active_channels_7d = state.db.usage_active_channel_counts("7 days").await?;

    // In-memory snapshots are infallible — snapshot once before publish phase.
    let conns_snapshot = state.conn_manager.per_community_ws_connections();
    let online_snapshot = state.conn_manager.per_community_users_online();
    let subs_snapshot = state.sub_registry.per_community_subscriptions();

    // --- Determine which community IDs receive per-community series (K1) ---
    //
    // `active_set` is the subset of host_map IDs that get per-community gauges
    // this tick. Fleet-wide totals (buzz_total_*) always emit regardless.
    let active_set: std::collections::HashSet<Uuid> = match per_community_mode {
        PerCommunityMode::All => host_map.keys().copied().collect(),
        PerCommunityMode::Off => std::collections::HashSet::new(),
    };

    // --- Publish phase: emit all metrics now that every query succeeded ---

    // --- A. Adoption stocks (DB-polled) ---

    // buzz_communities_total (no tag — fleet-wide count)
    metrics::gauge!("buzz_communities_total").set(community_total as f64);

    // buzz_community_users{community, type:human|agent}
    // Emit from host_map so communities that have zero users still get a 0
    // rather than keeping the last nonzero value until process restart.
    {
        let rows: HashMap<Uuid, _> = user_rows.into_iter().map(|r| (r.community_id, r)).collect();
        // Fleet totals (always emitted).
        let (total_human, total_agent): (i64, i64) = rows
            .values()
            .fold((0, 0), |(h, a), r| (h + r.human, a + r.agent));
        metrics::gauge!("buzz_total_users", "type" => "human").set(total_human as f64);
        metrics::gauge!("buzz_total_users", "type" => "agent").set(total_agent as f64);
        // Per-community series (gated by active_set).
        for (&id, community) in &host_map {
            if !active_set.contains(&id) {
                continue;
            }
            let (human, agent) = rows.get(&id).map(|r| (r.human, r.agent)).unwrap_or((0, 0));
            metrics::gauge!("buzz_community_users", "community" => community.clone(), "type" => "human")
                .set(human as f64);
            metrics::gauge!("buzz_community_users", "community" => community.clone(), "type" => "agent")
                .set(agent as f64);
        }
    }

    // buzz_community_channels{community, type}
    // Zero-fill across all (community, channel_type) pairs so a type that
    // drops to zero emits 0 rather than retaining its last nonzero value.
    {
        const CHANNEL_TYPES: &[&str] = &["stream", "forum", "dm", "workflow"];
        let rows: HashMap<(Uuid, &str), i64> = channel_rows
            .into_iter()
            .filter_map(|r| {
                let matched = CHANNEL_TYPES
                    .iter()
                    .find(|&&t| t == r.channel_type.as_str())
                    .map(|&t| ((r.community_id, t), r.count));
                if matched.is_none() {
                    warn!(
                        channel_type = %r.channel_type,
                        "usage_channel_counts: unrecognised channel_type — row skipped"
                    );
                }
                matched
            })
            .collect();
        // Fleet totals (always emitted).
        for &ct in CHANNEL_TYPES {
            let total: i64 = host_map
                .keys()
                .map(|id| rows.get(&(*id, ct)).copied().unwrap_or(0))
                .sum();
            metrics::gauge!("buzz_total_channels", "type" => ct).set(total as f64);
        }
        // Per-community series (gated by active_set).
        for (&id, community) in &host_map {
            if !active_set.contains(&id) {
                continue;
            }
            for &ct in CHANNEL_TYPES {
                let count = rows.get(&(id, ct)).copied().unwrap_or(0);
                metrics::gauge!(
                    "buzz_community_channels",
                    "community" => community.clone(),
                    "type" => ct
                )
                .set(count as f64);
            }
        }
    }

    // buzz_community_messages{community}
    // Emit 0 for communities with no messages so dashboards don't stale-read.
    {
        let rows: HashMap<Uuid, i64> = message_rows
            .into_iter()
            .map(|r| (r.community_id, r.count))
            .collect();
        // Fleet total (always emitted).
        let total: i64 = rows.values().sum();
        metrics::gauge!("buzz_total_messages").set(total as f64);
        // Per-community series (gated by active_set).
        for (&id, community) in &host_map {
            if !active_set.contains(&id) {
                continue;
            }
            let count = rows.get(&id).copied().unwrap_or(0);
            metrics::gauge!("buzz_community_messages", "community" => community.clone())
                .set(count as f64);
        }
    }

    // buzz_community_relay_members{community, role}
    // Zero-fill across all (community, role) pairs; relay_members.role is a
    // CHECK constraint over {'owner', 'admin', 'member'}.
    {
        const RELAY_ROLES: &[&str] = &["owner", "admin", "member"];
        let rows: HashMap<(Uuid, &str), i64> = relay_member_rows
            .into_iter()
            .filter_map(|r| {
                let matched = RELAY_ROLES
                    .iter()
                    .find(|&&role| role == r.role.as_str())
                    .map(|&role| ((r.community_id, role), r.count));
                if matched.is_none() {
                    warn!(
                        role = %r.role,
                        "usage_relay_member_counts: unrecognised role — row skipped"
                    );
                }
                matched
            })
            .collect();
        // Fleet totals (always emitted).
        for &role in RELAY_ROLES {
            let total: i64 = host_map
                .keys()
                .map(|id| rows.get(&(*id, role)).copied().unwrap_or(0))
                .sum();
            metrics::gauge!("buzz_total_relay_members", "role" => role).set(total as f64);
        }
        // Per-community series (gated by active_set).
        for (&id, community) in &host_map {
            if !active_set.contains(&id) {
                continue;
            }
            for &role in RELAY_ROLES {
                let count = rows.get(&(id, role)).copied().unwrap_or(0);
                metrics::gauge!(
                    "buzz_community_relay_members",
                    "community" => community.clone(),
                    "role" => role
                )
                .set(count as f64);
            }
        }
    }

    // buzz_community_workflows{community, status}
    // Zero-fill across all (community, status) pairs; workflow_status is a
    // DB enum: {'active', 'disabled', 'archived'}.
    {
        const WORKFLOW_STATUSES: &[&str] = &["active", "disabled", "archived"];
        let rows: HashMap<(Uuid, &str), i64> = workflow_rows
            .into_iter()
            .filter_map(|r| {
                let matched = WORKFLOW_STATUSES
                    .iter()
                    .find(|&&s| s == r.status.as_str())
                    .map(|&s| ((r.community_id, s), r.count));
                if matched.is_none() {
                    warn!(
                        status = %r.status,
                        "usage_workflow_counts: unrecognised workflow status — row skipped"
                    );
                }
                matched
            })
            .collect();
        // Fleet totals (always emitted).
        for &status in WORKFLOW_STATUSES {
            let total: i64 = host_map
                .keys()
                .map(|id| rows.get(&(*id, status)).copied().unwrap_or(0))
                .sum();
            metrics::gauge!("buzz_total_workflows", "status" => status).set(total as f64);
        }
        // Per-community series (gated by active_set).
        for (&id, community) in &host_map {
            if !active_set.contains(&id) {
                continue;
            }
            for &status in WORKFLOW_STATUSES {
                let count = rows.get(&(id, status)).copied().unwrap_or(0);
                metrics::gauge!(
                    "buzz_community_workflows",
                    "community" => community.clone(),
                    "status" => status
                )
                .set(count as f64);
            }
        }
    }

    // buzz_community_git_repos{community}
    // Emit 0 for communities with no repos.
    {
        let rows: HashMap<Uuid, i64> = git_repo_rows
            .into_iter()
            .map(|r| (r.community_id, r.count))
            .collect();
        // Fleet total (always emitted).
        let total: i64 = rows.values().sum();
        metrics::gauge!("buzz_total_git_repos").set(total as f64);
        // Per-community series (gated by active_set).
        for (&id, community) in &host_map {
            if !active_set.contains(&id) {
                continue;
            }
            let count = rows.get(&id).copied().unwrap_or(0);
            metrics::gauge!("buzz_community_git_repos", "community" => community.clone())
                .set(count as f64);
        }
    }

    // --- C. Engagement — windowed DAU/WAU/MAU + active channels ---
    // Emit 0 for window/type/community combos that had no activity; this
    // ensures a community that was active last tick but quiet this tick reads
    // 0 rather than retaining its last nonzero value.

    for (data, label) in [
        (active_users_1d, "1d"),
        (active_users_7d, "7d"),
        (active_users_30d, "30d"),
    ] {
        let rows: HashMap<Uuid, _> = data.into_iter().map(|r| (r.community_id, r)).collect();
        // Fleet totals (always emitted).
        let (total_human, total_agent, total_unknown): (i64, i64, i64) =
            rows.values().fold((0, 0, 0), |(h, a, u), r| {
                (h + r.human, a + r.agent, u + r.unknown)
            });
        metrics::gauge!("buzz_total_active_users", "window" => label, "type" => "human")
            .set(total_human as f64);
        metrics::gauge!("buzz_total_active_users", "window" => label, "type" => "agent")
            .set(total_agent as f64);
        metrics::gauge!("buzz_total_active_users", "window" => label, "type" => "unknown")
            .set(total_unknown as f64);
        // Per-community series (gated by active_set).
        for (&id, community) in &host_map {
            if !active_set.contains(&id) {
                continue;
            }
            let (human, agent, unknown) = rows
                .get(&id)
                .map(|r| (r.human, r.agent, r.unknown))
                .unwrap_or((0, 0, 0));
            metrics::gauge!(
                "buzz_community_active_users",
                "community" => community.clone(),
                "window" => label,
                "type" => "human"
            )
            .set(human as f64);
            metrics::gauge!(
                "buzz_community_active_users",
                "community" => community.clone(),
                "window" => label,
                "type" => "agent"
            )
            .set(agent as f64);
            metrics::gauge!(
                "buzz_community_active_users",
                "community" => community.clone(),
                "window" => label,
                "type" => "unknown"
            )
            .set(unknown as f64);
        }
    }

    for (data, label) in [(active_channels_1d, "1d"), (active_channels_7d, "7d")] {
        let rows: HashMap<Uuid, i64> = data
            .into_iter()
            .map(|r| (r.community_id, r.count))
            .collect();
        // Fleet total (always emitted).
        let total: i64 = rows.values().sum();
        metrics::gauge!("buzz_total_active_channels", "window" => label).set(total as f64);
        // Per-community series (gated by active_set).
        for (&id, community) in &host_map {
            if !active_set.contains(&id) {
                continue;
            }
            let count = rows.get(&id).copied().unwrap_or(0);
            metrics::gauge!(
                "buzz_community_active_channels",
                "community" => community.clone(),
                "window" => label
            )
            .set(count as f64);
        }
    }

    // --- D. Realtime — in-memory snapshots ---
    // All three gauges are derived from the in-memory conn/sub registries.
    // We emit from host_map so communities that just dropped to zero
    // (last connection closed, all subs removed) receive an explicit 0
    // rather than keeping the stale last value.

    // buzz_community_ws_connections{community}
    {
        // Fleet total (always emitted).
        let total: u64 = conns_snapshot.values().sum();
        metrics::gauge!("buzz_total_ws_connections").set(total as f64);
        // Per-community series (gated by active_set).
        for (&id, community) in &host_map {
            if !active_set.contains(&id) {
                continue;
            }
            let count = conns_snapshot
                .get(&CommunityId::from_uuid(id))
                .copied()
                .unwrap_or(0);
            metrics::gauge!("buzz_community_ws_connections", "community" => community.clone())
                .set(count as f64);
        }
    }

    // buzz_community_users_online_pod{community}
    // This is a pod-local distinct count — a pubkey connected to N pods is
    // counted once per pod. Dashboard queries should sum across pods to get
    // the fleet-wide total (with the caveat that multi-pod connections are
    // counted N times). The metric name "_pod" suffix makes this explicit.
    {
        // Fleet total (always emitted; same pod-local caveat applies).
        let total: u64 = online_snapshot.values().sum();
        metrics::gauge!("buzz_total_users_online_pod").set(total as f64);
        // Per-community series (gated by active_set).
        for (&id, community) in &host_map {
            if !active_set.contains(&id) {
                continue;
            }
            let count = online_snapshot
                .get(&CommunityId::from_uuid(id))
                .copied()
                .unwrap_or(0);
            metrics::gauge!("buzz_community_users_online_pod", "community" => community.clone())
                .set(count as f64);
        }
    }

    // buzz_community_subscriptions{community}
    {
        // Fleet total (always emitted).
        let total: u64 = subs_snapshot.values().sum();
        metrics::gauge!("buzz_total_subscriptions").set(total as f64);
        // Per-community series (gated by active_set).
        for (&id, community) in &host_map {
            if !active_set.contains(&id) {
                continue;
            }
            let count = subs_snapshot
                .get(&CommunityId::from_uuid(id))
                .copied()
                .unwrap_or(0);
            metrics::gauge!("buzz_community_subscriptions", "community" => community.clone())
                .set(count as f64);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use tokio_util::sync::CancellationToken;

    use super::{buzz_auto_migrate_enabled, run_periodic_until_cancelled};

    #[tokio::test(start_paused = true)]
    async fn periodic_loop_exits_immediately_on_cancellation() {
        let cancel = CancellationToken::new();
        let tick_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count_for_tick = Arc::clone(&tick_count);
        let task_cancel = cancel.clone();
        let task = tokio::spawn(async move {
            run_periodic_until_cancelled(Duration::from_secs(300), task_cancel, move || {
                let count = Arc::clone(&count_for_tick);
                async move {
                    count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            })
            .await;
        });

        tokio::task::yield_now().await;
        cancel.cancel();
        tokio::time::timeout(Duration::from_millis(1), task)
            .await
            .expect("loop must not wait for the next interval")
            .expect("loop task");
        assert!(tick_count.load(std::sync::atomic::Ordering::Relaxed) <= 1);
    }

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
