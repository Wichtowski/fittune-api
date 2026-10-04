//! Axum extractors whose rejections are rendered as [`ApiError`] JSON bodies.

use std::net::{IpAddr, SocketAddr};

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
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let ip = if let Some(name) = &state.config.client_ip_header {
            let invalid = || ApiError::BadRequest("Missing or invalid client IP header".into());
            let mut values = parts.headers.get_all(name).iter();
            let value = values
                .next()
                .ok_or_else(invalid)?
                .to_str()
                .map_err(|_| invalid())?;
            if values.next().is_some() {
                return Err(invalid());
            }
            let value = if name.as_str() == "x-forwarded-for" {
                value.rsplit(',').next().ok_or_else(invalid)?
            } else {
                value
            };
            value.trim().parse::<IpAddr>().map_err(|_| invalid())?
        } else {
            parts
                .extensions
                .get::<ConnectInfo<SocketAddr>>()
                .ok_or_else(|| ApiError::BadRequest("Client address is unavailable".into()))?
                .0
                .ip()
        };
        Ok(Self(ip.to_string()))
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

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use crate::{
        AppState,
        config::{Config, LogFormat, Registration},
        extract::ClientKey,
    };
    use axum::{
        extract::{ConnectInfo, FromRequestParts},
        http::{HeaderValue, Request},
    };
    use sqlx::postgres::PgPoolOptions;

    fn config(registration: Registration) -> Config {
        Config {
            openai_key: None,
            openai_endpoint: "https://api.openai.com/v1/responses".into(),
            ocr_endpoint: None,
            database_url: String::new(),
            db_max_connections: 1,
            bind_addr: "127.0.0.1:0".parse().expect("address"),
            cors_origins: vec![],
            session_ttl: std::time::Duration::from_secs(3600),
            app_version: "test".into(),
            log_format: LogFormat::Pretty,
            photo_storage: None,
            photo_max_count: 500,
            photo_max_bytes: 512 * 1024 * 1024,
            photo_upload_concurrency: 1,
            registration,
            client_ip_header: None,
        }
    }

    #[tokio::test]
    async fn client_ip_is_canonical_and_forwarded_for_uses_the_last_hop() {
        let db = PgPoolOptions::new()
            .connect_lazy("postgres://localhost/unused")
            .expect("lazy pool");
        let mut config = config(Registration::Open);
        config.client_ip_header = Some("x-forwarded-for".parse().expect("header"));
        let state = AppState::new(db, config);
        let (mut parts, _) = Request::builder()
            .header("x-forwarded-for", "spoofed, 2001:0db8:0:0::1 ")
            .body(())
            .expect("request")
            .into_parts();
        let ClientKey(ip) = ClientKey::from_request_parts(&mut parts, &state)
            .await
            .expect("IP");
        assert_eq!(ip, "2001:db8::1");
        parts
            .headers
            .append("x-forwarded-for", HeaderValue::from_static("198.51.100.2"));
        assert!(
            ClientKey::from_request_parts(&mut parts, &state)
                .await
                .is_err()
        );
        parts.headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("198.51.100.2, bad"),
        );
        assert!(
            ClientKey::from_request_parts(&mut parts, &state)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn without_a_proxy_configuration_client_headers_are_ignored_and_socket_info_is_required()
    {
        let db = PgPoolOptions::new()
            .connect_lazy("postgres://localhost/unused")
            .expect("lazy pool");
        let mut config = config(Registration::Open);
        config.client_ip_header = None;
        let state = AppState::new(db, config);
        let (mut parts, _) = Request::builder()
            .header("x-real-ip", "203.0.113.9")
            .body(())
            .expect("request")
            .into_parts();
        assert!(
            ClientKey::from_request_parts(&mut parts, &state)
                .await
                .is_err()
        );
        parts.extensions.insert(ConnectInfo(
            "198.51.100.2:1234".parse::<SocketAddr>().expect("socket"),
        ));
        let ClientKey(ip) = ClientKey::from_request_parts(&mut parts, &state)
            .await
            .expect("IP");
        assert_eq!(ip, "198.51.100.2");
    }
}
