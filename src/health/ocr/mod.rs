pub mod model;
pub mod parser;
mod provider;
mod routes;

use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use uuid::Uuid;

use crate::{
    error::{ApiError, ApiResult},
    rate_limit::RateLimiter,
};

pub use routes::router;

pub struct Runtime {
    pub client: reqwest::Client,
    pub ai: Arc<Semaphore>,
    pub rapid: Arc<Semaphore>,
    users: Mutex<HashSet<Uuid>>,
    pub ai_rate: RateLimiter,
    pub rapid_rate: RateLimiter,
}

impl Default for Runtime {
    fn default() -> Self {
        Self {
            client: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(3))
                .timeout(Duration::from_secs(25))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("valid OCR HTTP client"),
            ai: Arc::new(Semaphore::new(2)),
            rapid: Arc::new(Semaphore::new(1)),
            users: Mutex::new(HashSet::new()),
            ai_rate: RateLimiter::new(2, Duration::from_secs(60)),
            rapid_rate: RateLimiter::new(10, Duration::from_secs(60)),
        }
    }
}

pub struct Admission {
    runtime: Arc<Runtime>,
    user: Uuid,
    _permit: OwnedSemaphorePermit,
}
impl Drop for Admission {
    fn drop(&mut self) {
        self.runtime
            .users
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.user);
    }
}
impl Runtime {
    pub fn admit(self: &Arc<Self>, user: Uuid, ai: bool) -> ApiResult<Admission> {
        let permit = (if ai { &self.ai } else { &self.rapid })
            .clone()
            .try_acquire_owned()
            .map_err(|_| ApiError::OcrUnavailable("OCR is busy; try again shortly".into()))?;
        if !self
            .users
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(user)
        {
            return Err(ApiError::OcrUnavailable(
                "Another extraction is already running".into(),
            ));
        }
        Ok(Admission {
            runtime: self.clone(),
            user,
            _permit: permit,
        })
    }
}

pub async fn reserve_daily(db: &sqlx::PgPool, user: Uuid) -> ApiResult<()> {
    let mut tx = db.begin().await?;
    // ponytail: one quota lock for this single API instance, partition by day if throughput grows
    sqlx::query("SELECT pg_advisory_xact_lock(473330)")
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM ocr_ai_daily WHERE day < (now() AT TIME ZONE 'UTC')::date")
        .execute(&mut *tx)
        .await?;
    for (subject, max) in [(user.to_string(), 20), ("global".into(), 200)] {
        let count: Option<i32> = sqlx::query_scalar("INSERT INTO ocr_ai_daily (day,subject,attempts) VALUES ((now() AT TIME ZONE 'UTC')::date,$1,1) ON CONFLICT (day,subject) DO UPDATE SET attempts = ocr_ai_daily.attempts + 1 WHERE ocr_ai_daily.attempts < $2 RETURNING attempts")
            .bind(subject).bind(max).fetch_optional(&mut *tx).await?;
        if count.is_none() {
            let seconds: i64 = sqlx::query_scalar("SELECT ceil(extract(epoch FROM (((now() AT TIME ZONE 'UTC')::date + 1)::timestamp - (now() AT TIME ZONE 'UTC'))))::bigint").fetch_one(&mut *tx).await?;
            return Err(ApiError::OcrRateLimited(seconds as u64));
        }
    }
    tx.commit().await?;
    Ok(())
}
