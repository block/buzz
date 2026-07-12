use buzz_push_gateway::{
    apns::ApnsTransport,
    app_attest::AppAttestVerifier,
    authority::AuthorityStore,
    config::Config,
    grant::{GrantKey, GrantKeyring},
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
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(EnvFilter::from_default_env())
        .init();
    let c = Config::from_env()?;
    let metrics_handle = buzz_push_gateway::metrics::install()?;
    let transport = Arc::new(ApnsTransport::token(
        &fs::read(&c.apns_key_path)?,
        &c.apns_key_id,
        &c.apns_team_id,
        c.apns_topic,
    )?);
    let grant_keyring = GrantKeyring::new(
        c.grant_keys
            .iter()
            .map(|key| GrantKey::new(&key.id, &key.key))
            .collect::<Result<_, _>>()?,
    )?;
    let token_keyring = TokenKeyring::new(
        c.token_keys
            .iter()
            .map(|key| TokenKey::new(&key.id, &key.key))
            .collect::<Result<_, _>>()?,
    )?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(20)
        .connect(&c.database_url)
        .await?;
    sqlx::migrate!("../../migrations").run(&pool).await?;
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
    let app_attest = Arc::new(AppAttestVerifier::new(
        c.app_attest_app_id,
        fs::read(&c.app_attest_root_cert_path)?,
    )?);
    let accepting = Arc::new(AtomicBool::new(true));
    let (public, health) = router_with_metrics(
        AppState {
            grant_keyring: Arc::new(grant_keyring),
            app_attest,
            authority,
            token_keyring: Arc::new(token_keyring),
            transport,
            delivery_url: c.public_delivery_url,
            max_grant_lifetime_seconds: c.max_grant_lifetime_seconds,
            max_installation_lifetime_seconds: c.max_installation_lifetime_seconds,
            endpoint_quota_window_seconds: c.endpoint_quota_window_seconds,
            endpoint_quota_max_deliveries: c.endpoint_quota_max_deliveries,
            enabled_profiles: c.enabled_profiles,
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
        axum::serve(pl, public)
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
