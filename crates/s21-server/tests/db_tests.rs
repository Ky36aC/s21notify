//! Тесты БД на sqlite::memory: — CRUD, каскады, сохранение acked при замене.

use s21_core::{ActiveBooking, BookingInfo, UserSettings};
use s21_server::db;

async fn pool() -> sqlx::SqlitePool {
    db::connect("sqlite::memory:").await.unwrap()
}

#[tokio::test]
async fn создание_пользователя_с_настройками_и_снапшотом() {
    let p = pool().await;
    let id = db::create_user(&p, "ivan", b"enc").await.unwrap();

    let u = db::user_by_login(&p, "ivan").await.unwrap().unwrap();
    assert_eq!(u.id, id);
    assert_eq!(u.token_status, "ok");
    assert_eq!(u.offline_token_enc.as_deref(), Some(b"enc".as_ref()));
    assert!(!u.first_cycle_done);

    // логин уникален без учёта регистра
    let same = db::user_by_login(&p, "IVAN").await.unwrap();
    assert!(same.is_some());
    assert!(db::create_user(&p, "Ivan", b"x").await.is_err());

    // дефолтные настройки v2.1
    let s = db::get_settings(&p, id).await.unwrap();
    assert_eq!(s.remind_minutes, "30, 15, 3");
    assert!(s.notify_alarm);

    // пустой снапшот
    let snap = db::get_snapshot(&p, id).await.unwrap();
    assert!(snap.bookings.is_none());
}

#[tokio::test]
async fn привязки_и_каскадное_удаление() {
    let p = pool().await;
    let id = db::create_user(&p, "ivan", b"enc").await.unwrap();

    // /start до регистрации: чат запомнен, user_id пуст
    db::remember_chat(&p, "telegram", "111", "111", Some("ivan_tg"))
        .await
        .unwrap();
    let acc = db::account_by_ext(&p, "telegram", "111")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(acc.user_id, None);
    assert_eq!(db::active_accounts(&p, id).await.unwrap().len(), 0);

    // регистрация в miniapp цепляет чат к пользователю (chat_id из /start сохранён)
    db::attach_user(&p, id, "telegram", "111", "fallback", Some("ivan_tg"))
        .await
        .unwrap();
    let acc = db::account_by_ext(&p, "telegram", "111")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(acc.user_id, Some(id));
    assert_eq!(
        acc.chat_id, "111",
        "chat_id из /start не затирается фолбэком"
    );

    db::attach_user(&p, id, "max", "222", "chat222", None)
        .await
        .unwrap();
    assert_eq!(db::active_accounts(&p, id).await.unwrap().len(), 2);

    // блокировка гасит доставку, повторный /start возвращает active и держит user_id
    db::set_account_status(&p, "telegram", "111", "blocked")
        .await
        .unwrap();
    assert_eq!(db::active_accounts(&p, id).await.unwrap().len(), 1);
    db::remember_chat(&p, "telegram", "111", "111", None)
        .await
        .unwrap();
    let acc = db::account_by_ext(&p, "telegram", "111")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(acc.user_id, Some(id));
    assert_eq!(db::active_accounts(&p, id).await.unwrap().len(), 2);

    // удаление пользователя сносит всё каскадом
    db::delete_user(&p, id).await.unwrap();
    assert!(db::account_by_ext(&p, "telegram", "111")
        .await
        .unwrap()
        .is_none());
    assert_eq!(db::count_users(&p).await.unwrap(), 0);
}

#[tokio::test]
async fn перепривязка_мессенджера_к_другому_аккаунту() {
    let p = pool().await;
    let u1 = db::create_user(&p, "one", b"e1").await.unwrap();
    let u2 = db::create_user(&p, "two", b"e2").await.unwrap();
    db::attach_user(&p, u1, "telegram", "111", "111", None)
        .await
        .unwrap();
    // тот же телеграм входит под другим s21-логином → привязка переезжает
    db::attach_user(&p, u2, "telegram", "111", "111", None)
        .await
        .unwrap();
    let acc = db::account_by_ext(&p, "telegram", "111")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(acc.user_id, Some(u2));
    assert_eq!(db::active_accounts(&p, u1).await.unwrap().len(), 0);

    // висящие «ожидающие» привязки свежее недели не чистятся
    db::remember_chat(&p, "max", "999", "999", None)
        .await
        .unwrap();
    assert_eq!(db::cleanup_pending_accounts(&p).await.unwrap(), 0);
}

fn ab(bid: &str, start: &str) -> ActiveBooking {
    ActiveBooking {
        booking_id: bid.into(),
        start: start.into(),
        info: BookingInfo {
            start: start.into(),
            task: "P1".into(),
            verifier: "peer".into(),
            verifiable: "ivan".into(),
            online: false,
            status: "OPEN".into(),
        },
    }
}

#[tokio::test]
async fn commit_cycle_сохраняет_acked_по_живым_броням() {
    let p = pool().await;
    let id = db::create_user(&p, "ivan", b"enc").await.unwrap();
    let snap = s21_core::UserSnapshot::default();

    db::commit_cycle(&p, id, &snap, &[ab("b1", "T1"), ab("b2", "T2")])
        .await
        .unwrap();
    db::ack_booking(&p, id, "b1").await.unwrap();
    assert_eq!(
        db::acked_bookings(&p, id).await.unwrap(),
        ["b1".to_string()].into()
    );

    // b1 остаётся (перенос времени), b2 ушла, b3 новая: acked b1 переживает замену
    db::commit_cycle(&p, id, &snap, &[ab("b1", "T1x"), ab("b3", "T3")])
        .await
        .unwrap();
    let acked = db::acked_bookings(&p, id).await.unwrap();
    assert_eq!(acked, ["b1".to_string()].into());

    // а после исчезновения b1 подтверждение забыто
    db::commit_cycle(&p, id, &snap, &[ab("b3", "T3")])
        .await
        .unwrap();
    assert!(db::acked_bookings(&p, id).await.unwrap().is_empty());

    let u = db::user_by_id(&p, id).await.unwrap().unwrap();
    assert!(u.first_cycle_done);
    assert!(u.last_poll_at.is_some());
}

#[tokio::test]
async fn alarm_candidates_учитывает_настройки() {
    let p = pool().await;
    let id = db::create_user(&p, "ivan", b"enc").await.unwrap();
    db::commit_cycle(&p, id, &Default::default(), &[ab("b1", "T1")])
        .await
        .unwrap();
    assert_eq!(db::alarm_candidates(&p).await.unwrap().len(), 1);

    // выключенный будильник исключает кандидата
    let s = UserSettings {
        notify_alarm: false,
        ..Default::default()
    };
    db::save_settings(&p, id, &s).await.unwrap();
    assert!(db::alarm_candidates(&p).await.unwrap().is_empty());

    // подтверждённая бронь тоже не кандидат
    db::save_settings(&p, id, &UserSettings::default())
        .await
        .unwrap();
    db::ack_booking(&p, id, "b1").await.unwrap();
    assert!(db::alarm_candidates(&p).await.unwrap().is_empty());
}

#[tokio::test]
async fn снапшот_сохраняется_и_читается() {
    let p = pool().await;
    let id = db::create_user(&p, "ivan", b"enc").await.unwrap();
    let mut snap = s21_core::UserSnapshot::default();
    snap.seen_notifications = Some(vec!["n1".into()]);
    db::commit_cycle(&p, id, &snap, &[]).await.unwrap();
    let back = db::get_snapshot(&p, id).await.unwrap();
    assert_eq!(back.seen_notifications.unwrap(), vec!["n1".to_string()]);
}

#[tokio::test]
async fn deliveries_журнал_и_чистка() {
    let p = pool().await;
    let id = db::create_user(&p, "ivan", b"enc").await.unwrap();
    db::log_delivery(
        &p,
        Some(id),
        "telegram",
        "reminder",
        true,
        None,
        "⏰ Проверка",
    )
    .await
    .unwrap();
    db::log_delivery(&p, Some(id), "max", "alarm", false, Some("blocked"), "🚨")
        .await
        .unwrap();
    assert_eq!(db::cleanup_deliveries(&p).await.unwrap(), 0); // свежие не трогаем
}
