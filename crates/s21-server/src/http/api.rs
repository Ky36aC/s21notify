//! REST API miniapp.

use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use s21_core::parse_remind_minutes;
use s21_platform::PlatformError;

use crate::db;
use crate::http::jwt::{self, Claims};
use crate::poll::PollCommand;
use crate::state::AppState;

type ApiResult = Result<Json<Value>, (StatusCode, Json<Value>)>;

fn err(code: StatusCode, msg: &str) -> (StatusCode, Json<Value>) {
    (code, Json(json!({"error": msg})))
}

fn authed(state: &AppState, headers: &HeaderMap) -> Result<Claims, (StatusCode, Json<Value>)> {
    jwt::from_headers(&state.cfg.jwt_secret, headers)
        .ok_or_else(|| err(StatusCode::UNAUTHORIZED, "нет или истёк токен"))
}

// ------------------------------------------------------------------ /api/auth

#[derive(Deserialize)]
pub struct AuthReq {
    pub messenger: String,
    pub init_data: String,
}

pub async fn auth(State(state): State<Arc<AppState>>, Json(req): Json<AuthReq>) -> ApiResult {
    // DEV-режим: "dev:<ext_id>" без подписи (только при DEV_FAKE_AUTH=1)
    let ext_user_id = if state.cfg.dev_fake_auth && req.init_data.starts_with("dev:") {
        req.init_data.trim_start_matches("dev:").to_string()
    } else {
        let adapter = state
            .adapter(&req.messenger)
            .ok_or_else(|| err(StatusCode::BAD_REQUEST, "неизвестный мессенджер"))?;
        adapter
            .verify_miniapp_auth(&req.init_data)
            .ok_or_else(|| err(StatusCode::UNAUTHORIZED, "подпись initData не прошла"))?
            .ext_user_id
    };

    let account = db::account_by_ext(&state.pool, &req.messenger, &ext_user_id)
        .await
        .map_err(internal)?;
    let uid = account.and_then(|a| a.user_id);
    let token = jwt::issue(&state.cfg.jwt_secret, &req.messenger, &ext_user_id, uid);
    Ok(Json(json!({"token": token, "registered": uid.is_some()})))
}

// -------------------------------------------------------------------- /api/me

pub async fn me(State(state): State<Arc<AppState>>, headers: HeaderMap) -> ApiResult {
    let claims = authed(&state, &headers)?;
    let Some(uid) = claims.uid else {
        return Ok(Json(json!({"registered": false})));
    };
    let Some(user) = db::user_by_id(&state.pool, uid).await.map_err(internal)? else {
        return Ok(Json(json!({"registered": false})));
    };
    let linked: Vec<Value> = db::all_accounts(&state.pool, uid)
        .await
        .map_err(internal)?
        .iter()
        .map(|a| {
            json!({
                "messenger": a.messenger,
                "username": a.username,
                "status": a.status,
                "this_one": a.messenger == claims.messenger && a.ext_user_id == claims.ext,
            })
        })
        .collect();
    Ok(Json(json!({
        "registered": true,
        "s21_login": user.s21_login,
        "token_status": user.token_status,
        "linked": linked,
        "last_poll_at": user.last_poll_at,
        "last_poll_error": user.last_poll_error,
    })))
}

// -------------------------------------------------------------- /api/settings

pub async fn get_settings(State(state): State<Arc<AppState>>, headers: HeaderMap) -> ApiResult {
    let claims = authed(&state, &headers)?;
    let uid = claims
        .uid
        .ok_or_else(|| err(StatusCode::FORBIDDEN, "сначала зарегистрируйся"))?;
    let s = db::get_settings(&state.pool, uid).await.map_err(internal)?;
    Ok(Json(json!({
        "remind_minutes": s.remind_minutes,
        "notify_bookings": s.notify_bookings,
        "notify_changes": s.notify_changes,
        "notify_reminders": s.notify_reminders,
        "notify_feed": s.notify_feed,
        "notify_deadlines": s.notify_deadlines,
        "notify_alarm": s.notify_alarm,
    })))
}

#[derive(Deserialize)]
pub struct SettingsReq {
    pub remind_minutes: String,
    pub notify_bookings: bool,
    pub notify_changes: bool,
    pub notify_reminders: bool,
    pub notify_feed: bool,
    pub notify_deadlines: bool,
    pub notify_alarm: bool,
}

pub async fn put_settings(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<SettingsReq>,
) -> ApiResult {
    let claims = authed(&state, &headers)?;
    let uid = claims
        .uid
        .ok_or_else(|| err(StatusCode::FORBIDDEN, "сначала зарегистрируйся"))?;
    if req.remind_minutes.len() > 100 {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "слишком длинный список порогов",
        ));
    }
    // нормализуем через тот же парсер, что использует watcher
    let normalized = parse_remind_minutes(&req.remind_minutes)
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    let s = s21_core::UserSettings {
        remind_minutes: normalized.clone(),
        notify_bookings: req.notify_bookings,
        notify_changes: req.notify_changes,
        notify_reminders: req.notify_reminders,
        notify_feed: req.notify_feed,
        notify_deadlines: req.notify_deadlines,
        notify_alarm: req.notify_alarm,
    };
    db::save_settings(&state.pool, uid, &s)
        .await
        .map_err(internal)?;
    Ok(Json(json!({"ok": true, "remind_minutes": normalized})))
}

// ----------------------------------------------------------- /api/credentials

#[derive(Deserialize)]
pub struct CredentialsReq {
    pub login: String,
    pub password: String,
}

pub async fn credentials(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<CredentialsReq>,
) -> ApiResult {
    let claims = authed(&state, &headers)?;
    let login = req
        .login
        .split('@')
        .next()
        .unwrap_or("")
        .trim()
        .to_lowercase();
    if login.is_empty() || req.password.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "логин и пароль обязательны"));
    }

    // пароль используется ровно здесь и нигде не сохраняется
    let tokens = state
        .sessions
        .auth
        .login_password(&login, &req.password)
        .await
        .map_err(|e| match e {
            PlatformError::BadCredentials => err(
                StatusCode::UNAUTHORIZED,
                "Неверный логин или пароль. Логин — только короткий ник, без @student.21-school.ru",
            ),
            other => {
                tracing::warn!("login_password: {other}");
                err(
                    StatusCode::BAD_GATEWAY,
                    "платформа недоступна, попробуй позже",
                )
            }
        })?;
    let offline = tokens
        .refresh_token
        .ok_or_else(|| err(StatusCode::BAD_GATEWAY, "платформа не выдала offline-токен"))?;
    let enc = state.cipher.encrypt(&offline);

    let uid = match db::user_by_login(&state.pool, &login)
        .await
        .map_err(internal)?
    {
        Some(user) => {
            // перелогин существующего аккаунта: свежий токен, статус ok
            db::set_offline_token(&state.pool, user.id, &enc)
                .await
                .map_err(internal)?;
            state.sessions.evict(user.id);
            user.id
        }
        None => db::create_user(&state.pool, &login, &enc)
            .await
            .map_err(internal)?,
    };

    // привязываем текущий мессенджер (chat_id из /start, фолбэк — ext id)
    let chat_fallback = db::account_by_ext(&state.pool, &claims.messenger, &claims.ext)
        .await
        .map_err(internal)?
        .map(|a| a.chat_id)
        .unwrap_or_else(|| claims.ext.clone());
    db::attach_user(
        &state.pool,
        uid,
        &claims.messenger,
        &claims.ext,
        &chat_fallback,
        None,
    )
    .await
    .map_err(internal)?;

    let _ = state.poll_tx.send(PollCommand::Start(uid));
    let token = jwt::issue(
        &state.cfg.jwt_secret,
        &claims.messenger,
        &claims.ext,
        Some(uid),
    );
    Ok(Json(
        json!({"ok": true, "token": token, "s21_login": login}),
    ))
}

// -------------------------------------------------- /api/unlink, /api/account

pub async fn unlink(State(state): State<Arc<AppState>>, headers: HeaderMap) -> ApiResult {
    let claims = authed(&state, &headers)?;
    db::unlink_account(&state.pool, &claims.messenger, &claims.ext)
        .await
        .map_err(internal)?;
    Ok(Json(json!({"ok": true})))
}

pub async fn delete_account(State(state): State<Arc<AppState>>, headers: HeaderMap) -> ApiResult {
    let claims = authed(&state, &headers)?;
    let uid = claims
        .uid
        .ok_or_else(|| err(StatusCode::FORBIDDEN, "аккаунт не зарегистрирован"))?;
    db::delete_user(&state.pool, uid).await.map_err(internal)?;
    state.sessions.evict(uid);
    let _ = state.poll_tx.send(PollCommand::Stop(uid));
    Ok(Json(json!({"ok": true})))
}

// ------------------------------------------------------------------ /healthz

pub async fn healthz(State(state): State<Arc<AppState>>) -> ApiResult {
    let users = db::count_users(&state.pool).await.map_err(internal)?;
    Ok(Json(json!({
        "ok": true,
        "version": env!("CARGO_PKG_VERSION"),
        "users": users,
        "messengers": state.adapters.keys().collect::<Vec<_>>(),
    })))
}

fn internal<E: std::fmt::Display>(e: E) -> (StatusCode, Json<Value>) {
    tracing::error!("api: {e}");
    err(StatusCode::INTERNAL_SERVER_ERROR, "внутренняя ошибка")
}
