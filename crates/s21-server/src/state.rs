//! Общее состояние приложения.

use std::collections::HashMap;
use std::sync::Arc;

use sqlx::SqlitePool;

use s21_adapters::{MaxAdapter, MessengerAdapter, TelegramAdapter, Throttle};
use s21_platform::{AuthClient, GqlClient, PlatformUrls};

use crate::config::AppConfig;
use crate::crypto::TokenCipher;
use crate::poll::PollSender;
use crate::sessions::SessionManager;

pub struct AppState {
    pub cfg: AppConfig,
    pub pool: SqlitePool,
    pub cipher: TokenCipher,
    pub sessions: Arc<SessionManager>,
    pub adapters: HashMap<String, Arc<dyn MessengerAdapter>>,
    pub throttle: Arc<Throttle>,
    pub poll_tx: PollSender,
}

impl AppState {
    pub fn build(
        cfg: AppConfig,
        pool: SqlitePool,
        poll_tx: PollSender,
    ) -> anyhow::Result<Arc<Self>> {
        Self::build_with_adapters(cfg, pool, poll_tx, None)
    }

    /// Для тестов: подменить адаптеры моками.
    pub fn build_with_adapters(
        cfg: AppConfig,
        pool: SqlitePool,
        poll_tx: PollSender,
        adapters_override: Option<HashMap<String, Arc<dyn MessengerAdapter>>>,
    ) -> anyhow::Result<Arc<Self>> {
        let cipher = TokenCipher::from_base64(&cfg.encryption_key)?;
        let urls = PlatformUrls::from_env();
        let sessions = Arc::new(SessionManager::new(
            AuthClient::new(urls.clone()),
            GqlClient::new(urls)?,
            cipher.clone(),
            pool.clone(),
        ));

        let adapters = match adapters_override {
            Some(a) => a,
            None => {
                let mut adapters: HashMap<String, Arc<dyn MessengerAdapter>> = HashMap::new();
                if cfg.enabled_messengers.iter().any(|m| m == "telegram") {
                    if let Some(token) = &cfg.tg_bot_token {
                        adapters.insert(
                            "telegram".into(),
                            Arc::new(TelegramAdapter::new(token, &cfg.tg_webhook_secret)),
                        );
                    }
                }
                if cfg.enabled_messengers.iter().any(|m| m == "max") {
                    if let Some(token) = &cfg.max_bot_token {
                        adapters.insert(
                            "max".into(),
                            Arc::new(MaxAdapter::new(token, &cfg.max_api_url, cfg.max_html)?),
                        );
                    }
                }
                adapters
            }
        };

        Ok(Arc::new(Self {
            cfg,
            pool,
            cipher,
            sessions,
            adapters,
            throttle: Arc::new(Throttle::new()),
            poll_tx,
        }))
    }

    pub fn adapter(&self, messenger: &str) -> Option<Arc<dyn MessengerAdapter>> {
        self.adapters.get(messenger).cloned()
    }

    /// URL miniapp — корень публичного домена.
    pub fn miniapp_url(&self) -> String {
        self.cfg.public_url.clone()
    }
}
