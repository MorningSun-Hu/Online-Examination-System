pub mod config;
pub mod db;
pub mod error;
pub mod auth;
pub mod lan;

use axum::middleware;
use axum::routing::{get, post, put};
use axum::Router;
use sqlx::SqlitePool;

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub port: u16,
}

pub fn app(state: AppState) -> Router {
    let protected = Router::new()
        .route("/api/teacher/password", put(auth::change_password))
        .route("/api/teacher/lan-addresses", get(lan::lan_addresses))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth::teacher_auth,
        ));
    Router::new()
        .route("/api/health", get(health))
        .route("/api/teacher/setup", post(auth::setup))
        .route("/api/teacher/login", post(auth::login))
        .merge(protected)
        .with_state(state)
}

async fn health() -> &'static str {
    "ok"
}
