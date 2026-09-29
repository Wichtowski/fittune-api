use axum::{Router, extract::State, http::StatusCode, routing::get};
use uuid::Uuid;

use super::{
    model::{Product, ProductRequest, SearchQuery},
    repo,
};
use crate::{
    app::AppState,
    auth::Auth,
    error::{ApiError, ApiResult},
    extract::{Json, Path, Query},
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/products", get(search).post(create))
        .route("/products/{id}", get(detail).put(update))
}

async fn search(
    State(state): State<AppState>,
    auth: Auth,
    Query(query): Query<SearchQuery>,
) -> ApiResult<axum::Json<Vec<Product>>> {
    let q = query.q.trim();
    if q.chars().count() > 80 {
        return Err(ApiError::validation("q", "Search is at most 80 characters"));
    }
    let limit = query.limit.clamp(1, 50);
    Ok(axum::Json(
        repo::search(&state.db, auth.user_id(), q, limit).await?,
    ))
}

async fn detail(
    State(state): State<AppState>,
    _auth: Auth,
    Path(id): Path<Uuid>,
) -> ApiResult<axum::Json<Product>> {
    repo::get(&state.db, id)
        .await?
        .map(axum::Json)
        .ok_or(ApiError::NotFound("product"))
}

async fn create(
    State(state): State<AppState>,
    auth: Auth,
    Json(input): Json<ProductRequest>,
) -> ApiResult<(StatusCode, axum::Json<Product>)> {
    let input = input.validate()?;
    let product = repo::create(&state.db, auth.user_id(), &input).await?;
    Ok((StatusCode::CREATED, axum::Json(product)))
}

async fn update(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Json(input): Json<ProductRequest>,
) -> ApiResult<axum::Json<Product>> {
    let input = input.validate()?;
    repo::update(&state.db, auth.user_id(), id, &input)
        .await?
        .map(axum::Json)
        .ok_or(ApiError::NotFound("product"))
}
