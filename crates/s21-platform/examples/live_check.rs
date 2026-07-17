//! Живая проверка клиента платформы на своём аккаунте (фаза 3).
//!
//! Креды берёт из config.json v2.1 в корне репозитория (в .gitignore).
//! Печатает только метаданные — ни токенов, ни пароля.
//!
//! Запуск: cargo run -p s21-platform --example live_check

use serde_json::Value;

use s21_platform::*;

#[tokio::main]
async fn main() {
    let cfg: Value = serde_json::from_str(
        &std::fs::read_to_string("config.json").expect("нет config.json в текущем каталоге"),
    )
    .expect("битый config.json");
    let login = cfg["s21_username"].as_str().unwrap();
    let password = cfg["s21_password"].as_str().unwrap();

    let urls = PlatformUrls::from_env();
    let auth = AuthClient::new(urls.clone());
    let gql = GqlClient::new(urls).unwrap();

    println!("1. Логин с offline_access...");
    let tokens = auth.login_password(login, password).await.unwrap();
    let rt = tokens.refresh_token.clone().unwrap();
    let typ = jwt_payload(&rt).unwrap()["typ"]
        .as_str()
        .unwrap()
        .to_string();
    println!(
        "   ок: refresh typ={typ}, access expires_in={:?}с",
        tokens.expires_in
    );
    assert_eq!(typ, "Offline");

    println!("2. Refresh офлайн-токеном (как после рестарта сервиса)...");
    let fresh = auth.refresh(&rt).await.unwrap();
    println!("   ок: access expires_in={:?}с", fresh.expires_in);

    println!("3. REST context-info...");
    let ctx = gql.context_headers(&fresh.access_token).await.unwrap();
    let mut keys: Vec<_> = ctx.keys().collect();
    keys.sort();
    println!("   ок: заголовки {keys:?}");
    let session = PlatformSession {
        access_token: fresh.access_token.clone(),
        ctx_headers: ctx,
    };

    let now = chrono::Utc::now();
    println!("4. GraphQL calendarGetMyReviews...");
    let bookings = fetch_bookings(&gql, &session, now).await.unwrap();
    println!("   ок: предстоящих броней {}", bookings.len());

    println!("5. GraphQL deadlinesGetDeadlines (до 90 с)...");
    let deadlines = fetch_deadlines(&gql, &session).await.unwrap();
    println!("   ок: дедлайнов {}", deadlines.len());
    for d in &deadlines {
        println!("   - {} ({})", d.title, d.ts);
    }

    println!("6. Негатив: заведомо неверный пароль...");
    match auth.login_password(login, "заведомо-неверный").await {
        Err(PlatformError::BadCredentials) => println!("   ок: BadCredentials"),
        other => panic!("ожидал BadCredentials, получил {other:?}"),
    }

    println!();
    println!("ВЫВОД: s21-platform работает против живой платформы");
}
