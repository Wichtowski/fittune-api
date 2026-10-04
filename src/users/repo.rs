use sqlx::{PgExecutor, PgPool};
use uuid::Uuid;

use super::model::{ProfileUpdate, Role, User};

/// Column list matching [`User`]; a macro so it can be spliced into literal SQL with `concat!`.
macro_rules! user_columns {
    () => {
        "id, username, email, display_name, birthday, role, account_type, weight_unit, distance_unit, created_at"
    };
}

pub async fn find_by_id(db: impl PgExecutor<'_>, id: Uuid) -> sqlx::Result<Option<User>> {
    sqlx::query_as(concat!(
        "SELECT ",
        user_columns!(),
        " FROM users WHERE id = $1"
    ))
    .bind(id)
    .fetch_optional(db)
    .await
}

pub async fn list(db: &PgPool, limit: i64, offset: i64) -> sqlx::Result<Vec<User>> {
    sqlx::query_as(concat!(
        "SELECT ",
        user_columns!(),
        " FROM users ORDER BY created_at DESC, id LIMIT $1 OFFSET $2"
    ))
    .bind(limit)
    .bind(offset)
    .fetch_all(db)
    .await
}

pub async fn update_profile(db: &PgPool, id: Uuid, update: ProfileUpdate) -> sqlx::Result<User> {
    // Each nullable column gets a "touch" flag so explicit nulls can clear values.
    sqlx::query_as(concat!(
        "UPDATE users SET
            display_name  = CASE WHEN $2 THEN $3 ELSE display_name END,
            birthday      = CASE WHEN $4 THEN $5 ELSE birthday END,
            account_type  = CASE WHEN $6 THEN $7 ELSE account_type END,
            weight_unit   = COALESCE($8, weight_unit),
            distance_unit = COALESCE($9, distance_unit),
            updated_at    = now()
         WHERE id = $1
         RETURNING ",
        user_columns!()
    ))
    .bind(id)
    .bind(update.display_name.is_some())
    .bind(update.display_name.flatten())
    .bind(update.birthday.is_some())
    .bind(update.birthday.flatten())
    .bind(update.account_type.is_some())
    .bind(update.account_type.flatten())
    .bind(update.weight_unit)
    .bind(update.distance_unit)
    .fetch_one(db)
    .await
}

/// Changes the role of the user whose username or email matches `login`.
pub async fn set_role(db: &PgPool, login: &str, role: Role) -> sqlx::Result<Option<User>> {
    sqlx::query_as(concat!(
        "UPDATE users SET role = $2, updated_at = now()
         WHERE lower(username) = lower($1) OR lower(email) = lower($1)
         RETURNING ",
        user_columns!()
    ))
    .bind(login)
    .bind(role)
    .fetch_optional(db)
    .await
}

/// Deletes the account and everything it owns. Workouts and routines go first, because they
/// can reference the user's own exercises, which the account's own cascade removes.
///
/// Others can use an exercise a user created, so one that other people's workouts or routines
/// still reference cannot go with the account. It loses its owner and is archived: those
/// people keep it in their history, but nothing a user wrote becomes part of the library the
/// admins curate just because its author left. `shared` is left as it is, so a private exercise
/// (which only an admin can have used) does not become visible to everyone by losing its owner
pub async fn delete(db: &PgPool, id: Uuid) -> sqlx::Result<()> {
    let mut tx = db.begin().await?;
    for statement in [
        "DELETE FROM workouts WHERE user_id = $1",
        "DELETE FROM routines WHERE user_id = $1",
        "UPDATE exercises e
         SET owner_id = NULL, updated_at = now(), archived_at = COALESCE(e.archived_at, now())
         WHERE e.owner_id = $1
           AND (EXISTS (SELECT 1 FROM workout_exercises we WHERE we.exercise_id = e.id)
                OR EXISTS (SELECT 1 FROM routine_exercises re WHERE re.exercise_id = e.id))",
        "DELETE FROM users WHERE id = $1",
    ] {
        sqlx::query(statement).bind(id).execute(&mut *tx).await?;
    }
    tx.commit().await
}
