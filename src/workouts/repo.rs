use std::collections::HashMap;

use chrono::{DateTime, Utc};
use sqlx::{FromRow, PgConnection, PgPool, Postgres, Transaction};
use uuid::Uuid;

use super::model::{Workout, WorkoutDraft, WorkoutExercise, WorkoutSet, WorkoutSummary};
use crate::exercises::model::{Muscle, Tracking};

pub enum UpsertOutcome {
    Created,
    Updated,
    /// The id belongs to another user.
    Foreign,
    /// A newer revision is already stored.
    Stale,
}

/// Inserts or replaces a workout document. Children are rewritten wholesale inside the
/// caller's transaction; the upsert's row lock serialises concurrent writes to one workout.
pub async fn upsert(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    id: Uuid,
    draft: &WorkoutDraft,
) -> sqlx::Result<UpsertOutcome> {
    let inserted: Option<bool> = sqlx::query_scalar(
        "INSERT INTO workouts (id, user_id, routine_id, title, notes, started_at, ended_at, revision)
         VALUES ($1, $2, (SELECT id FROM routines WHERE id = $3 AND user_id = $2), $4, $5, $6, $7, $8)
         ON CONFLICT (id) DO UPDATE SET
            routine_id = EXCLUDED.routine_id,
            title = EXCLUDED.title,
            notes = EXCLUDED.notes,
            started_at = EXCLUDED.started_at,
            ended_at = EXCLUDED.ended_at,
            revision = EXCLUDED.revision,
            updated_at = now()
         WHERE workouts.user_id = EXCLUDED.user_id AND workouts.revision <= EXCLUDED.revision
         RETURNING xmax = 0",
    )
    .bind(id)
    .bind(user_id)
    .bind(draft.routine_id)
    .bind(&draft.title)
    .bind(&draft.notes)
    .bind(draft.started_at)
    .bind(draft.ended_at)
    .bind(draft.revision)
    .fetch_optional(&mut **tx)
    .await?;

    let Some(inserted) = inserted else {
        let owner: Option<Uuid> = sqlx::query_scalar("SELECT user_id FROM workouts WHERE id = $1")
            .bind(id)
            .fetch_optional(&mut **tx)
            .await?;
        return Ok(if owner == Some(user_id) {
            UpsertOutcome::Stale
        } else {
            UpsertOutcome::Foreign
        });
    };

    replace_children(tx, id, draft).await?;
    Ok(if inserted {
        UpsertOutcome::Created
    } else {
        UpsertOutcome::Updated
    })
}

async fn replace_children(
    tx: &mut Transaction<'_, Postgres>,
    workout_id: Uuid,
    draft: &WorkoutDraft,
) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM workout_exercises WHERE workout_id = $1")
        .bind(workout_id)
        .execute(&mut **tx)
        .await?;
    if draft.exercises.is_empty() {
        return Ok(());
    }

    let mut ex = ExerciseColumns::default();
    let mut sets = SetColumns::default();
    for (position, exercise) in (0_i32..).zip(&draft.exercises) {
        ex.id.push(exercise.id);
        ex.exercise_id.push(exercise.exercise_id);
        ex.position.push(position);
        ex.rest_seconds.push(exercise.rest_seconds);
        ex.notes.push(exercise.notes.clone());
        for (set_position, set) in (0_i32..).zip(&exercise.sets) {
            sets.id.push(set.id);
            sets.workout_exercise_id.push(exercise.id);
            sets.position.push(set_position);
            sets.kind.push(set.kind);
            sets.reps.push(set.reps);
            sets.weight_kg.push(set.weight_kg);
            sets.duration_seconds.push(set.duration_seconds);
            sets.distance_m.push(set.distance_m);
            sets.rpe.push(set.rpe);
            sets.completed.push(set.completed);
        }
    }

    sqlx::query(
        "INSERT INTO workout_exercises (id, workout_id, exercise_id, position, rest_seconds, notes)
         SELECT t.id, $1, t.exercise_id, t.position, t.rest_seconds, t.notes
         FROM UNNEST($2::uuid[], $3::uuid[], $4::int[], $5::int[], $6::text[])
              AS t(id, exercise_id, position, rest_seconds, notes)",
    )
    .bind(workout_id)
    .bind(&ex.id)
    .bind(&ex.exercise_id)
    .bind(&ex.position)
    .bind(&ex.rest_seconds)
    .bind(&ex.notes)
    .execute(&mut **tx)
    .await?;

    if sets.id.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO workout_sets (id, workout_exercise_id, position, kind, reps, weight_kg,
                                   duration_seconds, distance_m, rpe, completed)
         SELECT * FROM UNNEST($1::uuid[], $2::uuid[], $3::int[], $4::text[], $5::int[], $6::float8[],
                              $7::int[], $8::float8[], $9::float8[], $10::bool[])",
    )
    .bind(&sets.id)
    .bind(&sets.workout_exercise_id)
    .bind(&sets.position)
    .bind(&sets.kind)
    .bind(&sets.reps)
    .bind(&sets.weight_kg)
    .bind(&sets.duration_seconds)
    .bind(&sets.distance_m)
    .bind(&sets.rpe)
    .bind(&sets.completed)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

#[derive(Default)]
struct ExerciseColumns {
    id: Vec<Uuid>,
    exercise_id: Vec<Uuid>,
    position: Vec<i32>,
    rest_seconds: Vec<Option<i32>>,
    notes: Vec<Option<String>>,
}

#[derive(Default)]
struct SetColumns {
    id: Vec<Uuid>,
    workout_exercise_id: Vec<Uuid>,
    position: Vec<i32>,
    kind: Vec<super::model::SetKind>,
    reps: Vec<Option<i32>>,
    weight_kg: Vec<Option<f64>>,
    duration_seconds: Vec<Option<i32>>,
    distance_m: Vec<Option<f64>>,
    rpe: Vec<Option<f64>>,
    completed: Vec<bool>,
}

#[derive(FromRow)]
struct WorkoutRow {
    id: Uuid,
    routine_id: Option<Uuid>,
    title: String,
    notes: Option<String>,
    started_at: DateTime<Utc>,
    ended_at: Option<DateTime<Utc>>,
    revision: i64,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(FromRow)]
struct WorkoutExerciseRow {
    id: Uuid,
    exercise_id: Uuid,
    exercise_name: String,
    tracking: Tracking,
    primary_muscle: Muscle,
    notes: Option<String>,
    rest_seconds: Option<i32>,
}

pub async fn find(
    conn: &mut PgConnection,
    user_id: Uuid,
    id: Uuid,
) -> sqlx::Result<Option<Workout>> {
    let Some(row): Option<WorkoutRow> = sqlx::query_as(
        "SELECT id, routine_id, title, notes, started_at, ended_at, revision, created_at, updated_at
         FROM workouts WHERE id = $1 AND user_id = $2",
    )
    .bind(id)
    .bind(user_id)
    .fetch_optional(&mut *conn)
    .await?
    else {
        return Ok(None);
    };

    let exercise_rows: Vec<WorkoutExerciseRow> = sqlx::query_as(
        "SELECT we.id, we.exercise_id, e.name AS exercise_name, e.tracking, e.primary_muscle,
                we.notes, we.rest_seconds
         FROM workout_exercises we JOIN exercises e ON e.id = we.exercise_id
         WHERE we.workout_id = $1
         ORDER BY we.position",
    )
    .bind(id)
    .fetch_all(&mut *conn)
    .await?;

    let set_rows: Vec<WorkoutSet> = sqlx::query_as(
        "SELECT ws.id, ws.workout_exercise_id, ws.kind, ws.reps, ws.weight_kg, ws.duration_seconds,
                ws.distance_m, ws.rpe, ws.completed
         FROM workout_sets ws JOIN workout_exercises we ON we.id = ws.workout_exercise_id
         WHERE we.workout_id = $1
         ORDER BY ws.position",
    )
    .bind(id)
    .fetch_all(&mut *conn)
    .await?;

    let mut sets_by_exercise: HashMap<Uuid, Vec<WorkoutSet>> = HashMap::new();
    for set in set_rows {
        sets_by_exercise
            .entry(set.workout_exercise_id)
            .or_default()
            .push(set);
    }

    let exercises = exercise_rows
        .into_iter()
        .map(|row| WorkoutExercise {
            sets: sets_by_exercise.remove(&row.id).unwrap_or_default(),
            id: row.id,
            exercise_id: row.exercise_id,
            exercise_name: row.exercise_name,
            tracking: row.tracking,
            primary_muscle: row.primary_muscle,
            notes: row.notes,
            rest_seconds: row.rest_seconds,
        })
        .collect();

    Ok(Some(Workout {
        id: row.id,
        routine_id: row.routine_id,
        title: row.title,
        notes: row.notes,
        started_at: row.started_at,
        ended_at: row.ended_at,
        revision: row.revision,
        created_at: row.created_at,
        updated_at: row.updated_at,
        exercises,
    }))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatusFilter {
    InProgress,
    Completed,
}

pub struct ListParams {
    pub status: Option<StatusFilter>,
    pub before: Option<(DateTime<Utc>, Uuid)>,
    pub limit: i64,
}

/// Newest first, keyset-paginated on `(started_at, id)`.
pub async fn list(
    db: &PgPool,
    user_id: Uuid,
    params: &ListParams,
) -> sqlx::Result<Vec<WorkoutSummary>> {
    let (before_at, before_id) = params.before.unzip();
    sqlx::query_as(
        "SELECT w.id, w.routine_id, w.title, w.started_at, w.ended_at,
                EXTRACT(EPOCH FROM (w.ended_at - w.started_at))::bigint AS duration_seconds,
                COALESCE(agg.exercise_count, 0) AS exercise_count,
                COALESCE(agg.set_count, 0) AS set_count,
                COALESCE(agg.total_reps, 0) AS total_reps,
                COALESCE(agg.volume_kg, 0) AS volume_kg,
                COALESCE(agg.exercise_names, '{}') AS exercise_names
         FROM workouts w
         LEFT JOIN LATERAL (
             SELECT count(DISTINCT we.id) AS exercise_count,
                    count(ws.id) FILTER (WHERE ws.completed AND ws.kind <> 'warmup') AS set_count,
                    COALESCE(sum(ws.reps) FILTER (WHERE ws.completed AND ws.kind <> 'warmup'), 0)::bigint AS total_reps,
                    COALESCE(sum(ws.weight_kg * ws.reps) FILTER (WHERE ws.completed AND ws.kind <> 'warmup'), 0)::float8 AS volume_kg,
                    (SELECT array_agg(e.name ORDER BY we2.position)
                       FROM workout_exercises we2 JOIN exercises e ON e.id = we2.exercise_id
                      WHERE we2.workout_id = w.id) AS exercise_names
             FROM workout_exercises we
             LEFT JOIN workout_sets ws ON ws.workout_exercise_id = we.id
             WHERE we.workout_id = w.id
         ) agg ON true
         WHERE w.user_id = $1
           AND ($2::text IS NULL OR ($2 = 'in_progress') = (w.ended_at IS NULL))
           AND ($3::timestamptz IS NULL OR (w.started_at, w.id) < ($3, $4))
         ORDER BY w.started_at DESC, w.id DESC
         LIMIT $5",
    )
    .bind(user_id)
    .bind(params.status.map(|status| match status {
        StatusFilter::InProgress => "in_progress",
        StatusFilter::Completed => "completed",
    }))
    .bind(before_at)
    .bind(before_id)
    .bind(params.limit)
    .fetch_all(db)
    .await
}

pub async fn delete(db: &PgPool, user_id: Uuid, id: Uuid) -> sqlx::Result<bool> {
    let result = sqlx::query("DELETE FROM workouts WHERE id = $1 AND user_id = $2")
        .bind(id)
        .bind(user_id)
        .execute(db)
        .await?;
    Ok(result.rows_affected() > 0)
}
