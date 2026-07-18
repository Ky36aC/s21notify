//! Конфигурация из .env / переменных окружения.

fn var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

fn var_or(name: &str, default: &str) -> String {
    var(name).unwrap_or_else(|| default.to_string())
}

fn var_num<T: std::str::FromStr>(name: &str, default: T) -> T {
    var(name).and_then(|v| v.parse().ok()).unwrap_or(default)
}

/// Как бот получает апдейты. По умолчанию polling — работает без домена и
/// входящих портов (важно для РФ и локального запуска).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    Polling,
    Webhook,
}

impl Transport {
    fn parse(s: &str) -> Self {
        if s.eq_ignore_ascii_case("webhook") {
            Self::Webhook
        } else {
            Self::Polling
        }
    }
}

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub bind_addr: String,
    pub public_url: String,
    pub static_dir: String,
    pub database_url: String,
    pub encryption_key: String,
    pub jwt_secret: String,
    pub enabled_messengers: Vec<String>,
    pub tg_bot_token: Option<String>,
    pub tg_webhook_secret: String,
    pub tg_transport: Transport,
    pub max_bot_token: Option<String>,
    pub max_webhook_secret: String,
    pub max_transport: Transport,
    pub max_api_url: String,
    pub max_html: bool,
    pub poll_interval_sec: u64,
    pub deadline_poll_every: u64,
    pub max_concurrent_polls: usize,
    pub platform_rps: f64,
    pub admin_tg_chat_id: Option<String>,
    /// DEV: /api/auth принимает init_data вида "dev:<ext_id>" без подписи
    pub dev_fake_auth: bool,
}

impl AppConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        let cfg = Self {
            bind_addr: var_or("BIND_ADDR", "0.0.0.0:80"),
            public_url: var("PUBLIC_URL")
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "PUBLIC_URL не задан (публичный https-URL сервиса — кнопка miniapp и вебхуки)"
                    )
                })?
                .trim_end_matches('/')
                .to_string(),
            static_dir: var_or("STATIC_DIR", "static"),
            database_url: var_or("DATABASE_URL", "sqlite://s21notify.db"),
            encryption_key: var("ENCRYPTION_KEY")
                .ok_or_else(|| anyhow::anyhow!("ENCRYPTION_KEY не задан (base64, 32 байта)"))?,
            jwt_secret: var("JWT_SECRET").ok_or_else(|| anyhow::anyhow!("JWT_SECRET не задан"))?,
            enabled_messengers: var_or("ENABLED_MESSENGERS", "telegram,max")
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
            tg_bot_token: var("TG_BOT_TOKEN"),
            tg_webhook_secret: var_or("TG_WEBHOOK_SECRET", ""),
            tg_transport: Transport::parse(&var_or("TG_TRANSPORT", "polling")),
            max_bot_token: var("MAX_BOT_TOKEN"),
            max_webhook_secret: var_or("MAX_WEBHOOK_SECRET", ""),
            max_transport: Transport::parse(&var_or("MAX_TRANSPORT", "polling")),
            max_api_url: var_or("MAX_API_URL", s21_adapters::MAX_DEFAULT_BASE),
            max_html: var_or("MAX_HTML", "1") == "1",
            poll_interval_sec: var_num("POLL_INTERVAL_SEC", 90),
            deadline_poll_every: var_num("DEADLINE_POLL_EVERY", 10),
            max_concurrent_polls: var_num("MAX_CONCURRENT_POLLS", 8),
            platform_rps: var_num("PLATFORM_RPS", 5.0),
            admin_tg_chat_id: var("ADMIN_TG_CHAT_ID"),
            dev_fake_auth: var_or("DEV_FAKE_AUTH", "0") == "1",
        };
        Ok(cfg)
    }

    pub fn transport(&self, messenger: &str) -> Transport {
        match messenger {
            "max" => self.max_transport,
            _ => self.tg_transport,
        }
    }

    pub fn webhook_url(&self, messenger: &str) -> String {
        match messenger {
            // у MAX нет заголовка-секрета — секрет прямо в URL
            "max" => format!(
                "{}/webhook/max?s={}",
                self.public_url, self.max_webhook_secret
            ),
            m => format!("{}/webhook/{m}", self.public_url),
        }
    }
}
