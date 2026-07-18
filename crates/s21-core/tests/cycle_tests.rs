//! Сценарии из офлайн-теста v2.1 (test_v21.py): тексты сверены посимвольно
//! с выводом watcher.py. Все времена — UTC, вывод — по Москве (+03:00).

use chrono::{DateTime, TimeZone, Utc};
use std::collections::HashSet;

use s21_core::*;

fn now() -> DateTime<Utc> {
    // 17.07.2026 10:00 UTC = 13:00 МСК
    Utc.with_ymd_and_hms(2026, 7, 17, 10, 0, 0).unwrap()
}

fn booking(id: &str, start: &str, verifier: &str, verifiable: &str) -> Booking {
    Booking {
        id: id.into(),
        info: BookingInfo {
            start: start.into(),
            task: "P02D01".into(),
            verifier: verifier.into(),
            verifiable: verifiable.into(),
            online: false,
            status: "OPEN".into(),
        },
    }
}

fn fetched_bookings(list: Vec<Booking>) -> Fetched {
    Fetched {
        bookings: Some(list),
        ..Default::default()
    }
}

fn settings() -> UserSettings {
    UserSettings::default()
}

fn no_acked() -> HashSet<String> {
    HashSet::new()
}

/// Второй цикл после молчаливого первого: снапшот из first_cycle.
fn snapshot_with(list: Vec<Booking>) -> UserSnapshot {
    run_cycle(
        &UserSnapshot::default(),
        &fetched_bookings(list),
        &settings(),
        "ivan",
        now(),
        true,
        &no_acked(),
    )
    .snapshot
}

// ---------------------------------------------------------------- first_cycle

#[test]
fn first_cycle_молчит_но_пишет_снапшот_и_active() {
    let out = run_cycle(
        &UserSnapshot::default(),
        &fetched_bookings(vec![booking(
            "b1",
            "2026-07-17T12:00:00Z",
            "peer",
            "ivan",
        )]),
        &settings(),
        "ivan",
        now(),
        true,
        &no_acked(),
    );
    assert!(out.events.is_empty());
    assert_eq!(out.snapshot.bookings.as_ref().unwrap().len(), 1);
    assert_eq!(out.active.len(), 1);
    assert_eq!(out.active[0].booking_id, "b1");
}

#[test]
fn отсутствие_ключа_в_снапшоте_тоже_молчаливая_инициализация() {
    // не first_cycle, но секции bookings в снапшоте ещё нет (например, старт после сбоя)
    let out = run_cycle(
        &UserSnapshot::default(),
        &fetched_bookings(vec![booking(
            "b1",
            "2026-07-17T12:00:00Z",
            "peer",
            "ivan",
        )]),
        &settings(),
        "ivan",
        now(),
        false,
        &no_acked(),
    );
    assert!(out.events.is_empty());
    assert!(out.snapshot.bookings.is_some());
}

// ---------------------------------------------------------------- брони

#[test]
fn новая_бронь_уведомление() {
    let prev = snapshot_with(vec![]);
    let out = run_cycle(
        &prev,
        &fetched_bookings(vec![booking(
            "b1",
            "2026-07-20T09:30:00Z",
            "peer",
            "ivan",
        )]),
        &settings(),
        "ivan",
        now(),
        false,
        &no_acked(),
    );
    assert_eq!(out.events.len(), 1);
    assert_eq!(out.events[0].kind, EventKind::BookingNew);
    assert_eq!(
        out.events[0].html,
        "🔔 <b>Новая запись на проверку</b>\n📝 Тебя проверяет <b>peer</b>\n📦 P02D01\n🕐 20.07 12:30"
    );
}

#[test]
fn роль_проверяющего_и_онлайн() {
    let mut b = booking("b1", "2026-07-20T09:30:00Z", "ivan", "victim");
    b.info.online = true;
    let line = fmt_booking_line(&b.info, "ivan");
    assert_eq!(
        line,
        "🔍 Ты проверяешь <b>victim</b> (онлайн)\n📦 P02D01\n🕐 20.07 12:30"
    );
}

#[test]
fn перенос_с_было_и_сбросом_напоминаний() {
    let mut prev = snapshot_with(vec![booking(
        "b1",
        "2026-07-20T09:30:00Z",
        "peer",
        "ivan",
    )]);
    prev.reminded_bookings.insert("b1".into(), vec![30]);
    let out = run_cycle(
        &prev,
        &fetched_bookings(vec![booking(
            "b1",
            "2026-07-21T11:00:00Z",
            "peer",
            "ivan",
        )]),
        &settings(),
        "ivan",
        now(),
        false,
        &no_acked(),
    );
    assert_eq!(out.events.len(), 1);
    assert_eq!(out.events[0].kind, EventKind::BookingMoved);
    assert_eq!(
        out.events[0].html,
        "🔁 <b>Проверка перенесена</b>\n📝 Тебя проверяет <b>peer</b>\n📦 P02D01\n🕐 21.07 14:00\n(было 20.07 12:30)"
    );
    // reminded сброшен, затем заново заведён пустым в блоке напоминаний
    assert_eq!(out.snapshot.reminded_bookings.get("b1"), Some(&vec![]));
}

#[test]
fn отмена_будущей_брони() {
    let prev = snapshot_with(vec![booking(
        "b1",
        "2026-07-20T09:30:00Z",
        "peer",
        "ivan",
    )]);
    let out = run_cycle(
        &prev,
        &fetched_bookings(vec![]),
        &settings(),
        "ivan",
        now(),
        false,
        &no_acked(),
    );
    assert_eq!(out.events.len(), 1);
    assert_eq!(out.events[0].kind, EventKind::BookingCancelled);
    assert!(out.events[0]
        .html
        .starts_with("❌ <b>Проверка отменена</b>\n"));
    assert!(out.snapshot.reminded_bookings.is_empty());
    assert!(out.active.is_empty());
}

#[test]
fn исчезнувшая_прошедшая_бронь_молчит() {
    let prev = snapshot_with(vec![booking(
        "b1",
        "2026-07-17T09:00:00Z",
        "peer",
        "ivan",
    )]);
    let out = run_cycle(
        &prev,
        &fetched_bookings(vec![]),
        &settings(),
        "ivan",
        now(),
        false,
        &no_acked(),
    );
    assert!(out.events.is_empty());
}

// ---------------------------------------------------------------- напоминания

#[test]
fn каскад_порогов_по_отдельности() {
    // проверка через 25 мин: порог 30 сработал, 15 и 3 — ещё нет
    let prev = snapshot_with(vec![booking(
        "b1",
        "2026-07-17T10:25:00Z",
        "peer",
        "ivan",
    )]);
    let out = run_cycle(
        &prev,
        &fetched_bookings(vec![booking(
            "b1",
            "2026-07-17T10:25:00Z",
            "peer",
            "ivan",
        )]),
        &settings(),
        "ivan",
        now(),
        false,
        &no_acked(),
    );
    assert_eq!(out.events.len(), 1);
    assert_eq!(out.events[0].kind, EventKind::Reminder);
    assert!(out.events[0]
        .html
        .starts_with("⏰ <b>Проверка через 25 мин</b>\n"));
    assert!(
        out.events[0].ack_booking_id.is_none(),
        "кнопка только на минимальном пороге"
    );
    assert_eq!(out.snapshot.reminded_bookings["b1"], vec![30]);
}

#[test]
fn схлопывание_пропущенных_порогов_и_кнопка() {
    // бронь появилась в снапшоте, но напоминаний ещё не было; осталось 10 мин:
    // пороги 30 и 15 пересечены разом — одно сообщение, кнопка (минимальный порог 3 не пересечён)
    let prev = snapshot_with(vec![booking(
        "b1",
        "2026-07-17T10:10:00Z",
        "peer",
        "ivan",
    )]);
    let out = run_cycle(
        &prev,
        &fetched_bookings(vec![booking(
            "b1",
            "2026-07-17T10:10:00Z",
            "peer",
            "ivan",
        )]),
        &settings(),
        "ivan",
        now(),
        false,
        &no_acked(),
    );
    assert_eq!(out.events.len(), 1);
    assert!(out.events[0]
        .html
        .starts_with("⏰ <b>Проверка через 10 мин</b>\n"));
    assert!(out.events[0].ack_booking_id.is_none());
    assert_eq!(out.snapshot.reminded_bookings["b1"], vec![30, 15]);
}

#[test]
fn минимальный_порог_даёт_кнопку_и_предупреждение() {
    let mut prev = snapshot_with(vec![booking(
        "b1",
        "2026-07-17T10:02:00Z",
        "peer",
        "ivan",
    )]);
    prev.reminded_bookings.insert("b1".into(), vec![30, 15]);
    let out = run_cycle(
        &prev,
        &fetched_bookings(vec![booking(
            "b1",
            "2026-07-17T10:02:00Z",
            "peer",
            "ivan",
        )]),
        &settings(),
        "ivan",
        now(),
        false,
        &no_acked(),
    );
    assert_eq!(out.events.len(), 1);
    assert!(out.events[0]
        .html
        .starts_with("⏰ <b>Проверка через 2 мин</b>\n"));
    assert!(out.events[0]
        .html
        .ends_with("\n\nНажми кнопку, иначе перед стартом включу будильник 🚨"));
    assert_eq!(out.events[0].ack_booking_id.as_deref(), Some("b1"));
    assert_eq!(out.snapshot.reminded_bookings["b1"], vec![30, 15, 3]);
}

#[test]
fn кнопки_нет_если_уже_подтверждено_или_будильник_выключен() {
    let mut prev = snapshot_with(vec![booking(
        "b1",
        "2026-07-17T10:02:00Z",
        "peer",
        "ivan",
    )]);
    prev.reminded_bookings.insert("b1".into(), vec![30, 15]);
    let acked: HashSet<String> = ["b1".to_string()].into();
    let out = run_cycle(
        &prev,
        &fetched_bookings(vec![booking(
            "b1",
            "2026-07-17T10:02:00Z",
            "peer",
            "ivan",
        )]),
        &settings(),
        "ivan",
        now(),
        false,
        &acked,
    );
    assert!(out.events[0].ack_booking_id.is_none());
    assert!(!out.events[0].html.contains("будильник"));

    let mut s = settings();
    s.notify_alarm = false;
    let mut prev2 = snapshot_with(vec![booking(
        "b1",
        "2026-07-17T10:02:00Z",
        "peer",
        "ivan",
    )]);
    prev2.reminded_bookings.insert("b1".into(), vec![30, 15]);
    let out2 = run_cycle(
        &prev2,
        &fetched_bookings(vec![booking(
            "b1",
            "2026-07-17T10:02:00Z",
            "peer",
            "ivan",
        )]),
        &s,
        "ivan",
        now(),
        false,
        &no_acked(),
    );
    assert!(out2.events[0].ack_booking_id.is_none());
}

#[test]
fn прошедшая_бронь_без_напоминаний() {
    let prev = snapshot_with(vec![booking(
        "b1",
        "2026-07-17T09:59:00Z",
        "peer",
        "ivan",
    )]);
    let out = run_cycle(
        &prev,
        &fetched_bookings(vec![booking(
            "b1",
            "2026-07-17T09:59:00Z",
            "peer",
            "ivan",
        )]),
        &settings(),
        "ivan",
        now(),
        false,
        &no_acked(),
    );
    assert!(out.events.is_empty());
}

#[test]
fn тумблеры_гасят_свои_события() {
    let s = UserSettings {
        notify_bookings: false,
        notify_changes: false,
        notify_reminders: false,
        ..settings()
    };
    let prev = snapshot_with(vec![booking(
        "b1",
        "2026-07-20T09:30:00Z",
        "peer",
        "ivan",
    )]);
    let out = run_cycle(
        &prev,
        &fetched_bookings(vec![
            booking("b2", "2026-07-20T10:30:00Z", "peer", "ivan"), // новая
        ]),
        &s,
        "ivan",
        now(),
        false,
        &no_acked(),
    );
    assert!(out.events.is_empty()); // ни новой, ни отмены b1, ни напоминаний
}

// ---------------------------------------------------------------- лента

fn feed_item(id: &str, typ: &str, msg: &str) -> FeedItem {
    FeedItem {
        id: id.into(),
        group_name: "Оповещения".into(),
        message: msg.into(),
        time: "2026-07-17T09:55:00Z".into(),
        related_object_type: typ.into(),
    }
}

#[test]
fn лента_скипает_calendar_dashboard_пропускает_project() {
    let mut prev = UserSnapshot::default();
    prev.seen_notifications = Some(vec!["old".into()]);
    let fetched = Fetched {
        feed: Some(vec![
            feed_item("n1", "CALENDAR", "кто-то записался"),
            feed_item("n2", "DASHBOARD", "скоро проверка"),
            feed_item("n3", "PROJECT", "<b>Оценка</b> выставлена"),
        ]),
        ..Default::default()
    };
    let out = run_cycle(
        &prev,
        &fetched,
        &settings(),
        "ivan",
        now(),
        false,
        &no_acked(),
    );
    assert_eq!(out.events.len(), 1);
    assert_eq!(out.events[0].kind, EventKind::Feed);
    assert_eq!(
        out.events[0].html,
        "🏫 <b>Школа 21</b> · Оповещения\nОценка выставлена\n🕐 17.07 12:55"
    );
    let seen = out.snapshot.seen_notifications.unwrap();
    assert!(seen.contains(&"n1".to_string()) && seen.contains(&"old".to_string()));
}

#[test]
fn лента_first_run_молчит_и_запоминает() {
    let fetched = Fetched {
        feed: Some(vec![feed_item("n1", "PROJECT", "Оценка")]),
        ..Default::default()
    };
    let out = run_cycle(
        &UserSnapshot::default(),
        &fetched,
        &settings(),
        "ivan",
        now(),
        false,
        &no_acked(),
    );
    assert!(out.events.is_empty());
    assert_eq!(
        out.snapshot.seen_notifications.unwrap(),
        vec!["n1".to_string()]
    );
}

#[test]
fn лента_обрезает_хвост_seen_до_500() {
    let mut prev = UserSnapshot::default();
    prev.seen_notifications = Some((0..600).map(|i| format!("id{i}")).collect());
    let fetched = Fetched {
        feed: Some(vec![feed_item("new1", "PROJECT", "x")]),
        ..Default::default()
    };
    let out = run_cycle(
        &prev,
        &fetched,
        &settings(),
        "ivan",
        now(),
        false,
        &no_acked(),
    );
    let seen = out.snapshot.seen_notifications.unwrap();
    assert_eq!(seen.len(), 501); // new1 + 500 последних старых
    assert!(seen.contains(&"id599".to_string()));
    assert!(!seen.contains(&"id0".to_string()));
}

// ---------------------------------------------------------------- дедлайны и экзамены

fn deadline(id: &str, ts: &str, title: &str) -> DeadlineItem {
    DeadlineItem {
        id: id.into(),
        ts: ts.into(),
        title: title.into(),
    }
}

#[test]
fn дедлайн_новый_перенос_и_напоминание() {
    let mut prev = UserSnapshot::default();
    prev.deadlines = Some(
        [(
            "d1".to_string(),
            TsTitle {
                ts: "2026-07-25T20:45:00Z".into(),
                title: "P03D01 / P03D02".into(),
            },
        )]
        .into_iter()
        .collect(),
    );

    // d1 перенесён ближе (в окно 24 ч) + появился d2
    let fetched = Fetched {
        deadlines: Some(vec![
            deadline("d1", "2026-07-18T08:00:00Z", "P03D01 / P03D02"),
            deadline("d2", "2026-08-01T20:45:00Z", "P04D01"),
        ]),
        ..Default::default()
    };
    let out = run_cycle(
        &prev,
        &fetched,
        &settings(),
        "ivan",
        now(),
        false,
        &no_acked(),
    );
    let htmls: Vec<&str> = out.events.iter().map(|e| e.html.as_str()).collect();
    assert_eq!(out.events.len(), 3);
    assert_eq!(
        htmls[0],
        "📅 <b>Дедлайн перенесён</b>\nP03D01 / P03D02\n🕐 18.07 11:00 (было 25.07 23:45)"
    );
    // перенесённый d1 теперь в окне 24 ч → сразу и ⚠️ (22 ч осталось)
    assert_eq!(
        htmls[1],
        "⚠️ <b>Дедлайн через ~22 ч</b>\nP03D01 / P03D02\n🕐 18.07 11:00"
    );
    assert_eq!(htmls[2], "📅 <b>Новый дедлайн</b>\nP04D01\n🕐 01.08 23:45");
    assert_eq!(out.snapshot.reminded_deadlines, vec!["d1".to_string()]);
}

#[test]
fn дедлайн_напоминание_не_повторяется() {
    let mut prev = UserSnapshot::default();
    prev.deadlines = Some(
        [(
            "d1".to_string(),
            TsTitle {
                ts: "2026-07-18T08:00:00Z".into(),
                title: "P03D01".into(),
            },
        )]
        .into_iter()
        .collect(),
    );
    prev.reminded_deadlines = vec!["d1".into()];
    let fetched = Fetched {
        deadlines: Some(vec![deadline("d1", "2026-07-18T08:00:00Z", "P03D01")]),
        ..Default::default()
    };
    let out = run_cycle(
        &prev,
        &fetched,
        &settings(),
        "ivan",
        now(),
        false,
        &no_acked(),
    );
    assert!(out.events.is_empty());
}

#[test]
fn экзамен_новый_и_уже_завтра() {
    let mut prev = UserSnapshot::default();
    prev.exams = Some(Default::default());
    let fetched = Fetched {
        exams: Some(vec![ExamItem {
            id: "e1".into(),
            ts: "2026-07-18T07:00:00Z".into(),
            title: "Exam 02".into(),
        }]),
        ..Default::default()
    };
    let out = run_cycle(
        &prev,
        &fetched,
        &settings(),
        "ivan",
        now(),
        false,
        &no_acked(),
    );
    assert_eq!(out.events.len(), 2);
    assert_eq!(
        out.events[0].html,
        "🎓 <b>Новый экзамен</b>\nExam 02\n🕐 18.07 10:00"
    );
    assert_eq!(
        out.events[1].html,
        "🎓 <b>Экзамен уже завтра</b>\nExam 02\n🕐 18.07 10:00"
    );
}

#[test]
fn notify_deadlines_off_не_трогает_снапшот() {
    let s = UserSettings {
        notify_deadlines: false,
        ..settings()
    };
    let fetched = Fetched {
        deadlines: Some(vec![deadline("d1", "2026-07-18T08:00:00Z", "P03D01")]),
        ..Default::default()
    };
    let out = run_cycle(
        &UserSnapshot::default(),
        &fetched,
        &s,
        "ivan",
        now(),
        false,
        &no_acked(),
    );
    assert!(out.events.is_empty());
    assert!(out.snapshot.deadlines.is_none());
}

// ---------------------------------------------------------------- утилиты

#[test]
fn parse_remind_minutes_как_в_v21() {
    assert_eq!(parse_remind_minutes("30, 15, 3"), vec![30, 15, 3]);
    assert_eq!(parse_remind_minutes("3;15 30"), vec![30, 15, 3]);
    assert_eq!(parse_remind_minutes("15, 15, 15"), vec![15]);
    assert_eq!(parse_remind_minutes("0, 721, мусор"), vec![30]);
    assert_eq!(parse_remind_minutes(""), vec![30]);
    assert_eq!(parse_remind_minutes("1, 720"), vec![720, 1]);
}

#[test]
fn esc_и_strip_html_как_в_питоне() {
    assert_eq!(esc(r#"a<b>&"c'"#), "a&lt;b&gt;&amp;&quot;c&#x27;");
    assert_eq!(strip_html("  <p>Оценка <b>5</b></p> "), "Оценка 5");
    assert_eq!(strip_html(""), "");
}

#[test]
fn fmt_time_мск_и_мусор() {
    assert_eq!(fmt_time("2026-07-17T10:00:00Z"), "17.07 13:00");
    assert_eq!(fmt_time("2026-12-31T23:30:00Z"), "01.01 02:30");
    assert_eq!(fmt_time("не время"), "не время");
}

#[test]
fn alarm_message_текст() {
    let b = booking("b1", "2026-07-17T10:00:30Z", "peer", "ivan");
    assert_eq!(
        alarm_message(&b.info, "ivan", 30),
        "🚨🚨🚨 <b>ПРОВЕРКА ЧЕРЕЗ 30 СЕК!</b>\n📝 Тебя проверяет <b>peer</b>\n📦 P02D01\n🕐 17.07 13:00"
    );
    assert_eq!(ack_payload("b1"), "ack:b1");
}

#[test]
fn снапшот_сериализуется_в_формат_state_json() {
    let prev = snapshot_with(vec![booking(
        "b1",
        "2026-07-20T09:30:00Z",
        "peer",
        "ivan",
    )]);
    let json = serde_json::to_value(&prev).unwrap();
    assert_eq!(json["bookings"]["b1"]["start"], "2026-07-20T09:30:00Z");
    assert_eq!(json["bookings"]["b1"]["verifier"], "peer");
    // и обратно
    let back: UserSnapshot = serde_json::from_value(json).unwrap();
    assert_eq!(back.bookings.unwrap()["b1"].task, "P02D01");
}
