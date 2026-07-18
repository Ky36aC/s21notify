//! Адаптер MAX — собственный тонкий клиент Bot API (боевые грабли NotifyBot):
//! - база platform-api2.max.ru (переезд из-за отзыва GlobalSign);
//! - TLS-цепочка от Russian Trusted Root CA (НУЦ Минцифры) — свой клиент
//!   с вшитым PEM, НЕ подмешивать этот CA клиентам платформы/Telegram;
//! - токен в Authorization БЕЗ схемы Bearer;
//! - POST /messages: chat_id в QUERY-параметрах, иначе 400;
//! - клавиатура — top-level поле `keyboard`;
//! - bot_started кладёт user_id скаляром и приходит только при ПЕРВОМ старте,
//!   вернувшиеся шлют message_created с текстом "/start";
//! - текст сообщения лежит в message.body.text.

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::types::*;
use crate::webappdata;
use crate::MessengerAdapter;

/// MAX-сессия miniapp живёт долго — окно auth_date мягче телеграмного (сутки).
const AUTH_MAX_AGE_SECONDS: i64 = 86400;

pub const DEFAULT_BASE: &str = "https://platform-api2.max.ru";

pub struct MaxAdapter {
    token: String,
    base: String,
    http: reqwest::Client,
    /// HTML в MAX не подтверждён (риск 15) — при false шлём strip_html
    html_format: bool,
}

impl MaxAdapter {
    pub fn new(token: &str, base: &str, html_format: bool) -> anyhow::Result<Self> {
        let ca =
            reqwest::Certificate::from_pem(include_bytes!("../certs/russian_trusted_root_ca.pem"))?;
        let http = reqwest::Client::builder()
            .add_root_certificate(ca)
            .timeout(std::time::Duration::from_secs(15))
            .build()?;
        Ok(Self {
            token: token.to_string(),
            base: base.trim_end_matches('/').to_string(),
            http,
            html_format,
        })
    }

    fn classify_response(status: u16, text: &str, retry_after: Option<u64>) -> SendResult {
        let low = text.to_lowercase();
        if status == 429 {
            return SendResult {
                ok: false,
                fail_reason: Some(FailReason::Flood),
                error_text: Some(text.to_string()),
                retry_after,
            };
        }
        let reason = if low.contains("blocked") || low.contains("stopped") {
            FailReason::Blocked
        } else if low.contains("not found")
            || low.contains("not started")
            || low.contains("no dialog")
        {
            FailReason::NotStarted
        } else if low.contains("deactivated") || low.contains("deleted") {
            FailReason::Deactivated
        } else {
            FailReason::Unknown
        };
        SendResult::fail(
            reason,
            format!(
                "HTTP {status}: {}",
                text.chars().take(300).collect::<String>()
            ),
        )
    }
}

#[async_trait]
impl MessengerAdapter for MaxAdapter {
    fn id(&self) -> &'static str {
        "max"
    }

    async fn send_message(
        &self,
        chat_id: &str,
        html: &str,
        button: Option<MsgButton<'_>>,
    ) -> SendResult {
        let mut body = if self.html_format {
            json!({"text": html, "format": "html"})
        } else {
            json!({"text": s21_core::strip_html(html)})
        };
        match button {
            Some(MsgButton::Ack(payload)) => {
                body["keyboard"] = json!({
                    "buttons": [[{
                        "type": "callback",
                        "text": s21_core::ACK_BUTTON_TEXT,
                        "payload": payload,
                    }]]
                });
            }
            // тип кнопки MWA в кабинете уточняется — фолбэк обычной ссылкой
            Some(MsgButton::Miniapp { text, url }) => {
                body["keyboard"] = json!({
                    "buttons": [[{"type": "link", "text": text, "url": url}]]
                });
            }
            None => {}
        }
        let resp = self
            .http
            .post(format!("{}/messages", self.base))
            .query(&[("chat_id", chat_id)])
            .header("Authorization", &self.token)
            .json(&body)
            .send()
            .await;
        match resp {
            Ok(r) if r.status().is_success() => SendResult::success(),
            Ok(r) => {
                let status = r.status().as_u16();
                let retry_after = r
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<f64>().ok())
                    .map(|v| v as u64);
                let text = r.text().await.unwrap_or_default();
                Self::classify_response(status, &text, retry_after)
            }
            Err(e) => SendResult::fail(FailReason::Unknown, e.to_string()),
        }
    }

    fn parse_update(&self, raw: &Value) -> Option<IncomingUpdate> {
        parse_max_update(raw)
    }

    fn verify_miniapp_auth(&self, init_data: &str) -> Option<MiniappUser> {
        webappdata::verify(init_data, &self.token, AUTH_MAX_AGE_SECONDS, now_ts())
    }

    /// Тост через POST /answers. Правка клавиатуры «✅ Подтверждено» в MAX
    /// требует полной замены сообщения — проверяется живьём, пока только тост.
    async fn ack_callback(&self, upd: &IncomingUpdate, toast: &str) {
        let Some(cb_id) = &upd.callback_id else {
            return;
        };
        let _ = self
            .http
            .post(format!("{}/answers", self.base))
            .query(&[("callback_id", cb_id)])
            .header("Authorization", &self.token)
            .json(&json!({"notification": toast}))
            .send()
            .await;
    }

    /// POST /subscriptions {url}. Секрет — в самом URL (?s=...), заголовка у MAX нет.
    /// MAX снимает подписку после ~8 ч недоступности — вызывать при старте и раз в час.
    async fn set_webhook(&self, url: &str) -> anyhow::Result<()> {
        let resp = self
            .http
            .post(format!("{}/subscriptions", self.base))
            .header("Authorization", &self.token)
            .json(&json!({"url": url}))
            .send()
            .await?;
        if !resp.status().is_success() {
            anyhow::bail!(
                "MAX set_webhook HTTP {}: {}",
                resp.status(),
                resp.text()
                    .await
                    .unwrap_or_default()
                    .chars()
                    .take(300)
                    .collect::<String>()
            );
        }
        Ok(())
    }
}

fn now_ts() -> i64 {
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

/// user_id из объекта user (user_id либо id).
fn extract_user(user: &Value) -> (String, Option<String>) {
    let uid = {
        let a = v_str(user, "/user_id");
        if a.is_empty() {
            v_str(user, "/id")
        } else {
            a
        }
    };
    let username = user
        .get("username")
        .and_then(Value::as_str)
        .map(String::from);
    (uid, username)
}

/// chat_id: верхний уровень → scope.chat_id → scope.recipient.chat_id →
/// scope.chat.chat_id → user_id (в личном диалоге может совпадать).
fn extract_chat_id(raw: &Value, scope: &Value) -> String {
    for cand in [
        v_str(raw, "/chat_id"),
        v_str(scope, "/chat_id"),
        v_str(scope, "/recipient/chat_id"),
        v_str(scope, "/chat/chat_id"),
    ] {
        if !cand.is_empty() {
            return cand;
        }
    }
    let (uid, _) = extract_user(raw.get("user").unwrap_or(&Value::Null));
    if !uid.is_empty() {
        return uid;
    }
    v_str(raw, "/user_id")
}

fn extract_text(raw: &Value) -> String {
    for ptr in ["/message/body/text", "/message/text", "/text"] {
        let t = v_str(raw, ptr);
        if !t.is_empty() {
            return t.trim().to_string();
        }
    }
    String::new()
}

fn build(raw: &Value, kind: UpdateKind) -> IncomingUpdate {
    let message = raw.get("message").unwrap_or(&Value::Null);
    let sender = message
        .get("sender")
        .or_else(|| message.get("from"))
        .unwrap_or(&Value::Null);
    let user = raw.get("user").unwrap_or(sender);
    let (mut ext, mut username) = extract_user(user);
    // bot_started/bot_stopped кладут user_id СКАЛЯРОМ на верхний уровень
    if ext.is_empty() {
        ext = v_str(raw, "/user_id");
    }
    if username.is_none() {
        username = raw
            .get("username")
            .or_else(|| raw.get("user_name"))
            .and_then(Value::as_str)
            .map(String::from);
    }
    let chat_id = extract_chat_id(raw, message);
    let mut upd = IncomingUpdate::new(kind, ext, chat_id);
    upd.username = username;
    upd
}

/// Разбор webhook-события MAX → IncomingUpdate.
pub fn parse_max_update(raw: &Value) -> Option<IncomingUpdate> {
    let update_type = raw
        .get("update_type")
        .or_else(|| raw.get("type"))
        .and_then(Value::as_str)?;

    match update_type {
        "bot_started" => Some(build(raw, UpdateKind::Started)),
        "bot_stopped" | "dialog_removed" | "bot_removed" => {
            Some(build(raw, UpdateKind::StoppedOrBlocked))
        }
        "message_callback" => {
            let cb = raw.get("callback").unwrap_or(&Value::Null);
            let user = cb
                .get("user")
                .or_else(|| raw.get("user"))
                .unwrap_or(&Value::Null);
            let (ext, username) = extract_user(user);
            // chat_id ищем и в message (recipient), и в самом callback
            let message = raw.get("message").unwrap_or(&Value::Null);
            let mut chat_id = extract_chat_id(raw, message);
            if chat_id.is_empty() {
                chat_id = extract_chat_id(raw, cb);
            }
            if chat_id.is_empty() {
                chat_id = ext.clone();
            }
            let mut upd = IncomingUpdate::new(UpdateKind::Callback, ext, chat_id);
            upd.username = username;
            upd.payload = cb.get("payload").and_then(Value::as_str).map(String::from);
            let cb_id = {
                let a = v_str(cb, "/callback_id");
                if a.is_empty() {
                    v_str(cb, "/id")
                } else {
                    a
                }
            };
            upd.callback_id = if cb_id.is_empty() { None } else { Some(cb_id) };
            let mid = v_str(raw, "/message/body/mid");
            upd.message_id = if mid.is_empty() { None } else { Some(mid) };
            Some(upd)
        }
        "message_created" => {
            let text = extract_text(raw);
            let kind =
                if text == "/start" || text.starts_with("/start@") || text.starts_with("/start ") {
                    UpdateKind::Started
                } else if text.starts_with('/') {
                    UpdateKind::Text
                } else {
                    UpdateKind::Activity
                };
            let mut upd = build(raw, kind);
            upd.text = Some(text);
            Some(upd)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn bot_started_со_скалярным_user_id() {
        // боевой формат: объекта user нет, user_id и chat_id сверху
        let raw = json!({"update_type":"bot_started","timestamp":1,"chat_id":111,"user_id":456});
        let u = parse_max_update(&raw).unwrap();
        assert_eq!(u.kind, UpdateKind::Started);
        assert_eq!(u.ext_user_id, "456");
        assert_eq!(u.chat_id, "111");
    }

    #[test]
    fn вернувшийся_шлёт_start_текстом() {
        let raw = json!({"update_type":"message_created","message":{
            "sender":{"user_id":456,"username":"ivan"},
            "recipient":{"chat_id":111,"chat_type":"dialog"},
            "body":{"mid":"mid.001","seq":1,"text":"/start"}}});
        let u = parse_max_update(&raw).unwrap();
        assert_eq!(u.kind, UpdateKind::Started);
        assert_eq!(u.ext_user_id, "456");
        assert_eq!(u.chat_id, "111");
        assert_eq!(u.username.as_deref(), Some("ivan"));
    }

    #[test]
    fn команда_и_болтовня() {
        let cmd = json!({"update_type":"message_created","message":{
            "sender":{"user_id":456},"recipient":{"chat_id":111},
            "body":{"mid":"m","text":"/reviews"}}});
        assert_eq!(parse_max_update(&cmd).unwrap().kind, UpdateKind::Text);

        let chat = json!({"update_type":"message_created","message":{
            "sender":{"user_id":456},"recipient":{"chat_id":111},
            "body":{"mid":"m","text":"привет"}}});
        assert_eq!(parse_max_update(&chat).unwrap().kind, UpdateKind::Activity);
    }

    #[test]
    fn callback_и_остановка() {
        let cb = json!({"update_type":"message_callback",
            "callback":{"callback_id":"cbid1","payload":"ack:b1","user":{"user_id":456}},
            "message":{"recipient":{"chat_id":111},"body":{"mid":"mid.002","text":"x"}}});
        let u = parse_max_update(&cb).unwrap();
        assert_eq!(u.kind, UpdateKind::Callback);
        assert_eq!(u.payload.as_deref(), Some("ack:b1"));
        assert_eq!(u.callback_id.as_deref(), Some("cbid1"));
        assert_eq!(u.message_id.as_deref(), Some("mid.002"));
        assert_eq!(u.chat_id, "111");

        let stop = json!({"update_type":"bot_stopped","chat_id":111,"user_id":456});
        assert_eq!(
            parse_max_update(&stop).unwrap().kind,
            UpdateKind::StoppedOrBlocked
        );
        assert!(parse_max_update(&json!({"update_type":"что_то_новое"})).is_none());
    }

    #[test]
    fn са_сертификат_валиден() {
        assert!(reqwest::Certificate::from_pem(include_bytes!(
            "../certs/russian_trusted_root_ca.pem"
        ))
        .is_ok());
    }
}
