//! Адреса платформы: дефолты боевые, каждый переопределяется через env —
//! на этом строится офлайн-стенд с wiremock.

#[derive(Debug, Clone)]
pub struct PlatformUrls {
    /// База Keycloak до .../openid-connect (без завершающего /)
    pub auth_base: String,
    pub redirect_uri: String,
    pub rest_api: String,
    pub graphql_api: String,
}

impl Default for PlatformUrls {
    fn default() -> Self {
        Self {
            auth_base:
                "https://auth.21-school.ru/auth/realms/EduPowerKeycloak/protocol/openid-connect"
                    .into(),
            redirect_uri: "https://platform.21-school.ru/".into(),
            rest_api: "https://platform.21-school.ru/services/rest".into(),
            graphql_api: "https://platform.21-school.ru/services/graphql".into(),
        }
    }
}

impl PlatformUrls {
    pub fn from_env() -> Self {
        let d = Self::default();
        Self {
            auth_base: std::env::var("AUTH_BASE_URL").unwrap_or(d.auth_base),
            redirect_uri: std::env::var("PLATFORM_REDIRECT_URI").unwrap_or(d.redirect_uri),
            rest_api: std::env::var("PLATFORM_REST_URL").unwrap_or(d.rest_api),
            graphql_api: std::env::var("PLATFORM_GQL_URL").unwrap_or(d.graphql_api),
        }
    }
}
