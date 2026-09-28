//! Photos and videos demonstrating an exercise. Catalog photos are copied once from their public
//! source into object storage by a startup backfill and served without auth; videos are embedded
//! from their provider.

use std::{collections::HashMap, time::Duration};

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use axum::{Router, extract::State, http::header, response::IntoResponse, routing::get};
use image::ImageFormat;
use serde::Serialize;
use sqlx::{PgConnection, PgExecutor, PgPool};
use uuid::Uuid;

use super::model::Exercise;
use crate::{
    app::AppState,
    error::{ApiError, ApiResult},
    extract::Path,
    photos::PhotoStore,
};

const MAX_DOWNLOAD: usize = 5 * 1024 * 1024;
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, sqlx::Type)]
#[sqlx(type_name = "text", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    Photo,
    Video,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, sqlx::Type)]
#[sqlx(type_name = "text", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum MediaProvider {
    /// A photo in FitTune's own object storage
    Fittune,
    Youtube,
    Vimeo,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExerciseMedia {
    pub id: Uuid,
    pub kind: MediaKind,
    pub provider: MediaProvider,
    pub position: i32,
    /// Path of a stored photo, relative to the API origin
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// The provider's video id
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_id: Option<String>,
}

#[derive(sqlx::FromRow)]
struct MediaRow {
    id: Uuid,
    exercise_id: Uuid,
    kind: MediaKind,
    provider: MediaProvider,
    external_id: Option<String>,
    position: i32,
}

impl From<MediaRow> for ExerciseMedia {
    fn from(row: MediaRow) -> Self {
        let url = (row.provider == MediaProvider::Fittune)
            .then(|| format!("/api/v1/train/exercise-media/{}/file", row.id));
        Self {
            id: row.id,
            kind: row.kind,
            provider: row.provider,
            position: row.position,
            url,
            external_id: row.external_id,
        }
    }
}

/// Fills `media` and the derived `video_id` of every exercise with one query.
pub async fn attach(db: impl PgExecutor<'_>, exercises: &mut [Exercise]) -> sqlx::Result<()> {
    if exercises.is_empty() {
        return Ok(());
    }
    let ids: Vec<Uuid> = exercises.iter().map(|exercise| exercise.id).collect();
    let rows: Vec<MediaRow> = sqlx::query_as(
        "SELECT id, exercise_id, kind, provider, external_id, position FROM exercise_media
         WHERE exercise_id = ANY ($1)
         ORDER BY exercise_id, kind, position",
    )
    .bind(&ids)
    .fetch_all(db)
    .await?;

    let mut by_exercise: HashMap<Uuid, Vec<ExerciseMedia>> = HashMap::new();
    for row in rows {
        by_exercise
            .entry(row.exercise_id)
            .or_default()
            .push(row.into());
    }
    for exercise in exercises {
        exercise.media = by_exercise.remove(&exercise.id).unwrap_or_default();
        exercise.video_id = exercise
            .media
            .iter()
            .find(|media| media.provider == MediaProvider::Youtube)
            .and_then(|media| media.external_id.clone());
    }
    Ok(())
}

/// Stores the YouTube video an exercise request names as its first video. `None` removes a
/// YouTube first video but leaves one from another provider, which requests cannot express.
pub async fn set_youtube_video(
    db: &mut PgConnection,
    exercise_id: Uuid,
    video_id: Option<&str>,
) -> sqlx::Result<()> {
    match video_id {
        Some(video_id) => {
            sqlx::query(
                "INSERT INTO exercise_media (id, exercise_id, kind, provider, external_id, position)
                 VALUES ($1, $2, 'video', 'youtube', $3, 0)
                 ON CONFLICT (exercise_id, kind, position)
                 DO UPDATE SET provider = 'youtube', external_id = EXCLUDED.external_id",
            )
            .bind(Uuid::new_v4())
            .bind(exercise_id)
            .bind(video_id)
            .execute(db)
            .await?;
        }
        None => {
            sqlx::query(
                "DELETE FROM exercise_media
                 WHERE exercise_id = $1 AND kind = 'video' AND position = 0 AND provider = 'youtube'",
            )
            .bind(exercise_id)
            .execute(db)
            .await?;
        }
    }
    Ok(())
}

pub fn router() -> Router<AppState> {
    Router::new().route("/exercise-media/{id}/file", get(file))
}

/// Catalog photos are public domain and the same for everyone, so they are served without auth
/// (a plain `<img src>` works) and cached for good: a replaced photo gets a new id.
async fn file(State(state): State<AppState>, Path(id): Path<Uuid>) -> ApiResult<impl IntoResponse> {
    let key: String = sqlx::query_scalar(
        "SELECT m.storage_key FROM exercise_media m
         JOIN exercises e ON e.id = m.exercise_id
         WHERE m.id = $1 AND m.provider = 'fittune' AND e.owner_id IS NULL",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound("photo"))?;
    let store = state.photos.clone().ok_or(ApiError::StorageUnavailable)?;
    // Until the backfill has stored it; the app shows its fallback illustration meanwhile
    if !store.exists(&key).await.map_err(ApiError::Internal)? {
        return Err(ApiError::NotFound("photo"));
    }
    let bytes = store.get(&key).await.map_err(ApiError::Internal)?;
    Ok((
        [
            (header::CONTENT_TYPE, "image/jpeg"),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        bytes,
    ))
}

/// Downloads a photo's source
#[async_trait]
pub trait Fetch: Send + Sync {
    async fn fetch(&self, url: &str) -> Result<Vec<u8>>;
}

pub struct HttpFetch(reqwest::Client);

impl HttpFetch {
    pub fn new() -> Result<Self> {
        let client = reqwest::Client::builder()
            .https_only(true)
            .timeout(DOWNLOAD_TIMEOUT)
            .user_agent(concat!("fittune-api/", env!("CARGO_PKG_VERSION")))
            .build()
            .context("failed to build the HTTP client")?;
        Ok(Self(client))
    }
}

#[async_trait]
impl Fetch for HttpFetch {
    async fn fetch(&self, url: &str) -> Result<Vec<u8>> {
        let mut response = self.0.get(url).send().await?.error_for_status()?;
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if bytes.len() + chunk.len() > MAX_DOWNLOAD {
                bail!("larger than {MAX_DOWNLOAD} bytes");
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct BackfillSummary {
    pub stored: usize,
    pub present: usize,
    pub failed: usize,
}

/// Copies every photo with a source that is missing from storage. One failure does not stop the
/// rest, and failed photos are retried on the next run since only missing ones are fetched.
pub async fn backfill(
    db: &PgPool,
    store: &dyn PhotoStore,
    fetch: &dyn Fetch,
) -> Result<BackfillSummary> {
    let sources: Vec<(String, String)> = sqlx::query_as(
        "SELECT storage_key, source_url FROM exercise_media
         WHERE source_url IS NOT NULL ORDER BY storage_key",
    )
    .fetch_all(db)
    .await
    .context("failed to list photo sources")?;

    let mut summary = BackfillSummary::default();
    for (key, url) in sources {
        let result = async {
            if store.exists(&key).await? {
                return Ok(false);
            }
            let bytes = fetch.fetch(&url).await?;
            image::load_from_memory_with_format(&bytes, ImageFormat::Jpeg)
                .context("not a JPEG image")?;
            store.put(&key, bytes).await?;
            anyhow::Ok(true)
        }
        .await;
        match result {
            Ok(true) => summary.stored += 1,
            Ok(false) => summary.present += 1,
            Err(err) => {
                summary.failed += 1;
                tracing::warn!(
                    key,
                    url,
                    error = format!("{err:#}"),
                    "failed to store catalog photo"
                );
            }
        }
    }
    Ok(summary)
}
