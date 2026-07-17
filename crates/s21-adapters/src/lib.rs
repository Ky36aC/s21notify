//! Адаптеры мессенджеров: единый trait MessengerAdapter + реализации
//! для Telegram (teloxide-core) и MAX (собственный тонкий клиент).

mod max;
mod telegram;
mod throttle;
mod types;
pub mod webappdata;

use async_trait::async_trait;
use serde_json::Value;

pub use max::{parse_max_update, MaxAdapter, DEFAULT_BASE as MAX_DEFAULT_BASE};
pub use telegram::{parse_telegram_update, TelegramAdapter};
pub use throttle::Throttle;
pub use types::*;

#[async_trait]
pub trait MessengerAdapter: Send + Sync {
    /// 'telegram' | 'max' — совпадает с messenger_accounts.messenger
    fn id(&self) -> &'static str;

    /// HTML-сообщение; ack_payload = приложить кнопку «✅ Я за компом».
    async fn send_message(
        &self,
        chat_id: &str,
        html: &str,
        ack_payload: Option<&str>,
    ) -> SendResult;

    /// Сырой webhook-JSON → нормализованный апдейт (None = игнорировать).
    fn parse_update(&self, raw: &Value) -> Option<IncomingUpdate>;

    /// Проверка подписи initData miniapp.
    fn verify_miniapp_auth(&self, init_data: &str) -> Option<MiniappUser>;

    /// Ответ на нажатие кнопки: тост + подтверждающая правка клавиатуры (где умеем).
    async fn ack_callback(&self, upd: &IncomingUpdate, toast: &str);

    /// Регистрация вебхука (URL уже содержит секрет, если он в query).
    async fn set_webhook(&self, url: &str) -> anyhow::Result<()>;
}
