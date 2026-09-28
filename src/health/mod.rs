//! FitHealth: nutrition endpoints, mounted under `/api/v1/health`. Empty until the food diary
//! lands; unknown paths fall through to the API's JSON not found error

use axum::Router;

use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
}
