//! HTTP-слой: роуты API, вебхуки, статика miniapp.

pub mod api;
pub mod jwt;
pub mod webhooks;

use std::sync::Arc;

use axum::routing::{delete, get, post, put};
use axum::Router;
use tower_http::services::{ServeDir, ServeFile};

use crate::state::AppState;

pub fn router(state: Arc<AppState>) -> Router {
    let index = format!("{}/index.html", state.cfg.static_dir);
    let static_svc = ServeDir::new(&state.cfg.static_dir).fallback(ServeFile::new(index));

    Router::new()
        .route("/webhook/telegram", post(webhooks::telegram))
        .route("/webhook/max", post(webhooks::max))
        .route("/api/auth", post(api::auth))
        .route("/api/me", get(api::me))
        .route("/api/settings", get(api::get_settings))
        .route("/api/settings", put(api::put_settings))
        .route("/api/credentials", post(api::credentials))
        .route("/api/unlink", post(api::unlink))
        .route("/api/account", delete(api::delete_account))
        .route("/healthz", get(api::healthz))
        .fallback_service(static_svc)
        .with_state(state)
}
