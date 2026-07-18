//! Логин через форму Keycloak (порт kc_login из v2.1 + offline_access из спайка).
//!
//! Механика повторяет официальный веб-клиент (client_id=school21,
//! grant authorization_code), подсмотрена в github.com/s21toolkit/s21auth (MIT).
//! Пароль используется ровно один раз — сервис хранит только offline-токен.

use std::sync::OnceLock;
use std::time::Duration;

use base64::Engine;
use serde_json::Value;

use crate::error::{PlatformError, Result};
use crate::urls::PlatformUrls;

/// Ответ token endpoint. refresh_token при scope=offline_access — офлайн-токен
/// (JWT typ=Offline, без exp), живёт до смены пароля / logout all sessions.
#[derive(Debug, Clone)]
pub struct TokenSet {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in: Option<u64>,
}

/// Payload JWT без проверки подписи (нам нужны только exp/typ).
pub fn jwt_payload(token: &str) -> Option<Value> {
    let part = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(part.trim_end_matches('='))
        .ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Жив ли access-токен ещё хотя бы `margin` секунд.
pub fn token_valid(token: &str, margin: i64) -> bool {
    let Some(payload) = jwt_payload(token) else {
        return false;
    };
    let Some(exp) = payload.get("exp").and_then(Value::as_i64) else {
        return false;
    };
    exp - chrono::Utc::now().timestamp() > margin
}

pub struct AuthClient {
    urls: PlatformUrls,
}

impl AuthClient {
    pub fn new(urls: PlatformUrls) -> Self {
        Self { urls }
    }

    /// Полный логин формой: пароль → TokenSet с офлайн-токеном.
    ///
    /// Каждый вызов — свежий HTTP-клиент со своим cookie-jar: со старыми куками
    /// Keycloak уходит в SSO-редирект мимо формы логина (грабля v2).
    pub async fn login_password(&self, login: &str, password: &str) -> Result<TokenSet> {
        // платформа принимает только короткий ник, без @student.21-school.ru
        let login = login.split('@').next().unwrap_or(login).trim();

        let http = reqwest::Client::builder()
            .use_rustls_tls() // openssl-дефолт включён из-за Telegram — платформу держим на rustls
            .cookie_store(true)
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(30))
            .build()?;

        // 1. страница авторизации (редиректы следуем вручную)
        let auth_url = format!(
            "{}/auth?client_id=school21&response_mode=fragment&response_type=code\
             &scope=openid%20offline_access&state={}&nonce={}&redirect_uri={}",
            self.urls.auth_base,
            uuid::Uuid::new_v4(),
            uuid::Uuid::new_v4(),
            self.urls.redirect_uri,
        );
        let mut resp = http.get(&auth_url).send().await?;
        for _ in 0..5 {
            if !resp.status().is_redirection() {
                break;
            }
            let loc = location_of(&resp)?;
            resp = http.get(loc).send().await?;
        }
        let page = resp.text().await?;

        // 2. адрес формы логина
        let action_url = login_action_url(&page).ok_or(PlatformError::LoginFormNotFound)?;

        // 3. отправка формы и редиректы до code=
        let mut resp = http
            .post(&action_url)
            .form(&[("username", login), ("password", password)])
            .send()
            .await?;
        let mut location = resp
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let mut hops = 0;
        while !location.contains("code=") {
            if !resp.status().is_redirection() || location.is_empty() || hops >= 5 {
                return Err(PlatformError::BadCredentials);
            }
            resp = http.get(&location).send().await?;
            location = resp
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .to_string();
            hops += 1;
        }
        let code = extract_code(&location).ok_or(PlatformError::BadCredentials)?;

        // 4. обмен кода на токены
        let tok = self
            .token_request(
                &http,
                &[
                    ("grant_type", "authorization_code"),
                    ("client_id", "school21"),
                    ("code", &code),
                    ("redirect_uri", &self.urls.redirect_uri),
                ],
            )
            .await?;

        match &tok.refresh_token {
            Some(rt)
                if jwt_payload(rt)
                    .and_then(|p| p.get("typ").and_then(Value::as_str).map(String::from))
                    .as_deref()
                    == Some("Offline") =>
            {
                Ok(tok)
            }
            _ => Err(PlatformError::OfflineNotIssued),
        }
    }

    /// Access-токен по офлайн-токену, без пароля. `invalid_grant` = токен отозван.
    pub async fn refresh(&self, offline_token: &str) -> Result<TokenSet> {
        let http = reqwest::Client::builder()
            .use_rustls_tls() // см. login_password: держим платформу на rustls
            .timeout(Duration::from_secs(30))
            .build()?;
        self.token_request(
            &http,
            &[
                ("grant_type", "refresh_token"),
                ("client_id", "school21"),
                ("refresh_token", offline_token),
            ],
        )
        .await
    }

    async fn token_request(
        &self,
        http: &reqwest::Client,
        form: &[(&str, &str)],
    ) -> Result<TokenSet> {
        let resp = http
            .post(format!("{}/token", self.urls.auth_base))
            .form(form)
            .send()
            .await?;
        let body: Value = resp.json().await?;
        if let Some(access) = body.get("access_token").and_then(Value::as_str) {
            return Ok(TokenSet {
                access_token: access.to_string(),
                refresh_token: body
                    .get("refresh_token")
                    .and_then(Value::as_str)
                    .map(String::from),
                expires_in: body.get("expires_in").and_then(Value::as_u64),
            });
        }
        if body.get("error").and_then(Value::as_str) == Some("invalid_grant") {
            return Err(PlatformError::OfflineTokenDead);
        }
        // тело без токенов — печатаем только код ошибки, не содержимое
        let kind = body
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        Err(PlatformError::Token(kind))
    }
}

fn location_of(resp: &reqwest::Response) -> Result<&str> {
    resp.headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| PlatformError::Other("редирект без Location".into()))
}

/// Ищет `window.loginAction = "https://..."` либо `action="https://..."`.
pub(crate) fn login_action_url(page: &str) -> Option<String> {
    static RE_JS: OnceLock<regex::Regex> = OnceLock::new();
    static RE_FORM: OnceLock<regex::Regex> = OnceLock::new();
    // http допускается ради офлайн-стенда с wiremock; боевой Keycloak всегда https
    let re_js = RE_JS.get_or_init(|| {
        regex::Regex::new(r#"window\.loginAction\s*=\s*"(https?://[^"]+)""#).unwrap()
    });
    let re_form =
        RE_FORM.get_or_init(|| regex::Regex::new(r#"action="(https?://[^"]+)""#).unwrap());
    let m = re_js
        .captures(page)
        .or_else(|| re_form.captures(page))?
        .get(1)?
        .as_str();
    // в HTML-атрибуте амперсанды экранированы
    Some(m.replace("&amp;", "&"))
}

pub(crate) fn extract_code(location: &str) -> Option<String> {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| regex::Regex::new(r"code=([^&#]+)").unwrap());
    Some(re.captures(location)?.get(1)?.as_str().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn парсинг_login_action_из_js() {
        let page = r#"<script>window.loginAction = "https://auth.21-school.ru/auth/realms/EduPowerKeycloak/login-actions/authenticate?session_code=abc&amp;execution=def&amp;client_id=school21";</script>"#;
        assert_eq!(
            login_action_url(page).unwrap(),
            "https://auth.21-school.ru/auth/realms/EduPowerKeycloak/login-actions/authenticate?session_code=abc&execution=def&client_id=school21"
        );
    }

    #[test]
    fn парсинг_login_action_из_формы() {
        let page = r#"<form id="kc-form-login" action="https://auth.21-school.ru/x?a=1&amp;b=2" method="post">"#;
        assert_eq!(
            login_action_url(page).unwrap(),
            "https://auth.21-school.ru/x?a=1&b=2"
        );
        assert!(login_action_url("<html>ничего похожего</html>").is_none());
    }

    #[test]
    fn код_из_location() {
        assert_eq!(
            extract_code("https://platform.21-school.ru/#state=x&code=ab-cd.ef&other=1").unwrap(),
            "ab-cd.ef"
        );
        assert!(extract_code("https://platform.21-school.ru/#error=login").is_none());
    }

    #[test]
    fn jwt_payload_и_token_valid() {
        // {"exp": 9999999999, "typ": "Offline"} с мусорной подписью
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(r#"{"exp":9999999999,"typ":"Offline"}"#);
        let token = format!("head.{payload}.sig");
        assert_eq!(
            jwt_payload(&token).unwrap()["typ"].as_str().unwrap(),
            "Offline"
        );
        assert!(token_valid(&token, 60));
        assert!(!token_valid("не.жвт.вовсе", 60));
    }
}
