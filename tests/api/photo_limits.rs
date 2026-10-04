use std::sync::Arc;

use axum::http::{Method, StatusCode};
use fittune_api::{AppState, config::Registration};
use sqlx::PgPool;

use crate::{
    common::{TestApp, config, uuid},
    photos::{MemoryPhotos, upload_photo},
};

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn concurrent_uploads_cannot_exceed_the_user_quota(pool: PgPool) {
    let mut config = config(Registration::Open);
    config.photo_max_count = 1;
    config.photo_upload_concurrency = 2;
    let memory = Arc::new(MemoryPhotos::default());
    let mut state = AppState::new(pool.clone(), config);
    state.photos = Some(memory.clone());
    let app = TestApp::with_state(state);
    let owner = app.register("quotaowner").await;
    let other = app.register("quotaother").await;
    let id1 = uuid();
    let id2 = uuid();
    let (first, second) = tokio::join!(
        upload_photo(&app, &owner.token, &id1),
        upload_photo(&app, &owner.token, &id2)
    );
    assert!(matches!(
        (first, second),
        (StatusCode::CREATED, StatusCode::UNPROCESSABLE_ENTITY)
            | (StatusCode::UNPROCESSABLE_ENTITY, StatusCode::CREATED)
    ));
    assert_eq!(memory.0.lock().expect("lock").len(), 2);
    let winner = if first == StatusCode::CREATED {
        &id1
    } else {
        &id2
    };
    assert_eq!(
        upload_photo(&app, &owner.token, winner).await,
        StatusCode::OK
    );
    assert_eq!(
        upload_photo(&app, &other.token, &uuid()).await,
        StatusCode::CREATED
    );
    let actual: i64 = sqlx::query_scalar("SELECT sum(stored_bytes)::bigint FROM progress_photos")
        .fetch_one(&pool)
        .await
        .expect("stored sizes");
    let stored = memory.0.lock().expect("lock");
    assert_eq!(
        actual,
        stored.values().map(|bytes| bytes.len() as i64).sum::<i64>()
    );
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn byte_quota_rejects_before_writing_storage(pool: PgPool) {
    let mut config = config(Registration::Open);
    config.photo_max_bytes = 1;
    let memory = Arc::new(MemoryPhotos::default());
    let mut state = AppState::new(pool, config);
    state.photos = Some(memory.clone());
    let app = TestApp::with_state(state);
    let owner = app.register("bytequota").await;
    assert_eq!(
        upload_photo(&app, &owner.token, &uuid()).await,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert!(memory.0.lock().expect("lock").is_empty());
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn upload_admission_bounds_work_before_reading_the_image(pool: PgPool) {
    let memory = Arc::new(MemoryPhotos::default());
    let mut state = AppState::new(pool, config(Registration::Open));
    state.photos = Some(memory);
    let slots = state.photo_uploads.clone();
    let app = TestApp::with_state(state);
    let owner = app.register("decodequota").await;
    let permit = slots.clone().try_acquire_owned().expect("first slot");
    let (status, _, _) = app
        .raw(
            Method::PUT,
            &format!("/api/v1/train/progress-photos/{}", uuid()),
            Some(&owner.token),
            Some("multipart/form-data; boundary=test-boundary"),
            b"invalid multipart".to_vec(),
        )
        .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    drop(permit);
    assert_eq!(
        upload_photo(&app, &owner.token, &uuid()).await,
        StatusCode::CREATED
    );
    assert_eq!(slots.available_permits(), 1);
}
