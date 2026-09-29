use axum::{Router, extract::State, http::StatusCode, routing::get};
use uuid::Uuid;

use super::{
    model::{Lookup, Product, ProductRequest, SearchQuery, SearchResults},
    repo,
};
use crate::{
    app::AppState,
    auth::Auth,
    error::{ApiError, ApiResult},
    extract::{Json, Path, Query},
    health::barcode,
};

/// Imported listings shown next to FitHealth products in search
const OFF_RESULTS: i64 = 15;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/products", get(search).post(create))
        .route("/products/barcode/{code}", get(by_barcode))
        .route("/products/{id}", get(detail).put(update))
}

async fn search(
    State(state): State<AppState>,
    auth: Auth,
    Query(query): Query<SearchQuery>,
) -> ApiResult<axum::Json<SearchResults>> {
    let q = query.q.trim();
    if q.chars().count() > 80 {
        return Err(ApiError::validation("q", "Search is at most 80 characters"));
    }
    let limit = query.limit.clamp(1, 50);
    // One transaction, so the looser similarity threshold applies to both queries and no more
    let mut tx = state.db.begin().await?;
    let products = repo::search(&mut tx, auth.user_id(), q, limit).await?;
    let off = if q.chars().count() >= 2 {
        repo::search_off(&mut tx, q, OFF_RESULTS).await?
    } else {
        Vec::new()
    };
    tx.commit().await?;
    Ok(axum::Json(SearchResults { products, off }))
}

/// FitHealth's own product first, then the imported Open Food Facts listing
async fn by_barcode(
    State(state): State<AppState>,
    _auth: Auth,
    Path(code): Path<String>,
) -> ApiResult<axum::Json<Lookup>> {
    let code = barcode::normalize(&code)
        .ok_or_else(|| ApiError::validation("code", "Not a valid barcode"))?;
    if let Some(product) = repo::by_barcode(&state.db, &code).await? {
        return Ok(axum::Json(Lookup::Found { product }));
    }
    Ok(axum::Json(
        match repo::off_by_barcode(&state.db, &code).await? {
            Some(candidate) => Lookup::Off { candidate },
            None => Lookup::NotFound,
        },
    ))
}

async fn detail(
    State(state): State<AppState>,
    _auth: Auth,
    Path(id): Path<Uuid>,
) -> ApiResult<axum::Json<Product>> {
    repo::get(&state.db, id)
        .await?
        .map(axum::Json)
        .ok_or(ApiError::NotFound("product"))
}

async fn create(
    State(state): State<AppState>,
    auth: Auth,
    Json(input): Json<ProductRequest>,
) -> ApiResult<(StatusCode, axum::Json<Product>)> {
    let input = input.validate()?;
    let product = repo::create(&state.db, auth.user_id(), &input)
        .await
        .map_err(barcode_taken)?;
    Ok((StatusCode::CREATED, axum::Json(product)))
}

async fn update(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Json(input): Json<ProductRequest>,
) -> ApiResult<axum::Json<Product>> {
    let input = input.validate()?;
    repo::update(&state.db, auth.user_id(), id, &input)
        .await
        .map_err(barcode_taken)?
        .map(axum::Json)
        .ok_or(ApiError::NotFound("product"))
}

/// Two products cannot share a barcode; scanning it again finds the existing one
fn barcode_taken(err: sqlx::Error) -> ApiError {
    match &err {
        sqlx::Error::Database(db) if db.constraint() == Some("food_products_barcode_key") => {
            ApiError::Conflict(
                "A product with this barcode already exists. Scan it to use it.".into(),
            )
        }
        _ => err.into(),
    }
}
