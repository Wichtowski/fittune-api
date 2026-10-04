use axum::{Router, extract::State, http::StatusCode, routing::get};
use uuid::Uuid;

use super::{
    model::{Routine, RoutineDraft, RoutineRequest},
    repo,
};
use crate::{
    app::AppState,
    auth::Auth,
    error::{ApiError, ApiResult},
    exercises,
    extract::{Json, Path},
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/routines", get(list).post(create))
        .route("/routines/{id}", get(show).put(update).delete(delete))
}

async fn list(State(state): State<AppState>, auth: Auth) -> ApiResult<axum::Json<Vec<Routine>>> {
    Ok(axum::Json(repo::list(&state.db, auth.user_id()).await?))
}

async fn show(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> ApiResult<axum::Json<Routine>> {
    find(&state, &auth, id).await.map(axum::Json)
}

async fn create(
    State(state): State<AppState>,
    auth: Auth,
    Json(request): Json<RoutineRequest>,
) -> ApiResult<(StatusCode, axum::Json<Routine>)> {
    let draft = request.validate()?;
    ensure_exercises_visible(&state, &auth, &draft).await?;

    let mut tx = state.db.begin().await?;
    let id = repo::insert(&mut tx, auth.user_id(), &draft).await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        axum::Json(find(&state, &auth, id).await?),
    ))
}

async fn update(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Json(request): Json<RoutineRequest>,
) -> ApiResult<axum::Json<Routine>> {
    let draft = request.validate()?;
    ensure_exercises_visible(&state, &auth, &draft).await?;

    let mut tx = state.db.begin().await?;
    if !repo::update(&mut tx, auth.user_id(), id, &draft).await? {
        return Err(ApiError::NotFound("routine"));
    }
    tx.commit().await?;
    Ok(axum::Json(find(&state, &auth, id).await?))
}

async fn delete(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    if repo::delete(&state.db, auth.user_id(), id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound("routine"))
    }
}

async fn find(state: &AppState, auth: &Auth, id: Uuid) -> ApiResult<Routine> {
    repo::find(&state.db, auth.user_id(), id)
        .await?
        .ok_or(ApiError::NotFound("routine"))
}

async fn ensure_exercises_visible(
    state: &AppState,
    auth: &Auth,
    draft: &RoutineDraft,
) -> ApiResult<()> {
    let ids = draft.exercises.iter().map(|e| e.exercise_id);
    if exercises::repo::all_visible(&state.db, auth.viewer(), ids).await? {
        Ok(())
    } else {
        Err(ApiError::validation(
            "exercises",
            "Routine references an unknown exercise",
        ))
    }
}
