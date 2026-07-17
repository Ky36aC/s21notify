//! Единый слой отправки: во все активные привязки пользователя, с троттлингом,
//! журналом deliveries и обновлением статуса привязки по ошибкам.

use s21_adapters::{FailReason, MsgButton};
use s21_core::strip_html;

use crate::db;
use crate::state::AppState;

/// Шлёт HTML во все активные привязки. ack_bid = кнопка «✅ Я за компом».
pub async fn send_to_user(
    state: &AppState,
    user_id: i64,
    kind: &str,
    html: &str,
    ack_bid: Option<&str>,
) {
    let accounts = match db::active_accounts(&state.pool, user_id).await {
        Ok(a) => a,
        Err(e) => {
            tracing::error!("sender: привязки uid={user_id}: {e}");
            return;
        }
    };
    let preview = strip_html(html).lines().next().unwrap_or("").to_string();

    for acc in accounts {
        let Some(adapter) = state.adapter(&acc.messenger) else {
            continue;
        };
        let payload = ack_bid.map(s21_core::ack_payload);

        state.throttle.acquire(&acc.messenger, &acc.chat_id).await;
        let mut res = adapter
            .send_message(&acc.chat_id, html, payload.as_deref().map(MsgButton::Ack))
            .await;

        // один повтор после flood-паузы
        if !res.ok && res.fail_reason == Some(FailReason::Flood) {
            let wait = res.retry_after.unwrap_or(3).min(60);
            tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
            state.throttle.acquire(&acc.messenger, &acc.chat_id).await;
            res = adapter
                .send_message(&acc.chat_id, html, payload.as_deref().map(MsgButton::Ack))
                .await;
        }

        match res.fail_reason {
            Some(FailReason::Blocked)
            | Some(FailReason::Deactivated)
            | Some(FailReason::NotStarted) => {
                let status = res.fail_reason.unwrap().as_str();
                let _ =
                    db::set_account_status(&state.pool, &acc.messenger, &acc.ext_user_id, status)
                        .await;
            }
            _ => {}
        }
        let _ = db::log_delivery(
            &state.pool,
            Some(user_id),
            &acc.messenger,
            kind,
            res.ok,
            res.fail_reason.map(FailReason::as_str),
            &preview,
        )
        .await;
        if !res.ok {
            tracing::warn!(
                "sender: uid={user_id} {} {}: {:?} {}",
                acc.messenger,
                kind,
                res.fail_reason,
                res.error_text.as_deref().unwrap_or("")
            );
        }
    }
}

/// Сообщение «нужен перелогин» с кнопкой miniapp во все привязки.
pub async fn send_relogin_notice(state: &AppState, user_id: i64) {
    let accounts = db::active_accounts(&state.pool, user_id)
        .await
        .unwrap_or_default();
    let html = crate::texts::relogin_needed();
    let url = state.miniapp_url();
    for acc in accounts {
        let Some(adapter) = state.adapter(&acc.messenger) else {
            continue;
        };
        state.throttle.acquire(&acc.messenger, &acc.chat_id).await;
        adapter
            .send_message(
                &acc.chat_id,
                &html,
                Some(MsgButton::Miniapp {
                    text: crate::texts::MINIAPP_BUTTON,
                    url: &url,
                }),
            )
            .await;
        let _ = db::log_delivery(
            &state.pool,
            Some(user_id),
            &acc.messenger,
            "relogin",
            true,
            None,
            "нужен перелогин",
        )
        .await;
    }
}

/// Алерт админу (сломанный whitelist задевает всех пользователей).
pub async fn send_admin_alert(state: &AppState, text: &str) {
    let (Some(chat), Some(adapter)) = (
        state.cfg.admin_tg_chat_id.as_deref(),
        state.adapter("telegram"),
    ) else {
        tracing::error!("админ-алерт (некому слать): {text}");
        return;
    };
    state.throttle.acquire("telegram", chat).await;
    adapter
        .send_message(
            chat,
            &format!("🛑 <b>s21notify</b>\n{}", s21_core::esc(text)),
            None,
        )
        .await;
}
