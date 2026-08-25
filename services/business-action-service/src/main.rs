use business_action_service::{
    acceptance_engine, acceptance_router_with_gateway, catalog_from_path, ActionEngine, ActionMode,
    Config, PgActionStore,
};
use sqlx::postgres::PgPoolOptions;
use std::net::SocketAddr;
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "business_action_service=info".into()),
        )
        .json()
        .init();
    let config = Config::from_env().map_err(|error| format!("configuration error: {error}"))?;
    if !config.enabled {
        return Err("configuration error: BUSINESS_ACTION_ENABLED=true is required".into());
    }
    if config.mode != ActionMode::Acceptance {
        return Err("configuration error: production authorization and AssigneeResolver adapters are not installed; refusing Business Action writes".into());
    }
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&config.database_url)
        .await?;
    let store = PgActionStore::new(pool);
    store.migrate().await?;
    let mut engine = match store.load().await? {
        Some(state) => ActionEngine::from_state(
            catalog_from_path(&config.catalog_path, &config.catalog_version)
                .map_err(|error| format!("catalog error: {error}"))?,
            state,
        )?,
        None => {
            let mut seeded = acceptance_engine(Uuid::new_v4())?;
            seeded = ActionEngine::from_state(
                catalog_from_path(&config.catalog_path, &config.catalog_version)
                    .map_err(|error| format!("catalog error: {error}"))?,
                seeded.state,
            )?;
            seeded
        }
    };
    engine.configure_limits(
        chrono::Duration::from_std(config.work_item_draft_ttl)?,
        chrono::Duration::from_std(config.approval_draft_ttl)?,
        usize::try_from(config.max_active_items_per_finding)?,
    )?;
    store.save(&engine).await?;
    let app = acceptance_router_with_gateway(
        engine,
        config.allowed_origin,
        config.service_credential,
        config.gateway_base_url,
        usize::try_from(config.rate_limit_per_minute)?,
        store,
    )?;
    let address: SocketAddr = std::env::var("BUSINESS_ACTION_BIND")
        .unwrap_or_else(|_| "127.0.0.1:3012".into())
        .parse()?;
    tracing::info!(%address, mode = "acceptance", "Business Action service listening");
    let listener = tokio::net::TcpListener::bind(address).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
