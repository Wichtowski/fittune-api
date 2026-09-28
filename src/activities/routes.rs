use axum::{Router, extract::State, http::StatusCode, routing::get};
use chrono::Utc;
use serde::Deserialize;
use uuid::Uuid;

use super::{
    model::{Activity, ActivityKind, ActivityRequest},
    repo::{self, UpsertOutcome},
};
use crate::{
    app::AppState,
    auth::Auth,
    error::{ApiError, ApiResult},
    extract::{Json, Path, Query},
    pagination::{self, Page},
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/activities", get(list))
        .route("/activities/{id}", get(show).put(put).delete(delete))
}

#[derive(Debug, Deserialize)]
struct ListQuery {
    kind: Option<ActivityKind>,
    cursor: Option<String>,
    limit: Option<i64>,
}

async fn list(
    State(state): State<AppState>,
    auth: Auth,
    Query(query): Query<ListQuery>,
) -> ApiResult<axum::Json<Page<Activity>>> {
    let limit = pagination::clamp_limit(query.limit);
    let before = pagination::decode_cursor(query.cursor.as_deref())?;
    let rows = repo::list(&state.db, &[auth.user_id()], query.kind, before, limit + 1).await?;
    Ok(axum::Json(pagination::paginate(rows, limit, |a| {
        (a.started_at, a.id)
    })))
}

async fn show(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> ApiResult<axum::Json<Activity>> {
    repo::find(&state.db, auth.user_id(), id)
        .await?
        .map(axum::Json)
        .ok_or(ApiError::NotFound("activity"))
}

/// Creates or replaces the activity with a client-generated id.
async fn put(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Json(request): Json<ActivityRequest>,
) -> ApiResult<(StatusCode, axum::Json<Activity>)> {
    let draft = request.validate(Utc::now())?;
    match repo::upsert(&state.db, auth.user_id(), id, &draft).await? {
        UpsertOutcome::Created(activity) => Ok((StatusCode::CREATED, axum::Json(activity))),
        UpsertOutcome::Updated(activity) => Ok((StatusCode::OK, axum::Json(activity))),
        UpsertOutcome::Foreign => Err(ApiError::NotFound("activity")),
    }
}

async fn delete(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    if repo::delete(&state.db, auth.user_id(), id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound("activity"))
    }
}
