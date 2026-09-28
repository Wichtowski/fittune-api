use std::{collections::HashMap, sync::Arc};

use anyhow::{Result, bail};
use async_trait::async_trait;
use axum::http::{Method, StatusCode, header};
use fittune_api::exercises::media::{self, BackfillSummary, Fetch};
use image::{DynamicImage, codecs::jpeg::JpegEncoder};
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::{
    common::{TestApp, uuid},
    photos::MemoryPhotos,
};

/// Start and finish frames for every catalog exercise the app had photos for
const CATALOG_PHOTOS: usize = 43 * 2;

fn jpeg() -> Vec<u8> {
    let mut bytes = Vec::new();
    DynamicImage::new_rgb8(8, 8)
        .write_with_encoder(JpegEncoder::new(&mut bytes))
        .expect("encode JPEG");
    bytes
}

fn media_of_kind<'a>(exercise: &'a Value, kind: &str) -> Vec<&'a Value> {
    exercise["media"]
        .as_array()
        .expect("media array")
        .iter()
        .filter(|media| media["kind"] == kind)
        .collect()
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn catalog_exercises_list_their_photos_and_videos(pool: PgPool) {
    let app = TestApp::new(pool);
    let user = app.register("medialifter").await;

    let bench = app.catalog_exercise("Barbell Bench Press").await;
    let (status, exercise) = app
        .get(&format!("/api/v1/exercises/{bench}"), &user.token)
        .await;
    assert_eq!(status, StatusCode::OK);
    let photos = media_of_kind(&exercise, "photo");
    assert_eq!(photos.len(), 2);
    for (position, photo) in photos.iter().enumerate() {
        assert_eq!(photo["provider"], "fittune");
        assert_eq!(photo["position"], position);
        assert_eq!(
            photo["url"],
            format!(
                "/api/v1/exercise-media/{}/file",
                photo["id"].as_str().expect("id")
            )
        );
        assert!(photo.get("external_id").is_none());
    }
    let videos = media_of_kind(&exercise, "video");
    assert_eq!(videos.len(), 1);
    assert_eq!(videos[0]["provider"], "youtube");
    assert_eq!(videos[0]["external_id"], "hWbUlkb5Ms4");
    assert!(videos[0].get("url").is_none());
    assert_eq!(
        exercise["video_id"], "hWbUlkb5Ms4",
        "older apps still read the YouTube id"
    );

    let curl = app.catalog_exercise("Barbell Curl").await;
    let (_, exercise) = app
        .get(&format!("/api/v1/exercises/{curl}"), &user.token)
        .await;
    assert_eq!(media_of_kind(&exercise, "video")[0]["provider"], "vimeo");
    assert_eq!(
        exercise["video_id"],
        Value::Null,
        "only YouTube ids fill video_id"
    );

    let (_, list) = app.get("/api/v1/exercises", &user.token).await;
    let listed = list
        .as_array()
        .expect("list")
        .iter()
        .find(|exercise| exercise["name"] == "Barbell Bench Press")
        .expect("bench press listed");
    assert_eq!(listed["media"].as_array().map(Vec::len), Some(3));
    let (_, history) = app
        .get(&format!("/api/v1/exercises/{bench}/history"), &user.token)
        .await;
    assert_eq!(
        history["exercise"]["media"].as_array().map(Vec::len),
        Some(3)
    );
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn migration_seeds_every_catalog_photo_and_video(pool: PgPool) {
    let (photos, videos): (i64, i64) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE kind = 'photo'), count(*) FILTER (WHERE kind = 'video')
         FROM exercise_media",
    )
    .fetch_one(&pool)
    .await
    .expect("count media");
    assert_eq!(
        usize::try_from(photos).ok(),
        Some(CATALOG_PHOTOS),
        "every mapped catalog exercise has two frames"
    );
    assert_eq!(videos, 5);
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn custom_exercise_video_id_round_trips_through_media(pool: PgPool) {
    let app = TestApp::new(pool);
    let user = app.register("videoowner").await;
    let (status, created) = app
        .post(
            "/api/v1/exercises",
            Some(&user.token),
            json!({ "name": "Landmine Press", "tracking": "weight_reps",
                    "primary_muscle": "shoulders", "video_id": "dQw4w9WgXcQ" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["video_id"], "dQw4w9WgXcQ");
    assert_eq!(
        media_of_kind(&created, "video")[0]["external_id"],
        "dQw4w9WgXcQ"
    );
    let uri = format!("/api/v1/exercises/{}", created["id"].as_str().expect("id"));

    let (_, updated) = app
        .put(
            &uri,
            &user.token,
            json!({ "name": "Landmine Press", "tracking": "weight_reps",
                    "primary_muscle": "shoulders", "video_id": "aNUSgyWRJYA" }),
        )
        .await;
    assert_eq!(updated["video_id"], "aNUSgyWRJYA");
    assert_eq!(
        media_of_kind(&updated, "video").len(),
        1,
        "the video is replaced, not added"
    );

    let (_, cleared) = app
        .put(
            &uri,
            &user.token,
            json!({ "name": "Landmine Press", "tracking": "weight_reps", "primary_muscle": "shoulders" }),
        )
        .await;
    assert_eq!(cleared["video_id"], Value::Null);
    assert_eq!(cleared["media"], json!([]));
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn catalog_photo_files_are_public_and_cacheable(pool: PgPool) {
    let memory = Arc::new(MemoryPhotos::default());
    let app = TestApp::with_photos(pool.clone(), Some(memory.clone()));
    let user = app.register("photoviewer").await;
    let bench = app.catalog_exercise("Barbell Bench Press").await;
    let (_, exercise) = app
        .get(&format!("/api/v1/exercises/{bench}"), &user.token)
        .await;
    let photo = media_of_kind(&exercise, "photo")[0].clone();
    let url = photo["url"].as_str().expect("url");

    let (status, _, _) = app.raw(Method::GET, url, None, None, vec![]).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "not backfilled yet");

    let key: String =
        sqlx::query_scalar("SELECT storage_key FROM exercise_media WHERE id = $1::uuid")
            .bind(photo["id"].as_str())
            .fetch_one(&pool)
            .await
            .expect("storage key");
    memory.0.lock().expect("lock").insert(key, jpeg());
    let (status, headers, bytes) = app.raw(Method::GET, url, None, None, vec![]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CONTENT_TYPE], "image/jpeg");
    assert_eq!(
        headers[header::CACHE_CONTROL],
        "public, max-age=31536000, immutable"
    );
    assert_eq!(bytes, jpeg());

    let video_id = media_of_kind(&exercise, "video")[0]["id"]
        .as_str()
        .expect("id")
        .to_owned();
    for id in [video_id, uuid()] {
        let (status, _, _) = app
            .raw(
                Method::GET,
                &format!("/api/v1/exercise-media/{id}/file"),
                None,
                None,
                vec![],
            )
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn custom_exercise_photos_are_never_public(pool: PgPool) {
    let memory = Arc::new(MemoryPhotos::default());
    let app = TestApp::with_photos(pool.clone(), Some(memory.clone()));
    let user = app.register("privatephotos").await;
    let (_, created) = app
        .post(
            "/api/v1/exercises",
            Some(&user.token),
            json!({ "name": "Secret Press", "tracking": "weight_reps", "primary_muscle": "chest" }),
        )
        .await;
    let media_id = uuid();
    sqlx::query(
        "INSERT INTO exercise_media (id, exercise_id, kind, provider, storage_key, position)
         VALUES ($1::uuid, $2::uuid, 'photo', 'fittune', 'custom/secret.jpg', 0)",
    )
    .bind(&media_id)
    .bind(created["id"].as_str())
    .execute(&pool)
    .await
    .expect("insert custom photo");
    memory
        .0
        .lock()
        .expect("lock")
        .insert("custom/secret.jpg".into(), jpeg());

    let (status, _, _) = app
        .raw(
            Method::GET,
            &format!("/api/v1/exercise-media/{media_id}/file"),
            None,
            None,
            vec![],
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// Serves canned downloads instead of the network
struct FakeFetch(HashMap<String, Vec<u8>>);

#[async_trait]
impl Fetch for FakeFetch {
    async fn fetch(&self, url: &str) -> Result<Vec<u8>> {
        match self.0.get(url) {
            Some(bytes) => Ok(bytes.clone()),
            None => bail!("unreachable: {url}"),
        }
    }
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn backfill_stores_missing_catalog_photos_once(pool: PgPool) {
    let sources: Vec<(String, String)> = sqlx::query_as(
        "SELECT storage_key, source_url FROM exercise_media
         WHERE source_url IS NOT NULL ORDER BY storage_key",
    )
    .fetch_all(&pool)
    .await
    .expect("sources");
    assert_eq!(sources.len(), CATALOG_PHOTOS);

    let memory = MemoryPhotos::default();
    let (already_stored, _) = &sources[0];
    memory
        .0
        .lock()
        .expect("lock")
        .insert(already_stored.clone(), b"kept as is".to_vec());
    let (broken_key, broken_url) = &sources[1];
    let (_, unreachable_url) = &sources[2];
    let mut downloads: HashMap<String, Vec<u8>> = sources
        .iter()
        .map(|(_, url)| (url.clone(), jpeg()))
        .collect();
    downloads.insert(broken_url.clone(), b"<html>rate limited</html>".to_vec());
    downloads.remove(unreachable_url);

    let summary = media::backfill(&pool, &memory, &FakeFetch(downloads))
        .await
        .expect("backfill runs");
    assert_eq!(
        summary,
        BackfillSummary {
            stored: sources.len() - 3,
            present: 1,
            failed: 2,
        }
    );
    {
        let stored = memory.0.lock().expect("lock");
        assert_eq!(
            stored[already_stored], b"kept as is",
            "existing files are left alone"
        );
        assert!(
            !stored.contains_key(broken_key),
            "non-images are never stored"
        );
    }

    let summary = media::backfill(&pool, &memory, &FakeFetch(HashMap::new()))
        .await
        .expect("second run");
    assert_eq!(summary.present, sources.len() - 2);
    assert_eq!(summary.failed, 2, "only the missing files are retried");
}
