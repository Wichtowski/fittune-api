use axum::{
    Router,
    extract::{DefaultBodyLimit, Multipart, State},
    http::{HeaderValue, header},
    middleware,
    response::Response,
    routing::{get, post},
};
use serde_json::json;

use super::{
    model::{Extraction, ParseRequest},
    provider,
};
use crate::{
    app::AppState,
    auth::Auth,
    error::{ApiError, ApiResult},
    extract::Json,
};

const MAX_UPLOAD: usize = 4 * 1024 * 1024;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/ocr/capabilities", get(capabilities))
        .route(
            "/ocr/parse",
            post(parse).layer(DefaultBodyLimit::max(64 * 1024)),
        )
        .route(
            "/ocr/rapid",
            post(rapid).layer(DefaultBodyLimit::max(MAX_UPLOAD + 64 * 1024)),
        )
        .route(
            "/ocr/ai",
            post(ai).layer(DefaultBodyLimit::max(MAX_UPLOAD + 64 * 1024)),
        )
        .layer(middleware::map_response(
            |mut response: Response| async move {
                response
                    .headers_mut()
                    .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
                response
            },
        ))
}

async fn capabilities(State(state): State<AppState>, _auth: Auth) -> axum::Json<serde_json::Value> {
    axum::Json(
        json!({"ai":state.config.openai_key.is_some(),"rapid":state.config.ocr_endpoint.is_some()}),
    )
}

async fn parse(_auth: Auth, Json(input): Json<ParseRequest>) -> ApiResult<axum::Json<Extraction>> {
    input.validate()?;
    Ok(axum::Json(super::parser::parse(&input)))
}

struct Upload {
    image: Vec<u8>,
    text: String,
    column: Option<usize>,
}
async fn upload(mut multipart: Multipart) -> ApiResult<Upload> {
    let mut out = Upload {
        image: Vec::new(),
        text: String::new(),
        column: None,
    };
    let mut names = std::collections::HashSet::new();
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|_| ApiError::BadRequest("Invalid crop upload".into()))?
    {
        let name = field.name().unwrap_or("").to_owned();
        if !names.insert(name.clone()) {
            return Err(ApiError::validation("file", "Duplicate crop field"));
        }
        let bytes = field.bytes().await.map_err(|_| ApiError::PayloadTooLarge)?;
        match name.as_str() {
            "file" if out.image.is_empty() && bytes.len() <= MAX_UPLOAD => {
                out.image = bytes.to_vec()
            }
            "text" if bytes.len() <= 32 * 1024 => {
                out.text = String::from_utf8(bytes.to_vec())
                    .map_err(|_| ApiError::validation("text", "Invalid OCR text"))?
            }
            "column" if bytes.len() <= 3 => {
                out.column = Some(
                    std::str::from_utf8(&bytes)
                        .ok()
                        .and_then(|v| v.parse().ok())
                        .ok_or_else(|| ApiError::validation("column", "Invalid column"))?,
                )
            }
            _ => {
                return Err(ApiError::validation(
                    "file",
                    "Unexpected or oversized crop field",
                ));
            }
        }
    }
    if out.image.is_empty() {
        return Err(ApiError::validation("file", "Select a cropped label"));
    }
    out.image = tokio::task::spawn_blocking(move || provider::sanitize(&out.image))
        .await
        .map_err(|_| ApiError::OcrUnavailable("Could not prepare crop".into()))??;
    Ok(out)
}

async fn rapid(
    State(state): State<AppState>,
    auth: Auth,
    multipart: Multipart,
) -> ApiResult<axum::Json<Extraction>> {
    if state.config.ocr_endpoint.is_none() {
        return Err(ApiError::OcrUnavailable(
            "Server OCR is not configured".into(),
        ));
    }
    let _admission = state.ocr.admit(auth.user_id(), false)?;
    let upload = upload(multipart).await?;
    if !state.ocr.rapid_rate.try_record(&auth.user_id().to_string()) {
        return Err(ApiError::OcrRateLimited(60));
    }
    Ok(axum::Json(
        provider::rapid(&state, upload.image, upload.column).await?,
    ))
}

async fn ai(
    State(state): State<AppState>,
    auth: Auth,
    multipart: Multipart,
) -> ApiResult<axum::Json<Extraction>> {
    if state.config.openai_key.is_none() {
        return Err(ApiError::OcrUnavailable(
            "AI extraction is not configured".into(),
        ));
    }
    let _admission = state.ocr.admit(auth.user_id(), true)?;
    let upload = upload(multipart).await?;
    if !state.ocr.ai_rate.try_record(&auth.user_id().to_string()) {
        return Err(ApiError::OcrRateLimited(60));
    }
    super::reserve_daily(&state.db, auth.user_id()).await?;
    Ok(axum::Json(
        provider::ai(&state, upload.image, &upload.text).await?,
    ))
}
