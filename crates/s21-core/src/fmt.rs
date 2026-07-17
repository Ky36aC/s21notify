//! Форматирование: тексты уведомлений посимвольно совпадают с v2.1 (watcher.py).

use chrono::{DateTime, Utc};
use chrono_tz::Europe::Moscow;
use std::sync::OnceLock;

use crate::types::BookingInfo;

/// Аналог html.escape() из питона (с кавычками).
pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            _ => out.push(c),
        }
    }
    out
}

/// Убирает HTML-теги и обрезает пробелы по краям.
pub fn strip_html(s: &str) -> String {
    static TAG: OnceLock<regex::Regex> = OnceLock::new();
    let re = TAG.get_or_init(|| regex::Regex::new(r"<[^>]+>").unwrap());
    re.replace_all(s, "").trim().to_string()
}

/// ISO-время платформы (обычно с Z) → DateTime<Utc>; мусор → None.
pub fn parse_ts(iso: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(iso)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

/// «%d.%m %H:%M» по Москве; нераспарсенное — как есть (поведение v2.1).
pub fn fmt_time(iso: &str) -> String {
    match parse_ts(iso) {
        Some(t) => t.with_timezone(&Moscow).format("%d.%m %H:%M").to_string(),
        None => iso.to_string(),
    }
}

/// Секунды до ts → полные дни (не меньше 0); мусор → None.
pub fn days_left(ts: &str, now: DateTime<Utc>) -> Option<i64> {
    let t = parse_ts(ts)?;
    Some(((t - now).num_seconds().div_euclid(86400)).max(0))
}

/// Payload кнопки «✅ Я за компом».
pub fn ack_payload(bid: &str) -> String {
    format!("ack:{bid}")
}

/// Текст кнопки подтверждения.
pub const ACK_BUTTON_TEXT: &str = "✅ Я за компом";

pub fn fmt_booking_line(info: &BookingInfo, me: &str) -> String {
    let (role, who) = if info.verifier == me {
        ("🔍 Ты проверяешь", info.verifiable.as_str())
    } else {
        ("📝 Тебя проверяет", info.verifier.as_str())
    };
    let online = if info.online { " (онлайн)" } else { "" };
    format!(
        "{role} <b>{}</b>{online}\n📦 {}\n🕐 {}",
        esc(who),
        esc(&info.task),
        fmt_time(&info.start)
    )
}

pub fn alarm_message(info: &BookingInfo, me: &str, secs_left: i64) -> String {
    format!(
        "🚨🚨🚨 <b>ПРОВЕРКА ЧЕРЕЗ {secs_left} СЕК!</b>\n{}",
        fmt_booking_line(info, me)
    )
}
