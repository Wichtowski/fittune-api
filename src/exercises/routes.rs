use axum::{Router, extract::State, http::StatusCode, routing::get};
use serde::Deserialize;
use uuid::Uuid;

use super::{
    history::{self, ExerciseHistory},
    model::{Equipment, Exercise, ExerciseRequest, Muscle},
    repo::{self, ExerciseFilter},
};
use crate::{
    app::AppState,
    auth::Auth,
    error::{ApiError, ApiResult, unique_violation},
    extract::{Json, Path, Query},
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/exercises", get(list).post(create))
        .route("/exercises/{id}", get(show).put(update).delete(archive))
        .route("/exercises/{id}/history", get(show_history))
}

#[derive(Debug, Deserialize)]
struct ListQuery {
    q: Option<String>,
    muscle: Option<Muscle>,
    equipment: Option<Equipment>,
}

async fn list(
    State(state): State<AppState>,
    auth: Auth,
    Query(query): Query<ListQuery>,
) -> ApiResult<axum::Json<Vec<Exercise>>> {
    let filter = ExerciseFilter {
        search: query
            .q
            .map(|q| q.trim().to_owned())
            .filter(|q| !q.is_empty()),
        muscle: query.muscle,
        equipment: query.equipment,
    };
    Ok(axum::Json(
        repo::list(&state.db, auth.user_id(), &filter).await?,
    ))
}

async fn show(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> ApiResult<axum::Json<Exercise>> {
    Ok(axum::Json(visible(&state, &auth, id).await?))
}

async fn create(
    State(state): State<AppState>,
    auth: Auth,
    Json(request): Json<ExerciseRequest>,
) -> ApiResult<(StatusCode, axum::Json<Exercise>)> {
    let owner = if request.global {
        auth.require_admin()?;
        None
    } else {
        Some(auth.user_id())
    };
    let draft = request.validate()?;
    let exercise = repo::insert(&state.db, owner, &draft)
        .await
        .map_err(duplicate_name)?;
    Ok((StatusCode::CREATED, axum::Json(exercise)))
}

async fn update(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Json(request): Json<ExerciseRequest>,
) -> ApiResult<axum::Json<Exercise>> {
    let existing = editable(&state, &auth, id).await?;
    let draft = request.validate()?;
    let exercise = repo::update(&state.db, existing.id, &draft)
        .await
        .map_err(duplicate_name)?;
    Ok(axum::Json(exercise))
}

/// Archives rather than deletes, so past workouts keep their exercise.
async fn archive(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let existing = editable(&state, &auth, id).await?;
    repo::archive(&state.db, existing.id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
struct HistoryQuery {
    #[serde(default = "default_sessions")]
    sessions: usize,
}

fn default_sessions() -> usize {
    30
}

async fn show_history(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Query(query): Query<HistoryQuery>,
) -> ApiResult<axum::Json<ExerciseHistory>> {
    let exercise = visible(&state, &auth, id).await?;
    let rows = repo::history_sets(&state.db, auth.user_id(), id).await?;
    let mut sessions = history::build_sessions(rows);
    let records = history::compute_records(&sessions);
    sessions.truncate(query.sessions.clamp(1, 200));
    Ok(axum::Json(ExerciseHistory {
        exercise,
        records,
        sessions,
    }))
}

async fn visible(state: &AppState, auth: &Auth, id: Uuid) -> ApiResult<Exercise> {
    repo::find_visible(&state.db, auth.user_id(), id)
        .await?
        .ok_or(ApiError::NotFound("exercise"))
}

/// Custom exercises can be edited by their owner, catalog exercises only by admins.
async fn editable(state: &AppState, auth: &Auth, id: Uuid) -> ApiResult<Exercise> {
    let exercise = visible(state, auth, id).await?;
    if exercise.owner_id.is_none() {
        auth.require_admin()?;
    }
    Ok(exercise)
}

fn duplicate_name(err: sqlx::Error) -> ApiError {
    match unique_violation(&err) {
        Some("exercises_owner_name_key") => {
            ApiError::validation("name", "An exercise with this name already exists")
        }
        _ => err.into(),
    }
}
