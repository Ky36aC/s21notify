-- Схема v3. user = платформенный аккаунт; мессенджеры цепляются к нему
-- многие-к-одному (студент может привязать и Telegram, и MAX).

CREATE TABLE users (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  s21_login TEXT NOT NULL UNIQUE COLLATE NOCASE,       -- короткий ник
  token_status TEXT NOT NULL DEFAULT 'ok',             -- 'ok'|'needs_relogin'
  offline_token_enc BLOB,                              -- nonce(12) || AES-256-GCM(токен)
  relogin_notified_at TEXT,
  first_cycle_done INTEGER NOT NULL DEFAULT 0,
  last_poll_at TEXT,
  last_poll_error TEXT,
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE messenger_accounts (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  messenger TEXT NOT NULL,                             -- 'telegram'|'max'
  ext_user_id TEXT NOT NULL,
  chat_id TEXT NOT NULL,
  username TEXT,
  status TEXT NOT NULL DEFAULT 'active',               -- active|blocked|not_started|deactivated
  linked_at TEXT NOT NULL DEFAULT (datetime('now')),
  UNIQUE (messenger, ext_user_id)
);

-- 1:1 с users, дефолты = DEFAULTS v2.1
CREATE TABLE settings (
  user_id INTEGER PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
  remind_minutes TEXT NOT NULL DEFAULT '30, 15, 3',
  notify_bookings INTEGER NOT NULL DEFAULT 1,
  notify_changes INTEGER NOT NULL DEFAULT 1,
  notify_reminders INTEGER NOT NULL DEFAULT 1,
  notify_feed INTEGER NOT NULL DEFAULT 1,
  notify_deadlines INTEGER NOT NULL DEFAULT 1,
  notify_alarm INTEGER NOT NULL DEFAULT 1
);

-- аналог state.json v2.1: JSON-блоб, один писатель (poll-таска юзера)
CREATE TABLE user_state (
  user_id INTEGER PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
  snapshot TEXT NOT NULL DEFAULT '{}',
  updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- денормализация предстоящих броней для будильника (тик 5 с, без API)
CREATE TABLE active_bookings (
  user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  booking_id TEXT NOT NULL,
  start_ts TEXT NOT NULL,
  info TEXT NOT NULL,                                  -- JSON BookingInfo для fmt_booking_line
  acked INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (user_id, booking_id)
);
CREATE INDEX idx_active_bookings ON active_bookings(acked, start_ts);

-- журнал отправок, чистится (>14 дней)
CREATE TABLE deliveries (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  user_id INTEGER REFERENCES users(id) ON DELETE SET NULL,
  messenger TEXT NOT NULL,
  kind TEXT NOT NULL,
  ok INTEGER NOT NULL,
  fail_reason TEXT,
  preview TEXT,
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
