use axum::{routing::get, Router};

use crate::app::state::AppState;
use super::plugins::search_plugins;

/// Возвращает роутер с публичными эндпоинтами.
pub fn router(state: AppState) -> Router {
    Router::<AppState>::new()
        .route("/health", get(|| async { "ok" }))
        .route("/api/v1/plugins/search", get(search_plugins))
        .with_state(state)
}