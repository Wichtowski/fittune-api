use axum::{Router, extract::State, routing::get};
use serde::Deserialize;

use super::{
    model::{
        Bucket, ExerciseRecord, MuscleVolume, Overview, PeriodQuery, TimelinePoint, week_streak,
    },
    repo,
};
use crate::{
    app::AppState,
    auth::Auth,
    error::{ApiError, ApiResult},
    extract::Query,
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/stats/overview", get(overview))
        .route("/stats/timeline", get(timeline))
        .route("/stats/muscles", get(muscles))
        .route("/stats/records", get(records))
}

async fn overview(
    State(state): State<AppState>,
    auth: Auth,
    Query(query): Query<PeriodQuery>,
) -> ApiResult<axum::Json<Overview>> {
    let period = query.validate()?;
    let tz = query.tz.as_str();
    let current = repo::totals(&state.db, auth.user_id(), period, tz)
        .await
        .map_err(time_zone_error)?;
    let previous = repo::totals(&state.db, auth.user_id(), period.previous(), tz).await?;
    let (weeks, current_week) = repo::active_weeks(&state.db, auth.user_id(), tz).await?;

    Ok(axum::Json(Overview {
        from: period.from,
        to: period.to,
        current,
        previous,
        streak_weeks: week_streak(&weeks, current_week),
    }))
}

#[derive(Debug, Deserialize)]
struct TimelineQuery {
    #[serde(flatten)]
    period: PeriodQuery,
    #[serde(default = "default_bucket")]
    bucket: Bucket,
}

fn default_bucket() -> Bucket {
    Bucket::Week
}

async fn timeline(
    State(state): State<AppState>,
    auth: Auth,
    Query(query): Query<TimelineQuery>,
) -> ApiResult<axum::Json<Vec<TimelinePoint>>> {
    let period = query.period.validate()?;
    let points = repo::timeline(
        &state.db,
        auth.user_id(),
        period,
        &query.period.tz,
        query.bucket,
    )
    .await
    .map_err(time_zone_error)?;
    Ok(axum::Json(points))
}

async fn muscles(
    State(state): State<AppState>,
    auth: Auth,
    Query(query): Query<PeriodQuery>,
) -> ApiResult<axum::Json<Vec<MuscleVolume>>> {
    let period = query.validate()?;
    let rows = repo::muscles(&state.db, auth.user_id(), period, &query.tz)
        .await
        .map_err(time_zone_error)?;
    Ok(axum::Json(rows))
}

async fn records(
    State(state): State<AppState>,
    auth: Auth,
) -> ApiResult<axum::Json<Vec<ExerciseRecord>>> {
    Ok(axum::Json(repo::records(&state.db, auth.user_id()).await?))
}

/// Postgres rejects unknown time zone names with `invalid_parameter_value` (22023).
fn time_zone_error(err: sqlx::Error) -> ApiError {
    match &err {
        sqlx::Error::Database(db) if db.code().as_deref() == Some("22023") => {
            ApiError::validation("tz", "Unknown time zone")
        }
        _ => err.into(),
    }
}
