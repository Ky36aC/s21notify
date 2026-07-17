//! Живая проверка отправки себе в Telegram и MAX (фаза 5).
//!
//! Нужны переменные окружения (локально или в .env, который НЕ коммитится):
//!   TG_BOT_TOKEN, TG_TEST_CHAT_ID — свой chat_id у @s21notify_bot
//!   MAX_BOT_TOKEN, MAX_TEST_CHAT_ID — свой chat_id у MAX-бота
//!   MAX_API_URL (опционально, дефолт platform-api2.max.ru)
//!
//! Запуск: cargo run -p s21-adapters --example send_test
//! Пропускает мессенджер, для которого нет переменных.

use s21_adapters::{MaxAdapter, MessengerAdapter, TelegramAdapter, MAX_DEFAULT_BASE};

const HTML: &str = "🔔 <b>Проверка s21notify v3</b>\nЖирный, <i>курсив</i> и эмодзи 🍪\n🕐 если <b>жирный</b> виден — HTML работает";

#[tokio::main]
async fn main() {
    let _ = dotenvy::dotenv();

    match (
        std::env::var("TG_BOT_TOKEN"),
        std::env::var("TG_TEST_CHAT_ID"),
    ) {
        (Ok(token), Ok(chat)) => {
            let tg = TelegramAdapter::new(&token, "unused");
            let res = tg.send_message(&chat, HTML, Some("ack:test")).await;
            println!("telegram: {res:?}");
        }
        _ => println!("telegram: пропущен (нет TG_BOT_TOKEN/TG_TEST_CHAT_ID)"),
    }

    match (
        std::env::var("MAX_BOT_TOKEN"),
        std::env::var("MAX_TEST_CHAT_ID"),
    ) {
        (Ok(token), Ok(chat)) => {
            let base =
                std::env::var("MAX_API_URL").unwrap_or_else(|_| MAX_DEFAULT_BASE.to_string());
            let max = MaxAdapter::new(&token, &base, true).unwrap();
            let res = max.send_message(&chat, HTML, Some("ack:test")).await;
            println!("max (html): {res:?}");
            if !res.ok {
                let max_plain = MaxAdapter::new(&token, &base, false).unwrap();
                let res = max_plain.send_message(&chat, HTML, Some("ack:test")).await;
                println!("max (plain): {res:?}");
            }
        }
        _ => println!("max: пропущен (нет MAX_BOT_TOKEN/MAX_TEST_CHAT_ID)"),
    }
}
