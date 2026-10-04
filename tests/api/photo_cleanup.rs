use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use anyhow::{Result, bail};
use async_trait::async_trait;
use axum::{
    body::Body,
    http::{Method, StatusCode},
};
use fittune_api::{
    AppState,
    config::Registration,
    photos::{PhotoStore, cleanup},
};
use serde_json::json;
use sqlx::PgPool;

use crate::{
    common::{PASSWORD, TestApp, config, uuid},
    photos::{MemoryPhotos, upload_photo},
};

#[derive(Default)]
struct FaultyPhotos {
    memory: MemoryPhotos,
    fail_delete: AtomicBool,
    fail_thumb: AtomicBool,
}

#[async_trait]
impl PhotoStore for FaultyPhotos {
    async fn put(&self, key: &str, bytes: Vec<u8>) -> Result<()> {
        if self.fail_thumb.load(Ordering::SeqCst) && key.ends_with("/thumb.jpg") {
            bail!("storage write unavailable");
        }
        self.memory.put(key, bytes).await
    }
    async fn exists(&self, key: &str) -> Result<bool> {
        self.memory.exists(key).await
    }
    async fn get(&self, key: &str) -> Result<Option<Body>> {
        self.memory.get(key).await
    }
    async fn delete(&self, key: &str) -> Result<()> {
        if self.fail_delete.load(Ordering::SeqCst) {
            bail!("storage delete unavailable")
        }
        self.memory.delete(key).await
    }
}

async fn queued(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM photo_cleanup")
        .fetch_one(pool)
        .await
        .expect("queue size")
}

async fn retry_now(pool: &PgPool) {
    sqlx::query("UPDATE photo_cleanup SET available_at = now()")
        .execute(pool)
        .await
        .expect("schedule retry");
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn failed_storage_deletes_do_not_block_photo_or_account_deletion(pool: PgPool) {
    let storage = Arc::new(FaultyPhotos::default());
    let app = TestApp::with_photos(pool.clone(), Some(storage.clone()));
    let owner = app.register("deletefail").await;
    let id = uuid();
    assert_eq!(
        upload_photo(&app, &owner.token, &id).await,
        StatusCode::CREATED
    );
    storage.fail_delete.store(true, Ordering::SeqCst);
    assert_eq!(
        app.delete(&format!("/api/v1/train/progress-photos/{id}"), &owner.token)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(queued(&pool).await, 1);
    assert_eq!(storage.memory.0.lock().expect("lock").len(), 2);
    assert_eq!(
        upload_photo(&app, &owner.token, &uuid()).await,
        StatusCode::CREATED
    );
    assert_eq!(
        app.request(
            Method::DELETE,
            "/api/v1/me",
            Some(&owner.token),
            Some(json!({"password": PASSWORD}))
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(queued(&pool).await, 2);
    storage.fail_delete.store(false, Ordering::SeqCst);
    retry_now(&pool).await;
    assert_eq!(
        cleanup::drain(&pool, storage.as_ref(), 100)
            .await
            .expect("retry")
            .deleted,
        2
    );
    assert_eq!(queued(&pool).await, 0);
    assert!(storage.memory.0.lock().expect("lock").is_empty());
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn account_deletion_succeeds_with_photo_storage_unconfigured(pool: PgPool) {
    let memory = Arc::new(MemoryPhotos::default());
    let app = TestApp::with_photos(pool.clone(), Some(memory.clone()));
    let owner = app.register("nostorage").await;
    assert_eq!(
        upload_photo(&app, &owner.token, &uuid()).await,
        StatusCode::CREATED
    );
    let without_storage = TestApp::new(pool.clone());
    assert_eq!(
        without_storage
            .request(
                Method::DELETE,
                "/api/v1/me",
                Some(&owner.token),
                Some(json!({"password": PASSWORD}))
            )
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(queued(&pool).await, 1);
    assert_eq!(
        cleanup::drain(&pool, memory.as_ref(), 100)
            .await
            .expect("retry")
            .deleted,
        1
    );
    assert!(memory.0.lock().expect("lock").is_empty());
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn database_delete_failure_preserves_photos_and_does_not_enqueue_cleanup(pool: PgPool) {
    let memory = Arc::new(MemoryPhotos::default());
    let app = TestApp::with_photos(pool.clone(), Some(memory.clone()));
    let owner = app.register("dbdeletefail").await;
    let id = uuid();
    assert_eq!(
        upload_photo(&app, &owner.token, &id).await,
        StatusCode::CREATED
    );
    sqlx::query(
        "CREATE FUNCTION reject_user_delete() RETURNS trigger LANGUAGE plpgsql AS $$
        BEGIN RAISE EXCEPTION 'forced account delete failure'; END; $$",
    )
    .execute(&pool)
    .await
    .expect("failure function");
    sqlx::query(
        "CREATE TRIGGER reject_user_delete BEFORE DELETE ON users
        FOR EACH ROW EXECUTE FUNCTION reject_user_delete()",
    )
    .execute(&pool)
    .await
    .expect("failure trigger");
    assert_eq!(
        app.request(
            Method::DELETE,
            "/api/v1/me",
            Some(&owner.token),
            Some(json!({"password": PASSWORD}))
        )
        .await
        .0,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(queued(&pool).await, 0);
    assert_eq!(memory.0.lock().expect("lock").len(), 2);
    let (status, _, _) = app
        .raw(
            Method::GET,
            &format!("/api/v1/train/progress-photos/{id}/file"),
            Some(&owner.token),
            None,
            vec![],
        )
        .await;
    assert_eq!(status, StatusCode::OK);
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn partial_upload_failures_are_durable_and_cleanup_workers_are_idempotent(pool: PgPool) {
    let storage = Arc::new(FaultyPhotos::default());
    let app = TestApp::with_photos(pool.clone(), Some(storage.clone()));
    let owner = app.register("partialupload").await;
    storage.fail_thumb.store(true, Ordering::SeqCst);
    assert_eq!(
        upload_photo(&app, &owner.token, &uuid()).await,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(queued(&pool).await, 1);
    assert_eq!(storage.memory.0.lock().expect("lock").len(), 1);
    let (a, b) = tokio::join!(
        cleanup::drain(&pool, storage.as_ref(), 100),
        cleanup::drain(&pool, storage.as_ref(), 100)
    );
    assert_eq!(
        a.expect("worker a").deleted + b.expect("worker b").deleted,
        1
    );
    assert_eq!(queued(&pool).await, 0);
    assert!(storage.memory.0.lock().expect("lock").is_empty());
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn reconciliation_keeps_live_photos_and_inflight_uploads(pool: PgPool) {
    let memory = Arc::new(MemoryPhotos::default());
    let mut state = AppState::new(pool.clone(), config(Registration::Open));
    state.photos = Some(memory.clone());
    let app = TestApp::with_state(state);
    let owner = app.register("reconcile").await;
    assert_eq!(
        upload_photo(&app, &owner.token, &uuid()).await,
        StatusCode::CREATED
    );
    let live: String = sqlx::query_scalar("SELECT storage_key FROM progress_photos")
        .fetch_one(&pool)
        .await
        .expect("live key");
    assert_eq!(cleanup::queue_orphan(&pool, &live).await.expect("live"), 0);
    cleanup::reserve(&pool, "photos/pending")
        .await
        .expect("reservation");
    assert_eq!(
        cleanup::queue_orphan(&pool, "photos/pending")
            .await
            .expect("pending"),
        0
    );
    memory
        .put("photos/pending/full.jpg", vec![1])
        .await
        .expect("pending object");
    memory
        .put("photos/orphan/full.jpg", vec![1])
        .await
        .expect("orphan");
    assert_eq!(
        cleanup::queue_orphan(&pool, "photos/orphan")
            .await
            .expect("orphan"),
        1
    );
    assert_eq!(
        cleanup::drain(&pool, memory.as_ref(), 100)
            .await
            .expect("sweep")
            .deleted,
        1
    );
    assert_eq!(memory.0.lock().expect("lock").len(), 3);
    retry_now(&pool).await;
    assert_eq!(
        cleanup::drain(&pool, memory.as_ref(), 100)
            .await
            .expect("abandoned upload")
            .deleted,
        1
    );
    assert_eq!(memory.0.lock().expect("lock").len(), 2);
}
