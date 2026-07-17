//! Диффинг одного цикла опроса — порт семантики watcher.py v2.1.
//!
//! Отличие от v2.1 (решение v3): первый цикл пользователя молчит целиком —
//! снапшот записывается, события не генерируются. Пропущенные при этом пороги
//! напоминаний не помечаются, так что ближайший следующий цикл дошлёт одно
//! схлопнутое «⏰ Проверка через N мин».

use chrono::{DateTime, Duration, Utc};
use indexmap::IndexMap;
use std::collections::HashSet;

use crate::fmt::{esc, fmt_booking_line, fmt_time, parse_ts, strip_html};
use crate::types::*;

/// За сколько секунд до старта неподтверждённой брони начинает звонить будильник.
pub const ALARM_BEFORE_SEC: i64 = 45;
/// Минимальный интервал между сообщениями будильника.
pub const ALARM_REPEAT_SEC: i64 = 10;
/// Окно напоминания о дедлайне/экзамене.
pub const DEADLINE_REMIND_HOURS: i64 = 24;
/// Окно выборки экзаменов.
pub const DEADLINE_WINDOW_DAYS: i64 = 30;

/// Типы ленты, дублирующие собственные сообщения watcher'а
/// (CALENDAR — «кто-то записался», DASHBOARD — «проверка скоро начнётся»).
pub const SKIP_FEED_TYPES: [&str; 2] = ["CALENDAR", "DASHBOARD"];

/// «30, 15, 3» → отсортированные по убыванию пороги; мусор → [30]; clamp 1..=720.
pub fn parse_remind_minutes(value: &str) -> Vec<u32> {
    let mut minutes: Vec<u32> = value
        .split(|c: char| c == ',' || c == ';' || c.is_whitespace())
        .filter(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
        .filter_map(|p| p.parse::<u32>().ok())
        .filter(|&n| (1..=720).contains(&n))
        .collect();
    minutes.sort_unstable();
    minutes.dedup();
    minutes.reverse();
    if minutes.is_empty() {
        vec![30]
    } else {
        minutes
    }
}

/// Один цикл: prev-снапшот + свежие данные → новый снапшот, события, брони для будильника.
///
/// `acked` — брони, по которым нажато «я за компом» (живёт в БД, не в снапшоте).
pub fn run_cycle(
    prev: &UserSnapshot,
    fetched: &Fetched,
    settings: &UserSettings,
    me: &str,
    now: DateTime<Utc>,
    first_cycle: bool,
    acked: &HashSet<String>,
) -> CycleOutput {
    let mut snap = prev.clone();
    let mut events: Vec<OutEvent> = Vec::new();

    if let Some(bookings) = &fetched.bookings {
        check_bookings(
            bookings,
            settings,
            me,
            now,
            first_cycle,
            acked,
            &mut snap,
            &mut events,
        );
    }
    if settings.notify_feed {
        if let Some(feed) = &fetched.feed {
            check_feed(feed, first_cycle, &mut snap, &mut events);
        }
    }
    if settings.notify_deadlines {
        if let Some(deadlines) = &fetched.deadlines {
            check_deadlines(deadlines, now, first_cycle, &mut snap, &mut events);
        }
        if let Some(exams) = &fetched.exams {
            check_exams(exams, now, first_cycle, &mut snap, &mut events);
        }
    }

    let active = snap
        .bookings
        .as_ref()
        .map(|m| {
            m.iter()
                .map(|(bid, info)| ActiveBooking {
                    booking_id: bid.clone(),
                    start: info.start.clone(),
                    info: info.clone(),
                })
                .collect()
        })
        .unwrap_or_default();

    CycleOutput {
        snapshot: snap,
        events,
        active,
    }
}

#[allow(clippy::too_many_arguments)]
fn check_bookings(
    list: &[Booking],
    s: &UserSettings,
    me: &str,
    now: DateTime<Utc>,
    first_cycle: bool,
    acked: &HashSet<String>,
    snap: &mut UserSnapshot,
    events: &mut Vec<OutEvent>,
) {
    let current: IndexMap<String, BookingInfo> = list
        .iter()
        .map(|b| (b.id.clone(), b.info.clone()))
        .collect();
    // ключа не было = первое появление секции — инициализируемся молча
    let section_first = first_cycle || snap.bookings.is_none();
    let prev = snap.bookings.take().unwrap_or_default();
    let mut reminded = std::mem::take(&mut snap.reminded_bookings);
    let thresholds = parse_remind_minutes(&s.remind_minutes);

    if !section_first {
        for (bid, info) in &current {
            match prev.get(bid) {
                None => {
                    if s.notify_bookings {
                        events.push(OutEvent {
                            kind: EventKind::BookingNew,
                            html: format!(
                                "🔔 <b>Новая запись на проверку</b>\n{}",
                                fmt_booking_line(info, me)
                            ),
                            ack_booking_id: None,
                        });
                    }
                }
                Some(old) if old.start != info.start => {
                    if s.notify_changes {
                        events.push(OutEvent {
                            kind: EventKind::BookingMoved,
                            html: format!(
                                "🔁 <b>Проверка перенесена</b>\n{}\n(было {})",
                                fmt_booking_line(info, me),
                                fmt_time(&old.start)
                            ),
                            ack_booking_id: None,
                        });
                    }
                    reminded.shift_remove(bid);
                }
                _ => {}
            }
        }

        for (bid, old) in &prev {
            if current.contains_key(bid) {
                continue;
            }
            let still_future = parse_ts(&old.start).map(|t| t > now).unwrap_or(false);
            if still_future && s.notify_changes {
                events.push(OutEvent {
                    kind: EventKind::BookingCancelled,
                    html: format!("❌ <b>Проверка отменена</b>\n{}", fmt_booking_line(old, me)),
                    ack_booking_id: None,
                });
            }
            reminded.shift_remove(bid);
        }

        if s.notify_reminders {
            for (bid, info) in &current {
                let Some(start) = parse_ts(&info.start) else {
                    continue;
                };
                let left = start - now;
                if left <= Duration::zero() {
                    continue;
                }
                let fired = reminded.entry(bid.clone()).or_default();
                let due: Vec<u32> = thresholds
                    .iter()
                    .copied()
                    .filter(|t| !fired.contains(t) && left <= Duration::minutes(*t as i64))
                    .collect();
                if due.is_empty() {
                    continue;
                }
                // пересекли сразу несколько порогов (например, запись за 10 мин
                // до старта) — шлём одно сообщение, помечаем все пороги
                let minutes = (left.num_seconds() / 60).max(1);
                let mut html = format!(
                    "⏰ <b>Проверка через {minutes} мин</b>\n{}",
                    fmt_booking_line(info, me)
                );
                let mut ack = None;
                if due.iter().min() == thresholds.last() && s.notify_alarm && !acked.contains(bid) {
                    html.push_str("\n\nНажми кнопку, иначе перед стартом включу будильник 🚨");
                    ack = Some(bid.clone());
                }
                events.push(OutEvent {
                    kind: EventKind::Reminder,
                    html,
                    ack_booking_id: ack,
                });
                fired.extend(due);
            }
        }
    }

    reminded.retain(|bid, _| current.contains_key(bid));
    snap.reminded_bookings = reminded;
    snap.bookings = Some(current);
}

fn check_feed(
    items: &[FeedItem],
    first_cycle: bool,
    snap: &mut UserSnapshot,
    events: &mut Vec<OutEvent>,
) {
    let first_run = first_cycle || snap.seen_notifications.is_none();
    let seen_prev = snap.seen_notifications.take().unwrap_or_default();
    let seen_set: HashSet<&str> = seen_prev.iter().map(String::as_str).collect();

    for n in items {
        if first_run || seen_set.contains(n.id.as_str()) {
            continue;
        }
        if SKIP_FEED_TYPES.contains(&n.related_object_type.as_str()) {
            continue; // дубль нашего же уведомления о записи/напоминания
        }
        let msg = strip_html(&n.message);
        events.push(OutEvent {
            kind: EventKind::Feed,
            html: format!(
                "🏫 <b>Школа 21</b> · {}\n{}\n🕐 {}",
                esc(&n.group_name),
                esc(&msg),
                fmt_time(&n.time)
            ),
            ack_booking_id: None,
        });
    }

    // текущие id + хвост старых (до 500) — чтобы список не рос бесконечно
    let mut new_seen: Vec<String> = items.iter().map(|n| n.id.clone()).collect();
    let keep: HashSet<&str> = new_seen.iter().map(String::as_str).collect();
    let tail: Vec<String> = seen_prev
        .iter()
        .rev()
        .take(500)
        .filter(|id| !keep.contains(id.as_str()))
        .cloned()
        .collect();
    new_seen.extend(tail.into_iter().rev());
    snap.seen_notifications = Some(new_seen);
}

fn check_deadlines(
    items: &[DeadlineItem],
    now: DateTime<Utc>,
    first_cycle: bool,
    snap: &mut UserSnapshot,
    events: &mut Vec<OutEvent>,
) {
    let known = !first_cycle && snap.deadlines.is_some();
    let prev = snap.deadlines.take().unwrap_or_default();
    let mut reminded: Vec<String> = std::mem::take(&mut snap.reminded_deadlines);
    let mut current: IndexMap<String, TsTitle> = IndexMap::new();

    for item in items {
        current.insert(
            item.id.clone(),
            TsTitle {
                ts: item.ts.clone(),
                title: item.title.clone(),
            },
        );

        match prev.get(&item.id) {
            None if known => events.push(OutEvent {
                kind: EventKind::DeadlineNew,
                html: format!(
                    "📅 <b>Новый дедлайн</b>\n{}\n🕐 {}",
                    esc(&item.title),
                    fmt_time(&item.ts)
                ),
                ack_booking_id: None,
            }),
            Some(old) if known && old.ts != item.ts => {
                events.push(OutEvent {
                    kind: EventKind::DeadlineMoved,
                    html: format!(
                        "📅 <b>Дедлайн перенесён</b>\n{}\n🕐 {} (было {})",
                        esc(&item.title),
                        fmt_time(&item.ts),
                        fmt_time(&old.ts)
                    ),
                    ack_booking_id: None,
                });
                reminded.retain(|d| d != &item.id);
            }
            _ => {}
        }

        if !first_cycle {
            if let Some(t) = parse_ts(&item.ts) {
                let left = t - now;
                if !reminded.contains(&item.id)
                    && left > Duration::zero()
                    && left <= Duration::hours(DEADLINE_REMIND_HOURS)
                {
                    let hours = (left.num_seconds() / 3600).max(1);
                    events.push(OutEvent {
                        kind: EventKind::DeadlineSoon,
                        html: format!(
                            "⚠️ <b>Дедлайн через ~{hours} ч</b>\n{}\n🕐 {}",
                            esc(&item.title),
                            fmt_time(&item.ts)
                        ),
                        ack_booking_id: None,
                    });
                    reminded.push(item.id.clone());
                }
            }
        }
    }

    reminded.retain(|d| current.contains_key(d));
    snap.reminded_deadlines = reminded;
    snap.deadlines = Some(current);
}

fn check_exams(
    items: &[ExamItem],
    now: DateTime<Utc>,
    first_cycle: bool,
    snap: &mut UserSnapshot,
    events: &mut Vec<OutEvent>,
) {
    let known = !first_cycle && snap.exams.is_some();
    let prev = snap.exams.take().unwrap_or_default();
    let mut reminded: Vec<String> = std::mem::take(&mut snap.reminded_exams);
    let mut current: IndexMap<String, TsTitle> = IndexMap::new();

    for item in items {
        current.insert(
            item.id.clone(),
            TsTitle {
                ts: item.ts.clone(),
                title: item.title.clone(),
            },
        );

        if known && !prev.contains_key(&item.id) {
            events.push(OutEvent {
                kind: EventKind::ExamNew,
                html: format!(
                    "🎓 <b>Новый экзамен</b>\n{}\n🕐 {}",
                    esc(&item.title),
                    fmt_time(&item.ts)
                ),
                ack_booking_id: None,
            });
        }

        if !first_cycle {
            if let Some(t) = parse_ts(&item.ts) {
                let left = t - now;
                if !reminded.contains(&item.id)
                    && left > Duration::zero()
                    && left <= Duration::hours(DEADLINE_REMIND_HOURS)
                {
                    events.push(OutEvent {
                        kind: EventKind::ExamSoon,
                        html: format!(
                            "🎓 <b>Экзамен уже завтра</b>\n{}\n🕐 {}",
                            esc(&item.title),
                            fmt_time(&item.ts)
                        ),
                        ack_booking_id: None,
                    });
                    reminded.push(item.id.clone());
                }
            }
        }
    }

    reminded.retain(|e| current.contains_key(e));
    snap.reminded_exams = reminded;
    snap.exams = Some(current);
}
