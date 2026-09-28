//! Calls the real router in-process, so fixtures go through the same validation, ownership and
//! authorization checks as the app.

use anyhow::{Context, Result, bail};
use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use serde_json::Value;
use tower::ServiceExt;

const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone)]
pub struct Api {
    router: Router,
}

impl Api {
    pub fn new(router: Router) -> Self {
        Self { router }
    }

    pub async fn call(
        &self,
        method: Method,
        uri: &str,
        token: Option<&str>,
        body: Option<&Value>,
    ) -> Result<(StatusCode, Value)> {
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::USER_AGENT, "fittune-api seed-dev");
        if let Some(token) = token {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        let request = match body {
            Some(body) => builder
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string())),
            None => builder.body(Body::empty()),
        }
        .context("invalid fixture request")?;

        let response = self.router.clone().oneshot(request).await?;
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), MAX_RESPONSE_BYTES).await?;
        let value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes)
                .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into()))
        };
        Ok((status, value))
    }

    /// Like [`Api::call`], but any status outside `expected` is an error naming the request.
    pub async fn expect(
        &self,
        method: Method,
        uri: &str,
        token: Option<&str>,
        body: Option<&Value>,
        expected: &[StatusCode],
    ) -> Result<(StatusCode, Value)> {
        let (status, value) = self.call(method.clone(), uri, token, body).await?;
        if !expected.contains(&status) {
            bail!("{method} {uri} returned {status}: {value}");
        }
        Ok((status, value))
    }
}

/// Reads a string field such as an id out of a response.
pub fn text(value: &Value, pointer: &str) -> Result<String> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .with_context(|| format!("response has no `{pointer}`: {value}"))
}
