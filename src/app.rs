use std::{sync::Arc, time::Duration};

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    http::{HeaderName, HeaderValue, Method, Request, StatusCode, header},
    response::IntoResponse,
    routing::get,
};
use serde_json::json;
use sqlx::PgPool;
use tower::ServiceBuilder;
use tower_http::{
    cors::{AllowOrigin, CorsLayer},
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, RequestId, SetRequestIdLayer},
    sensitive_headers::SetSensitiveRequestHeadersLayer,
    timeout::TimeoutLayer,
    trace::{DefaultOnResponse, TraceLayer},
};
use tracing::Level;

use crate::{
    activities, auth, config::Config, error::ApiError, exercises, invites, places,
    rate_limit::RateLimiter, routines, stats, users, workouts,
};

const REQUEST_ID: HeaderName = HeaderName::from_static("x-request-id");
const MAX_BODY_BYTES: usize = 1024 * 1024;

/// Failed invite redemptions allowed per client in each window
const INVITE_FAILURES_PER_WINDOW: u32 = 10;
const INVITE_FAILURE_WINDOW: Duration = Duration::from_secs(15 * 60);

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub config: Arc<Config>,
    pub invite_attempts: Arc<RateLimiter>,
}

impl AppState {
    pub fn new(db: PgPool, config: Config) -> Self {
        Self {
            db,
            config: Arc::new(config),
            invite_attempts: Arc::new(RateLimiter::new(
                INVITE_FAILURES_PER_WINDOW,
                INVITE_FAILURE_WINDOW,
            )),
        }
    }
}

pub fn router(state: AppState) -> Router {
    let api = Router::new()
        .nest("/auth", auth::router())
        .merge(users::router())
        .merge(invites::router())
        .merge(exercises::router())
        .merge(routines::router())
        .merge(places::router())
        .merge(workouts::router())
        .merge(activities::router())
        .merge(stats::router())
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
        .layer(cors(&state.config.cors_origins))
        .layer(TimeoutLayer::with_status_code(StatusCode::REQUEST_TIMEOUT, Duration::from_secs(30)))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES));

    Router::new()
        .route("/health", get(health))
        .nest("/api/v1", api)
        .layer(middleware)
        .with_state(state)
}

fn cors(origins: &[String]) -> CorsLayer {
    let origins: Vec<HeaderValue> = origins
        .iter()
        .filter_map(|origin| HeaderValue::from_str(origin).ok())
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
        .expose_headers([REQUEST_ID])
        .max_age(Duration::from_secs(60 * 60))
}

async fn health(State(state): State<AppState>) -> impl IntoResponse {
    let database = sqlx::query("SELECT 1").execute(&state.db).await.is_ok();
    let status = if database {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    let body = json!({
        "status": if database { "ok" } else { "degraded" },
        "version": state.config.app_version,
        "database": if database { "ok" } else { "unreachable" },
    });
    (status, Json(body))
}
