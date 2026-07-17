//! GraphQL-клиент платформы + REST context-info.

use std::collections::HashMap;
use std::time::Duration;

use serde_json::{json, Value};

use crate::error::{PlatformError, Result};
use crate::urls::PlatformUrls;

/// Живая сессия пользователя: access-токен + context-заголовки.
/// Хранится только в памяти; после рестарта перерефрешивается.
#[derive(Debug, Clone)]
pub struct PlatformSession {
    pub access_token: String,
    pub ctx_headers: HashMap<String, String>,
}

pub struct GqlClient {
    urls: PlatformUrls,
    http: reqwest::Client,
}

impl GqlClient {
    pub fn new(urls: PlatformUrls) -> Result<Self> {
        Ok(Self {
            urls,
            // таймаут на каждый запрос свой (дедлайны — 90 с), у клиента общего нет
            http: reqwest::Client::builder().build()?,
        })
    }

    /// REST /edu-context/context-info → служебные заголовки для GraphQL.
    pub async fn context_headers(&self, access_token: &str) -> Result<HashMap<String, String>> {
        let resp = self
            .http
            .get(format!("{}/edu-context/context-info", self.urls.rest_api))
            .bearer_auth(access_token)
            .timeout(Duration::from_secs(30))
            .send()
            .await?;
        let status = resp.status();
        if status == 401 || status == 403 {
            return Err(PlatformError::Unauthorized(status.as_u16()));
        }
        if !status.is_success() {
            return Err(PlatformError::Other(format!("context-info HTTP {status}")));
        }
        let body: Value = resp.json().await?;
        let headers = body
            .pointer("/data/contextHeaders")
            .and_then(Value::as_object)
            .ok_or_else(|| PlatformError::Other("context-info без contextHeaders".into()))?;
        Ok(headers
            .iter()
            .map(|(k, v)| {
                let val = v
                    .as_str()
                    .map(String::from)
                    .unwrap_or_else(|| v.to_string());
                (k.clone(), val)
            })
            .collect())
    }

    /// Дословная GraphQL-операция. 401/403 → Unauthorized (сигнал на refresh),
    /// прочие не-2xx → Gql с заголовком x-bad-request (сломанный whitelist).
    pub async fn gql(
        &self,
        session: &PlatformSession,
        op: &str,
        query: &str,
        variables: Value,
        timeout: Duration,
    ) -> Result<Value> {
        let mut req = self
            .http
            .post(&self.urls.graphql_api)
            .bearer_auth(&session.access_token)
            .timeout(timeout)
            .json(&json!({
                "operationName": op,
                "query": query,
                "variables": variables,
            }));
        for (k, v) in &session.ctx_headers {
            req = req.header(k, v);
        }
        let resp = req.send().await?;
        let status = resp.status();
        if status == 401 || status == 403 {
            return Err(PlatformError::Unauthorized(status.as_u16()));
        }
        if !status.is_success() {
            let reason = resp
                .headers()
                .get("x-bad-request")
                .and_then(|v| v.to_str().ok())
                .map(String::from);
            let reason = match reason {
                Some(r) => r,
                None => {
                    let body = resp.text().await.unwrap_or_default();
                    body.chars().take(500).collect()
                }
            };
            return Err(PlatformError::Gql {
                status: status.as_u16(),
                op: op.to_string(),
                reason,
            });
        }
        let body: Value = resp.json().await?;
        if let Some(errors) = body.get("errors").filter(|e| !e.is_null()) {
            return Err(PlatformError::GqlErrors {
                op: op.to_string(),
                errors: errors.to_string(),
            });
        }
        body.get("data")
            .cloned()
            .ok_or_else(|| PlatformError::Other(format!("GraphQL [{op}]: ответ без data")))
    }
}
