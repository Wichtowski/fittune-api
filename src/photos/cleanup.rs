use std::time::Duration;

use anyhow::Result;
use sqlx::PgPool;

use super::PhotoStore;
use crate::app::AppState;

/// Records upload keys before writing storage, including cancelled or failed uploads
pub async fn reserve(db: &PgPool, key: &str) -> Result<()> {
    sqlx::query("INSERT INTO photo_cleanup (storage_key, available_at) VALUES ($1, now() + interval '1 hour')")
        .bind(key).execute(db).await?;
    Ok(())
}

/// Makes a known failed upload eligible for the next cleanup sweep
pub async fn expedite(db: &PgPool, key: &str) -> Result<()> {
    sqlx::query("UPDATE photo_cleanup SET available_at = now() WHERE storage_key = $1")
        .bind(key)
        .execute(db)
        .await?;
    Ok(())
}

/// Registers a reconciled orphan only when no live photo or pending upload owns its keys
pub async fn queue_orphan(db: &PgPool, key: &str) -> Result<u64> {
    Ok(sqlx::query(
        "INSERT INTO photo_cleanup (storage_key)
         SELECT $1 WHERE NOT EXISTS (SELECT 1 FROM progress_photos WHERE storage_key = $1)
         ON CONFLICT (storage_key) DO NOTHING",
    )
    .bind(key)
    .execute(db)
    .await?
    .rows_affected())
}

/// Outcome of one bounded cleanup batch
#[derive(Debug, Default)]
pub struct Summary {
    pub deleted: usize,
    pub failed: usize,
}

/// Retries a bounded batch, locking each key until its idempotent deletes finish
pub async fn drain(db: &PgPool, storage: &dyn PhotoStore, limit: usize) -> Result<Summary> {
    let mut summary = Summary::default();
    for _ in 0..limit {
        let mut tx = db.begin().await?;
        let key: Option<String> = sqlx::query_scalar(
            "SELECT storage_key FROM photo_cleanup WHERE available_at <= now()
             ORDER BY available_at, storage_key LIMIT 1 FOR UPDATE SKIP LOCKED",
        )
        .fetch_optional(&mut *tx)
        .await?;
        let Some(key) = key else { break };
        let referenced: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM progress_photos WHERE storage_key = $1)",
        )
        .bind(&key)
        .fetch_one(&mut *tx)
        .await?;
        let mut failed = false;
        if !referenced {
            for suffix in ["full", "thumb"] {
                if let Err(error) = storage.delete(&format!("{key}/{suffix}.jpg")).await {
                    tracing::warn!(key, %error, "photo cleanup failed; queued for retry");
                    failed = true;
                }
            }
        }
        if failed {
            sqlx::query("UPDATE photo_cleanup SET available_at = now() + interval '5 minutes' WHERE storage_key = $1")
                .bind(&key).execute(&mut *tx).await?;
            summary.failed += 1;
        } else {
            sqlx::query("DELETE FROM photo_cleanup WHERE storage_key = $1")
                .bind(&key)
                .execute(&mut *tx)
                .await?;
            summary.deleted += usize::from(!referenced);
        }
        tx.commit().await?;
    }
    Ok(summary)
}

/// A short eager cleanup preserves normal deletion UX without making storage an account dependency
pub async fn after_delete(state: &AppState) {
    let Some(storage) = &state.photos else { return };
    match tokio::time::timeout(
        Duration::from_secs(2),
        drain(&state.db, storage.as_ref(), 100),
    )
    .await
    {
        Ok(Ok(_)) => {}
        Ok(Err(error)) => tracing::warn!(%error, "photo cleanup deferred"),
        Err(_) => tracing::warn!("photo cleanup timed out; queued for retry"),
    }
}
