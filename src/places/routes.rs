use axum::{
    Router,
    extract::State,
    http::StatusCode,
    routing::{get, put},
};
use uuid::Uuid;

use super::{
    model::{Place, PlaceRequest},
    repo,
};
use crate::{
    app::AppState,
    auth::Auth,
    error::{ApiError, ApiResult},
    extract::{Json, Path},
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/places", get(list))
        .route("/places/{id}", put(save).delete(archive))
}

async fn list(State(state): State<AppState>, auth: Auth) -> ApiResult<axum::Json<Vec<Place>>> {
    Ok(axum::Json(repo::list(&state.db, auth.user_id()).await?))
}

async fn save(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Json(input): Json<PlaceRequest>,
) -> ApiResult<(StatusCode, axum::Json<Place>)> {
    let input = input.validate()?;
    let mut tx = state.db.begin().await?;
    let (created, place) = match repo::save(&mut tx, auth.user_id(), id, &input).await? {
        repo::SaveOutcome::Saved { created, place } => (created, place),
        repo::SaveOutcome::NotFound => return Err(ApiError::NotFound("place")),
        repo::SaveOutcome::LimitReached => {
            return Err(ApiError::Conflict(format!(
                "You can have at most {} places. Archive one to add another.",
                repo::MAX_PLACES
            )));
        }
    };
    tx.commit().await?;
    Ok((
        if created {
            StatusCode::CREATED
        } else {
            StatusCode::OK
        },
        axum::Json(place),
    ))
}

async fn archive(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    if repo::archive(&state.db, auth.user_id(), id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound("place"))
    }
}
