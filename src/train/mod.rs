//! FitTune: training endpoints, mounted under `/api/v1/train`

use axum::Router;

use crate::{AppState, activities, exercises, photos, places, routines, stats, workouts};

pub fn router() -> Router<AppState> {
    Router::new()
        .merge(exercises::router())
        .merge(exercises::media::router())
        .merge(routines::router())
        .merge(places::router())
        .merge(workouts::router())
        .merge(photos::router())
        .merge(activities::router())
        .merge(stats::router())
}
