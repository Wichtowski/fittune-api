use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgExecutor, PgPool};
use uuid::Uuid;

use super::{
    media,
    model::{Equipment, Exercise, ExerciseDraft, Muscle},
};
use crate::workouts::model::SetKind;

macro_rules! exercise_columns {
    () => {
        "id, owner_id, name, tracking, primary_muscle, secondary_muscles, equipment, requires, difficulty, \
         instructions, instructions_pl, owner_id IS NOT NULL AS is_custom, archived_at, created_at, updated_at"
    };
}

#[derive(Debug, Default)]
pub struct ExerciseFilter {
    pub search: Option<String>,
    pub muscle: Option<Muscle>,
    pub equipment: Option<Equipment>,
}

/// Active exercises visible to `user_id`: the shared catalog plus the user's own.
pub async fn list(
    db: &PgPool,
    user_id: Uuid,
    filter: &ExerciseFilter,
) -> sqlx::Result<Vec<Exercise>> {
    let mut exercises: Vec<Exercise> = sqlx::query_as(concat!(
        "SELECT ",
        exercise_columns!(),
        " FROM exercises
          WHERE (owner_id IS NULL OR owner_id = $1)
            AND archived_at IS NULL
            AND ($2::text IS NULL OR name ILIKE '%' || $2 || '%')
            AND ($3::text IS NULL OR primary_muscle = $3 OR $3 = ANY (secondary_muscles))
            AND ($4::text IS NULL OR equipment = $4)
          ORDER BY lower(name)"
    ))
    .bind(user_id)
    .bind(filter.search.as_deref().map(escape_like))
    .bind(filter.muscle)
    .bind(filter.equipment)
    .fetch_all(db)
    .await?;
    media::attach(db, &mut exercises).await?;
    Ok(exercises)
}

/// Any exercise visible to `user_id`, including archived ones (history still references them).
pub async fn find_visible(db: &PgPool, user_id: Uuid, id: Uuid) -> sqlx::Result<Option<Exercise>> {
    let mut exercise: Option<Exercise> = sqlx::query_as(concat!(
        "SELECT ",
        exercise_columns!(),
        " FROM exercises WHERE id = $1 AND (owner_id IS NULL OR owner_id = $2)"
    ))
    .bind(id)
    .bind(user_id)
    .fetch_optional(db)
    .await?;
    media::attach(db, exercise.as_mut_slice()).await?;
    Ok(exercise)
}

/// Whether every id in `ids` refers to an exercise visible to `user_id` (archived ones included).
pub async fn all_visible(
    db: impl PgExecutor<'_>,
    user_id: Uuid,
    ids: impl IntoIterator<Item = Uuid>,
) -> sqlx::Result<bool> {
    let mut ids: Vec<Uuid> = ids.into_iter().collect();
    ids.sort_unstable();
    ids.dedup();
    if ids.is_empty() {
        return Ok(true);
    }
    let visible: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM exercises WHERE id = ANY ($1) AND (owner_id IS NULL OR owner_id = $2)",
    )
    .bind(&ids)
    .bind(user_id)
    .fetch_one(db)
    .await?;
    Ok(usize::try_from(visible).ok() == Some(ids.len()))
}

/// Media are written on the same connection, so pass a transaction.
pub async fn insert(
    db: &mut PgConnection,
    owner_id: Option<Uuid>,
    draft: &ExerciseDraft,
) -> sqlx::Result<Exercise> {
    let exercise: Exercise = sqlx::query_as(concat!(
        "INSERT INTO exercises (id, owner_id, name, tracking, primary_muscle, secondary_muscles,
                                equipment, requires, difficulty, instructions)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
         RETURNING ",
        exercise_columns!()
    ))
    .bind(Uuid::new_v4())
    .bind(owner_id)
    .bind(&draft.name)
    .bind(draft.tracking)
    .bind(draft.primary_muscle)
    .bind(&draft.secondary_muscles)
    .bind(draft.equipment)
    .bind(&draft.requires)
    .bind(draft.difficulty)
    .bind(&draft.instructions)
    .fetch_one(&mut *db)
    .await?;
    with_video(db, exercise, draft).await
}

/// Media are written on the same connection, so pass a transaction.
pub async fn update(
    db: &mut PgConnection,
    id: Uuid,
    draft: &ExerciseDraft,
) -> sqlx::Result<Exercise> {
    let exercise: Exercise = sqlx::query_as(concat!(
        "UPDATE exercises SET name = $2, tracking = $3, primary_muscle = $4, secondary_muscles = $5,
                equipment = $6, requires = $7, difficulty = $8, instructions = $9,
                updated_at = now()
         WHERE id = $1
         RETURNING ",
        exercise_columns!()
    ))
    .bind(id)
    .bind(&draft.name)
    .bind(draft.tracking)
    .bind(draft.primary_muscle)
    .bind(&draft.secondary_muscles)
    .bind(draft.equipment)
    .bind(&draft.requires)
    .bind(draft.difficulty)
    .bind(&draft.instructions)
    .fetch_one(&mut *db)
    .await?;
    with_video(db, exercise, draft).await
}

/// Stores the draft's YouTube video and loads the exercise's media
async fn with_video(
    db: &mut PgConnection,
    mut exercise: Exercise,
    draft: &ExerciseDraft,
) -> sqlx::Result<Exercise> {
    media::set_youtube_video(&mut *db, exercise.id, draft.video_id.as_deref()).await?;
    media::attach(db, std::slice::from_mut(&mut exercise)).await?;
    Ok(exercise)
}

pub async fn archive(db: &PgPool, id: Uuid) -> sqlx::Result<()> {
    sqlx::query("UPDATE exercises SET archived_at = now(), updated_at = now() WHERE id = $1 AND archived_at IS NULL")
        .bind(id)
        .execute(db)
        .await?;
    Ok(())
}

/// One completed set of an exercise, flattened with its workout.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct HistorySetRow {
    pub workout_id: Uuid,
    pub workout_title: String,
    pub started_at: DateTime<Utc>,
    pub kind: SetKind,
    pub reps: Option<i32>,
    pub weight_kg: Option<f64>,
    pub duration_seconds: Option<i32>,
    pub distance_m: Option<f64>,
    pub rpe: Option<f64>,
    pub e1rm_kg: Option<f64>,
}

/// Completed sets of `exercise_id` in the user's finished workouts, newest workout first.
pub async fn history_sets(
    db: &PgPool,
    user_id: Uuid,
    exercise_id: Uuid,
) -> sqlx::Result<Vec<HistorySetRow>> {
    sqlx::query_as(
        "SELECT w.id AS workout_id, w.title AS workout_title, w.started_at,
                ws.kind, ws.reps, ws.weight_kg, ws.duration_seconds, ws.distance_m, ws.rpe,
                estimated_1rm(ws.weight_kg, ws.reps) AS e1rm_kg
         FROM workout_sets ws
         JOIN workout_exercises we ON we.id = ws.workout_exercise_id
         JOIN workouts w ON w.id = we.workout_id
         WHERE w.user_id = $1 AND we.exercise_id = $2 AND w.ended_at IS NOT NULL AND ws.completed
         ORDER BY w.started_at DESC, w.id, we.position, ws.position",
    )
    .bind(user_id)
    .bind(exercise_id)
    .fetch_all(db)
    .await
}

fn escape_like(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}
