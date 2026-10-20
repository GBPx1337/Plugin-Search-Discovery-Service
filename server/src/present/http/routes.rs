use axum::routing::{get, put};
use axum::Router;
use tower_http::cors::CorsLayer;

use super::plugins::{delete_plugin, index_plugin, search_plugins};
use crate::app::state::AppState;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/api/v1/plugins/search", get(search_plugins))
        .route(
            "/api/v1/index/plugins/:id", // `:id` тк axum 0.7
            put(index_plugin).delete(delete_plugin),
        )
        .layer(CorsLayer::permissive())
        .with_state(state)
}