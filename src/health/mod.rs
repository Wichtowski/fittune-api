//! FitHealth: nutrition endpoints, mounted under `/api/v1/health`. Unknown paths fall through
//! to the API's JSON not found error

pub mod exercise;
pub mod targets;

use axum::Router;

use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
}
