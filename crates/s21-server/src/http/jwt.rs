//! JWT miniapp-сессий: HS256, TTL 1 час. Miniapp держит токен в памяти
//! и на 401 просто повторяет /api/auth.

use axum::http::HeaderMap;
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

pub const TTL_SECONDS: i64 = 3600;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    /// "telegram:12345" / "max:678"
    pub sub: String,
    pub messenger: String,
    /// ext_user_id в мессенджере
    pub ext: String,
    /// id пользователя, если привязка уже зарегистрирована
    pub uid: Option<i64>,
    pub exp: i64,
}

pub fn issue(secret: &str, messenger: &str, ext: &str, uid: Option<i64>) -> String {
    let claims = Claims {
        sub: format!("{messenger}:{ext}"),
        messenger: messenger.to_string(),
        ext: ext.to_string(),
        uid,
        exp: chrono::Utc::now().timestamp() + TTL_SECONDS,
    };
    jsonwebtoken::encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .expect("HS256 encode не падает")
}

pub fn verify(secret: &str, token: &str) -> Option<Claims> {
    jsonwebtoken::decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::default(),
    )
    .map(|d| d.claims)
    .ok()
}

/// Достаёт и проверяет Bearer-токен из заголовков.
pub fn from_headers(secret: &str, headers: &HeaderMap) -> Option<Claims> {
    let auth = headers.get("authorization")?.to_str().ok()?;
    let token = auth.strip_prefix("Bearer ")?;
    verify(secret, token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn выпуск_и_проверка() {
        let t = issue("секрет", "telegram", "123", Some(7));
        let c = verify("секрет", &t).unwrap();
        assert_eq!(c.sub, "telegram:123");
        assert_eq!(c.uid, Some(7));
        assert!(verify("другой", &t).is_none());
        assert!(verify("секрет", "мусор").is_none());
    }
}
