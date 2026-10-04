use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use anyhow::Result;
use async_trait::async_trait;
use axum::{
    body::Body,
    http::{Method, StatusCode, header},
};
use fittune_api::photos::PhotoStore;
use image::{DynamicImage, ImageFormat};
use serde_json::Value;
use sqlx::PgPool;

use crate::{
    common::{PASSWORD, TestApp, uuid},
    workouts::workout,
};

#[derive(Default)]
pub struct MemoryPhotos(pub Mutex<HashMap<String, Vec<u8>>>, pub AtomicUsize);

#[async_trait]
impl PhotoStore for MemoryPhotos {
    async fn put(&self, key: &str, bytes: Vec<u8>) -> Result<()> {
        self.0.lock().expect("lock").insert(key.into(), bytes);
        Ok(())
    }
    async fn exists(&self, key: &str) -> Result<bool> {
        self.1.fetch_add(1, Ordering::SeqCst);
        Ok(self.0.lock().expect("lock").contains_key(key))
    }
    async fn get(&self, key: &str) -> Result<Option<Body>> {
        Ok(self
            .0
            .lock()
            .expect("lock")
            .get(key)
            .cloned()
            .map(Body::from))
    }
    async fn delete(&self, key: &str) -> Result<()> {
        self.0.lock().expect("lock").remove(key);
        Ok(())
    }
}

pub(super) fn png() -> Vec<u8> {
    let mut bytes = std::io::Cursor::new(Vec::new());
    DynamicImage::new_rgb8(8, 8)
        .write_to(&mut bytes, ImageFormat::Png)
        .expect("encode PNG");
    bytes.into_inner()
}

fn form(workout_id: &str, image: &[u8]) -> Vec<u8> {
    let mut body = format!("--test-boundary\r\nContent-Disposition: form-data; name=\"workout_id\"\r\n\r\n{workout_id}\r\n--test-boundary\r\nContent-Disposition: form-data; name=\"file\"; filename=\"photo.png\"\r\nContent-Type: image/png\r\n\r\n").into_bytes();
    body.extend_from_slice(image);
    body.extend_from_slice(b"\r\n--test-boundary--\r\n");
    body
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn private_upload_retry_read_and_delete(pool: PgPool) {
    let memory = Arc::new(MemoryPhotos::default());
    let app = TestApp::with_photos(pool, Some(memory.clone()));
    let owner = app.register("photolifter").await;
    let stranger = app.register("photostranger").await;
    let exercise = app.catalog_exercise("Barbell Bench Press").await;
    let workout_id = uuid();
    let (status, _) = app
        .put(
            &format!("/api/v1/train/workouts/{workout_id}"),
            &owner.token,
            workout(
                &exercise,
                "2026-09-20T17:00:00Z",
                Some("2026-09-20T18:00:00Z"),
                1,
            ),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let photo_id = uuid();
    let url = format!("/api/v1/train/progress-photos/{photo_id}");
    let body = form(&workout_id, &png());
    let (status, _, bytes) = app
        .raw(
            Method::PUT,
            &url,
            Some(&owner.token),
            Some("multipart/form-data; boundary=test-boundary"),
            body.clone(),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&bytes)
    );
    let photo: Value = serde_json::from_slice(&bytes).expect("photo metadata");
    assert_eq!(photo["workout_id"], workout_id);
    let (status, _, _) = app
        .raw(
            Method::PUT,
            &url,
            Some(&owner.token),
            Some("multipart/form-data; boundary=test-boundary"),
            body,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, owner_photos) = app.get("/api/v1/train/progress-photos", &owner.token).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(owner_photos.as_array().map(Vec::len), Some(1));
    let (status, stranger_photos) = app
        .get("/api/v1/train/progress-photos", &stranger.token)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(stranger_photos.as_array().map(Vec::len), Some(0));
    let file_url = format!("{url}/file");
    let (status, _, _) = app
        .raw(Method::GET, &file_url, Some(&stranger.token), None, vec![])
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _, _) = app.raw(Method::GET, &file_url, None, None, vec![]).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, headers, image) = app
        .raw(Method::GET, &file_url, Some(&owner.token), None, vec![])
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers.get(header::CONTENT_TYPE).expect("content type"),
        "image/jpeg"
    );
    assert!(image.starts_with(&[0xff, 0xd8]));
    assert_eq!(memory.0.lock().expect("lock").len(), 2);
    let (status, _) = app.delete(&url, &stranger.token).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = app.delete(&url, &owner.token).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(memory.0.lock().expect("lock").is_empty());

    let (status, _, _) = app
        .raw(
            Method::PUT,
            &format!("/api/v1/train/progress-photos/{}", uuid()),
            Some(&owner.token),
            Some("multipart/form-data; boundary=test-boundary"),
            form(&workout_id, &png()),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _) = app
        .request(
            Method::DELETE,
            "/api/v1/me",
            Some(&owner.token),
            Some(serde_json::json!({ "password": PASSWORD })),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(memory.0.lock().expect("lock").is_empty());
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn rejects_invalid_images_and_foreign_workouts(pool: PgPool) {
    let app = TestApp::with_photos(pool, Some(Arc::new(MemoryPhotos::default())));
    let owner = app.register("photoowner2").await;
    let other = app.register("photoother2").await;
    let exercise = app.catalog_exercise("Barbell Bench Press").await;
    let workout_id = uuid();
    app.put(
        &format!("/api/v1/train/workouts/{workout_id}"),
        &other.token,
        workout(
            &exercise,
            "2026-09-20T17:00:00Z",
            Some("2026-09-20T18:00:00Z"),
            1,
        ),
    )
    .await;
    let url = format!("/api/v1/train/progress-photos/{}", uuid());
    let (status, _, _) = app
        .raw(
            Method::PUT,
            &url,
            Some(&owner.token),
            Some("multipart/form-data; boundary=test-boundary"),
            form(&workout_id, &png()),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, _, _) = app
        .raw(
            Method::PUT,
            &url,
            Some(&other.token),
            Some("multipart/form-data; boundary=test-boundary"),
            form(&workout_id, b"not an image"),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn photo_route_allows_more_than_json_limit_but_rejects_oversized_uploads(pool: PgPool) {
    let app = TestApp::with_photos(pool, Some(Arc::new(MemoryPhotos::default())));
    let owner = app.register("photosizetest").await;
    let exercise = app.catalog_exercise("Barbell Bench Press").await;
    let workout_id = uuid();
    app.put(
        &format!("/api/v1/train/workouts/{workout_id}"),
        &owner.token,
        workout(
            &exercise,
            "2026-09-20T17:00:00Z",
            Some("2026-09-20T18:00:00Z"),
            1,
        ),
    )
    .await;
    let mut image = png();
    image.extend(vec![0; 2 * 1024 * 1024]);
    let (status, _, bytes) = app
        .raw(
            Method::PUT,
            &format!("/api/v1/train/progress-photos/{}", uuid()),
            Some(&owner.token),
            Some("multipart/form-data; boundary=test-boundary"),
            form(&workout_id, &image),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&bytes)
    );

    image.extend(vec![0; 9 * 1024 * 1024]);
    let (status, _, _) = app
        .raw(
            Method::PUT,
            &format!("/api/v1/train/progress-photos/{}", uuid()),
            Some(&owner.token),
            Some("multipart/form-data; boundary=test-boundary"),
            form(&workout_id, &image),
        )
        .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn upload_with_an_id_owned_by_another_user_is_a_conflict(pool: PgPool) {
    let memory = Arc::new(MemoryPhotos::default());
    let app = TestApp::with_photos(pool, Some(memory.clone()));
    let owner = app.register("idowner").await;
    let stranger = app.register("idclaimer").await;
    let url = format!("/api/v1/train/progress-photos/{}", uuid());
    let mut body = b"--test-boundary\r\nContent-Disposition: form-data; name=\"file\"; filename=\"photo.png\"\r\nContent-Type: image/png\r\n\r\n".to_vec();
    body.extend_from_slice(&png());
    body.extend_from_slice(b"\r\n--test-boundary--\r\n");
    let content_type = Some("multipart/form-data; boundary=test-boundary");

    let (status, _, _) = app
        .raw(
            Method::PUT,
            &url,
            Some(&owner.token),
            content_type,
            body.clone(),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let stored = memory.0.lock().expect("lock").len();

    let (status, _, _) = app
        .raw(Method::PUT, &url, Some(&stranger.token), content_type, body)
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(memory.0.lock().expect("lock").len(), stored);
}

pub(super) async fn upload_photo(app: &TestApp, token: &str, id: &str) -> StatusCode {
    let mut body = b"--test-boundary\r\nContent-Disposition: form-data; name=\"file\"; filename=\"photo.png\"\r\nContent-Type: image/png\r\n\r\n".to_vec();
    body.extend_from_slice(&png());
    body.extend_from_slice(b"\r\n--test-boundary--\r\n");
    app.raw(
        Method::PUT,
        &format!("/api/v1/train/progress-photos/{id}"),
        Some(token),
        Some("multipart/form-data; boundary=test-boundary"),
        body,
    )
    .await
    .0
}
