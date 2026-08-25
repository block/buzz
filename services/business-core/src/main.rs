use business_core::{
    b4::ProfitProjectionService, router, s1::OperationsService, AppState, Config, PgStore,
};
use sqlx::postgres::PgPoolOptions;
use std::time::Duration;
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "business_core=info".into()),
        )
        .json()
        .init();
    let config = Config::from_env().map_err(|error| format!("configuration error: {error}"))?;
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&config.database_url)
        .await?;
    let store = PgStore::new(pool);
    store.migrate().await?;
    if config.profit_projection_worker_enabled {
        let projection = ProfitProjectionService::with_retry_limit(
            store.clone(),
            config.profit_projection_retry_limit,
        );
        let batch_size = config.profit_projection_batch_size;
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(2));
            loop {
                interval.tick().await;
                if let Err(error) = projection
                    .project_pending(Uuid::nil(), Uuid::new_v4(), batch_size)
                    .await
                {
                    tracing::error!(error = %error, "profit projection worker iteration failed");
                }
            }
        });
    }
    let operating_subscriptions = OperationsService::new(
        store.clone(),
        config.profit_projection_worker_enabled,
        config.profit_data_stale_after_minutes,
    );
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            interval.tick().await;
            match operating_subscriptions
                .run_due_operating_subscriptions(20)
                .await
            {
                Ok(count) if count > 0 => {
                    tracing::info!(count, "operating snapshot subscriptions generated")
                }
                Ok(_) => {}
                Err(error) => {
                    tracing::error!(error = %error, "operating snapshot subscription iteration failed")
                }
            }
        }
    });
    let bind_addr = config.bind_addr;
    let app = router(AppState::new(store, &config));
    tracing::info!(address = %bind_addr, "Business Core S1 listening");
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
