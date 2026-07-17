//! Ошибки клиента платформы. Watcher различает три класса:
//! `OfflineTokenDead` (нужен перелогин пользователя), `Unauthorized`
//! (нужен refresh access-токена) и `Gql` с reason из `x-bad-request`
//! (сломался whitelist — алерт админу).

#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    #[error("Неверный логин или пароль платформы")]
    BadCredentials,

    #[error("Не нашёл форму логина Keycloak (изменилась разметка платформы?)")]
    LoginFormNotFound,

    #[error("offline-токен не выдан (Keycloak перестал разрешать offline_access?)")]
    OfflineNotIssued,

    #[error("offline-токен отозван — платформа требует новый вход (смена пароля?)")]
    OfflineTokenDead,

    #[error("нет авторизации (HTTP {0})")]
    Unauthorized(u16),

    #[error("GraphQL HTTP {status} [{op}]: {reason}")]
    Gql {
        status: u16,
        op: String,
        /// содержимое заголовка x-bad-request либо начало тела ответа
        reason: String,
    },

    #[error("GraphQL error [{op}]: {errors}")]
    GqlErrors { op: String, errors: String },

    #[error("token endpoint вернул ошибку: {0}")]
    Token(String),

    #[error("сетевая ошибка: {0}")]
    Http(#[from] reqwest::Error),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, PlatformError>;
