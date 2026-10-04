//! FitHealth: nutrition endpoints, mounted under `/api/v1/health`. Unknown paths fall through
//! to the API's JSON not found error

pub mod barcode;
pub mod body;
pub mod diary;
pub mod exercise;
pub mod meals;
pub mod nutrients;
pub mod ocr;
pub mod off;
pub mod products;
pub mod targets;

use axum::Router;

use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .merge(products::router())
        .merge(meals::router())
        .merge(diary::router())
        .merge(body::router())
        .merge(ocr::router())
}
