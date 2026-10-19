use std::sync::Arc;


use axum::Router;
use sea_orm::{ConnectOptions, Database};
use tokio::net::TcpListener;
use tokio::signal;

use server::app::services::plugin_search_service::PluginSearchService;
use server::app::state::AppState;
use server::config::AppConfig;
use server::infra::db::ctx::AppCtx;
use server::present::http::routes;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cfg = AppConfig::from_env()?;
    cfg.init_tracing();

    tracing::info!(
        host = %cfg.server_host,
        port = cfg.server_port,
        max_conn = cfg.db_max_connections,
        "starting server"
    );

    let mut opt = ConnectOptions::new(cfg.database_url.clone());
    opt.max_connections(cfg.db_max_connections)
        .min_connections(1)
        .connect_timeout(cfg.db_connect_timeout)
        .sqlx_logging(false); // не логируем каждый SQL в info
    let db = Database::connect(opt).await?;
    tracing::info!("connected to database");


    let ctx = Arc::new(AppCtx::new(db));
    let service = PluginSearchService::new(ctx.clone(), ctx.clone());
    let state = AppState::new(service);
    let app: Router = routes::router(state);


    let listener = TcpListener::bind(cfg.bind_addr()).await?;
    tracing::info!("listening on {}", listener.local_addr()?);

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    tracing::warn!("server stopped");
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => tracing::warn!("Ctrl+C received, shutting down"),
        _ = terminate => tracing::warn!("SIGTERM received, shutting down"),
    }
}