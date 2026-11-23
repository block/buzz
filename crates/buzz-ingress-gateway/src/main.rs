use buzz_ingress_gateway::{
    config::Config, forwarder::Forwarder, http::IntakeLimits, intake::RateLimiter,
    postgres::PostgresStore, relay_client::HttpRelayClient, router_with_metrics,
    secrets::SecretKeyring, AppState,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tracing_subscriber::EnvFilter;

/// Without `RUST_LOG`, keep the gateway's own warnings alongside `EnvFilter`'s
/// error-level default for all other targets.
fn log_env_filter(rust_log: Option<&str>) -> EnvFilter {
    EnvFilter::new(rust_log.unwrap_or("error,buzz_ingress_gateway=warn"))
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
        let runtime_role = std::env::var("BUZZ_INGRESS_RUNTIME_DATABASE_ROLE")?;
        PostgresStore::apply_migrations_and_grants(&pool, &runtime_role).await?;
        return Ok(());
    }
    let c = Config::from_env()?;
    let metrics_handle = buzz_ingress_gateway::metrics::install()?;
    let secrets = Arc::new(SecretKeyring::from_config(&c.secret_keyring)?);
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(20)
        .connect(&c.database_url)
        .await?;
    let store = Arc::new(PostgresStore::new(pool));
    store
        .reap_expired(chrono::Utc::now(), c.retention_days)
        .await?;
    let reaper_store = Arc::clone(&store);
    let retention_days = c.retention_days;
    let reaper = tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
        interval.tick().await;
        loop {
            interval.tick().await;
            if reaper_store
                .reap_expired(chrono::Utc::now(), retention_days)
                .await
                .is_err()
            {
                buzz_ingress_gateway::metrics::record_reaper_failure();
                tracing::warn!("ingress gateway retention reaper failed");
            }
        }
    });
    let forwarder = tokio::spawn(
        Forwarder {
            store: Arc::clone(&store) as Arc<dyn buzz_ingress_gateway::store::Store>,
            client: Arc::new(HttpRelayClient::new(c.signing_keys.clone())?),
            keys: c.signing_keys.clone(),
            scheme: c.relay_scheme,
            concurrency: c.forward_concurrency,
            retry_max_age: chrono::Duration::seconds(c.retry_max_age_seconds),
            now: chrono::Utc::now,
        }
        .run(),
    );
    let accepting = Arc::new(AtomicBool::new(true));
    let (public, private) = router_with_metrics(
        AppState {
            store,
            secrets,
            relay_pubkeys: Arc::new(c.relay_pubkeys.clone()),
            intake: IntakeLimits {
                max_body_bytes: c.max_body_bytes,
                timestamp_tolerance_seconds: c.timestamp_tolerance_seconds,
            },
            rate_limiter: Arc::new(RateLimiter::new(c.rate_limit_per_minute)),
            now: chrono::Utc::now,
            accepting: accepting.clone(),
        },
        Some(metrics_handle),
    );
    let public_listener = tokio::net::TcpListener::bind(c.listen_addr).await?;
    let private_listener = tokio::net::TcpListener::bind(c.private_addr).await?;
    let (ptx, prx) = tokio::sync::watch::channel(false);
    let (htx, hrx) = tokio::sync::watch::channel(false);
    let p = tokio::spawn(async move {
        axum::serve(public_listener, public)
            .with_graceful_shutdown(async move {
                let mut rx = prx;
                let _ = rx.changed().await;
            })
            .await
    });
    let h = tokio::spawn(async move {
        axum::serve(private_listener, private)
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
    forwarder.abort();
    reaper.abort();
    Ok(())
}

async fn shutdown_signal() -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut term = signal(SignalKind::terminate())?;
        tokio::select! { r = tokio::signal::ctrl_c() => r, _ = term.recv() => Ok(()) }
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
            tracing::info!(target: "buzz_ingress_gateway", "gateway-info");
            tracing::warn!(target: "buzz_ingress_gateway", "gateway-warn");
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
    }
}
