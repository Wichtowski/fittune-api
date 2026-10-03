use axum::{
    Router,
    extract::State,
    http::{HeaderValue, header},
    middleware,
    response::Response,
    routing::get,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    app::AppState,
    auth::Auth,
    error::{ApiError, ApiResult},
    extract::Json,
};

pub const OCR_MODELS: [&str; 3] = ["gpt-6-luna", "gpt-6.1-sol", "gpt-6-astra"];

#[derive(Serialize, sqlx::FromRow)]
pub struct ModelSetting {
    pub ocr_model: String,
    pub updated_by: Option<Uuid>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Serialize)]
struct Settings {
    #[serde(flatten)]
    setting: ModelSetting,
    models: &'static [&'static str],
    ai_configured: bool,
    server_ocr_configured: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateSettings {
    ocr_model: String,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/ocr-settings", get(show).put(update))
        .layer(middleware::map_response(
            |mut response: Response| async move {
                response
                    .headers_mut()
                    .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
                response
            },
        ))
}

pub async fn model_setting(db: &sqlx::PgPool) -> Result<ModelSetting, sqlx::Error> {
    sqlx::query_as(
        "SELECT ocr_model, updated_by, updated_at FROM app_settings WHERE singleton = TRUE",
    )
    .fetch_one(db)
    .await
}

async fn show(State(state): State<AppState>, auth: Auth) -> ApiResult<axum::Json<Settings>> {
    auth.require_admin()?;
    settings(&state).await
}

async fn settings(state: &AppState) -> ApiResult<axum::Json<Settings>> {
    Ok(axum::Json(Settings {
        setting: model_setting(&state.db).await?,
        models: &OCR_MODELS,
        ai_configured: state.config.openai_key.is_some(),
        server_ocr_configured: state.config.ocr_endpoint.is_some(),
    }))
}

async fn update(
    State(state): State<AppState>,
    auth: Auth,
    Json(input): Json<UpdateSettings>,
) -> ApiResult<axum::Json<Settings>> {
    auth.require_admin()?;
    if !OCR_MODELS.contains(&input.ocr_model.as_str()) {
        return Err(ApiError::validation(
            "ocr_model",
            "Choose a supported OCR model",
        ));
    }
    sqlx::query("UPDATE app_settings SET ocr_model = $1, updated_by = $2, updated_at = now() WHERE singleton = TRUE")
        .bind(&input.ocr_model).bind(auth.user_id()).execute(&state.db).await?;
    tracing::info!(admin_id = %auth.user_id(), model = %input.ocr_model, "OCR model changed");
    settings(&state).await
}
