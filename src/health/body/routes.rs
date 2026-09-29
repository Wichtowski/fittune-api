use axum::{
    Router,
    extract::State,
    http::StatusCode,
    routing::{get, put},
};
use chrono::NaiveDate;

use super::{
    model::{Profile, ProfileRequest, Weight, WeightRequest, WeightsQuery},
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
        .route("/profile", get(profile).put(save_profile))
        .route("/weights", get(weights))
        .route("/weights/{date}", put(save_weight).delete(delete_weight))
}

async fn profile(State(state): State<AppState>, auth: Auth) -> ApiResult<axum::Json<Profile>> {
    Ok(axum::Json(
        repo::profile(&state.db, auth.user_id())
            .await?
            .unwrap_or_default(),
    ))
}

async fn save_profile(
    State(state): State<AppState>,
    auth: Auth,
    Json(input): Json<ProfileRequest>,
) -> ApiResult<axum::Json<Profile>> {
    let input = input.validate()?;
    Ok(axum::Json(
        repo::save_profile(&state.db, auth.user_id(), &input).await?,
    ))
}

async fn weights(
    State(state): State<AppState>,
    auth: Auth,
    Query(query): Query<WeightsQuery>,
) -> ApiResult<axum::Json<Vec<Weight>>> {
    let limit = query.limit.clamp(1, 366);
    Ok(axum::Json(
        repo::weights(&state.db, auth.user_id(), limit).await?,
    ))
}

async fn save_weight(
    State(state): State<AppState>,
    auth: Auth,
    Path(date): Path<NaiveDate>,
    Json(input): Json<WeightRequest>,
) -> ApiResult<axum::Json<Weight>> {
    let input = input.validate()?;
    Ok(axum::Json(
        repo::save_weight(&state.db, auth.user_id(), date, input.weight_kg).await?,
    ))
}

async fn delete_weight(
    State(state): State<AppState>,
    auth: Auth,
    Path(date): Path<NaiveDate>,
) -> ApiResult<StatusCode> {
    if repo::delete_weight(&state.db, auth.user_id(), date).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound("weight"))
    }
}
