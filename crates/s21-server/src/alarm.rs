//! Будильник: одна таска на сервис, тик 5 с, в API не ходит — читает
//! active_bookings. За ALARM_BEFORE_SEC до старта неподтверждённой брони
//! шлёт 🚨 во все привязки каждые ALARM_REPEAT_SEC.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use tokio::time::Instant;

use s21_core::{alarm_message, parse_ts, BookingInfo, ALARM_BEFORE_SEC, ALARM_REPEAT_SEC};

use crate::state::AppState;
use crate::{db, sender};

pub async fn run(state: Arc<AppState>) {
    let mut last_sent: HashMap<(i64, String), Instant> = HashMap::new();
    loop {
        tokio::time::sleep(Duration::from_secs(5)).await;
        if let Err(e) = tick(&state, &mut last_sent).await {
            tracing::warn!("будильник: {e}");
        }
        last_sent.retain(|_, t| t.elapsed() < Duration::from_secs(600));
    }
}

async fn tick(
    state: &AppState,
    last_sent: &mut HashMap<(i64, String), Instant>,
) -> anyhow::Result<()> {
    let now = Utc::now();
    for cand in db::alarm_candidates(&state.pool).await? {
        let Some(start) = parse_ts(&cand.start_ts) else {
            continue;
        };
        let left = (start - now).num_seconds();
        if !(0 < left && left <= ALARM_BEFORE_SEC) {
            continue;
        }
        let key = (cand.user_id, cand.booking_id.clone());
        if last_sent
            .get(&key)
            .is_some_and(|t| t.elapsed() < Duration::from_secs(ALARM_REPEAT_SEC as u64))
        {
            continue;
        }
        last_sent.insert(key, Instant::now());

        let info: BookingInfo = serde_json::from_str(&cand.info).unwrap_or_default();
        let html = alarm_message(&info, &cand.s21_login, left);
        sender::send_to_user(state, cand.user_id, "alarm", &html, Some(&cand.booking_id)).await;
    }
    Ok(())
}
