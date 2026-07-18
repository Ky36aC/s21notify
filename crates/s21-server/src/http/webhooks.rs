//! Вебхуки мессенджеров: проверка секрета, мгновенный 200, обработка в spawn.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde_json::Value;

use s21_adapters::{IncomingUpdate, MsgButton, UpdateKind};

use crate::state::AppState;
use crate::{commands, db, texts};

pub async fn telegram(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(raw): Json<Value>,
) -> StatusCode {
    let got = headers
        .get("x-telegram-bot-api-secret-token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    if state.cfg.tg_webhook_secret.is_empty() || got != state.cfg.tg_webhook_secret {
        return StatusCode::FORBIDDEN;
    }
    dispatch(state, "telegram", raw);
    StatusCode::OK
}

pub async fn max(
    State(state): State<Arc<AppState>>,
    Query(q): Query<HashMap<String, String>>,
    Json(raw): Json<Value>,
) -> StatusCode {
    let got = q.get("s").map(String::as_str).unwrap_or_default();
    if state.cfg.max_webhook_secret.is_empty() || got != state.cfg.max_webhook_secret {
        return StatusCode::FORBIDDEN;
    }
    dispatch(state, "max", raw);
    StatusCode::OK
}

/// 200 отдаём сразу, работа — в фоне (иначе MAX через 8 ч недоступности снимет вебхук).
fn dispatch(state: Arc<AppState>, messenger: &'static str, raw: Value) {
    tokio::spawn(async move {
        if let Err(e) = handle_raw(&state, messenger, &raw).await {
            tracing::warn!("webhook {messenger}: {e}");
        }
    });
}

async fn handle_raw(state: &AppState, messenger: &str, raw: &Value) -> anyhow::Result<()> {
    let Some(adapter) = state.adapter(messenger) else {
        return Ok(());
    };
    let Some(upd) = adapter.parse_update(raw) else {
        return Ok(());
    };
    if upd.ext_user_id.is_empty() {
        return Ok(());
    }
    handle_update(state, messenger, &upd).await
}

pub async fn handle_update(
    state: &AppState,
    messenger: &str,
    upd: &IncomingUpdate,
) -> anyhow::Result<()> {
    let adapter = state
        .adapter(messenger)
        .ok_or_else(|| anyhow::anyhow!("нет адаптера {messenger}"))?;

    match upd.kind {
        UpdateKind::Started => {
            db::remember_chat(
                &state.pool,
                messenger,
                &upd.ext_user_id,
                &upd.chat_id,
                upd.username.as_deref(),
            )
            .await?;
            let account = db::account_by_ext(&state.pool, messenger, &upd.ext_user_id).await?;
            let user = match account.and_then(|a| a.user_id) {
                Some(uid) => db::user_by_id(&state.pool, uid).await?,
                None => None,
            };
            let html = match &user {
                Some(u) => texts::welcome_registered(&u.s21_login),
                None => texts::welcome_new(),
            };
            state.throttle.acquire(messenger, &upd.chat_id).await;
            send_with_miniapp(state, adapter.as_ref(), &upd.chat_id, &html).await;
        }

        UpdateKind::StoppedOrBlocked => {
            db::set_account_status(&state.pool, messenger, &upd.ext_user_id, "blocked").await?;
        }

        UpdateKind::Callback => {
            let payload = upd.payload.as_deref().unwrap_or_default();
            if let Some(bid) = payload.strip_prefix("ack:") {
                let account = db::account_by_ext(&state.pool, messenger, &upd.ext_user_id).await?;
                if let Some(uid) = account.and_then(|a| a.user_id) {
                    db::ack_booking(&state.pool, uid, bid).await?;
                    adapter.ack_callback(upd, texts::ACK_TOAST).await;
                    return Ok(());
                }
            }
            // noop и чужие payload — просто закрыть «часики»
            adapter.ack_callback(upd, "").await;
        }

        UpdateKind::Text => {
            let account = db::account_by_ext(&state.pool, messenger, &upd.ext_user_id).await?;
            let user = match account.and_then(|a| a.user_id) {
                Some(uid) => db::user_by_id(&state.pool, uid).await?,
                None => None,
            };
            let text = upd.text.as_deref().unwrap_or_default();
            match user {
                Some(u) => {
                    if let Some(reply) =
                        commands::handle_command(state, u.id, &u.s21_login, text).await
                    {
                        state.throttle.acquire(messenger, &upd.chat_id).await;
                        adapter.send_message(&upd.chat_id, &reply, None).await;
                    }
                }
                None => {
                    state.throttle.acquire(messenger, &upd.chat_id).await;
                    send_with_miniapp(
                        state,
                        adapter.as_ref(),
                        &upd.chat_id,
                        &texts::not_registered(),
                    )
                    .await;
                }
            }
        }

        UpdateKind::Activity => {}
    }
    Ok(())
}

/// Приглашение в miniapp. В server-режиме — кнопка web_app; в local-режиме
/// Telegram не примет кнопку на http://localhost, поэтому URL кладём в текст.
async fn send_with_miniapp(
    state: &AppState,
    adapter: &dyn s21_adapters::MessengerAdapter,
    chat_id: &str,
    html: &str,
) {
    let url = state.miniapp_url();
    if state.cfg.app_mode == crate::config::AppMode::Local {
        let msg = format!("{html}\n\nОткрой настройки в браузере: {url}");
        adapter.send_message(chat_id, &msg, None).await;
    } else {
        adapter
            .send_message(
                chat_id,
                html,
                Some(MsgButton::Miniapp {
                    text: texts::MINIAPP_BUTTON,
                    url: &url,
                }),
            )
            .await;
    }
}
