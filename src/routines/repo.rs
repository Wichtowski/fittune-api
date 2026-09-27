use std::collections::HashMap;

use chrono::{DateTime, Utc};
use sqlx::{FromRow, PgExecutor, PgPool, Postgres, Transaction, types::Json};
use uuid::Uuid;

use super::model::{Routine, RoutineDraft, RoutineExercise, SetTarget};
use crate::exercises::model::{Muscle, Tracking};

#[derive(FromRow)]
struct RoutineRow {
    id: Uuid,
    name: String,
    notes: Option<String>,
    last_performed_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(FromRow)]
struct RoutineExerciseRow {
    id: Uuid,
    routine_id: Uuid,
    exercise_id: Uuid,
    exercise_name: String,
    tracking: Tracking,
    primary_muscle: Muscle,
    rest_seconds: Option<i32>,
    notes: Option<String>,
    sets: Json<Vec<SetTarget>>,
}

/// Loads the user's routines (or just `only`) with their exercises.
async fn load(db: &PgPool, user_id: Uuid, only: Option<Uuid>) -> sqlx::Result<Vec<Routine>> {
    let routines: Vec<RoutineRow> = sqlx::query_as(
        "SELECT r.id, r.name, r.notes, r.created_at, r.updated_at,
                (SELECT max(w.started_at) FROM workouts w
                  WHERE w.routine_id = r.id AND w.ended_at IS NOT NULL) AS last_performed_at
         FROM routines r
         WHERE r.user_id = $1 AND ($2::uuid IS NULL OR r.id = $2)
         ORDER BY lower(r.name), r.id",
    )
    .bind(user_id)
    .bind(only)
    .fetch_all(db)
    .await?;

    let exercise_rows: Vec<RoutineExerciseRow> = sqlx::query_as(
        "SELECT re.id, re.routine_id, re.exercise_id, e.name AS exercise_name, e.tracking,
                e.primary_muscle, re.rest_seconds, re.notes, re.sets
         FROM routine_exercises re
         JOIN routines r ON r.id = re.routine_id
         JOIN exercises e ON e.id = re.exercise_id
         WHERE r.user_id = $1 AND ($2::uuid IS NULL OR r.id = $2)
         ORDER BY re.routine_id, re.position",
    )
    .bind(user_id)
    .bind(only)
    .fetch_all(db)
    .await?;

    let mut exercises: HashMap<Uuid, Vec<RoutineExercise>> = HashMap::new();
    for row in exercise_rows {
        exercises
            .entry(row.routine_id)
            .or_default()
            .push(RoutineExercise {
                id: row.id,
                exercise_id: row.exercise_id,
                exercise_name: row.exercise_name,
                tracking: row.tracking,
                primary_muscle: row.primary_muscle,
                rest_seconds: row.rest_seconds,
                notes: row.notes,
                sets: row.sets.0,
            });
    }

    Ok(routines
        .into_iter()
        .map(|row| Routine {
            exercises: exercises.remove(&row.id).unwrap_or_default(),
            id: row.id,
            name: row.name,
            notes: row.notes,
            last_performed_at: row.last_performed_at,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
        .collect())
}

pub async fn list(db: &PgPool, user_id: Uuid) -> sqlx::Result<Vec<Routine>> {
    load(db, user_id, None).await
}

pub async fn find(db: &PgPool, user_id: Uuid, id: Uuid) -> sqlx::Result<Option<Routine>> {
    Ok(load(db, user_id, Some(id)).await?.into_iter().next())
}

pub async fn insert(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    draft: &RoutineDraft,
) -> sqlx::Result<Uuid> {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO routines (id, user_id, name, notes) VALUES ($1, $2, $3, $4)")
        .bind(id)
        .bind(user_id)
        .bind(&draft.name)
        .bind(&draft.notes)
        .execute(&mut **tx)
        .await?;
    replace_exercises(tx, id, draft).await?;
    Ok(id)
}

/// Returns `false` when the routine does not exist or belongs to someone else.
pub async fn update(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    id: Uuid,
    draft: &RoutineDraft,
) -> sqlx::Result<bool> {
    let updated = sqlx::query(
        "UPDATE routines SET name = $3, notes = $4, updated_at = now() WHERE id = $1 AND user_id = $2",
    )
    .bind(id)
    .bind(user_id)
    .bind(&draft.name)
    .bind(&draft.notes)
    .execute(&mut **tx)
    .await?;
    if updated.rows_affected() == 0 {
        return Ok(false);
    }
    replace_exercises(tx, id, draft).await?;
    Ok(true)
}

async fn replace_exercises(
    tx: &mut Transaction<'_, Postgres>,
    routine_id: Uuid,
    draft: &RoutineDraft,
) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM routine_exercises WHERE routine_id = $1")
        .bind(routine_id)
        .execute(&mut **tx)
        .await?;
    for (position, exercise) in (0_i32..).zip(&draft.exercises) {
        sqlx::query(
            "INSERT INTO routine_exercises (id, routine_id, exercise_id, position, rest_seconds, notes, sets)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(Uuid::new_v4())
        .bind(routine_id)
        .bind(exercise.exercise_id)
        .bind(position)
        .bind(exercise.rest_seconds)
        .bind(&exercise.notes)
        .bind(Json(&exercise.sets))
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

pub async fn delete(db: impl PgExecutor<'_>, user_id: Uuid, id: Uuid) -> sqlx::Result<bool> {
    let result = sqlx::query("DELETE FROM routines WHERE id = $1 AND user_id = $2")
        .bind(id)
        .bind(user_id)
        .execute(db)
        .await?;
    Ok(result.rows_affected() > 0)
}
