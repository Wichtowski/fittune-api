pub mod cleanup;
mod storage;

pub use storage::{PhotoStore, S3PhotoStore};

use std::{io::Cursor, sync::Arc};

use anyhow::Result;
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Multipart, Path, Query, State},
    http::{StatusCode, header},
    response::IntoResponse,
    routing::get,
};
use chrono::{DateTime, Utc};
use image::{
    DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits, codecs::jpeg::JpegEncoder,
    imageops::FilterType,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    app::AppState,
    auth::Auth,
    error::{ApiError, ApiResult},
};

const MAX_UPLOAD: usize = 10 * 1024 * 1024;
const MAX_PIXELS: u64 = 12_000_000;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct ProgressPhoto {
    pub id: Uuid,
    pub workout_id: Option<Uuid>,
    pub width: i32,
    pub height: i32,
    pub bytes: i32,
    pub taken_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
struct StoredPhoto {
    id: Uuid,
    workout_id: Option<Uuid>,
    storage_key: String,
    width: i32,
    height: i32,
    bytes: i32,
    taken_at: DateTime<Utc>,
    created_at: DateTime<Utc>,
}

impl StoredPhoto {
    fn public(&self) -> ProgressPhoto {
        ProgressPhoto {
            id: self.id,
            workout_id: self.workout_id,
            width: self.width,
            height: self.height,
            bytes: self.bytes,
            taken_at: self.taken_at,
            created_at: self.created_at,
        }
    }
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/progress-photos", get(list).post(post_upload))
        .route(
            "/progress-photos/{id}",
            axum::routing::put(put_upload).delete(delete),
        )
        .route("/progress-photos/{id}/file", get(file))
        .layer(DefaultBodyLimit::max(MAX_UPLOAD + 4096))
}

fn store(state: &AppState) -> ApiResult<Arc<dyn PhotoStore>> {
    state.photos.clone().ok_or(ApiError::StorageUnavailable)
}

#[derive(Deserialize)]
struct ListQuery {
    workout_id: Option<Uuid>,
    limit: Option<i64>,
    offset: Option<i64>,
}

async fn list(
    State(state): State<AppState>,
    auth: Auth,
    Query(query): Query<ListQuery>,
) -> ApiResult<Json<Vec<ProgressPhoto>>> {
    let rows: Vec<StoredPhoto> = sqlx::query_as(
        "SELECT id, workout_id, storage_key, width, height, bytes, taken_at, created_at
         FROM progress_photos WHERE user_id = $1 AND ($2::uuid IS NULL OR workout_id = $2)
         ORDER BY taken_at DESC, id DESC LIMIT $3 OFFSET $4",
    )
    .bind(auth.user_id())
    .bind(query.workout_id)
    .bind(query.limit.unwrap_or(50).clamp(1, 100))
    .bind(query.offset.unwrap_or(0).max(0))
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rows.iter().map(StoredPhoto::public).collect()))
}

async fn post_upload(
    State(state): State<AppState>,
    auth: Auth,
    multipart: Multipart,
) -> ApiResult<(StatusCode, Json<ProgressPhoto>)> {
    upload(state, auth, Uuid::new_v4(), multipart).await
}

async fn put_upload(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    multipart: Multipart,
) -> ApiResult<(StatusCode, Json<ProgressPhoto>)> {
    upload(state, auth, id, multipart).await
}

async fn upload(
    state: AppState,
    auth: Auth,
    id: Uuid,
    mut multipart: Multipart,
) -> ApiResult<(StatusCode, Json<ProgressPhoto>)> {
    let storage = store(&state)?;
    if let Some(existing) = find(&state, auth.user_id(), id).await? {
        return Ok((StatusCode::OK, Json(existing.public())));
    }
    let permit = state
        .photo_uploads
        .clone()
        .try_acquire_owned()
        .map_err(|_| ApiError::RateLimited)?;
    let mut workout_id = None;
    let mut image = None;
    while let Some(field) = multipart.next_field().await.map_err(multipart_error)? {
        match field.name() {
            Some("workout_id") => {
                if workout_id.is_some() {
                    return Err(ApiError::BadRequest("duplicate workout_id".into()));
                }
                workout_id = Some(
                    field
                        .text()
                        .await
                        .map_err(multipart_error)?
                        .parse::<Uuid>()
                        .map_err(|_| ApiError::validation("workout_id", "Invalid workout id"))?,
                );
            }
            Some("file") => {
                if image.is_some() {
                    return Err(ApiError::BadRequest("upload one image at a time".into()));
                }
                image = Some(field.bytes().await.map_err(multipart_error)?);
            }
            _ => return Err(ApiError::BadRequest("unexpected form field".into())),
        }
    }
    let image = image.ok_or_else(|| ApiError::validation("file", "Choose a photo"))?;
    if image.len() > MAX_UPLOAD {
        return Err(ApiError::PayloadTooLarge);
    }
    if let Some(workout_id) = workout_id {
        let owns_completed: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM workouts WHERE id = $1 AND user_id = $2 AND ended_at IS NOT NULL)")
            .bind(workout_id).bind(auth.user_id()).fetch_one(&state.db).await?;
        if !owns_completed {
            return Err(ApiError::validation(
                "workout_id",
                "Choose a completed workout you own",
            ));
        }
    }
    // Keep the permit in the blocking task even if the request times out or disconnects
    let (_permit, processed) = tokio::task::spawn_blocking(move || (permit, process_image(&image)))
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    let processed = processed?;
    // Each attempt has its own keys, so a losing retry cannot overwrite or delete a winner
    let key = format!("photos/{}/{id}/{}", auth.user_id(), Uuid::new_v4());
    cleanup::reserve(&state.db, &key).await?;
    let mut tx = state.db.begin().await?;
    let user: Option<Uuid> = sqlx::query_scalar("SELECT id FROM users WHERE id = $1 FOR UPDATE")
        .bind(auth.user_id())
        .fetch_optional(&mut *tx)
        .await?;
    if user.is_none() {
        return Err(ApiError::Unauthorized);
    }
    let existing: Option<StoredPhoto> = sqlx::query_as(
        "SELECT id, workout_id, storage_key, width, height, bytes, taken_at, created_at
         FROM progress_photos WHERE id = $1 AND user_id = $2",
    )
    .bind(id)
    .bind(auth.user_id())
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(existing) = existing {
        return Ok((StatusCode::OK, Json(existing.public())));
    }
    let foreign_id: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM progress_photos WHERE id = $1 AND user_id <> $2)",
    )
    .bind(id)
    .bind(auth.user_id())
    .fetch_one(&mut *tx)
    .await?;
    if foreign_id {
        return Err(ApiError::Conflict("photo id is already in use".into()));
    }
    let full_len = processed.full.len();
    let stored_bytes = (full_len + processed.thumb.len()) as i64;
    let (count, bytes): (i64, i64) = sqlx::query_as(
        "SELECT count(*), COALESCE(sum(stored_bytes), 0)::bigint FROM progress_photos WHERE user_id = $1")
        .bind(auth.user_id()).fetch_one(&mut *tx).await?;
    if count >= state.config.photo_max_count || bytes + stored_bytes > state.config.photo_max_bytes
    {
        return Err(ApiError::validation(
            "file",
            "Photo quota reached; delete older photos before uploading more",
        ));
    }
    storage
        .put(&format!("{key}/full.jpg"), processed.full)
        .await
        .map_err(ApiError::Internal)?;
    if let Err(err) = storage
        .put(&format!("{key}/thumb.jpg"), processed.thumb)
        .await
    {
        cleanup::expedite(&state.db, &key).await?;
        return Err(ApiError::Internal(err));
    }
    let inserted: Result<StoredPhoto, sqlx::Error> = sqlx::query_as(
        "INSERT INTO progress_photos (id, user_id, workout_id, storage_key, width, height, bytes, stored_bytes)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         RETURNING id, workout_id, storage_key, width, height, bytes, taken_at, created_at",
    )
    .bind(id)
    .bind(auth.user_id())
    .bind(workout_id)
    .bind(&key)
    .bind(processed.width as i32)
    .bind(processed.height as i32)
    .bind(full_len as i32)
    .bind(stored_bytes)
    .fetch_one(&mut *tx)
    .await;
    match inserted {
        Ok(photo) => {
            sqlx::query("DELETE FROM photo_cleanup WHERE storage_key = $1")
                .bind(&key)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            Ok((StatusCode::CREATED, Json(photo.public())))
        }
        Err(err) => {
            tx.rollback().await?;
            cleanup::expedite(&state.db, &key).await?;
            if crate::error::unique_violation(&err).is_some() {
                return Err(ApiError::Conflict("photo id is already in use".into()));
            }
            Err(err.into())
        }
    }
}

fn multipart_error(error: axum::extract::multipart::MultipartError) -> ApiError {
    if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
        ApiError::PayloadTooLarge
    } else {
        ApiError::BadRequest("Invalid photo upload".into())
    }
}

struct Processed {
    full: Vec<u8>,
    thumb: Vec<u8>,
    width: u32,
    height: u32,
}

fn process_image(bytes: &Bytes) -> ApiResult<Processed> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| ApiError::validation("file", "Could not read image"))?;
    if !matches!(
        reader.format(),
        Some(ImageFormat::Jpeg | ImageFormat::Png | ImageFormat::WebP)
    ) {
        return Err(ApiError::validation(
            "file",
            "Use a JPEG, PNG, or WebP image",
        ));
    }
    let mut limits = Limits::default();
    limits.max_image_width = Some(6000);
    limits.max_image_height = Some(6000);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let mut decoder = reader
        .into_decoder()
        .map_err(|_| ApiError::validation("file", "Invalid or oversized image"))?;
    let (width, height) = decoder.dimensions();
    if u64::from(width) * u64::from(height) > MAX_PIXELS || decoder.total_bytes() > 64 * 1024 * 1024
    {
        return Err(ApiError::validation(
            "file",
            "Image dimensions are too large",
        ));
    }
    let orientation = decoder
        .orientation()
        .map_err(|_| ApiError::validation("file", "Invalid image metadata"))?;
    let mut decoded = DynamicImage::from_decoder(decoder)
        .map_err(|_| ApiError::validation("file", "Invalid or oversized image"))?;
    decoded.apply_orientation(orientation);
    let resized =
        DynamicImage::ImageRgb8(decoded.resize(2048, 2048, FilterType::Lanczos3).to_rgb8());
    let thumbnail = resized.thumbnail(384, 384);
    let mut full = Vec::new();
    JpegEncoder::new_with_quality(&mut full, 82)
        .encode_image(&resized)
        .map_err(|e| ApiError::Internal(e.into()))?;
    let mut thumb = Vec::new();
    JpegEncoder::new_with_quality(&mut thumb, 75)
        .encode_image(&thumbnail)
        .map_err(|e| ApiError::Internal(e.into()))?;
    Ok(Processed {
        full,
        thumb,
        width: resized.width(),
        height: resized.height(),
    })
}

async fn find(state: &AppState, user_id: Uuid, id: Uuid) -> ApiResult<Option<StoredPhoto>> {
    Ok(sqlx::query_as("SELECT id, workout_id, storage_key, width, height, bytes, taken_at, created_at FROM progress_photos WHERE id = $1 AND user_id = $2")
        .bind(id).bind(user_id).fetch_optional(&state.db).await?)
}

#[derive(Deserialize)]
struct FileQuery {
    size: Option<String>,
}

async fn file(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Query(query): Query<FileQuery>,
) -> ApiResult<impl IntoResponse> {
    let photo = find(&state, auth.user_id(), id)
        .await?
        .ok_or(ApiError::NotFound("photo"))?;
    let suffix = match query.size.as_deref() {
        None | Some("full") => "full",
        Some("thumb") => "thumb",
        _ => return Err(ApiError::BadRequest("Unknown image size".into())),
    };
    let bytes = store(&state)?
        .get(&format!("{}/{suffix}.jpg", photo.storage_key))
        .await
        .map_err(ApiError::Internal)?
        .ok_or(ApiError::NotFound("photo"))?;
    Ok((
        [
            (header::CONTENT_TYPE, "image/jpeg"),
            (header::CACHE_CONTROL, "private, no-store"),
        ],
        bytes,
    ))
}

async fn delete(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let key: Option<String> = sqlx::query_scalar(
        "DELETE FROM progress_photos WHERE id = $1 AND user_id = $2 RETURNING storage_key",
    )
    .bind(id)
    .bind(auth.user_id())
    .fetch_optional(&state.db)
    .await?;
    key.ok_or(ApiError::NotFound("photo"))?;
    cleanup::after_delete(&state).await;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reencoding_removes_trailing_metadata_and_creates_thumbnail() {
        let mut source = Cursor::new(Vec::new());
        DynamicImage::new_rgb8(16, 12)
            .write_to(&mut source, ImageFormat::Png)
            .expect("encode source");
        let mut source = source.into_inner();
        source.extend_from_slice(b"PRIVATE_GPS_METADATA");
        let image = process_image(&Bytes::from(source)).expect("process photo");
        assert!(image.full.starts_with(&[0xff, 0xd8]));
        assert!(
            !image
                .full
                .windows(20)
                .any(|window| window == b"PRIVATE_GPS_METADATA")
        );
        assert!(image.thumb.starts_with(&[0xff, 0xd8]));
    }

    #[tokio::test]
    #[ignore = "requires local RustFS; see README for the explicit command"]
    async fn rustfs_round_trip_when_configured() {
        use crate::config::PhotoStorageConfig;
        use http_body_util::BodyExt;
        let endpoint = std::env::var("RUSTFS_TEST_ENDPOINT").expect("test endpoint");
        let store = S3PhotoStore::new(&PhotoStorageConfig {
            endpoint,
            bucket: std::env::var("RUSTFS_TEST_BUCKET").expect("test bucket"),
            access_key: std::env::var("RUSTFS_TEST_ACCESS_KEY").expect("test access key"),
            secret_key: std::env::var("RUSTFS_TEST_SECRET_KEY").expect("test secret key"),
        });
        let key = format!("test/{}.jpg", Uuid::new_v4());
        store
            .put(&key, b"private test".to_vec())
            .await
            .expect("put");
        let body = store.get(&key).await.expect("get").expect("object exists");
        assert_eq!(
            body.collect().await.expect("read body").to_bytes(),
            b"private test"[..]
        );
        store.delete(&key).await.expect("delete");
        assert!(store.get(&key).await.expect("missing get").is_none());
    }
}
