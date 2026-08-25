use business_auth_gateway::{auth::JwtVerifier, router, AppState, Config, Store};
use sqlx::postgres::PgPoolOptions;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(EnvFilter::from_default_env())
        .init();
    let command = std::env::args().nth(1);
    if command.as_deref() == Some("--version") {
        println!("business-auth-gateway {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let database_url = std::env::var("DATABASE_URL")?;
    let pool = PgPoolOptions::new()
        .max_connections(if command.as_deref() == Some("--migrate-only") {
            1
        } else {
            20
        })
        .connect(&database_url)
        .await?;
    if command.as_deref() == Some("--migrate-only") {
        Store::migrate(&pool).await?;
        if let Ok(role) = std::env::var("BUSINESS_AUTH_RUNTIME_DATABASE_ROLE") {
            Store::grant_runtime(&pool, &role).await?;
        }
        return Ok(());
    }
    let config = Config::from_env().map_err(|e| format!("configuration error: {e}"))?;
    let store = Store::new(pool, config.clone());
    store.ready().await.map_err(|_| "database is not ready")?;
    if command.as_deref() == Some("--cleanup-once") {
        let (challenges, embed, business) = store.cleanup().await.map_err(|_| "cleanup failed")?;
        tracing::info!(
            expired_challenges = challenges,
            expired_embed_sessions = embed,
            expired_business_sessions = business,
            "session cleanup complete"
        );
        return Ok(());
    }
    let cleanup = store.clone();
    let interval = config.cleanup_interval;
    tokio::spawn(async move {
        let mut timer = tokio::time::interval(interval);
        timer.tick().await;
        loop {
            timer.tick().await;
            if cleanup.cleanup().await.is_err() {
                tracing::warn!(event = "session_cleanup_failed");
            }
        }
    });
    let state = AppState {
        store,
        verifier: JwtVerifier::new(&config),
        config: config.clone(),
    };
    let listener = tokio::net::TcpListener::bind(config.bind_addr).await?;
    tracing::info!(address=%config.bind_addr,"business auth gateway listening");
    axum::serve(listener, router(state))
        .with_graceful_shutdown(shutdown())
        .await?;
    Ok(())
}
async fn shutdown() {
    let _ = tokio::signal::ctrl_c().await;
}
