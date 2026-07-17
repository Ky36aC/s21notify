//! Живые платформенные сессии пользователей (access-токен + context-заголовки).
//! Только в памяти: после рестарта перерефрешиваются офлайн-токеном из БД.

use dashmap::DashMap;
use sqlx::SqlitePool;
use std::sync::Arc;

use s21_platform::{token_valid, AuthClient, GqlClient, PlatformError, PlatformSession};

use crate::crypto::TokenCipher;
use crate::db;

/// За сколько секунд до истечения access считаем его протухшим.
const TOKEN_MARGIN_SEC: i64 = 3600;

pub struct SessionManager {
    pub auth: AuthClient,
    pub gql: GqlClient,
    cipher: TokenCipher,
    pool: SqlitePool,
    map: DashMap<i64, Arc<PlatformSession>>,
}

impl SessionManager {
    pub fn new(auth: AuthClient, gql: GqlClient, cipher: TokenCipher, pool: SqlitePool) -> Self {
        Self {
            auth,
            gql,
            cipher,
            pool,
            map: DashMap::new(),
        }
    }

    /// Сессия пользователя; при протухшем access — refresh офлайн-токеном.
    /// `OfflineTokenDead` пробрасывается наверх (watcher переведёт в needs_relogin).
    pub async fn session_for(&self, user_id: i64) -> Result<Arc<PlatformSession>, PlatformError> {
        if let Some(s) = self.map.get(&user_id) {
            if token_valid(&s.access_token, TOKEN_MARGIN_SEC) {
                return Ok(s.clone());
            }
        }
        self.refresh(user_id).await
    }

    /// Принудительный refresh (например, после 401 от GraphQL).
    pub async fn refresh(&self, user_id: i64) -> Result<Arc<PlatformSession>, PlatformError> {
        let user = db::user_by_id(&self.pool, user_id)
            .await
            .map_err(|e| PlatformError::Other(e.to_string()))?
            .ok_or_else(|| PlatformError::Other(format!("нет пользователя {user_id}")))?;
        let enc = user
            .offline_token_enc
            .ok_or(PlatformError::OfflineTokenDead)?;
        let offline = self
            .cipher
            .decrypt(&enc)
            .map_err(|_| PlatformError::OfflineTokenDead)?;

        let tokens = self.auth.refresh(&offline).await?;
        let ctx = self.gql.context_headers(&tokens.access_token).await?;
        let session = Arc::new(PlatformSession {
            access_token: tokens.access_token,
            ctx_headers: ctx,
        });
        self.map.insert(user_id, session.clone());
        Ok(session)
    }

    pub fn evict(&self, user_id: i64) {
        self.map.remove(&user_id);
    }
}
