use buzz_push_gateway::{
    apns::ApnsTransport,
    app_attest::AppAttestVerifier,
    authority::AuthorityStore,
    config::Config,
    postgres::PostgresAuthorityStore,
    router_with_metrics,
    token::{TokenKey, TokenKeyring},
    AppState,
};
use std::{
    fs,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tracing_subscriber::EnvFilter;

/// Without `RUST_LOG`, keep the gateway's own warnings, such as App Attest
/// rejection reasons, alongside `EnvFilter`'s error-level default for all
/// other targets.
fn log_env_filter(rust_log: Option<&str>) -> EnvFilter {
    EnvFilter::new(rust_log.unwrap_or("error,buzz_push_gateway=warn"))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(log_env_filter(std::env::var("RUST_LOG").ok().as_deref()))
        .init();
    if std::env::args().nth(1).as_deref() == Some("--migrate-only") {
        let database_url = std::env::var("DATABASE_URL")?;
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect(&database_url)
            .await?;
        let runtime_role = std::env::var("BUZZ_PUSH_RUNTIME_DATABASE_ROLE")?;
        PostgresAuthorityStore::apply_migrations_and_grants(&pool, &runtime_role).await?;
        return Ok(());
    }
    let c = Config::from_env()?;
    let token_keyring = TokenKeyring::new(
        c.token_keys
            .iter()
            .map(|key| TokenKey::new(&key.id, &key.key))
            .collect::<Result<_, _>>()?,
    )?;
    let pool_options = sqlx::postgres::PgPoolOptions::new()
        .max_connections(20)
        .after_connect(|connection, _| Box::pin(async move {
                sqlx::raw_sql("SET lock_timeout='2s'; SET statement_timeout='10s'; SET idle_in_transaction_session_timeout='15s'")
                    .execute(connection).await?;
                Ok(())
            }));
    if std::env::args().nth(1).as_deref() == Some("--reencrypt-token-storage") {
        let pool = pool_options
            .max_connections(1)
            .connect(&c.database_url)
            .await?;
        token_keyring.reencrypt_storage(&pool).await?;
        tracing::info!("APNs token storage verified: zero rows require predecessor keys");
        return Ok(());
    }
    let metrics_handle = buzz_push_gateway::metrics::install()?;
    let app_attest_root = fs::read(&c.app_attest_root_cert_path)?;
    let configured = &c.application;
    let application = {
        let apple = AppAttestVerifier::with_environment(
            configured.app_attest_app_id.clone(),
            app_attest_root,
            configured.app_attest_environment,
        )?;
        tracing::info!(environment = ?apple.environment(), "App Attest verifier configured");
        let transport = Arc::new(ApnsTransport::certificate(
            &fs::read(&configured.apns_cert_path)?,
            configured.apns_topic.clone(),
            configured.apns_environment,
        )?);
        buzz_push_gateway::http::ApplicationRuntime {
            app_attest: Arc::new(apple),
            transport,
        }
    };
    let pool = pool_options.connect(&c.database_url).await?;
    token_keyring.audit_storage(&pool).await?;
    let authority = Arc::new(PostgresAuthorityStore::new(pool));
    authority
        .reap_expired(chrono::Utc::now().timestamp())
        .await?;
    let reaper_authority = Arc::clone(&authority);
    let reaper = tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
        interval.tick().await;
        loop {
            interval.tick().await;
            if reaper_authority
                .reap_expired(chrono::Utc::now().timestamp())
                .await
                .is_err()
            {
                buzz_push_gateway::metrics::record_reaper_failure();
                tracing::warn!("push gateway retention reaper failed");
            }
        }
    });
    let accepting = Arc::new(AtomicBool::new(true));
    let (public, health) = router_with_metrics(
        AppState {
            gateway_origin: c.gateway_origin,
            trusted_proxy_cidrs: c.trusted_proxy_cidrs,
            authority,
            token_keyring: Arc::new(token_keyring),
            application: Arc::new(application),
            max_grant_lifetime_seconds: c.max_grant_lifetime_seconds,
            max_installation_lifetime_seconds: c.max_installation_lifetime_seconds,
            endpoint_quota_window_seconds: c.endpoint_quota_window_seconds,
            endpoint_quota_max_deliveries: c.endpoint_quota_max_deliveries,
            now: || chrono::Utc::now().timestamp(),
            accepting: accepting.clone(),
        },
        Some(metrics_handle),
    );
    let pl = tokio::net::TcpListener::bind(c.bind_addr).await?;
    let hl = tokio::net::TcpListener::bind(c.health_addr).await?;
    let (ptx, prx) = tokio::sync::watch::channel(false);
    let (htx, hrx) = tokio::sync::watch::channel(false);
    let p = tokio::spawn(async move {
        axum::serve(
            pl,
            public.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .with_graceful_shutdown(async move {
            let mut rx = prx;
            let _ = rx.changed().await;
        })
        .await
    });
    let h = tokio::spawn(async move {
        axum::serve(hl, health)
            .with_graceful_shutdown(async move {
                let mut rx = hrx;
                let _ = rx.changed().await;
            })
            .await
    });
    shutdown_signal().await?;
    accepting.store(false, Ordering::SeqCst);
    let _ = ptx.send(true);
    let _ = tokio::time::timeout(std::time::Duration::from_secs(30), p).await;
    let _ = htx.send(true);
    let _ = h.await;
    reaper.abort();
    Ok(())
}
async fn shutdown_signal() -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut term = signal(SignalKind::terminate())?;
        tokio::select! {r=tokio::signal::ctrl_c()=>r,_=term.recv()=>Ok(())}
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await
    }
}

#[cfg(test)]
mod tests {
    use super::log_env_filter;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Capture {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn emitted(rust_log: Option<&str>) -> String {
        let capture = Capture::default();
        let writer = capture.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_env_filter(log_env_filter(rust_log))
            .with_writer(move || writer.clone())
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(target: "buzz_push_gateway", "gateway-info");
            tracing::warn!(target: "buzz_push_gateway", "gateway-warn");
            tracing::warn!(target: "sqlx", "dependency-warn");
            tracing::error!(target: "sqlx", "dependency-error");
        });
        let bytes = capture.0.lock().unwrap().clone();
        String::from_utf8(bytes).unwrap()
    }

    // The only test in this binary, so no other thread races tracing's
    // global callsite-interest cache.
    #[test]
    fn default_log_filter_keeps_gateway_warnings_and_dependency_errors() {
        let logs = emitted(None);
        assert!(logs.contains("gateway-warn"), "{logs}");
        assert!(logs.contains("dependency-error"), "{logs}");
        assert!(!logs.contains("gateway-info"), "{logs}");
        assert!(!logs.contains("dependency-warn"), "{logs}");

        let logs = emitted(Some("info"));
        assert!(logs.contains("gateway-info"), "{logs}");
        assert!(logs.contains("dependency-warn"), "{logs}");
    }
}
