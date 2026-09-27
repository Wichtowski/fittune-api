use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use super::model::{Activity, ActivityDraft, ActivityKind};

macro_rules! activity_columns {
    () => {
        "id, kind, title, notes, started_at, duration_seconds, distance_m, elevation_gain_m, \
         avg_heart_rate, calories, perceived_effort, created_at, updated_at"
    };
}

pub enum UpsertOutcome {
    Created(Activity),
    Updated(Activity),
    /// The id belongs to another user.
    Foreign,
}

pub async fn upsert(
    db: &PgPool,
    user_id: Uuid,
    id: Uuid,
    draft: &ActivityDraft,
) -> sqlx::Result<UpsertOutcome> {
    let row: Option<ActivityWithFlag> = sqlx::query_as(concat!(
        "INSERT INTO activities (id, user_id, kind, title, notes, started_at, duration_seconds, distance_m,
                                 elevation_gain_m, avg_heart_rate, calories, perceived_effort)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
         ON CONFLICT (id) DO UPDATE SET
            kind = EXCLUDED.kind, title = EXCLUDED.title, notes = EXCLUDED.notes,
            started_at = EXCLUDED.started_at, duration_seconds = EXCLUDED.duration_seconds,
            distance_m = EXCLUDED.distance_m, elevation_gain_m = EXCLUDED.elevation_gain_m,
            avg_heart_rate = EXCLUDED.avg_heart_rate, calories = EXCLUDED.calories,
            perceived_effort = EXCLUDED.perceived_effort, updated_at = now()
         WHERE activities.user_id = EXCLUDED.user_id
         RETURNING xmax = 0 AS inserted, ",
        activity_columns!()
    ))
    .bind(id)
    .bind(user_id)
    .bind(draft.kind)
    .bind(&draft.title)
    .bind(&draft.notes)
    .bind(draft.started_at)
    .bind(draft.duration_seconds)
    .bind(draft.distance_m)
    .bind(draft.elevation_gain_m)
    .bind(draft.avg_heart_rate)
    .bind(draft.calories)
    .bind(draft.perceived_effort)
    .fetch_optional(db)
    .await?;

    Ok(match row {
        Some(ActivityWithFlag {
            inserted: true,
            activity,
        }) => UpsertOutcome::Created(activity),
        Some(ActivityWithFlag {
            inserted: false,
            activity,
        }) => UpsertOutcome::Updated(activity),
        None => UpsertOutcome::Foreign,
    })
}

#[derive(sqlx::FromRow)]
struct ActivityWithFlag {
    inserted: bool,
    #[sqlx(flatten)]
    activity: Activity,
}

pub async fn find(db: &PgPool, user_id: Uuid, id: Uuid) -> sqlx::Result<Option<Activity>> {
    sqlx::query_as(concat!(
        "SELECT ",
        activity_columns!(),
        " FROM activities WHERE id = $1 AND user_id = $2"
    ))
    .bind(id)
    .bind(user_id)
    .fetch_optional(db)
    .await
}

/// Newest first, keyset-paginated on `(started_at, id)`.
pub async fn list(
    db: &PgPool,
    user_id: Uuid,
    kind: Option<ActivityKind>,
    before: Option<(DateTime<Utc>, Uuid)>,
    limit: i64,
) -> sqlx::Result<Vec<Activity>> {
    let (before_at, before_id) = before.unzip();
    sqlx::query_as(concat!(
        "SELECT ",
        activity_columns!(),
        " FROM activities
          WHERE user_id = $1
            AND ($2::text IS NULL OR kind = $2)
            AND ($3::timestamptz IS NULL OR (started_at, id) < ($3, $4))
          ORDER BY started_at DESC, id DESC
          LIMIT $5"
    ))
    .bind(user_id)
    .bind(kind)
    .bind(before_at)
    .bind(before_id)
    .bind(limit)
    .fetch_all(db)
    .await
}

pub async fn delete(db: &PgPool, user_id: Uuid, id: Uuid) -> sqlx::Result<bool> {
    let result = sqlx::query("DELETE FROM activities WHERE id = $1 AND user_id = $2")
        .bind(id)
        .bind(user_id)
        .execute(db)
        .await?;
    Ok(result.rows_affected() > 0)
}
