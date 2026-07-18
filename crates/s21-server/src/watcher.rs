//! Оркестрация опроса: tokio-таска на пользователя, глобальный семафор +
//! rate-limit на платформу, refresh по 401, needs_relogin при мёртвом токене.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use dashmap::DashMap;
use tokio::sync::{Notify, Semaphore};

use s21_core::{run_cycle, Fetched};
use s21_platform::{fetch_bookings, fetch_deadlines, fetch_exams, fetch_feed, PlatformError};

use crate::poll::{PollCommand, PollReceiver};
use crate::state::AppState;
use crate::{db, sender};

struct PollHandle {
    handle: tokio::task::JoinHandle<()>,
    notify: Arc<Notify>,
}

pub struct PollManager {
    state: Arc<AppState>,
    tasks: DashMap<i64, PollHandle>,
    semaphore: Arc<Semaphore>,
}

impl PollManager {
    pub fn new(state: Arc<AppState>) -> Arc<Self> {
        let semaphore = Arc::new(Semaphore::new(state.cfg.max_concurrent_polls));
        Arc::new(Self {
            state,
            tasks: DashMap::new(),
            semaphore,
        })
    }

    /// Главный цикл: поднять таски всех пользователей и слушать команды.
    pub async fn run(self: Arc<Self>, mut rx: PollReceiver) {
        match db::all_user_ids(&self.state.pool).await {
            Ok(ids) => {
                tracing::info!("watcher: поднимаю {} тасок опроса", ids.len());
                for uid in ids {
                    self.start(uid);
                }
            }
            Err(e) => tracing::error!("watcher: не прочитал пользователей: {e}"),
        }

        while let Some(cmd) = rx.recv().await {
            match cmd {
                PollCommand::Start(uid) => self.start(uid),
                PollCommand::Stop(uid) => self.stop(uid),
                PollCommand::CheckNow(uid) => {
                    if let Some(h) = self.tasks.get(&uid) {
                        h.notify.notify_one();
                    } else {
                        self.start(uid);
                    }
                }
            }
        }
    }

    pub fn active_tasks(&self) -> usize {
        self.tasks.len()
    }

    fn start(self: &Arc<Self>, uid: i64) {
        // перезапуск, если таска уже была (перелогин снимает needs_relogin)
        self.stop(uid);
        let notify = Arc::new(Notify::new());
        let mgr = self.clone();
        let n = notify.clone();
        let handle = tokio::spawn(async move { mgr.user_loop(uid, n).await });
        self.tasks.insert(uid, PollHandle { handle, notify });
    }

    fn stop(&self, uid: i64) {
        // без abort() таска-призрак опрашивала бы удалённого юзера (грабля v2)
        if let Some((_, h)) = self.tasks.remove(&uid) {
            h.handle.abort();
        }
    }

    async fn user_loop(self: Arc<Self>, uid: i64, notify: Arc<Notify>) {
        let interval = self.state.cfg.poll_interval_sec.max(5);
        // стартовый джиттер размазывает пользователей по интервалу
        let jitter = {
            let mut h = DefaultHasher::new();
            uid.hash(&mut h);
            h.finish() % interval
        };
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(jitter)) => {}
            _ = notify.notified() => {}
        }

        let mut tick: u64 = 0;
        let mut backoff: u64 = 0; // секунд дополнительной паузы после ошибок
        loop {
            let stop = self.poll_once(uid, tick, &mut backoff).await;
            if stop {
                self.tasks.remove(&uid);
                return;
            }
            tick += 1;

            // интервал ±10% (детерминированный сдвиг от uid и tick)
            let mut h = DefaultHasher::new();
            (uid, tick).hash(&mut h);
            let spread = interval / 5;
            let base = interval - interval / 10 + if spread > 0 { h.finish() % spread } else { 0 };
            let sleep = Duration::from_secs(base + backoff);
            tokio::select! {
                _ = tokio::time::sleep(sleep) => {}
                _ = notify.notified() => {}
            }
        }
    }

    /// Один цикл опроса. true = таску пора останавливать (needs_relogin/удалён).
    async fn poll_once(&self, uid: i64, tick: u64, backoff: &mut u64) -> bool {
        let state = &self.state;
        let user = match db::user_by_id(&state.pool, uid).await {
            Ok(Some(u)) => u,
            Ok(None) => return true,
            Err(e) => {
                tracing::error!("watcher uid={uid}: БД: {e}");
                return false;
            }
        };
        if user.token_status != "ok" {
            return true; // ждёт перелогина; Start() перезапустит
        }
        let accounts = db::active_accounts(&state.pool, uid)
            .await
            .unwrap_or_default();
        if accounts.is_empty() {
            return false; // некому слать — тихо пропускаем тик
        }

        let _permit = match self.semaphore.acquire().await {
            Ok(p) => p,
            Err(_) => return true,
        };

        match self.cycle(uid, &user, tick).await {
            Ok(()) => {
                *backoff = 0;
                false
            }
            Err(PlatformError::OfflineTokenDead) => {
                tracing::info!("watcher uid={uid}: offline-токен мёртв, нужен перелогин");
                let _ = db::mark_needs_relogin(&state.pool, uid).await;
                state.sessions.evict(uid);
                // ровно одно сообщение (mark ставит relogin_notified_at)
                if user.relogin_notified_at.is_none() {
                    sender::send_relogin_notice(state, uid).await;
                }
                true
            }
            Err(e) => {
                let msg = e.to_string();
                let _ = db::set_last_poll(&state.pool, uid, Some(&msg)).await;
                // сломанный whitelist задевает всех — алерт админу
                if let PlatformError::Gql {
                    status: 400,
                    reason,
                    op,
                    ..
                } = &e
                {
                    sender::send_admin_alert(
                        state,
                        &format!("GraphQL 400 [{op}]: {reason} — whitelist?"),
                    )
                    .await;
                }
                *backoff = (*backoff * 2).clamp(30, 900);
                tracing::warn!("watcher uid={uid}: {msg} (backoff {}с)", backoff);
                false
            }
        }
    }

    async fn cycle(&self, uid: i64, user: &db::User, tick: u64) -> Result<(), PlatformError> {
        let state = &self.state;
        let now = Utc::now();
        let gql = &state.sessions.gql;
        let settings = db::get_settings(&state.pool, uid).await.map_err(other)?;
        let with_deadlines =
            settings.notify_deadlines && tick.is_multiple_of(state.cfg.deadline_poll_every.max(1));

        // одна попытка refresh + повтор на 401 (протухший access посреди цикла)
        let mut session = state.sessions.session_for(uid).await?;
        let mut retried = false;
        let fetched = loop {
            let res: Result<Fetched, PlatformError> = async {
                let bookings = fetch_bookings(gql, &session, now).await?;
                let feed = if settings.notify_feed {
                    Some(fetch_feed(gql, &session).await?)
                } else {
                    None
                };
                let (deadlines, exams) = if with_deadlines {
                    (
                        Some(fetch_deadlines(gql, &session).await?),
                        Some(fetch_exams(gql, &session, now).await?),
                    )
                } else {
                    (None, None)
                };
                Ok(Fetched {
                    bookings: Some(bookings),
                    feed,
                    deadlines,
                    exams,
                })
            }
            .await;
            match res {
                Err(PlatformError::Unauthorized(_)) if !retried => {
                    retried = true;
                    session = state.sessions.refresh(uid).await?;
                }
                other => break other?,
            }
        };

        let prev = db::get_snapshot(&state.pool, uid).await.map_err(other)?;
        let acked = db::acked_bookings(&state.pool, uid).await.map_err(other)?;
        let out = run_cycle(
            &prev,
            &fetched,
            &settings,
            &user.s21_login,
            now,
            !user.first_cycle_done,
            &acked,
        );

        for ev in &out.events {
            sender::send_to_user(
                state,
                uid,
                ev.kind.as_str(),
                &ev.html,
                ev.ack_booking_id.as_deref(),
            )
            .await;
        }
        db::commit_cycle(&state.pool, uid, &out.snapshot, &out.active)
            .await
            .map_err(other)?;
        Ok(())
    }
}

fn other<E: std::fmt::Display>(e: E) -> PlatformError {
    PlatformError::Other(e.to_string())
}

/// Служебная таска: чистка deliveries и висящих привязок + перестановка
/// вебхука MAX (снимается после ~8 ч недоступности).
pub async fn housekeeping(state: Arc<AppState>) {
    loop {
        tokio::time::sleep(Duration::from_secs(3600)).await;
        // перестановка вебхука нужна только в webhook-режиме (MAX снимает подписку
        // после ~8 ч недоступности); в polling-режиме вебхука нет
        if state.cfg.transport("max") == crate::config::Transport::Webhook {
            if let Some(max) = state.adapter("max") {
                if let Err(e) = max.set_webhook(&state.cfg.webhook_url("max")).await {
                    tracing::warn!("housekeeping: вебхук MAX: {e}");
                }
            }
        }
        match db::cleanup_deliveries(&state.pool).await {
            Ok(n) if n > 0 => tracing::info!("housekeeping: удалено {n} старых deliveries"),
            Err(e) => tracing::warn!("housekeeping: deliveries: {e}"),
            _ => {}
        }
        if let Err(e) = db::cleanup_pending_accounts(&state.pool).await {
            tracing::warn!("housekeeping: pending accounts: {e}");
        }
    }
}
