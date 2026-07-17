//! E2E-стенд watcher+alarm: wiremock вместо платформы и Keycloak,
//! mock-адаптер вместо мессенджера. Циклы дёргаются через CheckNow.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine;
use serde_json::{json, Value};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

use s21_adapters::{IncomingUpdate, MessengerAdapter, MiniappUser, MsgButton, SendResult};
use s21_server::{config::AppConfig, db, poll, state::AppState, watcher};

/// env с адресами платформы глобален — тесты сериализуются.
fn env_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

// ------------------------------------------------------------- mock adapter

#[derive(Default)]
struct MockAdapter {
    pub sent: Mutex<Vec<(String, String, bool)>>, // (chat_id, html, с_кнопкой_ack)
}

#[async_trait]
impl MessengerAdapter for MockAdapter {
    fn id(&self) -> &'static str {
        "telegram"
    }
    async fn send_message(
        &self,
        chat_id: &str,
        html: &str,
        button: Option<MsgButton<'_>>,
    ) -> SendResult {
        let ack = matches!(button, Some(MsgButton::Ack(_)));
        self.sent
            .lock()
            .unwrap()
            .push((chat_id.to_string(), html.to_string(), ack));
        SendResult::success()
    }
    fn parse_update(&self, _raw: &Value) -> Option<IncomingUpdate> {
        None
    }
    fn verify_miniapp_auth(&self, _init_data: &str) -> Option<MiniappUser> {
        None
    }
    async fn ack_callback(&self, _upd: &IncomingUpdate, _toast: &str) {}
    async fn set_webhook(&self, _url: &str) -> anyhow::Result<()> {
        Ok(())
    }
}

// ------------------------------------------------------- поддельная платформа

/// Отвечает на GraphQL по operationName; ответ броней можно менять на лету.
struct GqlResponder {
    bookings: Arc<Mutex<Value>>,
}

impl Respond for GqlResponder {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
        let op = body["operationName"].as_str().unwrap_or("");
        let data = match op {
            "calendarGetMyReviews" => {
                json!({"student": {"getMyUpcomingBookings": *self.bookings.lock().unwrap()}})
            }
            "getUserNotifications" => {
                json!({"s21Notification": {"getS21Notifications": {"notifications": []}}})
            }
            "deadlinesGetDeadlines" => json!({"student": {"getDeadlines": []}}),
            "calendarGetExams" => json!({"student": {"getExams": []}}),
            _ => Value::Null,
        };
        ResponseTemplate::new(200).set_body_json(json!({"data": data}))
    }
}

fn jwt_with_exp(exp: i64) -> String {
    format!(
        "h.{}.s",
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(format!(r#"{{"exp":{exp},"typ":"Bearer"}}"#))
    )
}

struct Platform {
    server: MockServer,
    bookings: Arc<Mutex<Value>>,
}

async fn mock_platform(refresh_dies: bool) -> Platform {
    let server = MockServer::start().await;
    let bookings = Arc::new(Mutex::new(json!([])));

    if refresh_dies {
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(400).set_body_json(
                json!({"error": "invalid_grant", "error_description": "Session not active"}),
            ))
            .mount(&server)
            .await;
    } else {
        let access = jwt_with_exp(chrono::Utc::now().timestamp() + 86400);
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": access, "expires_in": 86400
            })))
            .mount(&server)
            .await;
    }

    Mock::given(method("GET"))
        .and(path("/edu-context/context-info"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": {"contextHeaders": {"X-EDU-SCHOOL-ID": "sch"}}
        })))
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(GqlResponder {
            bookings: bookings.clone(),
        })
        .mount(&server)
        .await;

    Platform { server, bookings }
}

fn booking_json(id: &str, start: &str) -> Value {
    json!({
        "id": id,
        "eventSlot": {"start": start},
        "task": {"goalName": "P02D01"},
        "verifierUser": {"login": "peer"},
        "verifiableStudent": {"user": {"login": "floriato"}},
        "isOnline": false,
        "bookingStatus": "OPEN"
    })
}

// --------------------------------------------------------------------- стенд

struct Stand {
    state: Arc<AppState>,
    adapter: Arc<MockAdapter>,
    tx: poll::PollSender,
    uid: i64,
}

async fn build_stand(platform: &Platform) -> Stand {
    std::env::set_var("AUTH_BASE_URL", platform.server.uri());
    std::env::set_var("PLATFORM_REST_URL", platform.server.uri());
    std::env::set_var(
        "PLATFORM_GQL_URL",
        format!("{}/graphql", platform.server.uri()),
    );

    let cfg = AppConfig {
        bind_addr: "127.0.0.1:0".into(),
        public_url: "https://example.test".into(),
        static_dir: "x".into(),
        database_url: "sqlite::memory:".into(),
        encryption_key: base64::engine::general_purpose::STANDARD.encode([9u8; 32]),
        jwt_secret: "s".into(),
        enabled_messengers: vec!["telegram".into()],
        tg_bot_token: None,
        tg_webhook_secret: "x".into(),
        max_bot_token: None,
        max_webhook_secret: "x".into(),
        max_api_url: "https://platform-api2.max.ru".into(),
        max_html: true,
        poll_interval_sec: 600, // циклы дёргаем вручную через CheckNow
        deadline_poll_every: 10,
        max_concurrent_polls: 8,
        platform_rps: 100.0,
        admin_tg_chat_id: None,
        dev_fake_auth: false,
    };
    let pool = db::connect("sqlite::memory:").await.unwrap();
    let (tx, rx) = poll::channel();
    let adapter = Arc::new(MockAdapter::default());
    let mut adapters: HashMap<String, Arc<dyn MessengerAdapter>> = HashMap::new();
    adapters.insert("telegram".into(), adapter.clone());
    let state = AppState::build_with_adapters(cfg, pool, tx.clone(), Some(adapters)).unwrap();

    let enc = state.cipher.encrypt("offline-token");
    let uid = db::create_user(&state.pool, "floriato", &enc)
        .await
        .unwrap();
    db::attach_user(&state.pool, uid, "telegram", "111", "111", None)
        .await
        .unwrap();

    tokio::spawn(watcher::PollManager::new(state.clone()).run(rx));
    Stand {
        state,
        adapter,
        tx,
        uid,
    }
}

fn clear_env() {
    std::env::remove_var("AUTH_BASE_URL");
    std::env::remove_var("PLATFORM_REST_URL");
    std::env::remove_var("PLATFORM_GQL_URL");
}

impl Stand {
    fn sent(&self) -> Vec<(String, String, bool)> {
        self.adapter.sent.lock().unwrap().clone()
    }

    fn check_now(&self) {
        let _ = self.tx.send(poll::PollCommand::CheckNow(self.uid));
    }

    /// CheckNow + ждать появления подстроки в отправленных сообщениях.
    async fn cycle_until_sent(&self, needle: &str) {
        self.check_now();
        for _ in 0..100 {
            tokio::time::sleep(Duration::from_millis(50)).await;
            if self.sent().iter().any(|(_, h, _)| h.contains(needle)) {
                return;
            }
        }
        panic!("не дождался «{needle}»; отправлено: {:?}", self.sent());
    }

    /// CheckNow + ждать завершения первого цикла (по first_cycle_done).
    async fn cycle_until_first_done(&self) {
        self.check_now();
        for _ in 0..100 {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let done = db::user_by_id(&self.state.pool, self.uid)
                .await
                .ok()
                .flatten()
                .map(|u| u.first_cycle_done)
                .unwrap_or(false);
            if done {
                return;
            }
        }
        panic!("первый цикл не завершился");
    }
}

// --------------------------------------------------------------------- тесты

#[tokio::test(flavor = "multi_thread")]
async fn бронь_перенос_отмена_сквозь_watcher() {
    let _guard = env_lock().lock().await;
    let platform = mock_platform(false).await;
    let stand = build_stand(&platform).await;

    // первый цикл молчит, но пишет снапшот
    stand.cycle_until_first_done().await;
    assert!(stand.sent().is_empty(), "первый цикл должен молчать");

    // новая бронь в будущем
    *platform.bookings.lock().unwrap() = json!([booking_json("b1", "2099-07-20T09:30:00Z")]);
    stand.cycle_until_sent("Новая запись").await;
    let sent = stand.sent();
    assert!(sent[0].1.contains("🔔 <b>Новая запись на проверку</b>"));
    assert_eq!(sent[0].0, "111");

    // перенос
    *platform.bookings.lock().unwrap() = json!([booking_json("b1", "2099-07-21T11:00:00Z")]);
    stand.cycle_until_sent("перенесена").await;

    // отмена будущей брони
    *platform.bookings.lock().unwrap() = json!([]);
    stand.cycle_until_sent("отменена").await;

    // ничего лишнего не слали
    let all = stand.sent();
    assert_eq!(all.len(), 3, "{all:?}");
    clear_env();
}

#[tokio::test(flavor = "multi_thread")]
async fn мёртвый_токен_одно_сообщение_и_стоп() {
    let _guard = env_lock().lock().await;
    let platform = mock_platform(true).await;
    let stand = build_stand(&platform).await;

    stand.cycle_until_sent("отозвала доступ").await;

    // повторные CheckNow не плодят сообщений (needs_relogin — опрос стоит)
    stand.check_now();
    tokio::time::sleep(Duration::from_millis(400)).await;
    stand.check_now();
    tokio::time::sleep(Duration::from_millis(400)).await;
    let relogin_msgs = stand
        .sent()
        .iter()
        .filter(|(_, h, _)| h.contains("отозвала доступ"))
        .count();
    assert_eq!(relogin_msgs, 1);

    let user = db::user_by_id(&stand.state.pool, stand.uid)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(user.token_status, "needs_relogin");
    clear_env();
}

#[tokio::test(flavor = "multi_thread")]
async fn будильник_шлёт_и_гаснет_по_ack() {
    let _guard = env_lock().lock().await;
    let platform = mock_platform(false).await;
    let stand = build_stand(&platform).await;

    // бронь через 35 секунд уже в первом (молчаливом) цикле; второй цикл шлёт
    // схлопнутое напоминание, будильник — от alarm-таски
    let soon = (chrono::Utc::now() + chrono::Duration::seconds(35)).to_rfc3339();
    *platform.bookings.lock().unwrap() = json!([booking_json("b1", &soon)]);
    stand.cycle_until_first_done().await;
    stand.cycle_until_sent("⏰").await;

    tokio::spawn(s21_server::alarm::run(stand.state.clone()));
    for _ in 0..160 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        if stand.sent().iter().any(|(_, h, _)| h.contains("🚨🚨🚨")) {
            break;
        }
    }
    let alarm = stand
        .sent()
        .into_iter()
        .find(|(_, h, _)| h.contains("🚨🚨🚨"))
        .expect("будильник не прозвенел");
    assert!(alarm.1.contains("ПРОВЕРКА ЧЕРЕЗ"));
    assert!(alarm.2, "у будильника должна быть кнопка ack");

    // ack гасит будильник
    db::ack_booking(&stand.state.pool, stand.uid, "b1")
        .await
        .unwrap();
    let count_before = stand
        .sent()
        .iter()
        .filter(|(_, h, _)| h.contains("🚨🚨🚨"))
        .count();
    tokio::time::sleep(Duration::from_secs(11)).await;
    let count_after = stand
        .sent()
        .iter()
        .filter(|(_, h, _)| h.contains("🚨🚨🚨"))
        .count();
    assert_eq!(
        count_before, count_after,
        "после ack звонков быть не должно"
    );
    clear_env();
}
