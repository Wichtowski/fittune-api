//! Axum extractors whose rejections are rendered as [`ApiError`] JSON bodies.

use std::{convert::Infallible, net::SocketAddr};

use axum::{
    extract::{ConnectInfo, FromRequest, FromRequestParts},
    http::request::Parts,
};

use crate::{app::AppState, error::ApiError};

#[derive(FromRequest)]
#[from_request(via(axum::Json), rejection(ApiError))]
pub struct Json<T>(pub T);

#[derive(FromRequestParts)]
#[from_request(via(axum::extract::Query), rejection(ApiError))]
pub struct Query<T>(pub T);

#[derive(FromRequestParts)]
#[from_request(via(axum::extract::Path), rejection(ApiError))]
pub struct Path<T>(pub T);

/// Identifies the client for rate limiting: the trusted proxy header when configured
/// (`FITTUNE_CLIENT_IP_HEADER`), otherwise the socket address
pub struct ClientKey(pub String);

impl FromRequestParts<AppState> for ClientKey {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Infallible> {
        let from_header = state
            .config
            .client_ip_header
            .as_ref()
            .and_then(|name| parts.headers.get(name))
            .and_then(|value| value.to_str().ok())
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        let from_socket = || {
            parts
                .extensions
                .get::<ConnectInfo<SocketAddr>>()
                .map(|ConnectInfo(addr)| addr.ip().to_string())
        };
        Ok(Self(
            from_header
                .or_else(from_socket)
                .unwrap_or_else(|| "unknown".to_owned()),
        ))
    }
}

/// Deserializes a field that distinguishes "absent" (`None`) from explicit `null`
/// (`Some(None)`), which PATCH endpoints need to clear optional values.
pub fn double_option<'de, T, D>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    T: serde::Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    serde::Deserialize::deserialize(deserializer).map(Some)
}
