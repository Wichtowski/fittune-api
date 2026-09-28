use std::{sync::Arc, time::Duration};

use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use fittune_api::{
    AppState,
    config::{Config, LogFormat, Registration},
    photos::PhotoStore,
    router,
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;

pub const PASSWORD: &str = "Squat#Depth1";

pub struct TestApp {
    router: Router,
    pub pool: PgPool,
}

pub struct TestUser {
    pub id: String,
    pub token: String,
}

impl TestApp {
    /// Open registration, so tests can create users freely
    pub fn new(pool: PgPool) -> Self {
        Self::with_registration_and_photos(pool, Registration::Open, None)
    }

    pub fn invite_only(pool: PgPool) -> Self {
        Self::with_registration_and_photos(pool, Registration::InviteOnly, None)
    }

    pub fn with_photos(pool: PgPool, photos: Option<Arc<dyn PhotoStore>>) -> Self {
        Self::with_registration_and_photos(pool, Registration::Open, photos)
    }

    fn with_registration_and_photos(
        pool: PgPool,
        registration: Registration,
        photos: Option<Arc<dyn PhotoStore>>,
    ) -> Self {
        let config = Config {
            database_url: String::new(),
            db_max_connections: 5,
            bind_addr: ([127, 0, 0, 1], 0).into(),
            cors_origins: vec!["http://localhost:5173".into()],
            session_ttl: Duration::from_secs(3600),
            app_version: "test".into(),
            log_format: LogFormat::Pretty,
            photo_storage: None,
            registration,
            client_ip_header: Some(header::HeaderName::from_static("x-real-ip")),
        };
        let mut state = AppState::new(pool.clone(), config);
        state.photos = photos;
        let router = router(state);
        Self { router, pool }
    }

    pub async fn raw(
        &self,
        method: Method,
        uri: &str,
        token: Option<&str>,
        content_type: Option<&str>,
        bytes: Vec<u8>,
    ) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
        let mut builder = Request::builder().method(method).uri(uri);
        if let Some(token) = token {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        if let Some(content_type) = content_type {
            builder = builder.header(header::CONTENT_TYPE, content_type);
        }
        let request = builder.body(Body::from(bytes)).expect("valid request");
        let response = self
            .router
            .clone()
            .oneshot(request)
            .await
            .expect("infallible router");
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("readable body")
            .to_bytes()
            .to_vec();
        (status, headers, bytes)
    }

    pub async fn request(
        &self,
        method: Method,
        uri: &str,
        token: Option<&str>,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        self.request_from(None, method, uri, token, body).await
    }

    /// A request as if forwarded by the proxy for client `ip`
    pub async fn request_from(
        &self,
        ip: Option<&str>,
        method: Method,
        uri: &str,
        token: Option<&str>,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder().method(method).uri(uri);
        if let Some(ip) = ip {
            builder = builder.header("x-real-ip", ip);
        }
        if let Some(token) = token {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        let request = match body {
            Some(body) => builder
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string())),
            None => builder.body(Body::empty()),
        }
        .expect("valid request");

        let response = self
            .router
            .clone()
            .oneshot(request)
            .await
            .expect("infallible router");
        let status = response.status();
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("readable body")
            .to_bytes();
        let value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes)
                .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into()))
        };
        (status, value)
    }

    pub async fn get(&self, uri: &str, token: &str) -> (StatusCode, Value) {
        self.request(Method::GET, uri, Some(token), None).await
    }

    pub async fn post(&self, uri: &str, token: Option<&str>, body: Value) -> (StatusCode, Value) {
        self.request(Method::POST, uri, token, Some(body)).await
    }

    pub async fn put(&self, uri: &str, token: &str, body: Value) -> (StatusCode, Value) {
        self.request(Method::PUT, uri, Some(token), Some(body))
            .await
    }

    pub async fn delete(&self, uri: &str, token: &str) -> (StatusCode, Value) {
        self.request(Method::DELETE, uri, Some(token), None).await
    }

    pub async fn register(&self, username: &str) -> TestUser {
        let (status, body) = self
            .post(
                "/api/v1/auth/register",
                None,
                json!({ "username": username, "email": format!("{username}@example.com"), "password": PASSWORD }),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "register failed: {body}");
        TestUser {
            id: body["user"]["id"].as_str().expect("user id").to_owned(),
            token: body["session"]["token"].as_str().expect("token").to_owned(),
        }
    }

    pub async fn make_admin(&self, user: &TestUser) {
        sqlx::query("UPDATE users SET role = 'admin' WHERE id = $1::uuid")
            .bind(&user.id)
            .execute(&self.pool)
            .await
            .expect("promote to admin");
    }

    /// Id of a catalog exercise by name.
    pub async fn catalog_exercise(&self, name: &str) -> String {
        sqlx::query_scalar::<_, uuid::Uuid>(
            "SELECT id FROM exercises WHERE owner_id IS NULL AND name = $1",
        )
        .bind(name)
        .fetch_one(&self.pool)
        .await
        .expect("catalog exercise exists")
        .to_string()
    }
}

pub fn uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}
