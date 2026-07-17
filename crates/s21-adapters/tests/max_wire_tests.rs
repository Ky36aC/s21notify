//! Проверка протокола MAX против wiremock: точный формат запросов,
//! который сломал бы боевой бот, будь он другим.

use serde_json::Value;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

use s21_adapters::{FailReason, MaxAdapter, MessengerAdapter};

async fn adapter(server: &MockServer) -> MaxAdapter {
    MaxAdapter::new("max-token-123", &server.uri(), true).unwrap()
}

#[tokio::test]
async fn отправка_chat_id_в_query_и_authorization_без_bearer() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/messages"))
        .and(query_param("chat_id", "111"))
        .and(header("Authorization", "max-token-123")) // БЕЗ Bearer
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;

    let res = adapter(&server)
        .await
        .send_message("111", "<b>тест</b>", None)
        .await;
    assert!(res.ok, "{res:?}");

    // тело: text + format=html, без chat_id
    let req: Request = server.received_requests().await.unwrap().pop().unwrap();
    let body: Value = serde_json::from_slice(&req.body).unwrap();
    assert_eq!(body["text"], "<b>тест</b>");
    assert_eq!(body["format"], "html");
    assert!(body.get("chat_id").is_none(), "chat_id должен быть в query");
}

#[tokio::test]
async fn кнопка_top_level_keyboard() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/messages"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    adapter(&server)
        .await
        .send_message(
            "111",
            "напоминание",
            Some(s21_adapters::MsgButton::Ack("ack:b1")),
        )
        .await;

    let req: Request = server.received_requests().await.unwrap().pop().unwrap();
    let body: Value = serde_json::from_slice(&req.body).unwrap();
    let btn = &body["keyboard"]["buttons"][0][0];
    assert_eq!(btn["type"], "callback");
    assert_eq!(btn["payload"], "ack:b1");
    assert_eq!(btn["text"], "✅ Я за компом");
    assert!(
        body.get("attachments").is_none(),
        "клавиатура top-level, не attachment"
    );
}

#[tokio::test]
async fn выключенный_html_шлёт_чистый_текст() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/messages"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let a = MaxAdapter::new("t", &server.uri(), false).unwrap();
    a.send_message("111", "🔔 <b>Новая запись</b>", None).await;

    let req: Request = server.received_requests().await.unwrap().pop().unwrap();
    let body: Value = serde_json::from_slice(&req.body).unwrap();
    assert_eq!(body["text"], "🔔 Новая запись");
    assert!(body.get("format").is_none());
}

#[tokio::test]
async fn классификация_429_и_блокировки() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/messages"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("Retry-After", "7")
                .set_body_string("Too Many Requests"),
        )
        .expect(1)
        .mount(&server)
        .await;
    let res = adapter(&server).await.send_message("1", "x", None).await;
    assert_eq!(res.fail_reason, Some(FailReason::Flood));
    assert_eq!(res.retry_after, Some(7));
    server.reset().await;

    Mock::given(method("POST"))
        .and(path("/messages"))
        .respond_with(ResponseTemplate::new(403).set_body_string("bot is stopped by user"))
        .mount(&server)
        .await;
    let res = adapter(&server).await.send_message("1", "x", None).await;
    assert_eq!(res.fail_reason, Some(FailReason::Blocked));
}

#[tokio::test]
async fn вебхук_и_ответ_на_callback() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/subscriptions"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/answers"))
        .and(query_param("callback_id", "cb1"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;

    let a = adapter(&server).await;
    a.set_webhook("https://s21notify.tobitrix.ru/webhook/max?s=secret")
        .await
        .unwrap();

    let mut upd = s21_adapters::IncomingUpdate::new(
        s21_adapters::UpdateKind::Callback,
        "456".into(),
        "111".into(),
    );
    upd.callback_id = Some("cb1".into());
    a.ack_callback(&upd, "Принято! Хорошей проверки 🍪").await;

    // тела: у подписки url, у ответа notification
    let reqs = server.received_requests().await.unwrap();
    let sub = reqs
        .iter()
        .find(|r| r.url.path() == "/subscriptions")
        .unwrap();
    let body: Value = serde_json::from_slice(&sub.body).unwrap();
    assert_eq!(
        body["url"],
        "https://s21notify.tobitrix.ru/webhook/max?s=secret"
    );
    let ans = reqs.iter().find(|r| r.url.path() == "/answers").unwrap();
    let body: Value = serde_json::from_slice(&ans.body).unwrap();
    assert_eq!(body["notification"], "Принято! Хорошей проверки 🍪");
}
