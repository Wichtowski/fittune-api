use axum::{
    Router,
    extract::State,
    http::StatusCode,
    routing::{get, patch, put},
};
use uuid::Uuid;

use super::{
    model::{Meal, MealRequest, OrderRequest},
    repo::{self, Outcome},
};
use crate::{
    app::AppState,
    auth::Auth,
    error::{ApiError, ApiResult},
    extract::{Json, Path},
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/meals", get(list).post(create))
        .route("/meals/order", put(reorder))
        .route("/meals/{id}", patch(rename).delete(archive))
}

async fn list(State(state): State<AppState>, auth: Auth) -> ApiResult<axum::Json<Vec<Meal>>> {
    Ok(axum::Json(repo::list(&state.db, auth.user_id()).await?))
}

async fn create(
    State(state): State<AppState>,
    auth: Auth,
    Json(input): Json<MealRequest>,
) -> ApiResult<(StatusCode, axum::Json<Meal>)> {
    let input = input.validate()?;
    match repo::create(&state.db, auth.user_id(), &input.name).await? {
        Outcome::Done(meal) => Ok((StatusCode::CREATED, axum::Json(meal))),
        Outcome::NotFound => Err(ApiError::NotFound("meal")),
        Outcome::Conflict(message) => Err(ApiError::Conflict(message.into())),
    }
}

async fn rename(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Json(input): Json<MealRequest>,
) -> ApiResult<axum::Json<Meal>> {
    let input = input.validate()?;
    repo::rename(&state.db, auth.user_id(), id, &input.name)
        .await?
        .map(axum::Json)
        .ok_or(ApiError::NotFound("meal"))
}

async fn reorder(
    State(state): State<AppState>,
    auth: Auth,
    Json(input): Json<OrderRequest>,
) -> ApiResult<axum::Json<Vec<Meal>>> {
    repo::reorder(&state.db, auth.user_id(), &input.ids)
        .await?
        .map(axum::Json)
        .ok_or_else(|| ApiError::validation("ids", "List every meal exactly once"))
}

async fn archive(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    match repo::archive(&state.db, auth.user_id(), id).await? {
        Outcome::Done(()) => Ok(StatusCode::NO_CONTENT),
        Outcome::NotFound => Err(ApiError::NotFound("meal")),
        Outcome::Conflict(message) => Err(ApiError::Conflict(message.into())),
    }
}
