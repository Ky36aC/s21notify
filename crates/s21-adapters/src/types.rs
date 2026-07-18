//! Общие типы адаптеров (контракт повторяет боевой NotifyBot).

/// Причина неудачной отправки — по ней Sender меняет статус привязки.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailReason {
    /// Пользователь заблокировал бота
    Blocked,
    /// Диалог не начат (нет /start)
    NotStarted,
    /// Аккаунт пользователя удалён
    Deactivated,
    /// Не хватает прав писать
    Privacy,
    /// Rate limit — повторить через retry_after
    Flood,
    Unknown,
}

impl FailReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Blocked => "blocked",
            Self::NotStarted => "not_started",
            Self::Deactivated => "deactivated",
            Self::Privacy => "privacy",
            Self::Flood => "flood",
            Self::Unknown => "unknown",
        }
    }
}

/// Кнопка под сообщением.
#[derive(Debug, Clone, Copy)]
pub enum MsgButton<'a> {
    /// «✅ Я за компом» с callback-payload ack:<bid>
    Ack(&'a str),
    /// Открыть miniapp (TG — web_app, MAX — link-фолбэк)
    Miniapp { text: &'a str, url: &'a str },
}

#[derive(Debug, Clone)]
pub struct SendResult {
    pub ok: bool,
    pub fail_reason: Option<FailReason>,
    pub error_text: Option<String>,
    /// Секунды из Retry-After при Flood
    pub retry_after: Option<u64>,
}

impl SendResult {
    pub fn success() -> Self {
        Self {
            ok: true,
            fail_reason: None,
            error_text: None,
            retry_after: None,
        }
    }

    pub fn fail(reason: FailReason, error_text: impl Into<String>) -> Self {
        Self {
            ok: false,
            fail_reason: Some(reason),
            error_text: Some(error_text.into()),
            retry_after: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateKind {
    /// /start или разблокировка — привязка активна
    Started,
    /// Блок/остановка бота — привязка гаснет
    StoppedOrBlocked,
    /// Нажатие inline-кнопки (payload: ack:<bid> / noop)
    Callback,
    /// Текстовая команда (/reviews, /agenda, …)
    Text,
    /// Прочая активность — игнорируется
    Activity,
}

/// Нормализованный входящий апдейт любого мессенджера.
#[derive(Debug, Clone)]
pub struct IncomingUpdate {
    pub kind: UpdateKind,
    pub ext_user_id: String,
    pub chat_id: String,
    pub username: Option<String>,
    /// Текст сообщения (для Text/Started)
    pub text: Option<String>,
    /// payload кнопки (для Callback)
    pub payload: Option<String>,
    /// id callback-запроса — для ack_callback
    pub callback_id: Option<String>,
    /// id сообщения с кнопкой — для правки клавиатуры
    pub message_id: Option<String>,
}

impl IncomingUpdate {
    pub fn new(kind: UpdateKind, ext_user_id: String, chat_id: String) -> Self {
        Self {
            kind,
            ext_user_id,
            chat_id,
            username: None,
            text: None,
            payload: None,
            callback_id: None,
            message_id: None,
        }
    }
}

/// Пачка апдейтов, полученная long polling'ом.
#[derive(Debug, Clone, Default)]
pub struct PollBatch {
    pub updates: Vec<IncomingUpdate>,
    /// Курсор для следующего запроса (offset у Telegram, marker у MAX).
    /// None — оставить прежний (пустая пачка / нет прогресса).
    pub next_cursor: Option<String>,
}

/// Проверенный пользователь miniapp (из initData / launch-params).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MiniappUser {
    pub ext_user_id: String,
    pub username: Option<String>,
    pub full_name: Option<String>,
}
