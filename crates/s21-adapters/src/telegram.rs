//! Адаптер Telegram поверх teloxide-core (только типизированные вызовы,
//! без диспетчера). Апдейты приходят вебхуком и разбираются вручную —
//! классификация и контракт повторяют боевой NotifyBot.

use async_trait::async_trait;
use serde_json::Value;
use teloxide_core::payloads::setters::*;
use teloxide_core::prelude::Requester;
use teloxide_core::types::{
    AllowedUpdate, ChatId, InlineKeyboardButton, InlineKeyboardMarkup, LinkPreviewOptions,
    MessageId, ParseMode, Recipient,
};
use teloxide_core::{ApiError, Bot, RequestError};

use crate::types::*;
use crate::webappdata;
use crate::MessengerAdapter;

/// Окно anti-replay для initData Telegram.
const AUTH_MAX_AGE_SECONDS: i64 = 300;

pub struct TelegramAdapter {
    bot: Bot,
    bot_token: String,
    webhook_secret: String,
}

impl TelegramAdapter {
    pub fn new(bot_token: &str, webhook_secret: &str) -> Self {
        Self {
            bot: Bot::new(bot_token),
            bot_token: bot_token.to_string(),
            webhook_secret: webhook_secret.to_string(),
        }
    }

    fn recipient(chat_id: &str) -> Recipient {
        match chat_id.parse::<i64>() {
            Ok(id) => Recipient::Id(ChatId(id)),
            Err(_) => Recipient::ChannelUsername(chat_id.to_string()),
        }
    }
}

/// Классификация 403/400 по тексту ошибки (боевой опыт NotifyBot —
/// не схлопывать всё в unknown).
pub fn classify_text(msg: &str, default: FailReason) -> FailReason {
    let low = msg.to_lowercase();
    if low.contains("bot was blocked") || low.contains("blocked by the user") {
        FailReason::Blocked
    } else if low.contains("user is deactivated") {
        FailReason::Deactivated
    } else if low.contains("chat not found")
        || low.contains("bot can't initiate")
        || low.contains("not started")
    {
        FailReason::NotStarted
    } else if low.contains("have no rights") || low.contains("not enough rights") {
        FailReason::Privacy
    } else {
        default
    }
}

fn classify_request_error(err: &RequestError) -> SendResult {
    match err {
        RequestError::RetryAfter(secs) => SendResult {
            ok: false,
            fail_reason: Some(FailReason::Flood),
            error_text: Some(err.to_string()),
            retry_after: Some(secs.seconds() as u64),
        },
        RequestError::Api(api) => {
            let reason = match api {
                ApiError::BotBlocked => FailReason::Blocked,
                ApiError::UserDeactivated => FailReason::Deactivated,
                ApiError::ChatNotFound
                | ApiError::CantInitiateConversation
                | ApiError::CantTalkWithBots => FailReason::NotStarted,
                ApiError::NotEnoughRightsToPostMessages => FailReason::Privacy,
                other => classify_text(&other.to_string(), FailReason::Unknown),
            };
            SendResult::fail(reason, api.to_string())
        }
        other => SendResult::fail(FailReason::Unknown, other.to_string()),
    }
}

fn button_markup(button: MsgButton<'_>) -> Option<InlineKeyboardMarkup> {
    let btn = match button {
        MsgButton::Ack(payload) => {
            InlineKeyboardButton::callback(s21_core::ACK_BUTTON_TEXT, payload)
        }
        MsgButton::Miniapp { text, url } => InlineKeyboardButton::web_app(
            text,
            teloxide_core::types::WebAppInfo {
                url: url.parse().ok()?,
            },
        ),
    };
    Some(InlineKeyboardMarkup::new([[btn]]))
}

fn confirmed_markup() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new([[InlineKeyboardButton::callback("✅ Подтверждено", "noop")]])
}

fn no_preview() -> LinkPreviewOptions {
    LinkPreviewOptions {
        is_disabled: true,
        url: None,
        prefer_small_media: false,
        prefer_large_media: false,
        show_above_text: false,
    }
}

#[async_trait]
impl MessengerAdapter for TelegramAdapter {
    fn id(&self) -> &'static str {
        "telegram"
    }

    async fn send_message(
        &self,
        chat_id: &str,
        html: &str,
        button: Option<MsgButton<'_>>,
    ) -> SendResult {
        let mut req = self
            .bot
            .send_message(Self::recipient(chat_id), html)
            .parse_mode(ParseMode::Html)
            .link_preview_options(no_preview());
        if let Some(markup) = button.and_then(button_markup) {
            req = req.reply_markup(markup);
        }
        match req.await {
            Ok(_) => SendResult::success(),
            Err(e) => classify_request_error(&e),
        }
    }

    fn parse_update(&self, raw: &Value) -> Option<IncomingUpdate> {
        parse_telegram_update(raw)
    }

    fn verify_miniapp_auth(&self, init_data: &str) -> Option<MiniappUser> {
        webappdata::verify(
            init_data,
            &self.bot_token,
            AUTH_MAX_AGE_SECONDS,
            chrono_now(),
        )
    }

    /// Ответ на callback: тост + замена кнопки на «✅ Подтверждено».
    async fn ack_callback(&self, upd: &IncomingUpdate, toast: &str) {
        if let Some(cb_id) = &upd.callback_id {
            let _ = self
                .bot
                .answer_callback_query(teloxide_core::types::CallbackQueryId(cb_id.clone()))
                .text(toast)
                .await;
        }
        if let Some(mid) = upd.message_id.as_ref().and_then(|m| m.parse::<i32>().ok()) {
            let _ = self
                .bot
                .edit_message_reply_markup(Self::recipient(&upd.chat_id), MessageId(mid))
                .reply_markup(confirmed_markup())
                .await;
        }
    }

    async fn set_webhook(&self, url: &str) -> anyhow::Result<()> {
        let url: reqwest::Url = url.parse()?;
        self.bot
            .set_webhook(url)
            .allowed_updates(vec![
                AllowedUpdate::Message,
                AllowedUpdate::CallbackQuery,
                AllowedUpdate::MyChatMember,
            ])
            .secret_token(self.webhook_secret.clone())
            .await?;
        Ok(())
    }
}

fn chrono_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn v_str(v: &Value, ptr: &str) -> String {
    match v.pointer(ptr) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

/// Разбор webhook-апдейта Telegram (сырой JSON) → IncomingUpdate.
pub fn parse_telegram_update(raw: &Value) -> Option<IncomingUpdate> {
    // --- callback_query ---
    if let Some(cb) = raw.get("callback_query") {
        let ext = v_str(cb, "/from/id");
        let chat = {
            let c = v_str(cb, "/message/chat/id");
            if c.is_empty() {
                ext.clone()
            } else {
                c
            }
        };
        let mut upd = IncomingUpdate::new(UpdateKind::Callback, ext, chat);
        upd.username = cb
            .pointer("/from/username")
            .and_then(Value::as_str)
            .map(String::from);
        upd.payload = cb.get("data").and_then(Value::as_str).map(String::from);
        upd.callback_id = cb.get("id").and_then(Value::as_str).map(String::from);
        let mid = v_str(cb, "/message/message_id");
        upd.message_id = if mid.is_empty() { None } else { Some(mid) };
        return Some(upd);
    }

    // --- my_chat_member: проактивная детекция блока/разблока ---
    if let Some(mcm) = raw.get("my_chat_member") {
        let status = v_str(mcm, "/new_chat_member/status");
        let kind = match status.as_str() {
            "kicked" | "left" => UpdateKind::StoppedOrBlocked,
            "member" => UpdateKind::Started,
            _ => UpdateKind::Activity,
        };
        let ext = {
            let f = v_str(mcm, "/from/id");
            if f.is_empty() {
                v_str(mcm, "/chat/id")
            } else {
                f
            }
        };
        let chat = {
            let c = v_str(mcm, "/chat/id");
            if c.is_empty() {
                ext.clone()
            } else {
                c
            }
        };
        let mut upd = IncomingUpdate::new(kind, ext, chat);
        upd.username = mcm
            .pointer("/from/username")
            .and_then(Value::as_str)
            .map(String::from);
        return Some(upd);
    }

    // --- message ---
    let msg = raw.get("message").or_else(|| raw.get("edited_message"))?;
    let ext = v_str(msg, "/from/id");
    let chat = {
        let c = v_str(msg, "/chat/id");
        if c.is_empty() {
            ext.clone()
        } else {
            c
        }
    };
    let text = msg
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    let kind = if text == "/start" || text.starts_with("/start@") || text.starts_with("/start ") {
        UpdateKind::Started
    } else if text.starts_with('/') {
        UpdateKind::Text
    } else {
        UpdateKind::Activity
    };
    let mut upd = IncomingUpdate::new(kind, ext, chat);
    upd.username = msg
        .pointer("/from/username")
        .and_then(Value::as_str)
        .map(String::from);
    upd.text = Some(text);
    Some(upd)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn старт_команда_и_болтовня() {
        let start = json!({"update_id":1,"message":{"message_id":10,
            "from":{"id":456,"username":"floriato"},"chat":{"id":456},"text":"/start"}});
        let u = parse_telegram_update(&start).unwrap();
        assert_eq!(u.kind, UpdateKind::Started);
        assert_eq!(u.ext_user_id, "456");
        assert_eq!(u.chat_id, "456");
        assert_eq!(u.username.as_deref(), Some("floriato"));

        let cmd = json!({"message":{"from":{"id":456},"chat":{"id":456},"text":"/reviews"}});
        assert_eq!(parse_telegram_update(&cmd).unwrap().kind, UpdateKind::Text);

        let chat = json!({"message":{"from":{"id":456},"chat":{"id":456},"text":"привет"}});
        assert_eq!(
            parse_telegram_update(&chat).unwrap().kind,
            UpdateKind::Activity
        );
    }

    #[test]
    fn callback_с_id_сообщения() {
        let cb = json!({"callback_query":{"id":"cb42","data":"ack:b1",
            "from":{"id":456,"username":"floriato"},
            "message":{"message_id":77,"chat":{"id":456}}}});
        let u = parse_telegram_update(&cb).unwrap();
        assert_eq!(u.kind, UpdateKind::Callback);
        assert_eq!(u.payload.as_deref(), Some("ack:b1"));
        assert_eq!(u.callback_id.as_deref(), Some("cb42"));
        assert_eq!(u.message_id.as_deref(), Some("77"));
    }

    #[test]
    fn my_chat_member_блок_и_разблок() {
        let kicked = json!({"my_chat_member":{"from":{"id":456},"chat":{"id":456},
            "new_chat_member":{"status":"kicked"}}});
        assert_eq!(
            parse_telegram_update(&kicked).unwrap().kind,
            UpdateKind::StoppedOrBlocked
        );
        let member = json!({"my_chat_member":{"from":{"id":456},"chat":{"id":456},
            "new_chat_member":{"status":"member"}}});
        assert_eq!(
            parse_telegram_update(&member).unwrap().kind,
            UpdateKind::Started
        );
    }

    #[test]
    fn классификация_ошибок_по_тексту() {
        let cases = [
            (
                "Forbidden: bot was blocked by the user",
                FailReason::Blocked,
            ),
            ("Forbidden: user is deactivated", FailReason::Deactivated),
            ("Bad Request: chat not found", FailReason::NotStarted),
            (
                "Forbidden: bot can't initiate conversation with a user",
                FailReason::NotStarted,
            ),
            (
                "Bad Request: have no rights to send a message",
                FailReason::Privacy,
            ),
            ("что-то ещё", FailReason::Unknown),
        ];
        for (text, expect) in cases {
            assert_eq!(classify_text(text, FailReason::Unknown), expect, "{text}");
        }
    }
}
