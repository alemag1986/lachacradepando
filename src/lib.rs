pub mod ai;
pub mod config;
pub mod db;
pub mod domain;
pub mod error;
pub mod telegram;
pub mod web;

use std::sync::Arc;
use axum::{Router, routing::get};
use sqlx::SqlitePool;
use config::Config;

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub config: Arc<Config>,
    pub http: reqwest::Client,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .merge(web::router())
        .merge(telegram::router())
        .layer(axum::extract::DefaultBodyLimit::max(1024 * 1024))
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .with_state(state)
}

async fn health(axum::extract::State(state): axum::extract::State<AppState>) -> axum::http::StatusCode {
    match sqlx::query("SELECT 1").execute(&state.pool).await {
        Ok(_) => axum::http::StatusCode::OK,
        Err(_) => axum::http::StatusCode::SERVICE_UNAVAILABLE,
    }
}
pub mod db;
pub mod domain;
