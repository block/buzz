use buzz_push_gateway::{
    apns::ApnsTransport,
    config::Config,
    grant::{GrantKey, GrantKeyring},
    router, AppState,
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
    let accepting = Arc::new(AtomicBool::new(true));
    let (public, health) = router(AppState {
        grant_keyring: Arc::new(grant_keyring),
        transport,
        delivery_url: c.public_delivery_url,
        issuance_url: c.public_issuance_url,
        max_grant_lifetime_seconds: c.max_grant_lifetime_seconds,
        enabled_profiles: c.enabled_profiles,
        authorized_relays: c.authorized_relays,
        now: || chrono::Utc::now().timestamp(),
        accepting: accepting.clone(),
    });
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
