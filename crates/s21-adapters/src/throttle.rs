//! Троттлинг отправок: token-bucket на мессенджер (30/с) + минимальный
//! интервал на чат (~1/с). Backoff на 429 делает Sender по retry_after.

use std::collections::HashMap;
use tokio::sync::Mutex;
use tokio::time::{Duration, Instant};

const PER_MESSENGER_RATE: f64 = 30.0; // сообщений в секунду
const PER_CHAT_INTERVAL: Duration = Duration::from_millis(1000);

struct Bucket {
    tokens: f64,
    last: Instant,
}

#[derive(Default)]
struct Inner {
    buckets: HashMap<String, Bucket>,
    chats: HashMap<(String, String), Instant>,
}

#[derive(Default)]
pub struct Throttle {
    inner: Mutex<Inner>,
}

impl Throttle {
    pub fn new() -> Self {
        Self::default()
    }

    /// Ждёт, пока можно слать в этот чат этого мессенджера.
    pub async fn acquire(&self, messenger: &str, chat_id: &str) {
        loop {
            let wait = {
                let mut inner = self.inner.lock().await;
                let now = Instant::now();

                let bucket = inner
                    .buckets
                    .entry(messenger.to_string())
                    .or_insert(Bucket {
                        tokens: PER_MESSENGER_RATE,
                        last: now,
                    });
                bucket.tokens = (bucket.tokens
                    + now.duration_since(bucket.last).as_secs_f64() * PER_MESSENGER_RATE)
                    .min(PER_MESSENGER_RATE);
                bucket.last = now;

                let chat_key = (messenger.to_string(), chat_id.to_string());
                let chat_wait = inner
                    .chats
                    .get(&chat_key)
                    .and_then(|t| (*t + PER_CHAT_INTERVAL).checked_duration_since(now))
                    .unwrap_or(Duration::ZERO);

                let bucket_wait = if inner.buckets[messenger].tokens >= 1.0 {
                    Duration::ZERO
                } else {
                    Duration::from_secs_f64(
                        (1.0 - inner.buckets[messenger].tokens) / PER_MESSENGER_RATE,
                    )
                };

                let wait = chat_wait.max(bucket_wait);
                if wait.is_zero() {
                    inner.buckets.get_mut(messenger).unwrap().tokens -= 1.0;
                    inner.chats.insert(chat_key, now);
                    // не даём карте чатов расти бесконечно
                    if inner.chats.len() > 4096 {
                        inner
                            .chats
                            .retain(|_, t| now.duration_since(*t) < Duration::from_secs(60));
                    }
                    return;
                }
                wait
            };
            tokio::time::sleep(wait).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn один_чат_не_чаще_раза_в_секунду() {
        let t = Throttle::new();
        let start = Instant::now();
        t.acquire("telegram", "1").await;
        t.acquire("telegram", "1").await;
        t.acquire("telegram", "1").await;
        assert!(start.elapsed() >= Duration::from_millis(2000));
    }

    #[tokio::test(start_paused = true)]
    async fn разные_чаты_идут_параллельно() {
        let t = Throttle::new();
        let start = Instant::now();
        for i in 0..20 {
            t.acquire("telegram", &i.to_string()).await;
        }
        // 20 сообщений в разные чаты укладываются в бюджет 30/с
        assert!(start.elapsed() < Duration::from_millis(500));
    }
}
