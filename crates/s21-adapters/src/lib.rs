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

    /// HTML-сообщение с опциональной кнопкой.
    async fn send_message(
        &self,
        chat_id: &str,
        html: &str,
        button: Option<MsgButton<'_>>,
    ) -> SendResult;

    /// Сырой webhook-JSON → нормализованный апдейт (None = игнорировать).
    fn parse_update(&self, raw: &Value) -> Option<IncomingUpdate>;

    /// Проверка подписи initData miniapp.
    fn verify_miniapp_auth(&self, init_data: &str) -> Option<MiniappUser>;

    /// Ответ на нажатие кнопки: тост + подтверждающая правка клавиатуры (где умеем).
    async fn ack_callback(&self, upd: &IncomingUpdate, toast: &str);

    /// Регистрация вебхука (URL уже содержит секрет, если он в query).
    async fn set_webhook(&self, url: &str) -> anyhow::Result<()>;

    /// Long polling: забрать пачку апдейтов начиная с `cursor` (offset/marker),
    /// ждать до `timeout_s` секунд. По умолчанию не поддерживается.
    async fn poll(&self, _cursor: Option<String>, _timeout_s: u64) -> anyhow::Result<PollBatch> {
        anyhow::bail!("long polling не поддерживается адаптером {}", self.id())
    }

    /// Снять вебхук/подписку — обязательно перед long polling (Telegram не отдаёт
    /// getUpdates при активном вебхуке). По умолчанию no-op.
    async fn delete_webhook(&self) -> anyhow::Result<()> {
        Ok(())
    }
}
