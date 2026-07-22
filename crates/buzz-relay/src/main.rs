use std::collections::{HashMap, HashSet};
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
use buzz_relay::storage_sweep;
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
enum EmissionScope {
    All,
    Off,
}

impl EmissionScope {
    fn from_env() -> Self {
        let raw = std::env::var("BUZZ_USAGE_METRICS_PER_COMMUNITY")
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        match raw.as_str() {
            "" | "all" => EmissionScope::All,
            "off" => EmissionScope::Off,
            other => {
                warn!(
                    value = other,
                    "BUZZ_USAGE_METRICS_PER_COMMUNITY: unknown value — defaulting to all"
                );
                EmissionScope::All
            }
        }
    }

    fn allows(&self, _community_id: &Uuid) -> bool {
        matches!(self, Self::All)
    }
}

const USAGE_METRICS_LOCK_KEY: i64 = 0x4255_5A5A_4D45_5452;

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
        audit_enabled = config.audit_enabled,
        "Config loaded"
    );

    let usage_interval_secs = usage_metrics_interval_secs();
    let usage_idle_timeout_secs = usage_metrics_idle_timeout_secs(usage_interval_secs);
    relay_metrics::install(config.metrics_port, usage_idle_timeout_secs);
    metrics::gauge!("buzz_audit_enabled").set(if config.audit_enabled { 1.0 } else { 0.0 });
    info!(
        port = config.metrics_port,
        idle_timeout_secs = usage_idle_timeout_secs,
        "Prometheus metrics exporter started"
    );

    let db_config = DbConfig {
        database_url: config.database_url.clone(),
        read_database_url: config.read_database_url.clone(),
        ..DbConfig::default()
    };
    let db = Db::new(&db_config).await.map_err(|e| {
        error!("Failed to connect to Postgres: {e}");
        anyhow::anyhow!("DB connection failed: {e}")
    })?;
    if db.has_read_pool() {
        info!("Postgres connected (writer + read replica)");
    } else {
        info!("Postgres connected");
    }

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

    // Freshness fence probe: cursor pages route to the replica only for
    // history the probe has verified as fully replayed. Deliberately AFTER
    // the migration decision: spawn_fence_probe first verifies the
    // commit-time floor guard (catalog shape + observed behavior through the
    // armed pool) against the live schema, so a relay running with
    // BUZZ_AUTO_MIGRATE off and migration 0021 unapplied can never open the
    // fence over an unenforced floor. Verification failure is loud but
    // non-fatal: the fence stays closed and every cursor page routes to the
    // writer.
    match db.spawn_fence_probe().await {
        Ok(true) => info!("Replica fence probe started (floor guard verified)"),
        Ok(false) => {}
        Err(e) => {
            error!(
                "Replica fence disabled — floor guard verification failed: {e}. \
                 All cursor reads stay on the writer."
            );
        }
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

    let audit = if config.audit_enabled {
        let audit_pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(5)
            .min_connections(1)
            .connect(&config.database_url)
            .await
            .map_err(|e| anyhow::anyhow!("Audit DB connection failed: {e}"))?;
        info!("Audit service ready");
        Some(AuditService::new(audit_pool))
    } else {
        info!("Audit logging disabled by BUZZ_AUDIT_ENABLED");
        None
    };

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
    // same Postgres over its own pool. Search is lag-tolerant, so it prefers
    // the read replica when one is configured.
    let search_db_url = config
        .read_database_url
        .as_deref()
        .unwrap_or(&config.database_url);
    let search_pool = sqlx::postgres::PgPoolOptions::new()
        .connect(search_db_url)
        .await
        .map_err(|e| anyhow::anyhow!("Search DB connection failed: {e}"))?;
    let search = SearchService::new(search_pool);
    info!(
        replica = config.read_database_url.is_some(),
        "Search service ready (Postgres FTS)"
    );

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

    // Inter-relay mesh (BUZZ_MESH seam). `boot_mesh` returns None when the
    // kill switch is off — nothing is bound, published, or spawned, so the
    // relay behaves byte-identically to a build without the mesh. When
    // enabled, a misconfigured mesh is fatal here (bind/Redis failure): an
    // operator who asked for the mesh gets it or gets told why not.
    if let Some(handle) = buzz_relay::mesh_boot::boot_mesh(
        &state.config,
        state.redis_pool.clone(),
        &state.relay_keypair,
        Arc::clone(&state.shutting_down),
    )
    .await?
    {
        let runtime_id = handle.local_runtime_id;
        // Register the per-profile inbound consumers (huddle datagram fan-in,
        // HuddleControl accept loop, reliable-stream accept + optional
        // BUZZ_MESH_DEMO_ECHO) before peers can route traffic here.
        handle.wire_consumers(
            Arc::clone(&state.audio_rooms),
            state.config.mesh_demo_echo,
            Arc::clone(&state.shutting_down),
        );
        if state.mesh.set(handle).is_err() {
            unreachable!("mesh handle is set exactly once, right here");
        }
        info!(runtime_id = %runtime_id, "Inter-relay mesh started");
    }

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

    // NIP-43: reconcile the event-backed roster for every provisioned
    // community before opening the listener. `relay_members` is canonical;
    // this repairs pre-snapshot communities and any publication that failed
    // after a membership transaction committed.
    if config.require_relay_membership {
        match buzz_relay::handlers::side_effects::reconcile_nip43_membership_snapshots(&state).await
        {
            Ok(count) => info!(count, "NIP-43 membership snapshots reconciled on startup"),
            Err(error) => {
                tracing::warn!(%error, "NIP-43 membership snapshot startup reconciliation failed")
            }
        }

        let reconcile_state = Arc::clone(&state);
        let interval_secs = std::env::var("BUZZ_NIP43_RECONCILE_INTERVAL_SECS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(60)
            .max(1);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
            interval.tick().await;
            loop {
                interval.tick().await;
                match buzz_relay::handlers::side_effects::reconcile_nip43_membership_snapshots(
                    &reconcile_state,
                )
                .await
                {
                    Ok(count) if count > 0 => {
                        info!(count, "NIP-43 membership snapshots repaired")
                    }
                    Ok(_) => {}
                    Err(error) => tracing::warn!(
                        %error,
                        "periodic NIP-43 membership snapshot reconciliation failed"
                    ),
                }
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

                if let Some(read_stats) = pool_state.db.read_pool_stats() {
                    let read_active = read_stats.size.saturating_sub(read_stats.idle);
                    metrics::gauge!("buzz_db_read_pool_size").set(read_stats.size as f64);
                    metrics::gauge!("buzz_db_read_pool_idle").set(read_stats.idle as f64);
                    metrics::gauge!("buzz_db_read_pool_active").set(read_active as f64);
                    metrics::gauge!("buzz_db_read_pool_max").set(read_stats.max as f64);

                    // Fence observability: 1 when replica routing is
                    // eligible, and the verified-freshness lag in seconds.
                    // Closed/stale fence reports open=0 with lag untouched.
                    match pool_state.db.fence().verified_through() {
                        Some(fence_ts) => {
                            let lag = (chrono::Utc::now() - fence_ts).num_seconds();
                            metrics::gauge!("buzz_db_replica_fence_open").set(1.0);
                            metrics::gauge!("buzz_db_replica_fence_lag_seconds").set(lag as f64);
                        }
                        None => {
                            metrics::gauge!("buzz_db_replica_fence_open").set(0.0);
                        }
                    }
                }

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
        let emission_scope = EmissionScope::from_env();
        let interval_secs = usage_interval_secs;
        let mut leader = None;
        let mut emitted_in_memory = HashSet::new();
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
                if let Err(e) = run_usage_metrics_tick(
                    &usage_state,
                    &emission_scope,
                    &mut leader,
                    &mut emitted_in_memory,
                )
                .await
                {
                    error!(error = %e, "Usage metrics tick failed — skipping");
                }
                metrics::gauge!("buzz_usage_poller_is_leader").set(if leader.is_some() {
                    1.0
                } else {
                    0.0
                });
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

/// Return the usage poll interval, with a floor that prevents a busy loop.
fn usage_metrics_interval_secs() -> u64 {
    std::env::var("BUZZ_USAGE_METRICS_INTERVAL_SECS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(300)
        .max(5)
}

/// Return a gauge lifetime that always outlives several usage-poller ticks.
fn usage_metrics_idle_timeout_secs(interval_secs: u64) -> u64 {
    let configured = std::env::var("BUZZ_USAGE_METRICS_IDLE_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.parse().ok());
    idle_timeout_secs(configured, interval_secs)
}

fn idle_timeout_secs(configured: Option<u64>, interval_secs: u64) -> u64 {
    configured
        .unwrap_or(900)
        .max(interval_secs.saturating_mul(3))
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum InMemoryMetricKey {
    WsConnections(String),
    UsersOnline(String),
    Subscriptions(String),
}

impl InMemoryMetricKey {
    fn set(&self, value: f64) {
        match self {
            Self::WsConnections(community) => {
                metrics::gauge!("buzz_community_ws_connections", "community" => community.clone())
                    .set(value);
            }
            Self::UsersOnline(community) => {
                metrics::gauge!("buzz_community_users_online_pod", "community" => community.clone())
                    .set(value);
            }
            Self::Subscriptions(community) => {
                metrics::gauge!("buzz_community_subscriptions", "community" => community.clone())
                    .set(value);
            }
        }
    }
}

/// Refresh the exporter recency for legacy event-driven gauges without changing
/// their values.
///
/// `metrics-util` 0.20.4 increments the Prometheus recorder's generation on
/// every gauge operation, including `increment(0.0)`. The recency policy uses
/// that generation, so this retains a steady gauge without a snapshot `set()`
/// racing the lifecycle-relative increments and decrements.
fn refresh_legacy_active_gauge_recency() {
    metrics::gauge!("buzz_ws_connections_active").increment(0.0);
    metrics::gauge!("buzz_subscriptions_active").increment(0.0);
}

/// Emit pod-local gauges and zero only label keys that disappeared since the
/// preceding tick. The key stores the resolved host label so a removed or
/// renamed community can still receive its final zero.
fn emit_in_memory_usage_metrics(
    state: &AppState,
    emission_scope: &EmissionScope,
    host_map: Option<&HashMap<Uuid, String>>,
    previously_emitted: &mut HashSet<InMemoryMetricKey>,
) {
    let connections = state.conn_manager.per_community_ws_connections();
    let users_online = state.conn_manager.per_community_users_online();
    let subscriptions = state.sub_registry.per_community_subscriptions();
    let total_connections = connections.values().sum::<u64>();
    let total_subscriptions = subscriptions.values().sum::<u64>();

    metrics::gauge!("buzz_total_ws_connections").set(total_connections as f64);
    metrics::gauge!("buzz_total_users_online_pod").set(users_online.values().sum::<u64>() as f64);
    metrics::gauge!("buzz_total_subscriptions").set(total_subscriptions as f64);
    refresh_legacy_active_gauge_recency();

    let Some(host_map) = host_map else {
        return;
    };

    let mut current = HashSet::new();
    for (id, host) in host_map {
        if !emission_scope.allows(id) {
            continue;
        }
        let community_id = CommunityId::from_uuid(*id);
        let keys_and_values = [
            (
                InMemoryMetricKey::WsConnections(host.clone()),
                connections.get(&community_id).copied(),
            ),
            (
                InMemoryMetricKey::UsersOnline(host.clone()),
                users_online.get(&community_id).copied(),
            ),
            (
                InMemoryMetricKey::Subscriptions(host.clone()),
                subscriptions.get(&community_id).copied(),
            ),
        ];
        for (key, value) in keys_and_values {
            if let Some(value) = value {
                key.set(value as f64);
                current.insert(key);
            }
        }
    }

    for key in dropped_in_memory_keys(previously_emitted, &current) {
        key.set(0.0);
    }
    *previously_emitted = current;
}

fn dropped_in_memory_keys(
    previously_emitted: &HashSet<InMemoryMetricKey>,
    current: &HashSet<InMemoryMetricKey>,
) -> Vec<InMemoryMetricKey> {
    previously_emitted.difference(current).cloned().collect()
}

/// Run one usage-metrics tick. Every pod emits its own in-memory gauges, while
/// one leader owns the heavier database-derived snapshot.
async fn run_usage_metrics_tick(
    state: &AppState,
    emission_scope: &EmissionScope,
    leader: &mut Option<buzz_db::UsageMetricsLeader>,
    emitted_in_memory: &mut HashSet<InMemoryMetricKey>,
) -> anyhow::Result<()> {
    let host_map: HashMap<Uuid, String> = match state.db.usage_community_hosts().await {
        Ok(hosts) => hosts
            .into_iter()
            .map(|community| (community.id, community.host))
            .collect(),
        Err(error) => {
            if leader.is_some() {
                warn!("Usage metrics leader demoting: host map collection failed");
                *leader = None;
            }
            emit_in_memory_usage_metrics(state, emission_scope, None, emitted_in_memory);
            return Err(error.into());
        }
    };
    emit_in_memory_usage_metrics(state, emission_scope, Some(&host_map), emitted_in_memory);

    let mut demoted = false;
    if let Some(leader_guard) = leader.as_mut() {
        if !leader_guard.is_live().await {
            warn!("Usage metrics leader lock connection failed liveness check; demoting");
            *leader = None;
            demoted = true;
        }
    }
    if leader.is_none() && !demoted {
        *leader = state
            .db
            .try_lock_usage_metrics(USAGE_METRICS_LOCK_KEY)
            .await?;
        if leader.is_some() {
            info!("Acquired usage metrics leader lock");
        }
    }
    if leader.is_some() {
        if let Err(error) = emit_db_usage_metrics(state, emission_scope, &host_map).await {
            warn!("Usage metrics leader demoting: DB collection failed");
            *leader = None;
            return Err(error);
        }
        run_storage_sweep_tick(state, emission_scope, &host_map).await;
    }

    Ok(())
}

/// Storage-sweep half of the leader-only tick: harvest/spawn (never awaits
/// the sweep itself) then re-emit whatever snapshot is cached. Split out of
/// [`run_usage_metrics_tick`] because it has its own always-on config
/// (independent of `EmissionScope`) and a hard kill switch — a disabled
/// sweep must never touch a single storage-family gauge, including the
/// health ones, so a relay without `s3:ListBucket` can turn the whole
/// feature off cleanly.
async fn run_storage_sweep_tick(
    state: &AppState,
    emission_scope: &EmissionScope,
    host_map: &HashMap<Uuid, String>,
) {
    static SWEEP_CONFIG: std::sync::OnceLock<storage_sweep::StorageSweepConfig> =
        std::sync::OnceLock::new();
    // SWEEP_CONFIG is a function-local OnceLock by design: it is localized
    // feature config consumed only by this code path, read once on the first
    // leader tick, and stable for the process lifetime (env is immutable).
    // Keeping it here avoids widening Config/AppState for a single consumer.
    let config = *SWEEP_CONFIG.get_or_init(storage_sweep::StorageSweepConfig::from_env);
    if !config.enabled {
        return;
    }

    let media_storage = Arc::clone(&state.media_storage);
    let max_objects = config.max_objects;
    storage_sweep::maybe_spawn_sweep(
        &state.storage_sweep,
        config.interval,
        config.timeout,
        async move {
            buzz_media::fold_bucket_listing(max_objects, move |token| {
                let media_storage = Arc::clone(&media_storage);
                async move { media_storage.list_page(token, 1000).await }
            })
            .await
        },
    )
    .await;

    storage_sweep::emit_storage_metrics(&state.storage_sweep, host_map, |id| {
        emission_scope.allows(id)
    })
    .await;
}

/// Emit the database-derived usage snapshot from the stable leader only.
async fn emit_db_usage_metrics(
    state: &AppState,
    emission_scope: &EmissionScope,
    host_map: &HashMap<Uuid, String>,
) -> anyhow::Result<()> {
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

    // --- Determine which community IDs receive per-community series (K1) ---
    //
    // `active_set` is the subset of host_map IDs that get per-community gauges
    // this tick. Fleet-wide totals (buzz_total_*) always emit regardless.
    let active_set: HashSet<Uuid> = host_map
        .keys()
        .filter(|id| emission_scope.allows(id))
        .copied()
        .collect();

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
        for (&id, community) in host_map {
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
        for (&id, community) in host_map {
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
        for (&id, community) in host_map {
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
        for (&id, community) in host_map {
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
        for (&id, community) in host_map {
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
        for (&id, community) in host_map {
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
        for (&id, community) in host_map {
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
        for (&id, community) in host_map {
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

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::sync::Arc;
    use std::time::Duration;

    use tokio_util::sync::CancellationToken;
    use uuid::Uuid;

    use super::{
        buzz_auto_migrate_enabled, dropped_in_memory_keys, idle_timeout_secs,
        refresh_legacy_active_gauge_recency, run_periodic_until_cancelled, EmissionScope,
        InMemoryMetricKey,
    };
    use metrics::GaugeFn;
    use metrics_util::{
        debugging::DebugValue,
        registry::{GenerationalAtomicStorage, Registry},
    };

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

    #[test]
    fn test_emission_scope_off_disallows_every_community() {
        assert!(EmissionScope::All.allows(&Uuid::new_v4()));
        assert!(!EmissionScope::Off.allows(&Uuid::new_v4()));
    }

    #[test]
    fn test_dropped_in_memory_keys_preserves_resolved_host_label() {
        let previous = HashSet::from([
            InMemoryMetricKey::WsConnections("removed.example".to_owned()),
            InMemoryMetricKey::UsersOnline("live.example".to_owned()),
        ]);
        let current = HashSet::from([InMemoryMetricKey::UsersOnline("live.example".to_owned())]);

        assert_eq!(
            dropped_in_memory_keys(&previous, &current),
            vec![InMemoryMetricKey::WsConnections(
                "removed.example".to_owned()
            )]
        );
    }

    #[test]
    fn test_legacy_gauge_recency_refresh_preserves_lifecycle_deltas() {
        let recorder = metrics_util::debugging::DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();

        metrics::with_local_recorder(&recorder, || {
            let connections = metrics::gauge!("buzz_ws_connections_active");
            let subscriptions = metrics::gauge!("buzz_subscriptions_active");
            connections.increment(1.0);
            subscriptions.increment(1.0);

            refresh_legacy_active_gauge_recency();

            connections.decrement(1.0);
            subscriptions.increment(1.0);
        });

        let values = snapshotter
            .snapshot()
            .into_vec()
            .into_iter()
            .map(|(key, _, _, value)| {
                let DebugValue::Gauge(value) = value else {
                    panic!("{} must be a gauge", key.key().name());
                };
                (key.key().name().to_owned(), value.into_inner())
            })
            .collect::<std::collections::HashMap<_, _>>();

        assert_eq!(values.get("buzz_ws_connections_active"), Some(&0.0));
        assert_eq!(values.get("buzz_subscriptions_active"), Some(&2.0));
    }

    #[test]
    fn test_legacy_gauge_recency_refresh_advances_generation() {
        let registry = Registry::new(GenerationalAtomicStorage::atomic());
        let key = metrics::Key::from_name("legacy");
        let gauge = registry.get_or_create_gauge(&key, Clone::clone);
        gauge.increment(1.0);
        let generation_before = gauge.get_generation();
        gauge.increment(0.0);

        assert!(gauge.get_generation() > generation_before);
    }

    #[test]
    fn test_idle_timeout_is_at_least_three_usage_intervals() {
        assert_eq!(idle_timeout_secs(None, 300), 900);
        assert_eq!(idle_timeout_secs(Some(10), 1_000), 3_000);
    }
}
