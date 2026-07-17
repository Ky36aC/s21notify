//! SQLite: подключение, миграции, репозитории.
//! Все запросы короткие; снапшот пишется одним UPDATE в транзакции цикла.

use std::collections::HashSet;
use std::str::FromStr;

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};

use s21_core::{ActiveBooking, UserSettings};

pub async fn connect(url: &str) -> anyhow::Result<SqlitePool> {
    let opts = SqliteConnectOptions::from_str(url)?
        .create_if_missing(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .busy_timeout(std::time::Duration::from_secs(5))
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(opts)
        .await?;
    sqlx::migrate!("../../migrations").run(&pool).await?;
    Ok(pool)
}

// ------------------------------------------------------------------ users

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct User {
    pub id: i64,
    pub s21_login: String,
    pub token_status: String,
    pub offline_token_enc: Option<Vec<u8>>,
    pub relogin_notified_at: Option<String>,
    pub first_cycle_done: bool,
    pub last_poll_at: Option<String>,
    pub last_poll_error: Option<String>,
}

const USER_COLS: &str = "id, s21_login, token_status, offline_token_enc, relogin_notified_at, \
                         first_cycle_done, last_poll_at, last_poll_error";

/// Создаёт пользователя вместе со строками settings и user_state.
pub async fn create_user(
    pool: &SqlitePool,
    s21_login: &str,
    token_enc: &[u8],
) -> anyhow::Result<i64> {
    let mut tx = pool.begin().await?;
    let id = sqlx::query("INSERT INTO users (s21_login, offline_token_enc) VALUES (?, ?)")
        .bind(s21_login)
        .bind(token_enc)
        .execute(&mut *tx)
        .await?
        .last_insert_rowid();
    sqlx::query("INSERT INTO settings (user_id) VALUES (?)")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO user_state (user_id) VALUES (?)")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(id)
}

pub async fn user_by_id(pool: &SqlitePool, id: i64) -> anyhow::Result<Option<User>> {
    Ok(
        sqlx::query_as(&format!("SELECT {USER_COLS} FROM users WHERE id = ?"))
            .bind(id)
            .fetch_optional(pool)
            .await?,
    )
}

pub async fn user_by_login(pool: &SqlitePool, login: &str) -> anyhow::Result<Option<User>> {
    Ok(sqlx::query_as(&format!(
        "SELECT {USER_COLS} FROM users WHERE s21_login = ?"
    ))
    .bind(login)
    .fetch_optional(pool)
    .await?)
}

pub async fn all_user_ids(pool: &SqlitePool) -> anyhow::Result<Vec<i64>> {
    let rows = sqlx::query("SELECT id FROM users").fetch_all(pool).await?;
    Ok(rows.iter().map(|r| r.get(0)).collect())
}

/// Перелогин/регистрация: новый шифрованный токен, статус ok.
pub async fn set_offline_token(pool: &SqlitePool, id: i64, token_enc: &[u8]) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE users SET offline_token_enc = ?, token_status = 'ok', relogin_notified_at = NULL \
         WHERE id = ?",
    )
    .bind(token_enc)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn mark_needs_relogin(pool: &SqlitePool, id: i64) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE users SET token_status = 'needs_relogin', relogin_notified_at = datetime('now') \
         WHERE id = ?",
    )
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn set_last_poll(pool: &SqlitePool, id: i64, error: Option<&str>) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE users SET last_poll_at = datetime('now'), last_poll_error = ? WHERE id = ?",
    )
    .bind(error)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn delete_user(pool: &SqlitePool, id: i64) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM users WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn count_users(pool: &SqlitePool) -> anyhow::Result<i64> {
    Ok(sqlx::query("SELECT COUNT(*) FROM users")
        .fetch_one(pool)
        .await?
        .get(0))
}

// ------------------------------------------------------ messenger_accounts

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct MessengerAccount {
    pub id: i64,
    /// None = /start был, но регистрация в miniapp ещё не пройдена
    pub user_id: Option<i64>,
    pub messenger: String,
    pub ext_user_id: String,
    pub chat_id: String,
    pub username: Option<String>,
    pub status: String,
}

const ACC_COLS: &str = "id, user_id, messenger, ext_user_id, chat_id, username, status";

/// /start или разблокировка: запоминаем chat_id и оживляем привязку,
/// НЕ трогая user_id (регистрация делается отдельно через attach_user).
pub async fn remember_chat(
    pool: &SqlitePool,
    messenger: &str,
    ext_user_id: &str,
    chat_id: &str,
    username: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO messenger_accounts (messenger, ext_user_id, chat_id, username) \
         VALUES (?, ?, ?, ?) \
         ON CONFLICT (messenger, ext_user_id) DO UPDATE SET \
           chat_id = excluded.chat_id, username = excluded.username, status = 'active'",
    )
    .bind(messenger)
    .bind(ext_user_id)
    .bind(chat_id)
    .bind(username)
    .execute(pool)
    .await?;
    Ok(())
}

/// Регистрация/перелогин из miniapp: привязка чата к платформенному аккаунту
/// (или перепривязка, если мессенджер входил под другим s21-логином).
pub async fn attach_user(
    pool: &SqlitePool,
    user_id: i64,
    messenger: &str,
    ext_user_id: &str,
    chat_id_fallback: &str,
    username: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO messenger_accounts (user_id, messenger, ext_user_id, chat_id, username) \
         VALUES (?, ?, ?, ?, ?) \
         ON CONFLICT (messenger, ext_user_id) DO UPDATE SET \
           user_id = excluded.user_id, status = 'active'",
    )
    .bind(user_id)
    .bind(messenger)
    .bind(ext_user_id)
    .bind(chat_id_fallback)
    .bind(username)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn account_by_ext(
    pool: &SqlitePool,
    messenger: &str,
    ext_user_id: &str,
) -> anyhow::Result<Option<MessengerAccount>> {
    Ok(sqlx::query_as(&format!(
        "SELECT {ACC_COLS} FROM messenger_accounts WHERE messenger = ? AND ext_user_id = ?"
    ))
    .bind(messenger)
    .bind(ext_user_id)
    .fetch_optional(pool)
    .await?)
}

/// Активные привязки пользователя — сюда идут уведомления.
pub async fn active_accounts(
    pool: &SqlitePool,
    user_id: i64,
) -> anyhow::Result<Vec<MessengerAccount>> {
    Ok(sqlx::query_as(&format!(
        "SELECT {ACC_COLS} FROM messenger_accounts WHERE user_id = ? AND status = 'active'"
    ))
    .bind(user_id)
    .fetch_all(pool)
    .await?)
}

/// Чистка «ожидающих» привязок старше недели (нажал /start и не зарегистрировался).
pub async fn cleanup_pending_accounts(pool: &SqlitePool) -> anyhow::Result<u64> {
    let res = sqlx::query(
        "DELETE FROM messenger_accounts \
         WHERE user_id IS NULL AND linked_at < datetime('now', '-7 days')",
    )
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

pub async fn all_accounts(
    pool: &SqlitePool,
    user_id: i64,
) -> anyhow::Result<Vec<MessengerAccount>> {
    Ok(sqlx::query_as(&format!(
        "SELECT {ACC_COLS} FROM messenger_accounts WHERE user_id = ?"
    ))
    .bind(user_id)
    .fetch_all(pool)
    .await?)
}

pub async fn set_account_status(
    pool: &SqlitePool,
    messenger: &str,
    ext_user_id: &str,
    status: &str,
) -> anyhow::Result<()> {
    sqlx::query("UPDATE messenger_accounts SET status = ? WHERE messenger = ? AND ext_user_id = ?")
        .bind(status)
        .bind(messenger)
        .bind(ext_user_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn unlink_account(
    pool: &SqlitePool,
    messenger: &str,
    ext_user_id: &str,
) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM messenger_accounts WHERE messenger = ? AND ext_user_id = ?")
        .bind(messenger)
        .bind(ext_user_id)
        .execute(pool)
        .await?;
    Ok(())
}

// --------------------------------------------------------------- settings

pub async fn get_settings(pool: &SqlitePool, user_id: i64) -> anyhow::Result<UserSettings> {
    let row = sqlx::query(
        "SELECT remind_minutes, notify_bookings, notify_changes, notify_reminders, \
                notify_feed, notify_deadlines, notify_alarm \
         FROM settings WHERE user_id = ?",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    Ok(match row {
        Some(r) => UserSettings {
            remind_minutes: r.get(0),
            notify_bookings: r.get(1),
            notify_changes: r.get(2),
            notify_reminders: r.get(3),
            notify_feed: r.get(4),
            notify_deadlines: r.get(5),
            notify_alarm: r.get(6),
        },
        None => UserSettings::default(),
    })
}

pub async fn save_settings(
    pool: &SqlitePool,
    user_id: i64,
    s: &UserSettings,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE settings SET remind_minutes = ?, notify_bookings = ?, notify_changes = ?, \
         notify_reminders = ?, notify_feed = ?, notify_deadlines = ?, notify_alarm = ? \
         WHERE user_id = ?",
    )
    .bind(&s.remind_minutes)
    .bind(s.notify_bookings)
    .bind(s.notify_changes)
    .bind(s.notify_reminders)
    .bind(s.notify_feed)
    .bind(s.notify_deadlines)
    .bind(s.notify_alarm)
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(())
}

// -------------------------------------------------------------- user_state

pub async fn get_snapshot(
    pool: &SqlitePool,
    user_id: i64,
) -> anyhow::Result<s21_core::UserSnapshot> {
    let row = sqlx::query("SELECT snapshot FROM user_state WHERE user_id = ?")
        .bind(user_id)
        .fetch_optional(pool)
        .await?;
    let json: String = match row {
        Some(r) => r.get(0),
        None => return Ok(Default::default()),
    };
    Ok(serde_json::from_str(&json).unwrap_or_default())
}

// ---------------------------------------------------------- active_bookings

pub async fn acked_bookings(pool: &SqlitePool, user_id: i64) -> anyhow::Result<HashSet<String>> {
    let rows =
        sqlx::query("SELECT booking_id FROM active_bookings WHERE user_id = ? AND acked = 1")
            .bind(user_id)
            .fetch_all(pool)
            .await?;
    Ok(rows.iter().map(|r| r.get::<String, _>(0)).collect())
}

/// Нажатие «✅ Я за компом»: гасит будильник по брони во всех мессенджерах.
pub async fn ack_booking(pool: &SqlitePool, user_id: i64, booking_id: &str) -> anyhow::Result<()> {
    sqlx::query("UPDATE active_bookings SET acked = 1 WHERE user_id = ? AND booking_id = ?")
        .bind(user_id)
        .bind(booking_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Итог цикла одной транзакцией: снапшот + замена active_bookings
/// (acked сохраняется по совпавшим booking_id) + last_poll + first_cycle_done.
pub async fn commit_cycle(
    pool: &SqlitePool,
    user_id: i64,
    snapshot: &s21_core::UserSnapshot,
    active: &[ActiveBooking],
) -> anyhow::Result<()> {
    let json = serde_json::to_string(snapshot)?;
    let mut tx = pool.begin().await?;
    sqlx::query(
        "UPDATE user_state SET snapshot = ?, updated_at = datetime('now') WHERE user_id = ?",
    )
    .bind(&json)
    .bind(user_id)
    .execute(&mut *tx)
    .await?;

    // acked по ещё живым броням переносим в новый набор
    let acked_rows =
        sqlx::query("SELECT booking_id FROM active_bookings WHERE user_id = ? AND acked = 1")
            .bind(user_id)
            .fetch_all(&mut *tx)
            .await?;
    let acked: HashSet<String> = acked_rows.iter().map(|r| r.get::<String, _>(0)).collect();

    sqlx::query("DELETE FROM active_bookings WHERE user_id = ?")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    for b in active {
        sqlx::query(
            "INSERT INTO active_bookings (user_id, booking_id, start_ts, info, acked) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(user_id)
        .bind(&b.booking_id)
        .bind(&b.start)
        .bind(serde_json::to_string(&b.info)?)
        .bind(acked.contains(&b.booking_id))
        .execute(&mut *tx)
        .await?;
    }

    sqlx::query(
        "UPDATE users SET last_poll_at = datetime('now'), last_poll_error = NULL, \
         first_cycle_done = 1 WHERE id = ?",
    )
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Брони для будильника: не подтверждены, стартуют в ближайшие `within_sec` секунд,
/// у владельца включены notify_alarm и notify_reminders.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AlarmCandidate {
    pub user_id: i64,
    pub booking_id: String,
    pub start_ts: String,
    pub info: String,
}

pub async fn alarm_candidates(pool: &SqlitePool) -> anyhow::Result<Vec<AlarmCandidate>> {
    Ok(sqlx::query_as(
        "SELECT ab.user_id, ab.booking_id, ab.start_ts, ab.info \
         FROM active_bookings ab \
         JOIN settings s ON s.user_id = ab.user_id \
         WHERE ab.acked = 0 AND s.notify_alarm = 1 AND s.notify_reminders = 1",
    )
    .fetch_all(pool)
    .await?)
}

// -------------------------------------------------------------- deliveries

pub async fn log_delivery(
    pool: &SqlitePool,
    user_id: Option<i64>,
    messenger: &str,
    kind: &str,
    ok: bool,
    fail_reason: Option<&str>,
    preview: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO deliveries (user_id, messenger, kind, ok, fail_reason, preview) \
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(user_id)
    .bind(messenger)
    .bind(kind)
    .bind(ok)
    .bind(fail_reason)
    .bind(preview.chars().take(100).collect::<String>())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn cleanup_deliveries(pool: &SqlitePool) -> anyhow::Result<u64> {
    let res = sqlx::query("DELETE FROM deliveries WHERE created_at < datetime('now', '-14 days')")
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}
