use std::sync::Arc;

use axum::Router;
use sea_orm::Database;
use tokio::net::TcpListener;

use server::app::services::plugin_search_service::PluginSearchService;
use server::app::state::AppState;
use server::dom::plugins::repo::PluginRepo;
use server::infra::db::ctx::AppCtx;
use server::present::http::routes;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // .env ДО tracing, чтобы RUST_LOG подхватился из него.
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let database_url = std::env::var("DATABASE_URL")
        .map_err(|_| anyhow::anyhow!("DATABASE_URL is not set"))?;

    let db = Database::connect(&database_url).await?;
    let ctx = Arc::new(AppCtx::new(db));

    let repo: Arc<dyn PluginRepo> = ctx.clone();
    let service = PluginSearchService::new(repo, ctx.clone());
    let state = AppState::new(service);

    let app: Router = routes::router(state);

    let listener = TcpListener::bind("0.0.0.0:8080").await?;
    tracing::info!("listening on {}", listener.local_addr()?);
    axum::serve(listener, app).await?;

    Ok(())
}