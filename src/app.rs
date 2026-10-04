use std::{sync::Arc, time::Duration};

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, MatchedPath, State},
    http::{HeaderName, HeaderValue, Method, Request, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use serde_json::json;
use sqlx::PgPool;
use tower::ServiceBuilder;
use tower_http::{
    compression::CompressionLayer,
    cors::{AllowOrigin, CorsLayer},
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, RequestId, SetRequestIdLayer},
    sensitive_headers::SetSensitiveRequestHeadersLayer,
    trace::{DefaultOnResponse, TraceLayer},
};
use tracing::Level;

use crate::{
    auth, config::Config, error::ApiError, friends, health, invites, photos,
    rate_limit::RateLimiter, train, users,
};

const REQUEST_ID: HeaderName = HeaderName::from_static("x-request-id");
const MAX_BODY_BYTES: usize = 1024 * 1024;

/// Failed invite redemptions allowed per client in each window
const INVITE_FAILURES_PER_WINDOW: u32 = 10;
const INVITE_FAILURE_WINDOW: Duration = Duration::from_secs(15 * 60);

/// Failed or in-flight password attempts allowed per key in each window
const LOGIN_FAILURES_PER_WINDOW: u32 = 10;
const LOGIN_FAILURE_WINDOW: Duration = Duration::from_secs(15 * 60);

/// Open registrations allowed per client in each window
const REGISTRATIONS_PER_WINDOW: u32 = 30;
const REGISTRATION_WINDOW: Duration = Duration::from_secs(60 * 60);

/// Username lookups and friend requests allowed per user each hour. The app serves a small
/// invited group, so this only stops a runaway client
const FRIEND_ACTIONS_PER_WINDOW: u32 = 300;
const FRIEND_ACTION_WINDOW: Duration = Duration::from_secs(60 * 60);

#[derive(Clone)]
pub struct AppState {
    pub ocr: Arc<health::ocr::Runtime>,
    pub db: PgPool,
    pub config: Arc<Config>,
    pub photos: Option<Arc<dyn photos::PhotoStore>>,
    pub photo_uploads: Arc<tokio::sync::Semaphore>,
    pub invite_attempts: Arc<RateLimiter>,
    pub friend_actions: Arc<RateLimiter>,
    pub login_attempts: Arc<RateLimiter>,
    pub registration_attempts: Arc<RateLimiter>,
}

impl AppState {
    pub fn new(db: PgPool, config: Config) -> Self {
        let photos = config.photo_storage.as_ref().map(|storage| {
            Arc::new(photos::S3PhotoStore::new(storage)) as Arc<dyn photos::PhotoStore>
        });
        Self {
            ocr: Arc::new(health::ocr::Runtime::default()),
            db,
            photo_uploads: Arc::new(tokio::sync::Semaphore::new(config.photo_upload_concurrency)),
            config: Arc::new(config),
            photos,
            invite_attempts: Arc::new(RateLimiter::new(
                INVITE_FAILURES_PER_WINDOW,
                INVITE_FAILURE_WINDOW,
            )),
            friend_actions: Arc::new(RateLimiter::new(
                FRIEND_ACTIONS_PER_WINDOW,
                FRIEND_ACTION_WINDOW,
            )),
            login_attempts: Arc::new(RateLimiter::new(
                LOGIN_FAILURES_PER_WINDOW,
                LOGIN_FAILURE_WINDOW,
            )),
            registration_attempts: Arc::new(RateLimiter::new(
                REGISTRATIONS_PER_WINDOW,
                REGISTRATION_WINDOW,
            )),
        }
    }
}

pub fn router(state: AppState) -> Router {
    let api = Router::new()
        .nest("/auth", auth::router())
        .merge(users::router())
        .merge(invites::router())
        .merge(crate::admin::router())
        .merge(friends::router())
        .nest("/train", train::router())
        .nest("/health", health::router())
        .fallback(|| async { ApiError::NotFound("route") });

    let middleware = ServiceBuilder::new()
        .layer(SetSensitiveRequestHeadersLayer::new([header::AUTHORIZATION]))
        .layer(SetRequestIdLayer::new(REQUEST_ID, MakeRequestUuid))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|request: &Request<_>| {
                    let request_id = request
                        .extensions()
                        .get::<RequestId>()
                        .and_then(|id| id.header_value().to_str().ok())
                        .unwrap_or("-");
                    tracing::info_span!("request", method = %request.method(), uri = %request.uri(), request_id)
                })
                .on_response(DefaultOnResponse::new().level(Level::INFO)),
        )
        .layer(PropagateRequestIdLayer::new(REQUEST_ID))
        // The exercise library is over a megabyte of JSON; already compressed images pass through
        .layer(CompressionLayer::new())
        .layer(cors(&state.config.cors_origins))
        .layer(middleware::from_fn(request_timeout))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES));

    Router::new()
        .route("/health", get(health_check))
        .nest("/api/v1", api)
        .layer(middleware)
        .with_state(state)
}

async fn request_timeout(request: axum::extract::Request, next: Next) -> Response {
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map(MatchedPath::as_str);
    let upload = matches!(request.method(), &Method::POST | &Method::PUT)
        && matches!(
            route,
            Some("/api/v1/train/progress-photos" | "/api/v1/train/progress-photos/{id}")
        );
    let timeout = Duration::from_secs(if upload { 120 } else { 30 });
    match tokio::time::timeout(timeout, next.run(request)).await {
        Ok(response) => response,
        Err(_) => StatusCode::REQUEST_TIMEOUT.into_response(),
    }
}

fn cors(origins: &[String]) -> CorsLayer {
    let origins: Vec<HeaderValue> = origins
        .iter()
        .map(|origin| HeaderValue::from_str(origin).expect("invalid configured CORS origin"))
        .collect();
    CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
        ])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE])
        .expose_headers([REQUEST_ID, header::RETRY_AFTER])
        .max_age(Duration::from_secs(60 * 60))
}

async fn health_check(State(state): State<AppState>) -> impl IntoResponse {
    let database = sqlx::query("SELECT 1").execute(&state.db).await.is_ok();
    let status = if database {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    let body = json!({
        "status": if database { "ok" } else { "degraded" },
        "database": if database { "ok" } else { "unreachable" },
    });
    (status, Json(body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, routing::post};
    use tower::ServiceExt;

    #[tokio::test(start_paused = true)]
    async fn slow_photo_uploads_get_two_minutes_while_other_requests_get_thirty_seconds() {
        for (path, method, delay, expected) in [
            (
                "/api/v1/train/progress-photos",
                Method::POST,
                35,
                StatusCode::OK,
            ),
            (
                "/api/v1/train/progress-photos/{id}",
                Method::PUT,
                35,
                StatusCode::OK,
            ),
            (
                "/api/v1/train/progress-photos",
                Method::POST,
                121,
                StatusCode::REQUEST_TIMEOUT,
            ),
            ("/api/v1/me", Method::POST, 35, StatusCode::REQUEST_TIMEOUT),
        ] {
            let app = Router::new()
                .route(
                    path,
                    post(move || async move {
                        tokio::time::sleep(Duration::from_secs(delay)).await;
                        StatusCode::OK
                    })
                    .put(move || async move {
                        tokio::time::sleep(Duration::from_secs(delay)).await;
                        StatusCode::OK
                    }),
                )
                .layer(middleware::from_fn(request_timeout));
            let uri = path.replace("{id}", "example");
            let response = app
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(uri)
                        .body(Body::empty())
                        .expect("request"),
                )
                .await
                .expect("response");
            assert_eq!(response.status(), expected);
        }
    }
}
