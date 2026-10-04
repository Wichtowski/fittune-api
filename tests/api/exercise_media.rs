use std::{collections::HashMap, sync::Arc};

use anyhow::{Result, bail};
use async_trait::async_trait;
use axum::http::{Method, StatusCode, header};
use fittune_api::exercises::media::{self, BackfillSummary, Fetch};
use image::{DynamicImage, ImageFormat, codecs::jpeg::JpegEncoder};
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::{
    common::{TestApp, uuid},
    photos::MemoryPhotos,
};

/// Exercises imported from the dataset, each with a thumbnail and an animation
const DATASET_EXERCISES: usize = 1324;
/// Start and finish frames of the first catalog's exercises that the dataset has no match for
const FIRST_CATALOG_PHOTOS: usize = 3 * 2;
const ATTRIBUTION: &str = "© Gym visual - https://gymvisual.com/";

fn jpeg() -> Vec<u8> {
    let mut bytes = Vec::new();
    DynamicImage::new_rgb8(8, 8)
        .write_with_encoder(JpegEncoder::new(&mut bytes))
        .expect("encode JPEG");
    bytes
}

fn gif() -> Vec<u8> {
    let mut bytes = std::io::Cursor::new(Vec::new());
    DynamicImage::new_rgba8(8, 8)
        .write_to(&mut bytes, ImageFormat::Gif)
        .expect("encode GIF");
    bytes.into_inner()
}

/// What the source of a stored file would serve
fn download(url: &str) -> Vec<u8> {
    if url.ends_with(".gif") { gif() } else { jpeg() }
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
        .get(&format!("/api/v1/train/exercises/{bench}"), &user.token)
        .await;
    assert_eq!(status, StatusCode::OK);
    let stored: Vec<&Value> = ["photo", "animation"]
        .into_iter()
        .flat_map(|kind| media_of_kind(&exercise, kind))
        .collect();
    assert_eq!(stored.len(), 2, "a thumbnail and an animation");
    for media in stored {
        assert_eq!(media["provider"], "fittune");
        assert_eq!(media["position"], 0);
        assert_eq!(media["attribution"], ATTRIBUTION);
        assert_eq!(
            media["url"],
            format!(
                "/api/v1/train/exercise-media/{}/file",
                media["id"].as_str().expect("id")
            )
        );
        assert!(media.get("external_id").is_none());
    }
    let kinds: Vec<&str> = exercise["media"]
        .as_array()
        .expect("media array")
        .iter()
        .filter_map(|media| media["kind"].as_str())
        .collect();
    assert_eq!(kinds, ["photo", "animation", "video"]);
    let videos = media_of_kind(&exercise, "video");
    assert_eq!(videos.len(), 1);
    assert_eq!(videos[0]["provider"], "youtube");
    assert_eq!(videos[0]["external_id"], "hWbUlkb5Ms4");
    assert!(videos[0].get("url").is_none());
    assert!(videos[0].get("attribution").is_none());
    assert_eq!(
        exercise["video_id"], "hWbUlkb5Ms4",
        "older apps still read the YouTube id"
    );

    let curl = app.catalog_exercise("Barbell Curl").await;
    let (_, exercise) = app
        .get(&format!("/api/v1/train/exercises/{curl}"), &user.token)
        .await;
    assert_eq!(media_of_kind(&exercise, "video")[0]["provider"], "vimeo");
    assert_eq!(
        exercise["video_id"],
        Value::Null,
        "only YouTube ids fill video_id"
    );

    let (_, list) = app.get("/api/v1/train/exercises", &user.token).await;
    let listed = list
        .as_array()
        .expect("list")
        .iter()
        .find(|exercise| exercise["name"] == "Barbell Bench Press")
        .expect("bench press listed");
    assert_eq!(listed["media"].as_array().map(Vec::len), Some(3));
    let (_, history) = app
        .get(
            &format!("/api/v1/train/exercises/{bench}/history"),
            &user.token,
        )
        .await;
    assert_eq!(
        history["exercise"]["media"].as_array().map(Vec::len),
        Some(3)
    );
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn migrations_seed_every_catalog_photo_animation_and_video(pool: PgPool) {
    let (photos, animations, videos): (i64, i64, i64) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE kind = 'photo'), count(*) FILTER (WHERE kind = 'animation'),
                count(*) FILTER (WHERE kind = 'video')
         FROM exercise_media",
    )
    .fetch_one(&pool)
    .await
    .expect("count media");
    assert_eq!(
        usize::try_from(photos).ok(),
        Some(DATASET_EXERCISES + FIRST_CATALOG_PHOTOS)
    );
    assert_eq!(usize::try_from(animations).ok(), Some(DATASET_EXERCISES));
    assert_eq!(videos, 5, "videos survive the merge with the dataset");

    let incomplete: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM exercises e
         WHERE e.catalog_ref IS NOT NULL AND (
             SELECT count(*) FILTER (WHERE m.kind = 'photo') <> 1
                 OR count(*) FILTER (WHERE m.kind = 'animation') <> 1
                 OR bool_or(m.kind <> 'video' AND (m.attribution IS DISTINCT FROM $1
                     OR m.source_url NOT LIKE 'https://raw.githubusercontent.com/%'))
             FROM exercise_media m WHERE m.exercise_id = e.id)",
    )
    .bind(ATTRIBUTION)
    .fetch_one(&pool)
    .await
    .expect("count incomplete");
    assert_eq!(
        incomplete, 0,
        "every dataset exercise has one credited thumbnail and animation"
    );
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn custom_exercise_video_id_round_trips_through_media(pool: PgPool) {
    let app = TestApp::new(pool);
    let user = app.register("videoowner").await;
    let (status, created) = app
        .post(
            "/api/v1/train/exercises",
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
    let uri = format!(
        "/api/v1/train/exercises/{}",
        created["id"].as_str().expect("id")
    );

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
async fn catalog_media_files_are_public_and_cacheable(pool: PgPool) {
    let memory = Arc::new(MemoryPhotos::default());
    let app = TestApp::with_photos(pool.clone(), Some(memory.clone()));
    let user = app.register("photoviewer").await;
    let bench = app.catalog_exercise("Barbell Bench Press").await;
    let (_, exercise) = app
        .get(&format!("/api/v1/train/exercises/{bench}"), &user.token)
        .await;

    for (kind, content_type, bytes) in [
        ("photo", "image/jpeg", jpeg()),
        ("animation", "image/gif", gif()),
    ] {
        let media = media_of_kind(&exercise, kind)[0].clone();
        let url = media["url"].as_str().expect("url");
        let (status, _, _) = app.raw(Method::GET, url, None, None, vec![]).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{kind} not backfilled yet");

        let key: String = sqlx::query_scalar(
            "UPDATE exercise_media SET stored_at = now() WHERE id = $1::uuid RETURNING storage_key",
        )
        .bind(media["id"].as_str())
        .fetch_one(&pool)
        .await
        .expect("storage key");
        let (status, _, _) = app.raw(Method::GET, url, None, None, vec![]).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{kind} lost by the bucket");
        let stored_at: Option<chrono::DateTime<chrono::Utc>> =
            sqlx::query_scalar("SELECT stored_at FROM exercise_media WHERE id = $1::uuid")
                .bind(media["id"].as_str())
                .fetch_one(&pool)
                .await
                .expect("stored_at");
        assert_eq!(
            stored_at, None,
            "a lost file is queued for the next backfill"
        );

        memory.0.lock().expect("lock").insert(key, bytes.clone());
        sqlx::query("UPDATE exercise_media SET stored_at = now() WHERE id = $1::uuid")
            .bind(media["id"].as_str())
            .execute(&pool)
            .await
            .expect("mark stored");
        let heads_before = memory.1.load(std::sync::atomic::Ordering::SeqCst);
        let (status, headers, body) = app.raw(Method::GET, url, None, None, vec![]).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers[header::CONTENT_TYPE], content_type);
        assert_eq!(
            headers[header::CACHE_CONTROL],
            "public, max-age=31536000, immutable"
        );
        assert_eq!(body, bytes);
        assert_eq!(
            memory.1.load(std::sync::atomic::Ordering::SeqCst),
            heads_before,
            "media reads must not send a redundant HEAD request"
        );
    }

    let video_id = media_of_kind(&exercise, "video")[0]["id"]
        .as_str()
        .expect("id")
        .to_owned();
    for id in [video_id, uuid()] {
        let (status, _, _) = app
            .raw(
                Method::GET,
                &format!("/api/v1/train/exercise-media/{id}/file"),
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
            "/api/v1/train/exercises",
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
            &format!("/api/v1/train/exercise-media/{media_id}/file"),
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
async fn backfill_stores_missing_catalog_media_once(pool: PgPool) {
    let sources: Vec<(String, String)> = sqlx::query_as(
        "SELECT storage_key, source_url FROM exercise_media
         WHERE source_url IS NOT NULL ORDER BY storage_key",
    )
    .fetch_all(&pool)
    .await
    .expect("sources");
    assert_eq!(sources.len(), DATASET_EXERCISES * 2 + FIRST_CATALOG_PHOTOS);

    let memory = MemoryPhotos::default();
    let (already_stored, _) = &sources[0];
    memory
        .0
        .lock()
        .expect("lock")
        .insert(already_stored.clone(), b"kept as is".to_vec());
    let (broken_key, broken_url) = &sources[1];
    let (_, unreachable_url) = &sources[2];
    let (mismatched_key, mismatched_url) = sources
        .iter()
        .skip(3)
        .find(|(_, url)| url.ends_with(".gif"))
        .expect("an animation");
    let mut downloads: HashMap<String, Vec<u8>> = sources
        .iter()
        .map(|(_, url)| (url.clone(), download(url)))
        .collect();
    downloads.insert(broken_url.clone(), b"<html>rate limited</html>".to_vec());
    downloads.insert(mismatched_url.clone(), jpeg());
    downloads.remove(unreachable_url);

    let summary = media::backfill(&pool, &memory, &FakeFetch(downloads))
        .await
        .expect("backfill runs");
    assert_eq!(
        summary,
        BackfillSummary {
            stored: sources.len() - 4,
            present: 1,
            failed: 3,
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
        assert!(
            !stored.contains_key(mismatched_key),
            "an animation has to be a GIF"
        );
        assert_eq!(stored.len(), sources.len() - 3);
    }
    let unmarked: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM exercise_media WHERE source_url IS NOT NULL AND stored_at IS NULL",
    )
    .fetch_one(&pool)
    .await
    .expect("count unmarked");
    assert_eq!(unmarked, 3, "everything in storage is marked stored");

    let summary = media::backfill(&pool, &memory, &FakeFetch(HashMap::new()))
        .await
        .expect("second run");
    assert_eq!(
        summary,
        BackfillSummary {
            stored: 0,
            present: 0,
            failed: 3,
        },
        "only the missing files are looked at again"
    );
}
