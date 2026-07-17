//! Команды бота — порт текстов и логики bot.py v2.1.
//! Вызываются от имени владельца привязки, ответ — HTML-строка.

use chrono::Utc;

use s21_core::{days_left, esc, fmt_booking_line, fmt_time, strip_html};
use s21_platform::{fetch_agenda, fetch_bookings, fetch_deadlines, fetch_experience};

use crate::poll::PollCommand;
use crate::state::AppState;
use crate::texts;

/// Обработка "/команды"; None = неизвестная команда (молчим, как v2.1).
pub async fn handle_command(
    state: &AppState,
    user_id: i64,
    s21_login: &str,
    text: &str,
) -> Option<String> {
    let cmd = text
        .split_whitespace()
        .next()
        .unwrap_or("")
        .split('@')
        .next()
        .unwrap_or("")
        .to_lowercase();
    let reply = match cmd.as_str() {
        "/reviews" => cmd_reviews(state, user_id, s21_login).await,
        "/agenda" => cmd_agenda(state, user_id).await,
        "/deadlines" => cmd_deadlines(state, user_id).await,
        "/status" => cmd_status(state, user_id, s21_login).await,
        "/check" => {
            let _ = state.poll_tx.send(PollCommand::CheckNow(user_id));
            Ok(texts::check_started())
        }
        "/help" => Ok(texts::HELP_TEXT.to_string()),
        _ => return None,
    };
    Some(reply.unwrap_or_else(|e| format!("⚠️ Не получилось: {}", esc(&e.to_string()))))
}

async fn cmd_reviews(state: &AppState, uid: i64, me: &str) -> anyhow::Result<String> {
    let session = state.sessions.session_for(uid).await?;
    let bookings = fetch_bookings(&state.sessions.gql, &session, Utc::now()).await?;
    if bookings.is_empty() {
        return Ok("Записей на проверку нет 🙌".into());
    }
    let parts: Vec<String> = bookings
        .iter()
        .map(|b| fmt_booking_line(&b.info, me))
        .collect();
    Ok(format!(
        "<b>Ближайшие проверки:</b>\n\n{}",
        parts.join("\n\n")
    ))
}

async fn cmd_agenda(state: &AppState, uid: i64) -> anyhow::Result<String> {
    let session = state.sessions.session_for(uid).await?;
    let events = fetch_agenda(&state.sessions.gql, &session, Utc::now()).await?;
    if events.is_empty() {
        return Ok("На ближайшую неделю событий нет 🙌".into());
    }
    let lines: Vec<String> = events
        .iter()
        .map(|e| {
            let label = if !e.label.is_empty() {
                e.label.clone()
            } else {
                let d = strip_html(&e.description);
                if !d.is_empty() {
                    d
                } else if !e.event_type.is_empty() {
                    e.event_type.clone()
                } else {
                    "?".into()
                }
            };
            format!("• {} — {}", fmt_time(&e.start), esc(&label))
        })
        .collect();
    Ok(format!("<b>События на 7 дней:</b>\n{}", lines.join("\n")))
}

async fn cmd_deadlines(state: &AppState, uid: i64) -> anyhow::Result<String> {
    let session = state.sessions.session_for(uid).await?;
    let now = Utc::now();
    let deadlines: Vec<_> = fetch_deadlines(&state.sessions.gql, &session)
        .await?
        .into_iter()
        .filter_map(|d| days_left(&d.ts, now).map(|left| (d, left)))
        .collect();
    if deadlines.is_empty() {
        return Ok("Дедлайнов нет 🙌".into());
    }
    let lines: Vec<String> = deadlines
        .iter()
        .map(|(d, left)| {
            let when = if *left == 0 {
                "СЕГОДНЯ".to_string()
            } else {
                format!("через {left} дн")
            };
            format!(
                "⏳ <b>{when}</b> ({})\nСдать любой из: {}",
                fmt_time(&d.ts),
                esc(&d.title)
            )
        })
        .collect();
    Ok(format!("<b>Дедлайны:</b>\n\n{}", lines.join("\n\n")))
}

async fn cmd_status(state: &AppState, uid: i64, me: &str) -> anyhow::Result<String> {
    let session = state.sessions.session_for(uid).await?;
    let xp = fetch_experience(&state.sessions.gql, &session).await?;
    Ok(format!(
        "<b>{}</b>\n🏅 Уровень: {}\n🍪 Печеньки: {}\n⭐ Code-review points: {}\n🪙 Коины: {}",
        esc(me),
        xp.level_code,
        xp.cookies,
        xp.code_review_points,
        xp.coins
    ))
}
