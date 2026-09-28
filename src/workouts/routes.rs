use axum::{Router, extract::State, http::StatusCode, routing::get};
use chrono::Utc;
use serde::Deserialize;
use uuid::Uuid;

use super::{
    model::{Workout, WorkoutRequest, WorkoutSummary},
    repo::{self, ListParams, StatusFilter, UpsertOutcome},
};
use crate::{
    app::AppState,
    auth::Auth,
    error::{ApiError, ApiResult, unique_violation},
    exercises,
    extract::{Json, Path, Query},
    pagination::{self, Page},
    places,
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/workouts", get(list))
        .route("/workouts/{id}", get(show).put(put).delete(delete))
}

#[derive(Debug, Deserialize)]
struct ListQuery {
    status: Option<StatusFilter>,
    cursor: Option<String>,
    limit: Option<i64>,
}

async fn list(
    State(state): State<AppState>,
    auth: Auth,
    Query(query): Query<ListQuery>,
) -> ApiResult<axum::Json<Page<WorkoutSummary>>> {
    let limit = pagination::clamp_limit(query.limit);
    let params = ListParams {
        status: query.status,
        before: pagination::decode_cursor(query.cursor.as_deref())?,
        limit: limit + 1,
    };
    let rows = repo::list(&state.db, &[auth.user_id()], &params).await?;
    Ok(axum::Json(pagination::paginate(rows, limit, |w| {
        (w.started_at, w.id)
    })))
}

async fn show(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> ApiResult<axum::Json<Workout>> {
    let mut conn = state.db.acquire().await?;
    repo::find(&mut conn, auth.user_id(), id)
        .await?
        .map(axum::Json)
        .ok_or(ApiError::NotFound("workout"))
}

/// Creates or replaces the workout with a client-generated id.
async fn put(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Json(request): Json<WorkoutRequest>,
) -> ApiResult<(StatusCode, axum::Json<Workout>)> {
    let draft = request.validate(Utc::now())?;

    let exercise_ids = draft.exercises.iter().map(|e| e.exercise_id);
    if !exercises::repo::all_visible(&state.db, auth.user_id(), exercise_ids).await? {
        return Err(ApiError::validation(
            "exercises",
            "Workout references an unknown exercise",
        ));
    }

    let mut tx = state.db.begin().await?;
    if let Some(Some(version_id)) = draft.place_version_id
        && !places::repo::owns_version(&mut tx, auth.user_id(), version_id).await?
    {
        return Err(ApiError::validation(
            "place_version_id",
            "Workout references an unknown place",
        ));
    }
    let outcome = repo::upsert(&mut tx, auth.user_id(), id, &draft)
        .await
        .map_err(|err| match unique_violation(&err) {
            Some(_) => ApiError::Conflict("An exercise or set id is already in use".to_owned()),
            None => err.into(),
        })?;
    let status = match outcome {
        UpsertOutcome::Created => StatusCode::CREATED,
        UpsertOutcome::Updated => StatusCode::OK,
        UpsertOutcome::Foreign => return Err(ApiError::NotFound("workout")),
        UpsertOutcome::Stale => {
            return Err(ApiError::Conflict(
                "A newer revision of this workout has already been saved".to_owned(),
            ));
        }
    };
    let workout = repo::find(&mut tx, auth.user_id(), id)
        .await?
        .ok_or(ApiError::NotFound("workout"))?;
    tx.commit().await?;
    Ok((status, axum::Json(workout)))
}

async fn delete(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    if repo::delete(&state.db, auth.user_id(), id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound("workout"))
    }
}
