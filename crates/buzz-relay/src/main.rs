use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use tracing::{error, info, warn};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

fn log_env_filter(rust_log: Option<&str>) -> EnvFilter {
    EnvFilter::new(rust_log.unwrap_or("buzz_relay=info"))
}
use uuid::Uuid;

use buzz_audit::AuditService;
use buzz_auth::AuthService;
use buzz_core::CommunityId;
use buzz_db::{Db, DbConfig};
use buzz_pubsub::PubSubManager;
use buzz_search::SearchService;

use buzz_relay::config::{Config, MAX_DRAIN_JITTER_MS};
use buzz_relay::lifecycle::{BootTracker, LifecycleReason, StartupPhase};
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

async fn connect_audit_pool(config: &DbConfig) -> anyhow::Result<sqlx::PgPool> {
    let audit_config = DbConfig {
        read_database_url: None,
        max_connections: 5,
        min_connections: 1,
        ..config.clone()
    };
    Db::connect_writer_pool(&audit_config)
        .await
        .map_err(Into::into)
}

fn relay_keypair_from_config(relay_private_key: Option<&str>) -> anyhow::Result<nostr::Keys> {
    let hex = relay_private_key.ok_or_else(|| {
        anyhow::anyhow!(
            "BUZZ_RELAY_PRIVATE_KEY must be set. Run `just bootstrap` for local \
             development or configure a stable 32-byte hex private key."
        )
    })?;
    nostr::Keys::parse(hex).map_err(|e| anyhow::anyhow!("invalid BUZZ_RELAY_PRIVATE_KEY: {e}"))
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

fn main() -> anyhow::Result<()> {
    let (runtime, boot) = BootTracker::start_before_runtime(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
    })
    .map_err(|error| anyhow::anyhow!("failed to build Tokio runtime: {error}"))?;
    runtime.block_on(run_relay_main(boot))
}

async fn run_relay_main(boot: BootTracker) -> anyhow::Result<()> {
    // Install the ring CryptoProvider for rustls. Required before any rustls
    // TLS connection (rediss:// to ElastiCache, wss://, S3 over TLS): both
    // aws-lc-rs and ring are compiled in transitively, so rustls can't
    // auto-select a provider and would panic at first use without this.
    let (mut boot, ()) = boot
        .run_required(
            StartupPhase::CryptoInit,
            || {
                rustls::crypto::ring::default_provider()
                    .install_default()
                    .map_err(|_provider| ())
            },
            |_error| LifecycleReason::ProviderConflict,
        )
        .map_err(|()| {
            anyhow::anyhow!(
                "failed to install rustls crypto provider: another provider is already installed"
            )
        })?;

    // JSON-only structured logs — simple, machine-parseable, CAKE-compatible.
    // If OTEL_EXPORTER_OTLP_ENDPOINT is set, also attach an OpenTelemetry tracing
    // layer that exports spans via OTLP gRPC alongside the JSON stdout logs.
    //
    // Build a single shared Resource (service.name=buzz-relay by default, overridable
    // via OTEL_SERVICE_NAME) for the trace provider so that Datadog can identify
    // spans under the correct service identity.
    let tracing_init = boot.start(StartupPhase::TracingInit);
    let resource = telemetry::service_resource();
    let tracer_init = telemetry::try_init_tracer(resource.clone());
    let otel_enabled = matches!(&tracer_init, telemetry::TracerInit::Enabled(_));
    let otel_layer = match &tracer_init {
        telemetry::TracerInit::Enabled(p) => {
            use opentelemetry::trace::TracerProvider as _;
            Some(tracing_opentelemetry::layer().with_tracer(p.tracer("buzz-relay")))
        }
        _ => None,
    };
    let trace_context_lookup = telemetry::TraceContextLookup::default();
    let trace_context_lookup_layer = otel_enabled.then(|| {
        trace_context_lookup
            .clone()
            .with_filter(tracing_subscriber::filter::LevelFilter::OFF)
    });

    tracing_subscriber::registry()
        .with(
            fmt::layer()
                .json()
                .event_format(trace_context_lookup.json_formatter(otel_enabled))
                .with_filter(log_env_filter(std::env::var("RUST_LOG").ok().as_deref())),
        )
        .with(otel_layer.map(|layer| {
            layer.with_filter(telemetry::otel_env_filter(
                std::env::var("BUZZ_OTEL_FILTER").ok().as_deref(),
            ))
        }))
        .with(trace_context_lookup_layer)
        .init();

    // Log any exporter-build failure now that the subscriber is installed.
    match &tracer_init {
        telemetry::TracerInit::Enabled(_) => tracing_init.succeed(),
        // Structured logging is installed regardless of whether optional OTLP
        // export is configured, so the phase itself completed successfully.
        telemetry::TracerInit::Disabled => tracing_init.succeed(),
        telemetry::TracerInit::ExporterBuildFailed(_) => {
            tracing_init.degrade(LifecycleReason::ExporterBuild);
            boot.mark_degraded(LifecycleReason::ExporterBuild);
            // Do not log the raw exporter error: OTLP endpoint URLs can carry
            // credentials. The bounded lifecycle reason is sufficient here.
            warn!("Failed to build OTLP trace exporter; distributed tracing disabled");
        }
    }

    info!("Starting buzz-relay");

    let (next_boot, config) = boot
        .run_required(StartupPhase::ConfigLoad, Config::from_env, |_error| {
            LifecycleReason::ConfigInvalid
        })
        .map_err(|error| {
            error!("Invalid configuration: {error}");
            anyhow::anyhow!("Configuration error: {error}")
        })?;
    boot = next_boot;

    let key_failure = if config.relay_private_key.is_some() {
        LifecycleReason::RequiredInvalid
    } else {
        LifecycleReason::Missing
    };
    let (next_boot, relay_keypair) = boot.run_required(
        StartupPhase::KeyLoad,
        || relay_keypair_from_config(config.relay_private_key.as_deref()),
        |_error| key_failure,
    )?;
    boot = next_boot;
    info!(
        bind_addr = %config.bind_addr,
        relay_url = %config.relay_url,
        health_port = config.health_port,
        metrics_port = config.metrics_port,
        max_frame_bytes = config.max_frame_bytes,
        audit_enabled = config.audit_enabled,
        push_enabled = config.push_enabled,
        "Config loaded"
    );

    let usage_interval_secs = usage_metrics_interval_secs();
    let usage_idle_timeout_secs = usage_metrics_idle_timeout_secs(usage_interval_secs);
    let (boot, ()) = boot.run_required(
        StartupPhase::MetricsBind,
        || relay_metrics::try_install(config.metrics_port, usage_idle_timeout_secs),
        |error| match error.failure() {
            relay_metrics::MetricsInstallFailure::Bind => LifecycleReason::Bind,
            relay_metrics::MetricsInstallFailure::RecorderConflict => {
                LifecycleReason::RecorderConflict
            }
            relay_metrics::MetricsInstallFailure::ExporterBuild => LifecycleReason::ExporterBuild,
        },
    )?;
    boot.finish();
    metrics::gauge!("buzz_audit_enabled").set(if config.audit_enabled { 1.0 } else { 0.0 });
    metrics::gauge!("buzz_push_enabled").set(if config.push_enabled { 1.0 } else { 0.0 });
    info!(
        port = config.metrics_port,
        idle_timeout_secs = usage_idle_timeout_secs,
        "Prometheus metrics exporter started"
    );

    let db_config = DbConfig {
        database_url: config.database_url.clone(),
        read_database_url: config.read_database_url.clone(),
        replica_read_max_age_ms: config.replica_read_max_age_ms,
        max_connections: config.db_pool_size,
        read_max_connections: config.db_read_pool_size,
        ..DbConfig::default()
    }
    .with_session_timeouts_from_env();
    let db = Db::new(&db_config).await.map_err(|e| {
        error!("Failed to connect to Postgres: {e}");
        anyhow::anyhow!("DB connection failed: {e}")
    })?;
    if db.has_read_pool() {
        info!("Postgres connected (writer + lazy read replica pool)");
        // Reader-down at boot must not crash or block the relay; this warn-only
        // ping is the sole boot-time visibility that the replica is unreachable
        // (the lazy pool with min_connections=0 dials nothing until first use).
        db.spawn_read_pool_boot_ping();
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

    db.validate_deletion_serving_catalog().await.map_err(|e| {
        error!("Community deletion serving-fence validation failed: {e}");
        anyhow::anyhow!("Community deletion serving fence is unsafe: {e}")
    })?;
    info!("Community deletion serving fences verified");

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
            match db.ensure_configured_community_for_bootstrap(&host).await {
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

    let (audit, audit_metrics_pool) = if config.audit_enabled {
        let audit_pool = connect_audit_pool(&db_config)
            .await
            .map_err(|e| anyhow::anyhow!("Audit DB connection failed: {e}"))?;
        info!("Audit service ready");
        let metrics_pool = audit_pool.clone();
        (Some(AuditService::new(audit_pool)), Some(metrics_pool))
    } else {
        info!("Audit logging disabled by BUZZ_AUDIT_ENABLED");
        (None, None)
    };

    let redis_pool = {
        let mut cfg = deadpool_redis::Config::from_url(&config.redis_url);
        cfg.pool = Some(deadpool_redis::PoolConfig::new(config.redis_pool_size));
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

    // Spawn Redis pub/sub subscriber for NIP-FI cross-pod disconnect commands.
    // Remote pods publish to this global channel after accepting a disconnect
    // command; every pod merges the deny entry and closes matching sessions.
    let pubsub_for_nip_fi = Arc::clone(&pubsub);
    tokio::spawn(async move { pubsub_for_nip_fi.run_nip_fi_disconnect_subscriber().await });

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
    let search_metrics_pool = search_pool.clone();
    let search = SearchService::new(search_pool);
    info!(
        replica = config.read_database_url.is_some(),
        "Search service ready (Postgres FTS)"
    );

    let workflow_config = buzz_workflow::WorkflowConfig::default();
    let workflow_engine = Arc::new(WorkflowEngine::new(db.clone(), workflow_config));

    config
        .media
        .validate()
        .map_err(|e| anyhow::anyhow!("invalid media config: {e}"))?;
    let media_storage = buzz_media::MediaStorage::new(&config.media)
        .map_err(|e| anyhow::anyhow!("failed to initialize media storage: {e}"))?;
    info!("Media storage connected");

    let (mut app_state, audit_shutdown) = AppState::new(
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
    // NIP-FI S4: construct deny map + command verifier from startup config,
    // before Arc::new so we can mutate app_state directly.
    // Delegates to install_nip_fi_command_components which owns JWKS warmup,
    // the refresh loop, build_nip_fi_command_components, and both state assignments.
    {
        let nip_fi = &config.nip_fi;
        if let Some(key_source) = app_state.nip_fi_jwks_source.clone() {
            // Share the exact Arc that nip_fi_verifier already holds so warmup
            // and the background refresh loop populate the same snapshot cache
            // that assertion verification reads synchronously via key_set().
            buzz_relay::api::nip_fi::install_nip_fi_command_components(
                &mut app_state,
                nip_fi.mode,
                &nip_fi.registry,
                key_source,
                &nip_fi.jwks_configs,
                &nip_fi.command_configs,
            )
            .await
            .map_err(|e| anyhow::anyhow!("NIP-FI startup failed: {e}"))?;
        } else if nip_fi.is_enforce() {
            return Err(anyhow::anyhow!(
                "NIP-FI: failed to construct JWKS key source \
                 (empty or duplicate issuer config)"
            ));
        }
    }
    let state = Arc::new(app_state);

    // NIP-FI JWKS warm + background refresh.
    //
    // Per [FI-TRACE-DEPENDENCY-FAIL-CLOSED]: a JWKS warm failure at startup
    // MUST NOT abort the relay. The relay starts and HTTP-protected routes deny
    // with `authorization_unavailable` (503) until a snapshot lands. The
    // background loop retries automatically.
    //
    // The background task is cancelled cleanly on shutdown via a
    // CancellationToken so it does not outlive the process.
    let nip_fi_jwks_cancel = tokio_util::sync::CancellationToken::new();
    if let Some(ref jwks_source) = state.nip_fi_jwks_source.clone() {
        let jwks_configs = state.config.nip_fi.jwks_configs.clone();
        info!(
            issuer_count = jwks_configs.len(),
            "NIP-FI: warming JWKS snapshots for HTTP enforcement"
        );
        let issuer_ids: Vec<String> = jwks_configs.iter().map(|c| c.issuer.clone()).collect();
        warm_nip_fi_jwks_snapshots(jwks_source, &issuer_ids).await;
        // Background refresh loop: independent per-issuer cadence.
        let refresh_source = Arc::clone(jwks_source);
        let refresh_configs = jwks_configs.clone();
        let refresh_cancel = nip_fi_jwks_cancel.clone();
        tokio::spawn(async move {
            nip_fi_jwks_refresh_loop(
                refresh_configs
                    .iter()
                    .map(|c| (c.issuer.clone(), c.contract.refresh_interval_seconds()))
                    .collect(),
                move |issuer| {
                    let src = Arc::clone(&refresh_source);
                    let iss = issuer.to_owned();
                    Box::pin(async move { src.get_snapshot(&iss).await.is_some() })
                },
                refresh_cancel,
            )
            .await;
        });
    }

    // Inter-relay mesh (BUZZ_MESH seam). `boot_mesh` returns None when the
    // kill switch is off — nothing is bound, published, or spawned, so the
    // relay behaves byte-identically to a build without the mesh. When
    // enabled, a misconfigured mesh is fatal here (bind/Redis failure): an
    // operator who asked for the mesh gets it or gets told why not.
    if let Some(handle) = buzz_relay::mesh_boot::boot_mesh(
        &state.config,
        state.redis_pool.clone(),
        state.db.clone(),
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

    match state.db.verify_channel_roster_fence().await {
        Ok(()) => {
            info!("Channel roster fence verified");
        }
        Err(error) => {
            error!(%error, "Channel roster fence validation failed");
            return Err(anyhow::anyhow!(
                "Channel roster fence is unsafe; apply or repair migration 0032 before starting this relay: {error}"
            ));
        }
    }

    // Repair legacy NIP-29 channel rosters that were persisted while the
    // canonical member query still truncated at 1,000 rows. Validation above
    // makes migration 0032 a code/schema compatibility gate before the new
    // replacement protocol or listener can serve traffic.
    match buzz_relay::handlers::side_effects::reconcile_large_channel_member_snapshots(&state).await
    {
        Ok(count) if count > 0 => info!(count, "large channel member snapshots repaired"),
        Ok(_) => {}
        Err(error) => {
            tracing::warn!(%error, "large channel member snapshot startup reconciliation failed")
        }
    }

    // NIP-43: reconcile the event-backed roster for every provisioned
    // community before opening the listener. `relay_members` is canonical;
    // this repairs pre-snapshot communities and any publication that failed
    // after a membership transaction committed.
    if config.require_relay_membership {
        match buzz_relay::handlers::side_effects::reconcile_nip43_membership_snapshots_with_purpose(
            &state,
            buzz_relay::handlers::side_effects::Nip43ReconciliationPurpose::Bootstrap,
        )
        .await
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
                match buzz_relay::handlers::side_effects::reconcile_nip43_membership_snapshots_with_purpose(
                    &reconcile_state,
                    buzz_relay::handlers::side_effects::Nip43ReconciliationPurpose::Maintenance,
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
                        chrono::Utc::now(),
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

    // NIP-PL matcher and worker are enabled as one unit behind the explicit
    // deployment opt-in. The gateway URL alone never enables push.
    if state.config.push_enabled {
        tokio::spawn(buzz_relay::push_runtime::run_matcher(Arc::clone(&state)));
        tokio::spawn(buzz_relay::push_runtime::run_delivery_worker(Arc::clone(
            &state,
        )));
        info!("NIP-PL push matcher and delivery worker started");
    } else {
        info!("NIP-PL push disabled by BUZZ_PUSH_ENABLED");
    }

    // Admin outbox delivery worker — drives `relay_admin_outbox` rows.
    // Uses DB-level leases (held_by / lease_expires_at) so multiple pods can
    // run the worker concurrently without double-delivery.
    {
        let outbox_state = Arc::clone(&state);
        tokio::spawn(
            async move { buzz_relay::handlers::admin_outbox_worker::run(outbox_state).await },
        );
        info!("Admin outbox delivery worker started");
    }

    // Action recovery worker: re-drives stranded relay_admin_actions rows whose
    // action lease expired before the enforcement state machine completed.
    // Crash safety: a process that died between claim and finalization leaves
    // an action in pending/enforcing; this worker resumes from the persisted
    // step_marker state without re-running the mutation.
    {
        let action_state = Arc::clone(&state);
        tokio::spawn(
            async move { buzz_relay::handlers::admin_action_worker::run(action_state).await },
        );
        info!("Admin action recovery worker started");
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

    // Cross-pod NIP-FI disconnect consumer: receive deny entries from remote
    // pods, merge them into the local deny map (same max(until) rule), and
    // close any matching sessions.  Every pod subscribes; the publishing pod
    // also receives its own message and applies it — this is idempotent because
    // the deny entry was already inserted locally before the publish.
    //
    // The consumer delegates to `apply_nip_fi_disconnect` which owns all
    // validation, merge, and session-close logic.  This keeps the loop body
    // minimal and makes the exact production path testable end-to-end.
    {
        let state_for_nip_fi = Arc::clone(&state);
        let mut rx = state_for_nip_fi.pubsub.subscribe_nip_fi_disconnect();
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(msg) => {
                        let now = chrono::Utc::now();
                        buzz_relay::api::nip_fi::apply_nip_fi_disconnect(
                            &state_for_nip_fi,
                            &msg,
                            now,
                        );
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        metrics::counter!("buzz_nip_fi_disconnect_lag_total").increment(n);
                        tracing::warn!("NIP-FI disconnect consumer lagged by {n} messages");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        tracing::error!("NIP-FI disconnect broadcast channel closed");
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
                let read_stats = pool_state.db.read_pool_stats();
                relay_metrics::record_db_pool_metrics(relay_metrics::DbPoolMetricsInput {
                    writer: db_stats,
                    reader: read_stats,
                    audit: audit_metrics_pool
                        .as_ref()
                        .map(buzz_db::DbPoolStats::from_pool),
                    search: buzz_db::DbPoolStats::from_pool(&search_metrics_pool),
                });
                pool_state.db.refresh_pool_waiter_metrics();

                if read_stats.is_some() {
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
                    // Probe liveness, ungated by staleness: how long since
                    // the probe last committed a heartbeat token.
                    if let Some(age) = pool_state.db.fence().heartbeat_age() {
                        metrics::gauge!("buzz_db_replica_heartbeat_age_seconds")
                            .set(age.as_secs_f64());
                    }
                }

                let rs = pool_state.redis_pool.status();
                metrics::gauge!("buzz_redis_pool_available").set(rs.available as f64);
                metrics::gauge!("buzz_redis_pool_size").set(rs.size as f64);
                metrics::gauge!("buzz_redis_pool_max").set(rs.max_size as f64);
                metrics::gauge!("buzz_redis_pool_waiting").set(rs.waiting as f64);

                let deletion_store = pool_state.db.deletion_store();
                match deletion_store.reap_expired_serving_write_leases(1000).await {
                    Ok(reaped) => metrics::counter!("buzz_deletion_serving_leases_reaped_total")
                        .increment(reaped),
                    Err(error) => tracing::warn!(%error, "serving-lease reaper failed"),
                }
                match deletion_store.serving_lease_stats().await {
                    Ok(stats) => {
                        metrics::gauge!("buzz_deletion_serving_leases_active")
                            .set(stats.active as f64);
                        metrics::gauge!("buzz_deletion_serving_leases_expired")
                            .set(stats.expired as f64);
                        metrics::gauge!("buzz_deletion_serving_leases_dead_tuples")
                            .set(stats.dead_tuples as f64);
                    }
                    Err(error) => tracing::warn!(%error, "serving-lease metrics failed"),
                }
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

#[cfg(test)]
mod env_filter_tests {
    use super::log_env_filter;
    use buzz_relay::telemetry::otel_env_filter;
    use tracing_subscriber::prelude::*;

    #[test]
    fn unset_enables_datastore_only_for_otel_filter() {
        let logs = tracing_subscriber::registry().with(log_env_filter(None));
        tracing::subscriber::with_default(logs, || {
            assert!(!tracing::enabled!(target: "buzz_datastore", tracing::Level::INFO));
            assert!(tracing::enabled!(target: "buzz_relay", tracing::Level::INFO));
        });

        let otel = tracing_subscriber::registry().with(otel_env_filter(None));
        tracing::subscriber::with_default(otel, || {
            assert!(tracing::enabled!(target: "buzz_datastore", tracing::Level::INFO));
        });
    }

    #[test]
    fn explicit_datastore_off_is_preserved_alone() {
        assert_eq!(
            otel_env_filter(Some("buzz_datastore=off")).to_string(),
            "buzz_datastore=off"
        );
    }

    #[test]
    fn explicit_datastore_debug_is_preserved_alone() {
        assert_eq!(
            otel_env_filter(Some("buzz_datastore=debug")).to_string(),
            "buzz_datastore=debug"
        );
    }

    #[test]
    fn log_and_otel_filters_are_configured_independently() {
        assert_eq!(log_env_filter(Some("warn")).to_string(), "warn");
        assert_eq!(
            otel_env_filter(Some("buzz_relay=debug")).to_string(),
            "buzz_relay=debug"
        );
    }
}

/// Warm NIP-FI JWKS snapshots for all configured issuers at startup.
///
/// Calls `source.get_snapshot(issuer)` once per configured issuer and logs
/// the outcome.  On success, the snapshot is cached and the relay is ready to
/// validate federated assertions.  On failure, the relay starts and HTTP-protected
/// routes deny with 503 until the background loop delivers a snapshot.
///
/// Raw `iss` values are never logged (NIP-FI.md:777-779); only the
/// `issuer_index` diagnostic code appears in log output.
///
/// Extracted from `run_relay_main` for unit-testability.
/// [FI-TRACE-DEPENDENCY-FAIL-CLOSED]
async fn warm_nip_fi_jwks_snapshots<F: buzz_auth::JwksFetcher>(
    source: &buzz_auth::ProductionJwksSource<F>,
    issuer_ids: &[String],
) {
    for (idx, issuer) in issuer_ids.iter().enumerate() {
        match source.get_snapshot(issuer).await {
            Some(_) => {
                // issuer_index is a non-identifying diagnostic code.
                // Raw `iss` is excluded from logs per NIP-FI.md:777-779.
                info!(issuer_index = idx, "NIP-FI: JWKS snapshot warmed");
            }
            None => {
                warn!(
                    issuer_index = idx,
                    "NIP-FI: JWKS warm failed — HTTP ingress will deny 503 until \
                     a snapshot lands; background refresh will retry"
                );
            }
        }
    }
}

/// Background JWKS refresh loop for NIP-FI issuers.
///
/// Sleeps until the nearest due issuer, runs the fetch for each overdue issuer,
/// then records the post-fetch instant as the new baseline.  Scheduling from
/// the post-fetch instant keeps the interval at least `interval_secs` even under
/// nonzero network latency (pre-fetch scheduling would drift the interval
/// backward by the fetch latency on every cycle).
///
/// `fetch` returns `true` if the snapshot was successfully refreshed, `false`
/// on fetch failure (the loop continues either way; hard-deadline enforcement
/// lives in the JWKS source itself).
///
/// Extracted from `run_relay_main` for unit-testability.  [FI-TRACE-DEPENDENCY-FAIL-CLOSED]
async fn nip_fi_jwks_refresh_loop<F, Fut>(
    // `(issuer_id, interval_seconds)` pairs, one per configured issuer.
    issuers: Vec<(String, u64)>,
    // Async fetch callback: `issuer → true (success) / false (failure)`.
    mut fetch: F,
    cancel: CancellationToken,
) where
    F: FnMut(&str) -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let mut intervals: Vec<(String, u64, tokio::time::Instant)> = issuers
        .into_iter()
        .map(|(issuer, interval)| (issuer, interval, tokio::time::Instant::now()))
        .collect();

    loop {
        // Sleep until the next scheduled refresh across all issuers.
        let next = intervals
            .iter()
            .map(|(_, interval, last)| *last + std::time::Duration::from_secs(*interval))
            .min()
            .unwrap_or_else(|| tokio::time::Instant::now() + std::time::Duration::from_secs(300));
        tokio::select! {
            _ = tokio::time::sleep_until(next) => {}
            _ = cancel.cancelled() => break,
        }
        let now = tokio::time::Instant::now();
        for (idx, (issuer, interval, last)) in intervals.iter_mut().enumerate() {
            if now >= *last + std::time::Duration::from_secs(*interval) {
                if !fetch(issuer).await {
                    // issuer_index is a non-identifying diagnostic code.
                    // Raw `iss` is excluded from logs per NIP-FI.md:777-779.
                    warn!(
                        issuer_index = idx,
                        "NIP-FI: background JWKS refresh returned no snapshot"
                    );
                }
                // Schedule the NEXT refresh from when this fetch completed,
                // not from the instant captured before the await.  Scheduling
                // from the pre-fetch snapshot drifts the interval backward by
                // the fetch latency on every cycle; scheduling from post-fetch
                // keeps the interval at least `interval_secs` even under
                // nonzero network latency.  The hard-deadline contract
                // (jwks_hard_deadline_seconds) is enforced by the JWKS source
                // itself, not by this timer.
                *last = tokio::time::Instant::now();
            }
        }
    }
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
///
/// ## Shutdown budget
///
/// The full teardown, measured from SIGTERM, is bounded as follows:
///
/// 1. `5s` grace. Readiness returns 503 immediately, then the process
///    sleeps 5 seconds so Kubernetes stops routing new traffic before any
///    listener closes.
/// 2. `GRACEFUL_DRAIN_TIMEOUT` (`30s`) hard drain. Started at the end of the
///    grace, this backstops the whole drain and force-exits the process if
///    exceeded. It bounds everything after the grace, not the grace itself.
///
/// A single WebSocket can therefore stay open, from SIGTERM, for up to:
///
/// ```text
///   5s grace  +  up to 20s jitter  +  up to 5s close-frame ack  =  30s
///   (fixed)      (MAX_DRAIN_JITTER_MS)  (RESTART_CLOSE_ACK_TIMEOUT)
/// ```
///
/// The 5s grace runs before the 30s hard-drain clock starts, so the jitter
/// (capped at [`buzz_relay::config::MAX_DRAIN_JITTER_MS`] = 20s) plus the
/// per-connection close-frame ack wait (`RESTART_CLOSE_ACK_TIMEOUT` = 5s in
/// `state.rs`) sum to 25s and stay inside the 30s hard drain. Total worst
/// case from SIGTERM to forced exit is 5s + 30s = 35s. Both fit inside the
/// chart's `terminationGracePeriodSeconds: 60` (`deploy/charts/buzz/values.yaml`),
/// which leaves headroom but assumes no `preStop` hook adds further delay.
/// With jitter off (`BUZZ_DRAIN_JITTER_MS=0`, the default) sockets close
/// all-at-once right after the grace, so the per-socket delay collapses to
/// roughly the 5s grace plus the ack wait.
const GRACEFUL_DRAIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

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
    let shutdown_state = Arc::clone(&state);
    let drain_conn_manager = Arc::clone(&state.conn_manager);
    let drain_jitter_ms = state.config.drain_jitter_ms;
    let tx = shutdown_tx.clone();
    // TODO(coverage): `serve`'s shutdown wiring has no automated test. The
    // jittered drain helper (`ConnectionManager::drain_all_jittered`) is
    // covered in `state.rs`, but coverage of the helper is not coverage of
    // its use here: the three wiring facts below are currently unguarded, and
    // mutating any one of them leaves the suite green.
    //   1. Jitter dispatch: `drain_jitter_ms == 0` must pick `drain_all`, and
    //      a non-zero value must pick `drain_all_jittered(drain_jitter_ms)`.
    //      A mutant that inverts this condition ships jitter-off in prod.
    //   2. The shutdown handle must be awaited before the abort. Dropping the
    //      `shutdown_handle.await` (both the UDS and TCP-only return paths) is
    //      the exact shape of the previously shipped detached-timer bug,
    //      relocated from the helper to the call site: the runtime can exit
    //      before delayed closes flush, so no client sees a 1012.
    //   3. `shutdown_tx.send(true)` must reach every listener's
    //      `with_graceful_shutdown` future, on both the UDS and TCP-only paths.
    //
    // A focused test would refactor the drain/dispatch decision and the
    // listener-shutdown fan-out into a small seam that does not need a bound
    // socket or a real SIGTERM. One shape: extract the body of this spawned
    // task into a `run_graceful_shutdown(state, shutdown_tx)` fn parameterised
    // over a signal future and a clock, inject a fake `ConnectionManager`
    // (or a trait over `drain_all` / `drain_all_jittered`) that records which
    // path ran, drive it with `tokio::time` paused, and assert: (a) the right
    // drain path ran for jitter 0 vs non-zero, (b) the drain future completed
    // before the abort fired, and (c) each subscribed `watch` receiver
    // observed `true`. This keeps the test off real ports and off wall-clock
    // sleeps. Not implemented here. This comment records the plan only.
    let shutdown_handle = tokio::spawn(async move {
        shutdown_signal().await;
        shutdown_state.begin_shutdown();
        info!("Shutdown signal received — readiness now returns 503");
        // 5s grace: let K8s stop routing new traffic before we close listeners.
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        info!("Starting graceful drain (30s timeout)");
        let _ = tx.send(true);
        // Keep the original process-level backstop alive while listener and
        // upgraded-socket shutdown proceeds. The caller aborts it only after
        // Axum and the owned jitter drain have both completed.
        let hard_shutdown = tokio::spawn(async {
            tokio::time::sleep(GRACEFUL_DRAIN_TIMEOUT).await;
            tracing::error!("Drain timeout exceeded — forcing exit");
            std::process::exit(1);
        });
        let hard_shutdown_abort = hard_shutdown.abort_handle();
        // Stop accepting first, then close every live socket. Jitter off (the
        // default) uses the original synchronous all-at-once drain; jitter on
        // retains ownership of every delayed close until its 1012 frame has
        // been flushed and acknowledged (or its send loop cancelled).
        let closed = if drain_jitter_ms == 0 {
            drain_conn_manager.drain_all()
        } else {
            drain_conn_manager.drain_all_jittered(drain_jitter_ms).await
        };
        info!(
            connections = closed,
            jitter_ms = drain_jitter_ms,
            max_jitter_ms = MAX_DRAIN_JITTER_MS,
            "Signalled restart close to all live WebSocket connections"
        );
        hard_shutdown_abort
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

        let hard_shutdown = shutdown_handle
            .await
            .map_err(|e| anyhow::anyhow!("Shutdown task failed: {e}"))?;
        uds_handle.abort();
        hard_shutdown.abort();
        // The JWKS refresh loop exits via the shutting_down flag set by AppState::shutdown().
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

    let hard_shutdown = shutdown_handle
        .await
        .map_err(|e| anyhow::anyhow!("Shutdown task failed: {e}"))?;
    hard_shutdown.abort();
    // The JWKS refresh loop exits via the shutting_down flag set by AppState::shutdown().
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
    metrics::gauge!("buzz_ws_authenticated_connections_active").increment(0.0);
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
        let invite_retention_cutoff = chrono::Utc::now() - chrono::Duration::days(30);
        match state
            .db
            .reap_expired_relay_invites(invite_retention_cutoff)
            .await
        {
            Ok(deleted) if deleted > 0 => {
                info!(deleted, "reaped expired relay invites");
            }
            Ok(_) => {}
            Err(error) => {
                warn!(error = %error, "failed to reap expired relay invites");
            }
        }
        run_storage_sweep_tick(state, emission_scope, &host_map).await;
    }

    Ok(())
}

/// Read worker storage snapshots only on the existing leader metrics tick.
async fn run_storage_sweep_tick(
    state: &AppState,
    emission_scope: &EmissionScope,
    host_map: &HashMap<Uuid, String>,
) {
    static MODE: std::sync::OnceLock<storage_sweep::StorageMetricsMode> =
        std::sync::OnceLock::new();
    let mode = *MODE.get_or_init(storage_sweep::StorageMetricsMode::from_env);
    if let Err(error) = storage_sweep::run_storage_metrics_tick(
        &state.db,
        &state.storage_sweep,
        mode,
        host_map,
        |id| emission_scope.allows(id),
    )
    .await
    {
        warn!(error = %error, "failed to load stored storage snapshot; retrying next usage tick");
    }
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
        buzz_auto_migrate_enabled, connect_audit_pool, dropped_in_memory_keys, idle_timeout_secs,
        nip_fi_jwks_refresh_loop, refresh_legacy_active_gauge_recency, relay_keypair_from_config,
        run_periodic_until_cancelled, EmissionScope, InMemoryMetricKey,
    };
    use buzz_db::DbConfig;
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

    async fn audit_writer_pool_installs_timeouts_and_bounds_advisory_lock_waits() {
        let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
        let pool = connect_audit_pool(&DbConfig {
            database_url,
            max_connections: 2,
            min_connections: 0,
            lock_timeout_ms: 500,
            idle_txn_timeout_ms: 60_000,
            statement_timeout_ms: 0,
            ..DbConfig::default()
        })
        .await
        .expect("connect audit writer pool");

        let (lock, idle, statement): (String, String, String) = sqlx::query_as(
            "SELECT current_setting('lock_timeout'), \
                    current_setting('idle_in_transaction_session_timeout'), \
                    current_setting('statement_timeout')",
        )
        .fetch_one(&pool)
        .await
        .expect("read effective audit writer GUCs");
        assert_eq!(lock, "500ms");
        assert_eq!(idle, "1min");
        assert_eq!(statement, "0");

        let lock_key = i64::from_be_bytes(
            Uuid::new_v4().as_bytes()[..8]
                .try_into()
                .expect("eight UUID bytes"),
        );
        let mut holder = pool.acquire().await.expect("audit lock holder");
        sqlx::query("SELECT pg_advisory_lock($1)")
            .bind(lock_key)
            .execute(&mut *holder)
            .await
            .expect("hold audit advisory lock");

        let started = std::time::Instant::now();
        let mut waiter = pool.acquire().await.expect("audit lock waiter");
        let error = sqlx::query("SELECT pg_advisory_lock($1)")
            .bind(lock_key)
            .execute(&mut *waiter)
            .await
            .expect_err("audit advisory-lock waiter must time out");
        let code = match &error {
            sqlx::Error::Database(db_error) => db_error.code().map(|code| code.to_string()),
            other => panic!("expected database error, got {other:?}"),
        };
        assert_eq!(code.as_deref(), Some("55P03"));
        assert!(started.elapsed() < Duration::from_secs(5));

        sqlx::query("SELECT pg_advisory_unlock($1)")
            .bind(lock_key)
            .execute(&mut *holder)
            .await
            .expect("release audit advisory lock");
    }

    mod postgres_tests {
        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn audit_writer_pool_installs_timeouts_and_bounds_advisory_lock_waits() {
            super::audit_writer_pool_installs_timeouts_and_bounds_advisory_lock_waits().await;
        }
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
    fn configured_relay_identity_is_preserved() {
        let configured = nostr::Keys::generate();
        let secret = configured.secret_key().to_secret_hex();

        let selected = relay_keypair_from_config(Some(&secret)).expect("configured key");

        assert_eq!(selected.public_key(), configured.public_key());
    }

    #[test]
    fn missing_relay_identity_is_rejected() {
        let result = relay_keypair_from_config(None);

        assert!(result.is_err());
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
            let authenticated = metrics::gauge!("buzz_ws_authenticated_connections_active");
            let subscriptions = metrics::gauge!("buzz_subscriptions_active");
            connections.increment(1.0);
            authenticated.increment(1.0);
            subscriptions.increment(1.0);

            refresh_legacy_active_gauge_recency();

            connections.decrement(1.0);
            authenticated.decrement(1.0);
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
        assert_eq!(
            values.get("buzz_ws_authenticated_connections_active"),
            Some(&0.0)
        );
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

    // ── F4: JWKS refresh-interval anchoring ───────────────────────────────────
    //
    // The fix: `*last = tokio::time::Instant::now()` is called AFTER the fetch
    // awaits, not before (where `now` was captured pre-fetch).  Under nonzero
    // fetch latency, scheduling from pre-fetch would drift the interval backward
    // on every cycle.
    //
    // Test matrix:
    //   A. Nonzero fetch latency: a 10s fetch inside a 60s interval → the next
    //      refresh is scheduled 60s after the fetch completes (70s from start),
    //      not 60s after the pre-fetch `now` (which would be ≈60s from start).
    //   B. Fetch failure still advances `last`, preventing a tight-loop.
    //      The loop continues; the third cycle fires at the correct deadline.
    //   C. Hard deadline with early-cache: when interval=60 and hard_deadline=90s,
    //      the loop fires at T=60; at T=70 (post-fetch) last=70, next due at
    //      T=130. The `hard_deadline` is enforced by `ProductionJwksSource`, NOT
    //      by the timer loop — the loop only tracks refresh cadence.
    //   D. The production adapter's `.is_some()` contract: "a live snapshot
    //      exists" (not "the last fetch succeeded"). After a failed refresh the
    //      previously-cached snapshot may still be live; `src.get_snapshot().is_some()`
    //      returns true in that case.  The timer loop treats the boolean as an
    //      opaque "notify" / "warn" signal, not as a freshness oracle.
    //
    // Falsifying mutation: change `*last = tokio::time::Instant::now()` to
    // `*last = now` (where `now` is the pre-await snapshot).  Test A fails
    // because the second refresh fires at T≈60s rather than T≈70s.

    /// Nonzero fetch latency: second refresh must be anchored to post-fetch instant.
    #[tokio::test(start_paused = true)]
    async fn jwks_refresh_interval_anchored_to_post_fetch_instant() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let fetch_count = Arc::new(AtomicUsize::new(0));
        let second_fetch_instant: Arc<std::sync::Mutex<Option<tokio::time::Instant>>> =
            Arc::new(std::sync::Mutex::new(None));

        let count_clone = Arc::clone(&fetch_count);
        let instant_clone = Arc::clone(&second_fetch_instant);
        let cancel = CancellationToken::new();
        let cancel_task = cancel.clone();

        // 10s simulated fetch latency, 60s interval.
        let fetch_latency = Duration::from_secs(10);
        let interval_secs = 60u64;

        let task = tokio::spawn(async move {
            nip_fi_jwks_refresh_loop(
                vec![("issuer-a".to_string(), interval_secs)],
                move |_issuer| {
                    let n = count_clone.fetch_add(1, Ordering::SeqCst);
                    let instant_ref = Arc::clone(&instant_clone);
                    let latency = fetch_latency;
                    Box::pin(async move {
                        // Simulate nonzero fetch latency.
                        tokio::time::sleep(latency).await;
                        if n == 1 {
                            // Record when the second fetch completes.
                            *instant_ref.lock().unwrap() = Some(tokio::time::Instant::now());
                        }
                        true // success
                    })
                },
                cancel_task,
            )
            .await;
        });

        // Time T=0: loop starts with last=now.
        tokio::task::yield_now().await;

        // Advance to T=60s: first refresh becomes due.
        tokio::time::advance(Duration::from_secs(60)).await;
        tokio::task::yield_now().await;

        // Advance through the 10s fetch latency to T=70s.
        tokio::time::advance(Duration::from_secs(10)).await;
        tokio::task::yield_now().await;

        // At T=70 the first fetch completes; last is now ~70s.
        // A second refresh is due 60s later, at T=130.  Verify it does NOT fire at T=120.
        tokio::time::advance(Duration::from_secs(59)).await; // T=129
        tokio::task::yield_now().await;
        assert_eq!(
            fetch_count.load(Ordering::SeqCst),
            1,
            "second refresh must NOT fire before post-fetch last + interval_secs; \
             at T=129 only one fetch should have completed. \
             Falsifying mutation: use pre-fetch `now` for `last` update → second fetch fires at T≈120"
        );

        // Advance to T=131: second refresh is now overdue (post-fetch last + 60 ≤ 131).
        tokio::time::advance(Duration::from_secs(2)).await; // T=131
        tokio::task::yield_now().await;
        // Sleep through the 10s fetch latency.
        tokio::time::advance(Duration::from_secs(10)).await;
        tokio::task::yield_now().await;

        assert_eq!(
            fetch_count.load(Ordering::SeqCst),
            2,
            "second refresh must have fired by T=141 (post-fetch last ~70 + 60 + 10 fetch latency)"
        );

        cancel.cancel();
        task.await.expect("refresh loop task");
    }

    /// Fetch failure still advances `last`: no tight-loop and the third cycle fires
    /// at the correct deadline.
    #[tokio::test(start_paused = true)]
    async fn jwks_refresh_interval_advances_last_on_failure() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let fetch_count = Arc::new(AtomicUsize::new(0));
        let count_for_fetch = Arc::clone(&fetch_count);
        let cancel = CancellationToken::new();
        let cancel_task = cancel.clone();

        let task = tokio::spawn(async move {
            nip_fi_jwks_refresh_loop(
                vec![("issuer-b".to_string(), 60)],
                move |_| {
                    let c = Arc::clone(&count_for_fetch);
                    Box::pin(async move {
                        c.fetch_add(1, Ordering::SeqCst);
                        false // always fails
                    })
                },
                cancel_task,
            )
            .await;
        });

        tokio::task::yield_now().await;

        // First fire at T=60.
        tokio::time::advance(Duration::from_secs(60)).await;
        tokio::task::yield_now().await;
        assert_eq!(
            fetch_count.load(Ordering::SeqCst),
            1,
            "first refresh at T=60"
        );

        // Second fire at T=120: failure advances `last` so no tight-loop.
        tokio::time::advance(Duration::from_secs(60)).await;
        tokio::task::yield_now().await;
        assert_eq!(
            fetch_count.load(Ordering::SeqCst),
            2,
            "second refresh at T=120; failure must still advance last. \
             Falsifying mutation: omit `*last = Instant::now()` on failure → tight-loop"
        );

        cancel.cancel();
        task.await.expect("refresh loop task");
    }

    // ── Test C: hard-deadline is enforced by ProductionJwksSource, not the timer ──
    //
    // The timer loop is cadence-only: it fires at `last + interval_secs`.
    // The `hard_deadline` in `ProductionJwksSource` is a separate contract that
    // the timer loop does not enforce directly.  This test proves the timer loop
    // fires at T=60 (interval) and then again at approximately T=130
    // (post-fetch last ~70 + interval 60), with no spurious fires in between.
    //
    // A "cache-returns-early" fetch is simulated by the fetch returning `true`
    // (a live snapshot is available).  This is the `.is_some()` contract: the
    // production adapter returns `true` when a snapshot exists, which may be a
    // cached snapshot even after a transient failure — NOT "fetch succeeded".
    //
    // Falsifying mutation (timer): advancing the interval check to use
    // `last + hard_deadline_secs` instead of `last + interval_secs` would
    // cause the first fire to happen at T=90 instead of T=60; the T=60 assert
    // would fire.  This confirms the timer does not conflate hard_deadline with
    // interval.
    #[tokio::test(start_paused = true)]
    async fn jwks_refresh_interval_is_cadence_only_not_hard_deadline() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let fetch_count = Arc::new(AtomicUsize::new(0));
        let count_clone = Arc::clone(&fetch_count);
        let cancel = CancellationToken::new();
        let cancel_task = cancel.clone();

        // interval=60s, simulating a hard_deadline of 90s at the source level.
        // The timer loop receives only (issuer, interval=60) — it knows nothing
        // about hard_deadline.  The hard_deadline enforcement belongs to
        // ProductionJwksSource, not here.
        let task = tokio::spawn(async move {
            nip_fi_jwks_refresh_loop(
                vec![("issuer-c".to_string(), 60)],
                move |_| {
                    let c = Arc::clone(&count_clone);
                    Box::pin(async move {
                        c.fetch_add(1, Ordering::SeqCst);
                        // Returns true = "a live snapshot exists" (cache-hit).
                        // This is the production .is_some() contract.
                        true
                    })
                },
                cancel_task,
            )
            .await;
        });

        tokio::task::yield_now().await;

        // First fire at T=60 (interval).
        tokio::time::advance(Duration::from_secs(60)).await;
        tokio::task::yield_now().await;
        assert_eq!(
            fetch_count.load(Ordering::SeqCst),
            1,
            "first refresh fires at T=60 (interval boundary). \
             Falsifying mutation: if the loop used hard_deadline instead of interval → fires at T=90"
        );

        // No spurious fire at T=89 (before the hard-deadline would matter).
        tokio::time::advance(Duration::from_secs(29)).await; // T=89
        tokio::task::yield_now().await;
        assert_eq!(
            fetch_count.load(Ordering::SeqCst),
            1,
            "no spurious fire at T=89; next scheduled fire is at T=60+60=120 (cadence only)"
        );

        // Next fire at T=120 (post-fetch last=60 + interval=60).
        tokio::time::advance(Duration::from_secs(31)).await; // T=120
        tokio::task::yield_now().await;
        assert_eq!(
            fetch_count.load(Ordering::SeqCst),
            2,
            "second refresh fires at T=120; cadence-only scheduling confirmed"
        );

        cancel.cancel();
        task.await.expect("refresh loop task");
    }
}

// ── F5: Composition — production timer + ProductionJwksSource ─────────────────
//
// Tests that `nip_fi_jwks_refresh_loop` and `ProductionJwksSource` compose
// correctly across four scenarios that individual tests cannot cover separately:
//
// 1. **Nonzero fetch latency**: a 10s simulated fetch inside a 60s interval;
//    the second refresh is anchored to post-fetch instant (T≈130, not T≈120).
// 2. **Not-due cache hit**: at T=59 (one second before the interval) the source
//    returns the cached snapshot immediately without fetching; fetch_count stays
//    at 1.  The timer loop's boolean return (`is_some()`) correctly reflects a
//    live snapshot from the cache.
// 3. **Failure does not extend snapshot freshness**: after a failed refresh
//    the source's hard_deadline is unchanged (no new snapshot was committed);
//    the previous snapshot remains valid until its original deadline.
// 4. **Hard-deadline cleared by source**: the source clears an expired snapshot
//    on the next `get_snapshot()` call; the timer loop then fires a second
//    fetch and the source commits the fresh snapshot.
//
// ProductionJwksSource uses a controlled clock (`new_with_clock`) so tests
// advance time without wall-clock sleeps.  The timer loop uses tokio's paused
// clock for its own `sleep_until`.
//
// Falsifying mutations:
//   - Remove clock injection → test times out (wall time, unpaused).
//   - Remove `*last = Instant::now()` post-fetch → second refresh fires at T≈120
//     (before T=130 assertion) — test A fails.
//   - Return `true` from the failure path → false positive on the failure test.
//
// These tests are in `mod composition_tests` to isolate their `use` declarations.
#[cfg(test)]
mod composition_tests {
    use buzz_auth::{
        IssuerJwksConfig, JwksFetchError, JwksSourceContract, ProductionJwksSource,
        ScriptedJwksFetcher,
    };
    use std::sync::atomic::Ordering;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio_util::sync::CancellationToken;

    use super::nip_fi_jwks_refresh_loop;
    use super::warm_nip_fi_jwks_snapshots;

    fn test_jwks(kid: &str) -> String {
        format!(
            r#"{{"keys":[{{"kty":"EC","crv":"P-256","x":"f83OJ3D2xF1Bg8vub9tLe1gHMzV76e8Tus9uPHvRVEU","y":"x_FEzRu9m36HLN_tue659LNpXW6pCyStikYjKIWI5a0","use":"sig","alg":"ES256","kid":"{kid}"}}]}}"#
        )
    }

    fn make_source_config(issuer: &str, refresh: u64, hard_deadline: u64) -> IssuerJwksConfig {
        IssuerJwksConfig {
            issuer: issuer.to_owned(),
            contract: JwksSourceContract::new(
                format!("https://{issuer}/.well-known/jwks.json"),
                refresh,
                hard_deadline,
            )
            .expect("valid test contract"),
        }
    }

    // ── Composition A: nonzero fetch latency, not-due cache hit ─────────────
    //
    // Scenario:
    //   T=0: source has no snapshot.  Timer fires at T=60, fetch takes 10s.
    //   T=59: cache NOT due (age < refresh=60) — source returns None (not yet
    //         populated) but the timer's boolean treats None as "no snapshot".
    //         Actually we advance to T=70 to pass the first fetch, then test
    //         the not-due case at T=129 (post-fetch last ≈70, next due ≈130).
    //   T=129: cache is NOT due (age = 129-70 = 59 < 60); timer has not re-fired.
    //   T=131: cache is due; second fetch fires and completes at T=141.
    //
    // The not-due case proves `ProductionJwksSource.get_snapshot()` returns the
    // cached snapshot without fetching when `age < refresh_interval`.  After the
    // first fetch at T=70, the timer advances `last` to T=70; the next sleep
    // waits until T=70+60=130.  So at T=129 the timer has not re-fired:
    // `callback_start_count` stays at 1 and `fetch_count` stays at 1.
    // The test proves fetch_count stays at 1 at T=129 and advances to 2 by T=141.
    #[tokio::test(start_paused = true)]
    async fn composition_nonzero_latency_and_not_due_cache_hit() {
        const ISSUER: &str = "comp-a.issuer.test";
        const REFRESH: u64 = 60;
        const HARD_DEADLINE: u64 = 90;
        const FETCH_LATENCY_SECS: u64 = 10;

        // Controlled clock for the source: starts at T0.
        let t0_secs = chrono::Utc::now().timestamp();
        let clock_secs = Arc::new(std::sync::atomic::AtomicI64::new(t0_secs));
        let clock2 = Arc::clone(&clock_secs);
        let clock3 = Arc::clone(&clock_secs); // for the task closure
        let now_fn: Arc<dyn Fn() -> chrono::DateTime<chrono::Utc> + Send + Sync> =
            Arc::new(move || {
                chrono::DateTime::from_timestamp(clock2.load(Ordering::SeqCst), 0)
                    .unwrap_or(chrono::DateTime::UNIX_EPOCH)
            });

        // Two responses: first succeeds (populates cache), second succeeds.
        let fetcher = ScriptedJwksFetcher::new([Ok(test_jwks("key-a1")), Ok(test_jwks("key-a2"))]);
        let fetcher_count = Arc::clone(&fetcher.call_count);

        // Track how many times the timer callback is *entered* (not just how many
        // fetches complete).  This distinguishes the pre-fetch-last timing mutation:
        // restoring `last = now` before the fetch would cause the second callback to
        // fire at T≈120 (not T≈130), because pre-fetch `last=T60` yields
        // next_due = T60+60=T120; post-fetch `last=T70` yields next_due = T70+60=T130.
        // At T=129 the second callback has already entered under the mutation →
        // callback_start_count == 2.
        let callback_start_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let callback_start_count_task = Arc::clone(&callback_start_count);

        let source = Arc::new(
            ProductionJwksSource::new_with_clock(
                vec![make_source_config(ISSUER, REFRESH, HARD_DEADLINE)],
                fetcher,
                Arc::clone(&now_fn),
            )
            .expect("valid source"),
        );

        let cancel = CancellationToken::new();
        let cancel_task = cancel.clone();
        let source_task = Arc::clone(&source);

        let task = tokio::spawn(async move {
            nip_fi_jwks_refresh_loop(
                vec![(ISSUER.to_owned(), REFRESH)],
                move |issuer| {
                    let s = Arc::clone(&source_task);
                    let issuer = issuer.to_owned();
                    let clock_ref = Arc::clone(&clock3);
                    let start_ctr = Arc::clone(&callback_start_count_task);
                    Box::pin(async move {
                        // Record callback entry before any fetch work.
                        start_ctr.fetch_add(1, Ordering::SeqCst);
                        // Simulate nonzero fetch latency by advancing tokio time
                        // and the source clock by FETCH_LATENCY_SECS.
                        // The source clock advances so `fetched_at` is set correctly.
                        tokio::time::sleep(Duration::from_secs(FETCH_LATENCY_SECS)).await;
                        clock_ref.fetch_add(FETCH_LATENCY_SECS as i64, Ordering::SeqCst);
                        s.get_snapshot(&issuer).await.is_some()
                    })
                },
                cancel_task,
            )
            .await;
        });

        // T=0: loop starts.
        tokio::task::yield_now().await;

        // Advance to T=60: first refresh due.
        tokio::time::advance(Duration::from_secs(60)).await;
        clock_secs.store(t0_secs + 60, Ordering::SeqCst);
        tokio::task::yield_now().await;

        // Advance through 10s fetch latency to T=70.
        tokio::time::advance(Duration::from_secs(10)).await;
        tokio::task::yield_now().await;

        // At T=70: first fetch completed; post-fetch last ≈70; next due ≈130.
        // cache has a snapshot with fetched_at ≈70.
        assert_eq!(
            fetcher_count.load(Ordering::SeqCst),
            1,
            "composition A: exactly one fetch at T=70"
        );

        // T=129: NOT due (age = 129-70 = 59 < 60). No second fetch.
        tokio::time::advance(Duration::from_secs(59)).await; // T=129
        clock_secs.store(t0_secs + 129, Ordering::SeqCst);
        tokio::task::yield_now().await;

        // T=129: assert both timing and cache-hit claims.
        assert_eq!(
            callback_start_count.load(Ordering::SeqCst),
            1,
            "composition A: timer callback must NOT have been entered a second time at T=129. \
             Falsifying mutation: restore pre-fetch `last = now` in the refresh loop → \
             second callback fires at T≈120 (not T≈130: pre-fetch `last=T60` yields \
             next_due = T60+60=T120; post-fetch `last=T70` yields next_due = T70+60=T130; \
             the mutation makes the callback enter at T≈120, before T=129) → \
             callback_start_count == 2 at T=129."
        );
        assert_eq!(
            fetcher_count.load(Ordering::SeqCst),
            1,
            "composition A: cache not-due at T=129 — no second fetch.              Falsifying mutation: remove the not-due short-circuit from get_snapshot →              second fetcher call at T=129 → count == 2."
        );

        // Verify the source serves the cached snapshot without fetching.
        // get_snapshot advances the source clock by 0 (no latency here).
        let snap = source.get_snapshot(ISSUER).await;
        assert!(
            snap.is_some(),
            "composition A: cached snapshot must be live at T=129 (hard_deadline is T≈160)"
        );
        assert_eq!(
            fetcher_count.load(Ordering::SeqCst),
            1,
            "composition A: get_snapshot at T=129 must NOT trigger a new fetch"
        );

        // T=131: second refresh is due (post-fetch last ≈70 + 60 = 130 ≤ 131).
        tokio::time::advance(Duration::from_secs(2)).await; // T=131
        clock_secs.store(t0_secs + 131, Ordering::SeqCst);
        tokio::task::yield_now().await;

        // Advance through 10s fetch latency.
        tokio::time::advance(Duration::from_secs(10)).await; // T=141
        clock_secs.store(t0_secs + 141, Ordering::SeqCst);
        tokio::task::yield_now().await;

        assert_eq!(
            fetcher_count.load(Ordering::SeqCst),
            2,
            "composition A: second fetch by T=141 (post-fetch last ≈70 + interval 60 + fetch 10)"
        );

        cancel.cancel();
        task.await.expect("refresh loop task");
    }

    // ── Composition B: fetch failure does not extend snapshot freshness ──────
    //
    // A successful first fetch populates the snapshot (hard_deadline = T0+90).
    // Verified claims (Claims 1-2 via direct source; Claim 3 via timer callback;
    // Claim 4 via direct source recovery):
    //
    //   1. After a failed refresh at T=61, the previous snapshot is still served
    //      (it is before its hard_deadline: T0+61 < T0+90).
    //   2. The generation is unchanged (no new snapshot committed on failure).
    //   3. At T=122 (past hard_deadline T0+90), the timer callback returns false
    //      (source returns None — snapshot cleared, continuing failure cannot revive it).
    //      Confirmed via both the timer callback signal and a direct source call.
    //   4. After a successful fetch at T=123, the snapshot is recovered — Some.
    //
    // Clock design: both layers share a single Tokio paused clock.
    //   - `nip_fi_jwks_refresh_loop` uses `tokio::time::sleep_until`.
    //   - `ProductionJwksSource` uses a `now_fn` bridged to the same paused clock
    //     via `t0_instant` / `t0_utc` offsets — advancing Tokio time drives both.
    //
    // Response queue (5 total):
    //   response[0]: ok  — warm at T=0 (direct call)
    //   response[1]: fail — stale at T=61 (direct call, snapshot still live)
    //   response[2]: fail — timer callback due T=121, observes T=122 (source T=122 > deadline T=90
    //                        → snapshot cleared → fetch → None → callback returns false)
    //   response[3]: fail — direct call at T=122 (confirms still None; Claim 3 source)
    //   response[4]: ok  — direct call at T=123 (recovery; Claim 4)
    //
    // Sequence:
    //   T=0:   warm (response[0]). Timer NOT yet spawned (Claims 1-2 use direct calls).
    //   T=61:  direct call → stale fail (response[1]) → live snapshot → Claims 1+2.
    //   T=61:  spawn timer. Timer `last = T=61`. First callback due at T=61+60=T=121.
    //   T=122: advance Tokio past the T=121 due time; callback observes T=122. Source: T=122 > deadline=90
    //          → snapshot cleared → fetch response[2]=fail → None → callback returns
    //          false → warn! emitted. callback_count=1.
    //   Wait for callback_count >= 1, then cancel.
    //   T=122: direct call (response[3]=fail) → None. Claim 3 confirmed.
    //   T=123: direct call (response[4]=ok) → Some. Claim 4 confirmed.
    //
    // Falsifying mutation: "store snapshot on failure with extended deadline"
    // sets deadline to T0+61+90=T0+151. At observed T=122:
    //   - now=T0+122 >= deadline=T0+151 is FALSE → snapshot live, NOT cleared.
    //   - age_secs = T0+122 - T0+61 = 61 >= 60 → stale → fetch response[2]=fail.
    //   - Snapshot NOT cleared → fetch fails → but old snapshot kept (live) → Some.
    //   - Callback returns TRUE (not false) → callback_returned_false stays false
    //   - assertion fires: callback_returned_false must be true.
    //   At T=122 direct call: same logic → Some → Claim 3 (is_none()) assertion fires.
    #[tokio::test(start_paused = true)]
    async fn composition_failure_does_not_extend_snapshot_freshness() {
        const ISSUER: &str = "comp-b.issuer.test";
        const REFRESH: u64 = 60;
        const HARD_DEADLINE: u64 = 90;

        // Bridge the source clock to the Tokio paused clock so both layers see
        // identical time when tokio::time::advance() is called.
        let t0_instant = tokio::time::Instant::now();
        let t0_utc = chrono::Utc::now(); // stable: paused runtime
        let t0_instant_b = t0_instant;
        let t0_utc_b = t0_utc;
        let now_fn: Arc<dyn Fn() -> chrono::DateTime<chrono::Utc> + Send + Sync> =
            Arc::new(move || {
                let elapsed_secs = tokio::time::Instant::now()
                    .duration_since(t0_instant_b)
                    .as_secs() as i64;
                t0_utc_b
                    + chrono::Duration::try_seconds(elapsed_secs)
                        .unwrap_or(chrono::Duration::zero())
            });

        // Five responses: [ok, fail, fail, fail, ok] — see sequence above.
        let fetcher = ScriptedJwksFetcher::new([
            Ok(test_jwks("key-b1")),
            Err(JwksFetchError::NetworkError),
            Err(JwksFetchError::NetworkError),
            Err(JwksFetchError::NetworkError),
            Ok(test_jwks("key-b2")),
        ]);
        let fetcher_count = Arc::clone(&fetcher.call_count);

        let source = Arc::new(
            ProductionJwksSource::new_with_clock(
                vec![make_source_config(ISSUER, REFRESH, HARD_DEADLINE)],
                fetcher,
                Arc::clone(&now_fn),
            )
            .expect("valid source"),
        );

        // ── T=0: warm the cache ─────────────────────────────────────────────
        // Direct call: source sees T=0, no snapshot → fetch response[0]=ok → Some.
        let snap_before = source.get_snapshot(ISSUER).await.expect("initial snapshot");
        let generation_before = snap_before.generation();
        assert_eq!(
            fetcher_count.load(Ordering::SeqCst),
            1,
            "composition B: one fetch for initial warm (response[0]=ok)"
        );

        // ── T=61: stale direct call (Claims 1+2) ───────────────────────────
        // Age = 61 >= 60 → stale → fetch response[1]=fail.
        // Snapshot still live (T=61 < hard_deadline=T=90) → Some returned.
        tokio::time::advance(Duration::from_secs(61)).await;
        tokio::task::yield_now().await;

        let snap_after_fail = source.get_snapshot(ISSUER).await;

        // Claim 1: failed refresh at T=61 still returns the previous live snapshot.
        assert!(
            snap_after_fail.is_some(),
            "composition B: failed refresh at T=61 MUST still serve the cached snapshot \
             (T=61 < hard_deadline T=90). \
             Falsifying mutation: clear snapshot on failure → None at T=61."
        );
        assert_eq!(
            fetcher_count.load(Ordering::SeqCst),
            2,
            "composition B: second fetch attempted (response[1]=fail)"
        );

        // Claim 2: generation unchanged (no new snapshot committed on failure).
        let generation_after_fail = snap_after_fail.unwrap().generation();
        assert_eq!(
            generation_before, generation_after_fail,
            "composition B: fetch failure MUST NOT advance the snapshot generation — \
             the cached snapshot is unchanged. \
             Falsifying mutation: commit a new snapshot on failure → generation changes."
        );

        // ── T=61: spawn timer loop ──────────────────────────────────────────
        // Spawned at T=61; timer records `last = T=61`. First callback due at T=121.
        //
        // The timer callback calls source.get_snapshot() and returns its is_some().
        // At T=121 (past hard_deadline T=90) it must return false (snapshot absent).
        let callback_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let callback_count_task = Arc::clone(&callback_count);
        let callback_returned_false = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let callback_returned_false_task = Arc::clone(&callback_returned_false);

        let cancel = CancellationToken::new();
        let cancel_task = cancel.clone();
        let source_task = Arc::clone(&source);

        let task = tokio::spawn(async move {
            nip_fi_jwks_refresh_loop(
                vec![(ISSUER.to_owned(), REFRESH)],
                move |iss| {
                    let s = Arc::clone(&source_task);
                    let iss = iss.to_owned();
                    let ctr = Arc::clone(&callback_count_task);
                    let flag = Arc::clone(&callback_returned_false_task);
                    Box::pin(async move {
                        let result = s.get_snapshot(&iss).await.is_some();
                        if !result {
                            flag.store(true, Ordering::SeqCst);
                        }
                        ctr.fetch_add(1, Ordering::SeqCst);
                        result
                    })
                },
                cancel_task,
            )
            .await;
        });

        // Yield once so the spawned task initializes and records `last = T=61`.
        tokio::task::yield_now().await;

        // ── Advance to T=122 ────────────────────────────────────────────────
        // Timer is due at T=121 (last=T=61, next=T=61+60=T=121 ≤ T=122), but
        // `tokio::time::advance` jumps to T=122 before the task runs, so the
        // callback observes T=122 > hard_deadline=T=90 → snapshot cleared →
        // fetch response[2]=fail → None. Callback returns false.
        // callback_returned_false=true. callback_count=1.
        //
        // Falsifying mutation: extend deadline to T=61+90=T=151 on failure.
        // At T=122: T=122 < T=151 → snapshot NOT cleared; age_secs=122-61=61 >= 60
        // → stale → fetch response[2]=fail → snapshot not cleared (still live) → Some.
        // Callback returns true → callback_returned_false stays false → assertion fires.
        tokio::time::advance(Duration::from_secs(61)).await; // T=61 → T=122
                                                             // Bounded yield: let the spawned timer task run its callback (observed at T=122).
                                                             // At most 10_000 yields; if the callback never fires this diagnostic fails
                                                             // rather than hanging forever.  A virtual tokio::time::timeout would also
                                                             // create a fake timer that never fires while the clock is paused — hence
                                                             // the explicit iteration bound.
        for i in 0..10_000usize {
            if callback_count.load(Ordering::SeqCst) >= 1 {
                break;
            }
            if i == 9_999 {
                panic!(
                    "composition B: callback_count never reached 1 after 10_000 yields. \
                     The spawned refresh loop task may have panicked or stalled."
                );
            }
            tokio::task::yield_now().await;
        }

        // Claim 3 via timer: callback_returned_false proves the timer observed None.
        assert!(
            callback_returned_false.load(Ordering::SeqCst),
            "composition B: timer callback at T=122 MUST return false (source returns None \
             when snapshot is past hard_deadline=T=90 and fetch keeps failing). \
             Falsifying mutation: extend deadline on failure (T=61+90=T=151) → \
             snapshot still live at T=122 → callback returns true → this assertion fires."
        );

        cancel.cancel();
        task.await.expect("refresh loop task");

        // ── Claim 3 via direct source call ──────────────────────────────────
        // T=122: past hard_deadline=T=90 → snapshot cleared → fetch response[3]=fail → None.
        // Confirms the source still returns None after the timer observed expiry.
        let snap_at_expiry = source.get_snapshot(ISSUER).await;
        assert!(
            snap_at_expiry.is_none(),
            "composition B: at T=122 (past hard_deadline T=90), get_snapshot MUST return None. \
             Snapshot must be cleared and continuing fetch failure cannot revive it. \
             Falsifying mutation: extend deadline on failure → snapshot live at T=122 → Some."
        );
        assert_eq!(
            fetcher_count.load(Ordering::SeqCst),
            4, // warm(1) + stale-fail(2) + timer-expiry(3) + direct-expiry(4)
            "composition B: fourth fetch at T=122 (direct expiry verification, response[3]=fail)"
        );

        // ── Claim 4: recovery ────────────────────────────────────────────────
        // Advance to T=123 (still past hard_deadline=T=90, snapshot absent).
        // get_snapshot forces a fetch → response[4]=ok → Some.
        tokio::time::advance(Duration::from_secs(1)).await; // T=122 → T=123
        tokio::task::yield_now().await;
        let snap_after_recovery = source.get_snapshot(ISSUER).await;
        assert!(
            snap_after_recovery.is_some(),
            "composition B: recovery fetch at T=123 MUST return a new snapshot \
             (response[4]=ok). \
             Falsifying mutation: make get_snapshot always return Some (never trigger a fetch) \
             → response[4]=ok never consumed → generation unchanged → assert_ne!(generation) \
             below fires."
        );
        assert_eq!(
            fetcher_count.load(Ordering::SeqCst),
            5,
            "composition B: fifth fetch at T=123 (recovery, response[4]=ok)"
        );
        let generation_after_recovery = snap_after_recovery.unwrap().generation();
        // New JWKS content "key-b2" ≠ "key-b1" → generation must have advanced.
        assert_ne!(
            generation_before, generation_after_recovery,
            "composition B: recovery fetch with different JWKS MUST advance the generation. \
             Falsifying mutation: never commit a new snapshot → generation unchanged."
        );
    }

    // ── Composition C: privacy — issuer URL must not appear in log output ────
    //
    // `ProductionJwksSource` logs `warn!(error = %err, ...)` on fetch failure
    // and the timer loop logs `warn!(issuer_index = idx, ...)` on no-snapshot.
    // The startup warm writers log `info!(issuer_index = idx, ...)` on success
    // and `warn!(issuer_index = idx, ...)` on failure.
    // None of these paths must echo the raw issuer URL or JWKS URI.
    //
    // This test drives four paths:
    //   1. `warm_nip_fi_jwks_snapshots()` success → startup `info!` (no URI).
    //   2. `warm_nip_fi_jwks_snapshots()` failure → startup `warn!` (no URI).
    //   3. `ProductionJwksSource::get_snapshot()` fetch fail →
    //      library `warn!(error = %err, "nip-fi jwks fetch failed...")` (no URI).
    //   4. `nip_fi_jwks_refresh_loop` no-snapshot →
    //      `warn!(issuer_index = idx, "NIP-FI: background JWKS refresh returned no snapshot")`
    //      (no URI). Requires source clock past hard_deadline so get_snapshot
    //      returns None (snapshot cleared) and the callback returns false.
    //
    // Falsifying mutation (timer path): add `issuer_uri = config.contract.jwks_uri()`
    // to any warn! → sentinel appears in captured output → assertion fires.
    //
    // Clock design: both layers share a single paused Tokio clock.
    //   - `nip_fi_jwks_refresh_loop` uses `tokio::time::sleep_until` and
    //     `tokio::time::Instant::now()` — controlled by `start_paused` runtime.
    //   - `ProductionJwksSource` uses an injected `now_fn` — bridged to the
    //     same paused Tokio clock via `t0_instant` / `t0_utc` offsets so both
    //     layers observe the same time when Tokio time is advanced.
    //
    // Sequence (all times are Tokio-paused clock offsets from T=0):
    //   T=0:  spawn timer loop; yield so it records `last = Instant::now() = T0`.
    //         First callback due at T=60.
    //   T=0:  warm (paths 1+2) consumes responses[0]=ok, [1]=fail.
    //         Snapshot for issuer_ok: fetched_at=T0, hard_deadline=T90.
    //         All subsequent fetcher calls return NetworkError (queue exhausted).
    //   T=61: advance Tokio → timer due at T=60 runs at T=61 (still live: T61 < T90,
    //         stale age=61 → fetch → NetworkError → live snapshot returned; post-fetch
    //         last=T61).
    //         Path 3 direct call at T=61: same result → fetch-fail warn! ✓.
    //   T=121 (T=61+60): advance Tokio → timer fires at T=121 (second fire:
    //         last=T61, next_due=T121). Source clock at T=121: now=T121 > deadline=T90
    //         → snapshot cleared → fetch fails → None → callback returns false →
    //         path-4 warn! ✓. Wait for callback count >= 2 then cancel.
    //
    // Falsifying mutation (path 4): bridge now_fn to a fixed clock at T=61 →
    // at timer-fire T=121, source sees T=61 < T=90 → snapshot live → callback
    // returns true → no warn! → path-4 assertion fails.
    //
    // Uses `#[test]` + manual runtime so `with_default` wraps all async execution.
    #[test]
    fn composition_log_does_not_leak_issuer_url() {
        use std::io::Write;

        // Sentinel must be lowercase: JwksSourceContract::new() canonicalizes
        // the URI and lowercases the host component. An uppercase sentinel would
        // never match the canonicalized output even when the URI leaks.
        const SENTINEL: &str = "sentinel-issuer-url-9f4e2b1a";
        const REFRESH: u64 = 60;
        const HARD_DEADLINE: u64 = 90;

        // Two issuers: one warm-success, one warm-fail.
        let issuer_ok = format!("https://{SENTINEL}.ok.example.invalid");
        let issuer_fail = format!("https://{SENTINEL}.fail.example.invalid");
        let jwks_uri_ok = format!("https://{SENTINEL}.cdn-ok.example.invalid/jwks.json");
        let jwks_uri_fail = format!("https://{SENTINEL}.cdn-fail.example.invalid/jwks.json");

        // Capture log output.
        let buf = Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
        #[derive(Clone)]
        struct MakeCapturing(Arc<std::sync::Mutex<Vec<u8>>>);
        struct CapturingWriter(Arc<std::sync::Mutex<Vec<u8>>>);
        impl Write for CapturingWriter {
            fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(data);
                Ok(data.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for MakeCapturing {
            type Writer = CapturingWriter;
            fn make_writer(&'a self) -> CapturingWriter {
                CapturingWriter(Arc::clone(&self.0))
            }
        }
        let subscriber = tracing_subscriber::fmt()
            .with_writer(MakeCapturing(Arc::clone(&buf)))
            .with_ansi(false)
            .finish();

        tracing::subscriber::with_default(subscriber, || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_time()
                .start_paused(true)
                .build()
                .expect("runtime");

            rt.block_on(async {
                // Anchor the source clock to Tokio's paused time.
                // Both layers (timer loop + source) share this clock:
                // advancing Tokio time drives the source clock identically.
                let t0_instant = tokio::time::Instant::now();
                let t0_utc = chrono::Utc::now(); // stable: paused runtime
                let t0_instant_c = t0_instant;
                let t0_utc_c = t0_utc;
                let now_fn: Arc<dyn Fn() -> chrono::DateTime<chrono::Utc> + Send + Sync> =
                    Arc::new(move || {
                        let elapsed_secs = tokio::time::Instant::now()
                            .duration_since(t0_instant_c)
                            .as_secs() as i64;
                        t0_utc_c
                            + chrono::Duration::try_seconds(elapsed_secs)
                                .unwrap_or(chrono::Duration::zero())
                    });

                // Queue: [warm-ok, warm-fail]; all subsequent calls → NetworkError.
                let fetcher = ScriptedJwksFetcher::new([
                    Ok(r#"{"keys":[{"kty":"EC","crv":"P-256","x":"f83OJ3D2xF1Bg8vub9tLe1gHMzV76e8Tus9uPHvRVEU","y":"x_FEzRu9m36HLN_tue659LNpXW6pCyStikYjKIWI5a0","use":"sig","alg":"ES256","kid":"k1"}]}"#.to_string()),
                    Err(JwksFetchError::NetworkError),
                ]);

                let source = Arc::new(
                    ProductionJwksSource::new_with_clock(
                        vec![
                            IssuerJwksConfig {
                                issuer: issuer_ok.clone(),
                                contract: JwksSourceContract::new(
                                    jwks_uri_ok.clone(),
                                    REFRESH,
                                    HARD_DEADLINE,
                                )
                                .expect("valid test contract"),
                            },
                            IssuerJwksConfig {
                                issuer: issuer_fail.clone(),
                                contract: JwksSourceContract::new(
                                    jwks_uri_fail.clone(),
                                    REFRESH,
                                    HARD_DEADLINE,
                                )
                                .expect("valid test contract"),
                            },
                        ],
                        fetcher,
                        Arc::clone(&now_fn),
                    )
                    .expect("valid source"),
                );

                // Count callback completions so we know when path 4 has fired.
                // Callback 1 (at T=61): snapshot live → true (no warn!).
                // Callback 2 (at T=121): snapshot past deadline → false → path-4 warn!.
                let callback_count =
                    Arc::new(std::sync::atomic::AtomicUsize::new(0));
                let callback_count_task = Arc::clone(&callback_count);

                // Spawn timer loop at T=0. `last = Instant::now() = T0`.
                // First callback due at T=60 (runs at T=61); second at T=121 (post-fetch
                // last=T61).
                let cancel = tokio_util::sync::CancellationToken::new();
                let cancel_task = cancel.clone();
                let source_task = Arc::clone(&source);
                let issuer_task = issuer_ok.clone();
                let task = tokio::spawn(async move {
                    nip_fi_jwks_refresh_loop(
                        vec![(issuer_task.clone(), REFRESH)],
                        move |iss| {
                            let s = Arc::clone(&source_task);
                            let iss = iss.to_owned();
                            let ctr = Arc::clone(&callback_count_task);
                            Box::pin(async move {
                                let result = s.get_snapshot(&iss).await.is_some();
                                ctr.fetch_add(1, Ordering::SeqCst);
                                result
                            })
                        },
                        cancel_task,
                    )
                    .await;
                });

                // Yield once: spawned task initializes, records `last = T0`.
                tokio::task::yield_now().await;

                // Paths 1+2: startup warm writers.
                // Consumes responses[0]=ok (issuer_ok) and [1]=fail (issuer_fail).
                // Emits:  info!(issuer_index=0, "NIP-FI: JWKS snapshot warmed")
                //         warn!(issuer_index=1, "NIP-FI: JWKS warm failed…")
                // After warm: issuer_ok snapshot has fetched_at=T0, hard_deadline=T0+90.
                // Queue exhausted; all subsequent fetcher calls → NetworkError.
                let issuer_ids = vec![issuer_ok.clone(), issuer_fail.clone()];
                warm_nip_fi_jwks_snapshots(&*source, &issuer_ids).await;

                // Path 3: library fetch-fail warn!.
                // Advance to T=61 (past refresh interval=60, before hard_deadline=90).
                // During the advance, the timer fires at T=60 but resolves at T=61:
                //   source clock=T61, age=61 >= 60 → stale → fetch → NetworkError
                //   → fetch-fail warn! [path 3 precursor] → live snapshot returned
                //   → callback 1 returns true (no path-4 warn!); last=T61.
                // Then path 3 direct call at T=61 also produces fetch-fail warn! ✓.
                tokio::time::advance(std::time::Duration::from_secs(61)).await;
                // Bounded yield: allow the T=60 timer callback to run.
                for i in 0..10_000usize {
                    if callback_count.load(Ordering::SeqCst) >= 1 {
                        break;
                    }
                    if i == 9_999 {
                        panic!(
                            "privacy C: callback_count never reached 1 after 10_000 yields at T=61. \
                             The spawned privacy timer task may have panicked or stalled."
                        );
                    }
                    tokio::task::yield_now().await;
                }
                // Path 3: direct call; produces `warn!(error = %err, "nip-fi jwks fetch failed…")`.
                let _ = source.get_snapshot(&issuer_ok).await;

                // Path 4: timer loop no-snapshot warn!.
                // Advance from T=61 to T=121 (60 more seconds).
                // Timer fires at T=121 (post-fetch last=T61, next_due=T61+60=T121).
                // Source clock via now_fn = T=121 >= hard_deadline=T=90:
                //   → snapshot cleared
                //   → fetch fails (NetworkError) → None
                //   → callback 2 returns false
                //   → `warn!(issuer_index=idx, "NIP-FI: background JWKS refresh
                //       returned no snapshot")` ✓.
                //
                // Falsifying mutation: bridge now_fn to a fixed clock at T=61 →
                // at T=121 source sees T=61 < T=90 → snapshot live → callback true
                // → no warn! → path-4 assertion fires.
                tokio::time::advance(std::time::Duration::from_secs(60)).await;
                // Bounded yield: wait for callback 2 (count >= 2) to confirm path-4 has run.
                for i in 0..10_000usize {
                    if callback_count.load(Ordering::SeqCst) >= 2 {
                        break;
                    }
                    if i == 9_999 {
                        panic!(
                            "privacy C: callback_count never reached 2 after 10_000 yields at T=121. \
                             The spawned privacy timer task may have panicked or stalled."
                        );
                    }
                    tokio::task::yield_now().await;
                }
                cancel.cancel();
                let _ = task.await;
            });
        });

        let captured = String::from_utf8(buf.lock().unwrap().clone()).unwrap_or_default();

        // Assert startup warm-success was captured (path 1).
        assert!(
            captured.contains("JWKS snapshot warmed"),
            "Expected info! 'NIP-FI: JWKS snapshot warmed' from startup warm success path. \
             Captured (first 500 chars):\n{}",
            &captured[..captured.len().min(500)]
        );
        // Assert startup warm-fail was captured (path 2).
        assert!(
            captured.contains("JWKS warm failed"),
            "Expected warn! 'NIP-FI: JWKS warm failed' from startup warm failure path. \
             Captured (first 500 chars):\n{}",
            &captured[..captured.len().min(500)]
        );
        // Assert library fetch-fail warn! was captured (path 3).
        // Fired by the T=60 timer callback and/or the T=61 direct call — either path
        // produces `warn!(error = %err, "nip-fi jwks fetch failed…")`.
        assert!(
            captured.contains("nip-fi jwks fetch failed"),
            "Expected warn! 'nip-fi jwks fetch failed' from ProductionJwksSource. \
             The log capture infrastructure may be broken. \
             Captured (first 500 chars):\n{}",
            &captured[..captured.len().min(500)]
        );
        // Assert timer background warn! was captured (path 4).
        // Fired by callback 2 at Tokio T=121: source clock T=121 > deadline T=90
        // → get_snapshot returns None → callback returns false → warn! emitted.
        //
        // Falsifying mutation: bridge now_fn to a fixed T=61 clock →
        // source at T=121 still sees T=61 < T=90 → snapshot live → callback true
        // → no warn! → this assertion fails.
        assert!(
            captured.contains("background JWKS refresh returned no snapshot"),
            "Expected timer warn! 'NIP-FI: background JWKS refresh returned no snapshot'. \
             Fired when callback 2 (Tokio T=121) finds source clock T=121 > hard_deadline T=90. \
             Captured (first 500 chars):\n{}",
            &captured[..captured.len().min(500)]
        );
        // Assert no sentinel in any log output.
        assert!(
            !captured.contains(SENTINEL),
            "NIP-FI logs MUST NOT contain the raw issuer URL or JWKS URI. \
             Sentinel '{SENTINEL}' found in captured output. \
             Falsifying mutation: add issuer_uri to any warn! call → sentinel appears.\n\
             Captured (first 500 chars):\n{}",
            &captured[..captured.len().min(500)]
        );
    }
}
