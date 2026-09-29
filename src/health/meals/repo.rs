use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use super::model::{DEFAULT_MEALS, MAX_MEALS, Meal};

pub enum Outcome<T> {
    Done(T),
    NotFound,
    Conflict(&'static str),
}

/// Creates the default meals the first time a user needs them. Locks the user row so two
/// first requests cannot both create a set
pub async fn ensure_defaults(conn: &mut PgConnection, user_id: Uuid) -> sqlx::Result<()> {
    sqlx::query("SELECT 1 FROM users WHERE id = $1 FOR UPDATE")
        .bind(user_id)
        .execute(&mut *conn)
        .await?;
    let any: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM diary_meals WHERE user_id = $1)")
            .bind(user_id)
            .fetch_one(&mut *conn)
            .await?;
    if !any {
        for (position, name) in (0i32..).zip(DEFAULT_MEALS) {
            sqlx::query(
                "INSERT INTO diary_meals (id, user_id, name, position) VALUES ($1, $2, $3, $4)",
            )
            .bind(Uuid::new_v4())
            .bind(user_id)
            .bind(name)
            .bind(position)
            .execute(&mut *conn)
            .await?;
        }
    }
    Ok(())
}

pub async fn active(conn: &mut PgConnection, user_id: Uuid) -> sqlx::Result<Vec<Meal>> {
    sqlx::query_as(
        "SELECT id, name, position FROM diary_meals
         WHERE user_id = $1 AND archived_at IS NULL ORDER BY position, created_at, id",
    )
    .bind(user_id)
    .fetch_all(conn)
    .await
}

pub async fn list(db: &PgPool, user_id: Uuid) -> sqlx::Result<Vec<Meal>> {
    let mut tx = db.begin().await?;
    ensure_defaults(&mut tx, user_id).await?;
    let meals = active(&mut tx, user_id).await?;
    tx.commit().await?;
    Ok(meals)
}

pub async fn create(db: &PgPool, user_id: Uuid, name: &str) -> sqlx::Result<Outcome<Meal>> {
    let mut tx = db.begin().await?;
    ensure_defaults(&mut tx, user_id).await?;
    let meals = active(&mut tx, user_id).await?;
    if i64::try_from(meals.len()).unwrap_or(i64::MAX) >= MAX_MEALS {
        return Ok(Outcome::Conflict(
            "You can have at most 10 meals. Delete one to add another.",
        ));
    }
    let position = meals.last().map_or(0, |m| m.position + 1);
    let meal = sqlx::query_as(
        "INSERT INTO diary_meals (id, user_id, name, position) VALUES ($1, $2, $3, $4)
         RETURNING id, name, position",
    )
    .bind(Uuid::new_v4())
    .bind(user_id)
    .bind(name)
    .bind(position)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Outcome::Done(meal))
}

pub async fn rename(
    db: &PgPool,
    user_id: Uuid,
    id: Uuid,
    name: &str,
) -> sqlx::Result<Option<Meal>> {
    sqlx::query_as(
        "UPDATE diary_meals SET name = $3 WHERE id = $1 AND user_id = $2 AND archived_at IS NULL
         RETURNING id, name, position",
    )
    .bind(id)
    .bind(user_id)
    .bind(name)
    .fetch_optional(db)
    .await
}

/// `ids` must name every active meal exactly once
pub async fn reorder(db: &PgPool, user_id: Uuid, ids: &[Uuid]) -> sqlx::Result<Option<Vec<Meal>>> {
    let mut tx = db.begin().await?;
    ensure_defaults(&mut tx, user_id).await?;
    let mut current: Vec<Uuid> = active(&mut tx, user_id)
        .await?
        .into_iter()
        .map(|m| m.id)
        .collect();
    let mut wanted = ids.to_vec();
    current.sort();
    wanted.sort();
    wanted.dedup();
    if current != wanted || wanted.len() != ids.len() {
        return Ok(None);
    }
    for (position, id) in (0i32..).zip(ids) {
        sqlx::query("UPDATE diary_meals SET position = $3 WHERE id = $1 AND user_id = $2")
            .bind(id)
            .bind(user_id)
            .bind(position)
            .execute(&mut *tx)
            .await?;
    }
    let meals = active(&mut tx, user_id).await?;
    tx.commit().await?;
    Ok(Some(meals))
}

/// Archives a meal; its past entries keep pointing at it. The last active meal stays
pub async fn archive(db: &PgPool, user_id: Uuid, id: Uuid) -> sqlx::Result<Outcome<()>> {
    let mut tx = db.begin().await?;
    ensure_defaults(&mut tx, user_id).await?;
    let meals = active(&mut tx, user_id).await?;
    if !meals.iter().any(|m| m.id == id) {
        return Ok(Outcome::NotFound);
    }
    if meals.len() <= 1 {
        return Ok(Outcome::Conflict("Keep at least one meal."));
    }
    sqlx::query("UPDATE diary_meals SET archived_at = now() WHERE id = $1 AND user_id = $2")
        .bind(id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Outcome::Done(()))
}
