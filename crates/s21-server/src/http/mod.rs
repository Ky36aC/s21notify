//! HTTP-слой: роуты API, вебхуки, статика miniapp.

pub mod api;
pub mod jwt;
pub mod webhooks;

use std::sync::Arc;

use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::Router;
use tower_http::services::{ServeDir, ServeFile};

use crate::state::AppState;

/// miniapp/dist, встроенный в бинарь (самодостаточный .exe). Пустой, если
/// trunk не собирал miniapp перед сборкой — тогда работает отдача с диска.
#[derive(rust_embed::RustEmbed)]
#[folder = "../../miniapp/dist"]
struct Assets;

pub fn router(state: Arc<AppState>) -> Router {
    let api = Router::new()
        .route("/webhook/telegram", post(webhooks::telegram))
        .route("/webhook/max", post(webhooks::max))
        .route("/api/auth", post(api::auth))
        .route("/api/me", get(api::me))
        .route("/api/settings", get(api::get_settings))
        .route("/api/settings", put(api::put_settings))
        .route("/api/credentials", post(api::credentials))
        .route("/api/unlink", post(api::unlink))
        .route("/api/account", delete(api::delete_account))
        .route("/healthz", get(api::healthz));

    // статика: если рядом лежит каталог static/ (серверный деплой) — отдаём с диска
    // (можно обновлять miniapp без пересборки); иначе — встроенную в бинарь.
    let static_index = std::path::Path::new(&state.cfg.static_dir).join("index.html");
    let with_static = if static_index.exists() {
        let index = format!("{}/index.html", state.cfg.static_dir);
        api.fallback_service(ServeDir::new(&state.cfg.static_dir).fallback(ServeFile::new(index)))
    } else {
        api.fallback(embedded_asset)
    };

    with_static.with_state(state)
}

/// Отдача встроенной статики; неизвестный путь → index.html (SPA-фолбэк).
async fn embedded_asset(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let lookup = if path.is_empty() { "index.html" } else { path };
    if let Some(file) = Assets::get(lookup) {
        return (
            [(header::CONTENT_TYPE, file.metadata.mimetype().to_string())],
            file.data.into_owned(),
        )
            .into_response();
    }
    match Assets::get("index.html") {
        Some(file) => (
            [(header::CONTENT_TYPE, "text/html; charset=utf-8".to_string())],
            file.data.into_owned(),
        )
            .into_response(),
        None => (
            StatusCode::NOT_FOUND,
            "miniapp не собран (нет встроенной статики)",
        )
            .into_response(),
    }
}
