//! Long polling мессенджеров: одна таска на мессенджер тянет апдейты
//! (getUpdates у Telegram, GET /updates у MAX) и передаёт их в общий
//! http::webhooks::handle_update — тот же путь, что и у вебхуков.
//!
//! Работает без домена и входящих портов: бот сам ходит к API. Это основной
//! режим (в РФ вебхуки к Telegram ненадёжны, а локально домена нет вовсе).

use std::sync::Arc;
use std::time::Duration;

use crate::http::webhooks::handle_update;
use crate::state::AppState;

/// Сколько сервер API держит long-poll соединение открытым, ожидая апдейт.
const POLL_TIMEOUT_SEC: u64 = 25;
/// Пауза после ошибки, чтобы не молотить API в цикле.
const ERROR_BACKOFF_SEC: u64 = 5;

pub async fn run(state: Arc<AppState>, messenger: String) {
    let Some(adapter) = state.adapter(&messenger) else {
        return;
    };
    // long polling и вебхук взаимоисключающи (TG вернёт 409 при активном вебхуке) —
    // снимаем подписку перед стартом
    if let Err(e) = adapter.delete_webhook().await {
        tracing::warn!("{messenger}: снятие вебхука перед polling: {e}");
    }
    tracing::info!("{messenger}: long polling запущен");

    let mut cursor: Option<String> = None;
    loop {
        match adapter.poll(cursor.clone(), POLL_TIMEOUT_SEC).await {
            Ok(batch) => {
                cursor = batch.next_cursor.or(cursor);
                for upd in &batch.updates {
                    if let Err(e) = handle_update(&state, &messenger, upd).await {
                        tracing::warn!("polling {messenger}: обработка апдейта: {e}");
                    }
                }
            }
            Err(e) => {
                tracing::warn!("polling {messenger}: {e}; пауза {ERROR_BACKOFF_SEC} с");
                tokio::time::sleep(Duration::from_secs(ERROR_BACKOFF_SEC)).await;
            }
        }
    }
}
