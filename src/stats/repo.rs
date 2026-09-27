//! Aggregation queries. Parameters follow one convention: `$1` user id, `$2`/`$3` inclusive
//! from/to dates, `$4` IANA time zone. Only finished workouts and completed, non-warm-up sets count.

use chrono::NaiveDate;
use sqlx::PgPool;
use uuid::Uuid;

use super::model::{Bucket, ExerciseRecord, MuscleVolume, Period, TimelinePoint, Totals};

/// `started_at` lies within the period, both ends taken as local midnights in `$4`.
macro_rules! in_period {
    ($column:literal) => {
        concat!(
            $column,
            " >= ($2::date)::timestamp AT TIME ZONE $4 AND ",
            $column,
            " < ($3::date + 1)::timestamp AT TIME ZONE $4"
        )
    };
}

pub async fn totals(db: &PgPool, user_id: Uuid, period: Period, tz: &str) -> sqlx::Result<Totals> {
    sqlx::query_as(concat!(
        "WITH w AS (
            SELECT id, started_at, ended_at FROM workouts
            WHERE user_id = $1 AND ended_at IS NOT NULL AND ", in_period!("started_at"), "
         ),
         s AS (
            SELECT ws.reps, ws.weight_kg FROM workout_sets ws
            JOIN workout_exercises we ON we.id = ws.workout_exercise_id
            JOIN w ON w.id = we.workout_id
            WHERE ws.completed AND ws.kind <> 'warmup'
         ),
         a AS (
            SELECT duration_seconds, distance_m FROM activities
            WHERE user_id = $1 AND ", in_period!("started_at"), "
         )
         SELECT
            (SELECT count(*) FROM w) AS workouts,
            (SELECT COALESCE(sum(EXTRACT(EPOCH FROM ended_at - started_at)), 0)::bigint FROM w) AS workout_seconds,
            (SELECT count(*) FROM s) AS sets,
            (SELECT COALESCE(sum(reps), 0)::bigint FROM s) AS reps,
            (SELECT COALESCE(sum(weight_kg * reps), 0)::float8 FROM s) AS volume_kg,
            (SELECT count(*) FROM a) AS activities,
            (SELECT COALESCE(sum(duration_seconds), 0)::bigint FROM a) AS activity_seconds,
            (SELECT COALESCE(sum(distance_m), 0)::float8 FROM a) AS activity_distance_m"
    ))
    .bind(user_id)
    .bind(period.from)
    .bind(period.to)
    .bind(tz)
    .fetch_one(db)
    .await
}

/// Distinct Monday-based weeks with any finished workout or activity, newest first,
/// and the current week, both in `tz`.
pub async fn active_weeks(
    db: &PgPool,
    user_id: Uuid,
    tz: &str,
) -> sqlx::Result<(Vec<NaiveDate>, NaiveDate)> {
    let weeks = sqlx::query_scalar(
        "SELECT DISTINCT date_trunc('week', started_at AT TIME ZONE $2)::date AS week
         FROM (
            SELECT started_at FROM workouts WHERE user_id = $1 AND ended_at IS NOT NULL
            UNION ALL
            SELECT started_at FROM activities WHERE user_id = $1
         ) sessions
         ORDER BY week DESC
         LIMIT 520",
    )
    .bind(user_id)
    .bind(tz)
    .fetch_all(db)
    .await?;
    let current = sqlx::query_scalar("SELECT date_trunc('week', now() AT TIME ZONE $1)::date")
        .bind(tz)
        .fetch_one(db)
        .await?;
    Ok((weeks, current))
}

/// One point per bucket in the period, zero-filled.
pub async fn timeline(
    db: &PgPool,
    user_id: Uuid,
    period: Period,
    tz: &str,
    bucket: Bucket,
) -> sqlx::Result<Vec<TimelinePoint>> {
    sqlx::query_as(concat!(
        "WITH series AS (
            SELECT generate_series(date_trunc($5, $2::date::timestamp),
                                   date_trunc($5, $3::date::timestamp),
                                   ('1 ' || $5)::interval)::date AS bucket
         ),
         w AS (
            SELECT date_trunc($5, started_at AT TIME ZONE $4)::date AS bucket,
                   count(*) AS workouts,
                   sum(EXTRACT(EPOCH FROM ended_at - started_at))::bigint AS workout_seconds
            FROM workouts
            WHERE user_id = $1 AND ended_at IS NOT NULL AND ",
        in_period!("started_at"),
        "
            GROUP BY 1
         ),
         s AS (
            SELECT date_trunc($5, w.started_at AT TIME ZONE $4)::date AS bucket,
                   count(*) AS sets,
                   COALESCE(sum(ws.reps), 0)::bigint AS reps,
                   COALESCE(sum(ws.weight_kg * ws.reps), 0)::float8 AS volume_kg
            FROM workout_sets ws
            JOIN workout_exercises we ON we.id = ws.workout_exercise_id
            JOIN workouts w ON w.id = we.workout_id
            WHERE w.user_id = $1 AND w.ended_at IS NOT NULL AND ws.completed AND ws.kind <> 'warmup'
              AND ",
        in_period!("w.started_at"),
        "
            GROUP BY 1
         ),
         a AS (
            SELECT date_trunc($5, started_at AT TIME ZONE $4)::date AS bucket,
                   count(*) AS activities,
                   sum(duration_seconds)::bigint AS activity_seconds,
                   COALESCE(sum(distance_m), 0)::float8 AS activity_distance_m
            FROM activities
            WHERE user_id = $1 AND ",
        in_period!("started_at"),
        "
            GROUP BY 1
         )
         SELECT series.bucket,
                COALESCE(w.workouts, 0) AS workouts,
                COALESCE(w.workout_seconds, 0) AS workout_seconds,
                COALESCE(s.sets, 0) AS sets,
                COALESCE(s.reps, 0) AS reps,
                COALESCE(s.volume_kg, 0) AS volume_kg,
                COALESCE(a.activities, 0) AS activities,
                COALESCE(a.activity_seconds, 0) AS activity_seconds,
                COALESCE(a.activity_distance_m, 0) AS activity_distance_m
         FROM series
         LEFT JOIN w USING (bucket)
         LEFT JOIN s USING (bucket)
         LEFT JOIN a USING (bucket)
         ORDER BY series.bucket"
    ))
    .bind(user_id)
    .bind(period.from)
    .bind(period.to)
    .bind(tz)
    .bind(bucket.as_sql())
    .fetch_all(db)
    .await
}

/// Working sets and volume per primary muscle, most trained first.
pub async fn muscles(
    db: &PgPool,
    user_id: Uuid,
    period: Period,
    tz: &str,
) -> sqlx::Result<Vec<MuscleVolume>> {
    sqlx::query_as(concat!(
        "SELECT e.primary_muscle AS muscle,
                count(*) AS sets,
                COALESCE(sum(ws.weight_kg * ws.reps), 0)::float8 AS volume_kg
         FROM workout_sets ws
         JOIN workout_exercises we ON we.id = ws.workout_exercise_id
         JOIN workouts w ON w.id = we.workout_id
         JOIN exercises e ON e.id = we.exercise_id
         WHERE w.user_id = $1 AND w.ended_at IS NOT NULL AND ws.completed AND ws.kind <> 'warmup'
           AND ",
        in_period!("w.started_at"),
        "
         GROUP BY e.primary_muscle
         ORDER BY sets DESC, muscle"
    ))
    .bind(user_id)
    .bind(period.from)
    .bind(period.to)
    .bind(tz)
    .fetch_all(db)
    .await
}

/// All-time bests per exercise, most recently trained first.
pub async fn records(db: &PgPool, user_id: Uuid) -> sqlx::Result<Vec<ExerciseRecord>> {
    sqlx::query_as(
        "SELECT e.id AS exercise_id, e.name AS exercise_name, e.tracking, e.primary_muscle,
                max(ws.weight_kg) AS max_weight_kg,
                max(estimated_1rm(ws.weight_kg, ws.reps)) AS best_e1rm_kg,
                max(ws.reps) AS max_reps,
                max(ws.duration_seconds) AS max_duration_seconds,
                max(ws.distance_m) AS max_distance_m,
                count(DISTINCT w.id) AS sessions,
                max(w.started_at) AS last_performed_at
         FROM workout_sets ws
         JOIN workout_exercises we ON we.id = ws.workout_exercise_id
         JOIN workouts w ON w.id = we.workout_id
         JOIN exercises e ON e.id = we.exercise_id
         WHERE w.user_id = $1 AND w.ended_at IS NOT NULL AND ws.completed AND ws.kind <> 'warmup'
         GROUP BY e.id
         ORDER BY last_performed_at DESC, e.name",
    )
    .bind(user_id)
    .fetch_all(db)
    .await
}
