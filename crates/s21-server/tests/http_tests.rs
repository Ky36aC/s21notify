//! Интеграционные тесты HTTP-слоя: tower::oneshot без реальной сети,
//! Keycloak — wiremock.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use base64::Engine;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use s21_server::{config::AppConfig, db, http, poll, state::AppState};

fn test_config() -> AppConfig {
    // env не трогаем — собираем конфиг руками
    AppConfig {
        app_mode: s21_server::config::AppMode::Server,
        bind_addr: "127.0.0.1:0".into(),
        public_url: "https://example.com".into(),
        static_dir: "does-not-exist".into(),
        database_url: "sqlite::memory:".into(),
        encryption_key: base64::engine::general_purpose::STANDARD.encode([7u8; 32]),
        jwt_secret: "test-jwt-secret".into(),
        enabled_messengers: vec!["telegram".into(), "max".into()],
        tg_bot_token: Some("123:tg-token".into()),
        tg_webhook_secret: "tg-hook-secret".into(),
        tg_transport: s21_server::config::Transport::Webhook,
        max_bot_token: Some("max-token".into()),
        max_webhook_secret: "max-hook-secret".into(),
        max_transport: s21_server::config::Transport::Webhook,
        max_api_url: "https://platform-api2.max.ru".into(),
        max_html: true,
        poll_interval_sec: 90,
        deadline_poll_every: 10,
        max_concurrent_polls: 8,
        platform_rps: 5.0,
        admin_tg_chat_id: None,
        dev_fake_auth: true,
    }
}

async fn build_app() -> (axum::Router, Arc<AppState>, poll::PollReceiver) {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    let (tx, rx) = poll::channel();
    let state = AppState::build(test_config(), pool, tx).unwrap();
    (http::router(state.clone()), state, rx)
}

async fn call(router: &axum::Router, req: Request<Body>) -> (StatusCode, Value) {
    let resp = router.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, body)
}

fn post_json(uri: &str, body: Value, bearer: Option<&str>) -> Request<Body> {
    let mut b = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json");
    if let Some(t) = bearer {
        b = b.header("authorization", format!("Bearer {t}"));
    }
    b.body(Body::from(body.to_string())).unwrap()
}

// ------------------------------------------------------------------- вебхуки

#[tokio::test]
async fn вебхук_без_секрета_403() {
    let (router, _s, _rx) = build_app().await;

    let req = Request::builder()
        .method("POST")
        .uri("/webhook/telegram")
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .unwrap();
    let (status, _) = call(&router, req).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let req = Request::builder()
        .method("POST")
        .uri("/webhook/telegram")
        .header("content-type", "application/json")
        .header("x-telegram-bot-api-secret-token", "tg-hook-secret")
        .body(Body::from("{}"))
        .unwrap();
    let (status, _) = call(&router, req).await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = call(
        &router,
        post_json("/webhook/max?s=неверный", json!({}), None),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = call(
        &router,
        post_json("/webhook/max?s=max-hook-secret", json!({}), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

// ------------------------------------------------------------------ /api/auth

#[tokio::test]
async fn auth_локальный_режим_по_привязке() {
    let mut cfg = test_config();
    cfg.app_mode = s21_server::config::AppMode::Local;
    let pool = db::connect("sqlite::memory:").await.unwrap();
    let (tx, _rx) = poll::channel();
    let state = AppState::build(cfg, pool, tx).unwrap();
    let router = http::router(state.clone());

    // до /start привязки нет → 409 с подсказкой нажать /start
    let (status, _) = call(
        &router,
        post_json(
            "/api/auth",
            json!({"messenger":"telegram","init_data":"local"}),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    // /start в личном боте: запомнили chat_id
    db::remember_chat(&state.pool, "telegram", "999", "999", Some("student"))
        .await
        .unwrap();

    // теперь auth отдаёт токен, registered=false (логин Ш21 ещё не введён)
    let (status, body) = call(
        &router,
        post_json(
            "/api/auth",
            json!({"messenger":"telegram","init_data":"local"}),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["registered"], false);
    assert!(body["token"].is_string());
}

#[tokio::test]
async fn auth_dev_режим_и_битый_init_data() {
    let (router, _s, _rx) = build_app().await;

    // dev-режим включён в тестовом конфиге
    let (status, body) = call(
        &router,
        post_json(
            "/api/auth",
            json!({"messenger": "telegram", "init_data": "dev:12345"}),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["registered"], false);
    assert!(body["token"].as_str().unwrap().len() > 20);

    // настоящий initData с мусорной подписью — отказ
    let (status, _) = call(
        &router,
        post_json(
            "/api/auth",
            json!({"messenger": "telegram", "init_data": "auth_date=1&user=%7B%22id%22%3A1%7D&hash=deadbeef"}),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // /api/me без токена — 401
    let req = Request::builder()
        .method("GET")
        .uri("/api/me")
        .body(Body::empty())
        .unwrap();
    let (status, _) = call(&router, req).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

// --------------------------------------------------------- полная регистрация

/// Поддельный Keycloak: страница логина + редирект с code + token endpoint.
async fn mock_keycloak(offline_typ: &str) -> MockServer {
    let server = MockServer::start().await;
    let base = server.uri();

    Mock::given(method("GET"))
        .and(path("/auth"))
        .respond_with(ResponseTemplate::new(200).set_body_string(format!(
            r#"<script>window.loginAction = "{base}/login-actions/authenticate?session_code=x";</script>"#
        )))
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/login-actions/authenticate"))
        .and(body_string_contains("password=verysecret"))
        .respond_with(
            ResponseTemplate::new(302)
                .insert_header("location", "https://platform.21-school.ru/#code=okcode"),
        )
        .mount(&server)
        .await;
    // неверный пароль → 200 со страницей ошибки (без Location)
    Mock::given(method("POST"))
        .and(path("/login-actions/authenticate"))
        .respond_with(ResponseTemplate::new(200).set_body_string("Invalid login"))
        .mount(&server)
        .await;

    let refresh_jwt = format!(
        "h.{}.s",
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(format!(r#"{{"typ":"{offline_typ}"}}"#))
    );
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "acc-token",
            "refresh_token": refresh_jwt,
            "expires_in": 86400,
        })))
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn регистрация_перелогин_и_второй_мессенджер() {
    let kc = mock_keycloak("Offline").await;
    // AuthClient читает адреса из env — переопределяем на mock
    std::env::set_var("AUTH_BASE_URL", kc.uri());

    let (router, state, mut rx) = build_app().await;

    // токен miniapp из Telegram
    let (_, body) = call(
        &router,
        post_json(
            "/api/auth",
            json!({"messenger": "telegram", "init_data": "dev:111"}),
            None,
        ),
    )
    .await;
    let tg_jwt = body["token"].as_str().unwrap().to_string();

    // неверный пароль
    let (status, body) = call(
        &router,
        post_json(
            "/api/credentials",
            json!({"login": "ivan", "password": "wrong"}),
            Some(&tg_jwt),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");

    // успешная регистрация (логин с хвостом нормализуется)
    let (status, body) = call(
        &router,
        post_json(
            "/api/credentials",
            json!({"login": "Ivan@student.21-school.ru", "password": "verysecret"}),
            Some(&tg_jwt),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["s21_login"], "ivan");
    let tg_jwt2 = body["token"].as_str().unwrap().to_string();

    // watcher получил команду поднять таску
    let uid = match rx.try_recv().unwrap() {
        poll::PollCommand::Start(uid) => uid,
        other => panic!("ожидал Start, получил {other:?}"),
    };

    // offline-токен лежит шифрованным и расшифровывается нашим ключом
    let user = db::user_by_id(&state.pool, uid).await.unwrap().unwrap();
    let enc = user.offline_token_enc.unwrap();
    assert_ne!(enc, b"h".to_vec());
    let dec = state.cipher.decrypt(&enc).unwrap();
    assert!(dec.starts_with("h."), "расшифрованный офлайн-токен");

    // /api/me теперь registered
    let req = Request::builder()
        .method("GET")
        .uri("/api/me")
        .header("authorization", format!("Bearer {tg_jwt2}"))
        .body(Body::empty())
        .unwrap();
    let (_, me) = call(&router, req).await;
    assert_eq!(me["registered"], true);
    assert_eq!(me["s21_login"], "ivan");
    assert_eq!(me["token_status"], "ok");

    // второй мессенджер (MAX) цепляется к ТОМУ ЖЕ пользователю
    let (_, body) = call(
        &router,
        post_json(
            "/api/auth",
            json!({"messenger": "max", "init_data": "dev:222"}),
            None,
        ),
    )
    .await;
    let max_jwt = body["token"].as_str().unwrap().to_string();
    let (status, _) = call(
        &router,
        post_json(
            "/api/credentials",
            json!({"login": "ivan", "password": "verysecret"}),
            Some(&max_jwt),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        match rx.try_recv().unwrap() {
            poll::PollCommand::Start(u) => u,
            other => panic!("{other:?}"),
        },
        uid,
        "тот же пользователь"
    );
    let accs = db::active_accounts(&state.pool, uid).await.unwrap();
    assert_eq!(accs.len(), 2);

    // настройки: чтение дефолтов и нормализация порогов
    let req = Request::builder()
        .method("GET")
        .uri("/api/settings")
        .header("authorization", format!("Bearer {tg_jwt2}"))
        .body(Body::empty())
        .unwrap();
    let (_, s) = call(&router, req).await;
    assert_eq!(s["remind_minutes"], "30, 15, 3");

    let mut put = json!({
        "remind_minutes": "5;60 5", "notify_bookings": true, "notify_changes": true,
        "notify_reminders": true, "notify_feed": false, "notify_deadlines": true,
        "notify_alarm": true});
    let req = Request::builder()
        .method("PUT")
        .uri("/api/settings")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {tg_jwt2}"))
        .body(Body::from(put.take().to_string()))
        .unwrap();
    let (status, body) = call(&router, req).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["remind_minutes"], "60, 5");

    // удаление аккаунта: каскад + команда Stop
    let req = Request::builder()
        .method("DELETE")
        .uri("/api/account")
        .header("authorization", format!("Bearer {tg_jwt2}"))
        .body(Body::empty())
        .unwrap();
    let (status, _) = call(&router, req).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(db::count_users(&state.pool).await.unwrap(), 0);
    assert!(matches!(
        rx.try_recv().unwrap(),
        poll::PollCommand::Stop(u) if u == uid
    ));

    std::env::remove_var("AUTH_BASE_URL");
}

#[tokio::test]
async fn healthz_отвечает() {
    let (router, _s, _rx) = build_app().await;
    let req = Request::builder()
        .method("GET")
        .uri("/healthz")
        .body(Body::empty())
        .unwrap();
    let (status, body) = call(&router, req).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ok"], true);
    assert_eq!(body["users"], 0);
}
