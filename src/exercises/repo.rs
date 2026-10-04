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
        exercise_columns!("instructions, instructions_pl")
    };
    ($instructions:literal) => {
        concat!(
            "id, owner_id, name, tracking, primary_muscle, secondary_muscles, equipment, requires, difficulty, ",
            $instructions,
            ", owner_id IS NOT NULL AS is_custom, archived_at, created_at, updated_at, \
             (SELECT COALESCE(NULLIF(btrim(u.display_name), ''), u.username) FROM users u WHERE u.id = owner_id) AS created_by"
        )
    };
}

/// Who is asking: decides which exercises exist for them
#[derive(Debug, Clone, Copy)]
pub struct Viewer {
    pub user_id: Uuid,
    pub admin: bool,
}

/// Everything shared (the catalog and what users created since exercises are shared), the
/// viewer's own, and for admins everything. Deliberately not "has no owner": a private
/// exercise that outlives its owner's account has none either and must stay private.
/// Binds the viewer's id as `$1` and whether they are an admin as `$2`
macro_rules! visible {
    () => {
        "(shared OR owner_id = $1 OR $2)"
    };
}

#[derive(Debug, Default)]
pub struct ExerciseFilter {
    pub search: Option<String>,
    pub muscle: Option<Muscle>,
    pub equipment: Option<Equipment>,
}

/// Active exercises visible to `viewer`: the catalog and what users created. Instruction
/// texts are left out: they are most of the library's size, clients keep the whole list for
/// offline use, and only the screen of one exercise shows them.
pub async fn list(
    db: &PgPool,
    viewer: Viewer,
    filter: &ExerciseFilter,
) -> sqlx::Result<Vec<Exercise>> {
    let mut exercises: Vec<Exercise> = sqlx::query_as(concat!(
        "SELECT ",
        exercise_columns!("NULL::text AS instructions, NULL::text AS instructions_pl"),
        " FROM exercises
          WHERE ",
        visible!(),
        " AND archived_at IS NULL
            AND ($3::text IS NULL OR name ILIKE '%' || $3 || '%')
            AND ($4::text IS NULL OR primary_muscle = $4 OR $4 = ANY (secondary_muscles))
            AND ($5::text IS NULL OR equipment = $5)
          ORDER BY lower(name), owner_id NULLS FIRST, id"
    ))
    .bind(viewer.user_id)
    .bind(viewer.admin)
    .bind(filter.search.as_deref().map(escape_like))
    .bind(filter.muscle)
    .bind(filter.equipment)
    .fetch_all(db)
    .await?;
    media::attach(db, &mut exercises).await?;
    for exercise in &mut exercises {
        exercise.is_own = exercise.owner_id == Some(viewer.user_id);
    }
    Ok(exercises)
}

/// Any exercise visible to `viewer`, including archived ones (history still references them).
pub async fn find_visible(db: &PgPool, viewer: Viewer, id: Uuid) -> sqlx::Result<Option<Exercise>> {
    let mut exercise: Option<Exercise> = sqlx::query_as(concat!(
        "SELECT ",
        exercise_columns!(),
        " FROM exercises WHERE ",
        visible!(),
        " AND id = $3"
    ))
    .bind(viewer.user_id)
    .bind(viewer.admin)
    .bind(id)
    .fetch_optional(db)
    .await?;
    media::attach(db, exercise.as_mut_slice()).await?;
    if let Some(exercise) = &mut exercise {
        exercise.is_own = exercise.owner_id == Some(viewer.user_id);
    }
    Ok(exercise)
}

/// Whether every id in `ids` refers to an exercise visible to `viewer` (archived ones included).
pub async fn all_visible(
    db: impl PgExecutor<'_>,
    viewer: Viewer,
    ids: impl IntoIterator<Item = Uuid>,
) -> sqlx::Result<bool> {
    let mut ids: Vec<Uuid> = ids.into_iter().collect();
    ids.sort_unstable();
    ids.dedup();
    if ids.is_empty() {
        return Ok(true);
    }
    let visible: i64 = sqlx::query_scalar(concat!(
        "SELECT count(*) FROM exercises WHERE ",
        visible!(),
        " AND id = ANY ($3)"
    ))
    .bind(viewer.user_id)
    .bind(viewer.admin)
    .bind(&ids)
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
