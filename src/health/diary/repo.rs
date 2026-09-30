use chrono::NaiveDate;
use sqlx::PgPool;
use uuid::Uuid;

use super::model::{DayMealRow, Entry, EntryRequest};
use crate::{activities::model::ActivityKind, health::exercise::ActivitySession};

/// Column list shared by the queries below; a macro so `concat!` can splice it into literal SQL
macro_rules! entry_columns {
    () => {
        "id, date, meal_id, product_id, amount, unit, product_name, product_brand, energy_kcal,
    protein_g, fat_g, carbs_g, saturated_fat_g, sugars_g, fiber_g, salt_g"
    };
}

pub enum SaveOutcome {
    Saved { created: bool, entry: Entry },
    NotFound(&'static str),
}

pub async fn save(
    db: &PgPool,
    user_id: Uuid,
    id: Uuid,
    input: &EntryRequest,
) -> sqlx::Result<SaveOutcome> {
    let mut tx = db.begin().await?;
    let existing: Option<(Uuid, Uuid, Option<Uuid>)> = sqlx::query_as(
        "SELECT user_id, meal_id, product_id FROM diary_entries WHERE id = $1 FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    if existing.is_some_and(|(owner, _, _)| owner != user_id) {
        return Ok(SaveOutcome::NotFound("entry"));
    }

    // A deleted meal only accepts the entries it already has
    let meal_ok: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM diary_meals WHERE id = $1 AND user_id = $2
            AND (archived_at IS NULL OR id = $3))",
    )
    .bind(input.meal_id)
    .bind(user_id)
    .bind(existing.map(|(_, meal, _)| meal))
    .fetch_one(&mut *tx)
    .await?;
    if !meal_ok {
        return Ok(SaveOutcome::NotFound("meal"));
    }

    let same_product = existing.is_some_and(|(_, _, product)| product == Some(input.product_id));
    let entry: Entry = if same_product {
        sqlx::query_as(concat!(
            "UPDATE diary_entries SET date = $2, meal_id = $3, amount = $4, updated_at = now()
             WHERE id = $1 RETURNING ",
            entry_columns!()
        ))
        .bind(id)
        .bind(input.date)
        .bind(input.meal_id)
        .bind(input.amount)
        .fetch_one(&mut *tx)
        .await?
    } else {
        // New entry or another product: copy the product in as it is now
        let saved: Option<Entry> = sqlx::query_as(concat!("INSERT INTO diary_entries (id, user_id, date, meal_id, product_id, amount, unit, product_name,
                product_brand, energy_kcal, protein_g, fat_g, carbs_g, saturated_fat_g, sugars_g, fiber_g, salt_g)
             SELECT $1, $2, $3, $4, p.id, $6, p.unit, p.name, p.brand, p.energy_kcal, p.protein_g, p.fat_g, p.carbs_g,
                p.saturated_fat_g, p.sugars_g, p.fiber_g, p.salt_g
             FROM food_products p WHERE p.id = $5
             ON CONFLICT (id) DO UPDATE SET date = excluded.date, meal_id = excluded.meal_id,
                product_id = excluded.product_id, amount = excluded.amount, unit = excluded.unit, product_name = excluded.product_name,
                product_brand = excluded.product_brand, energy_kcal = excluded.energy_kcal,
                protein_g = excluded.protein_g, fat_g = excluded.fat_g, carbs_g = excluded.carbs_g,
                saturated_fat_g = excluded.saturated_fat_g, sugars_g = excluded.sugars_g,
                fiber_g = excluded.fiber_g, salt_g = excluded.salt_g, updated_at = now()
             RETURNING ", entry_columns!()))
        .bind(id)
        .bind(user_id)
        .bind(input.date)
        .bind(input.meal_id)
        .bind(input.product_id)
        .bind(input.amount)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(saved) = saved else {
            return Ok(SaveOutcome::NotFound("product"));
        };
        saved
    };
    tx.commit().await?;
    Ok(SaveOutcome::Saved {
        created: existing.is_none(),
        entry,
    })
}

pub async fn delete(db: &PgPool, user_id: Uuid, id: Uuid) -> sqlx::Result<bool> {
    Ok(
        sqlx::query("DELETE FROM diary_entries WHERE id = $1 AND user_id = $2")
            .bind(id)
            .bind(user_id)
            .execute(db)
            .await?
            .rows_affected()
            > 0,
    )
}

/// Fails with SQLSTATE 22023 for an unknown time zone
pub async fn check_time_zone(db: &PgPool, tz: &str) -> sqlx::Result<()> {
    sqlx::query("SELECT now() AT TIME ZONE $1")
        .bind(tz)
        .execute(db)
        .await?;
    Ok(())
}

pub async fn day_meals(
    db: &PgPool,
    user_id: Uuid,
    date: NaiveDate,
) -> sqlx::Result<Vec<DayMealRow>> {
    sqlx::query_as(
        "SELECT m.id, m.name, m.archived_at IS NOT NULL AS archived FROM diary_meals m
         WHERE m.user_id = $1 AND (m.archived_at IS NULL
            OR EXISTS(SELECT 1 FROM diary_entries e WHERE e.meal_id = m.id AND e.date = $2))
         ORDER BY m.position, m.created_at, m.id",
    )
    .bind(user_id)
    .bind(date)
    .fetch_all(db)
    .await
}

pub async fn day_entries(db: &PgPool, user_id: Uuid, date: NaiveDate) -> sqlx::Result<Vec<Entry>> {
    sqlx::query_as(concat!(
        "SELECT ",
        entry_columns!(),
        " FROM diary_entries WHERE user_id = $1 AND date = $2 ORDER BY created_at, id"
    ))
    .bind(user_id)
    .bind(date)
    .fetch_all(db)
    .await
}

pub async fn birthday(db: &PgPool, user_id: Uuid) -> sqlx::Result<Option<NaiveDate>> {
    sqlx::query_scalar("SELECT birthday FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(db)
        .await
}

/// Activities that started on `date` in `tz`
pub async fn activities(
    db: &PgPool,
    user_id: Uuid,
    date: NaiveDate,
    tz: &str,
) -> sqlx::Result<Vec<ActivitySession>> {
    let rows: Vec<(ActivityKind, i32, Option<f64>, Option<i32>)> = sqlx::query_as(
        "SELECT kind, duration_seconds, distance_m, calories FROM activities
         WHERE user_id = $1 AND started_at >= ($2::date)::timestamp AT TIME ZONE $3
            AND started_at < ($2::date + 1)::timestamp AT TIME ZONE $3",
    )
    .bind(user_id)
    .bind(date)
    .bind(tz)
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(kind, duration_seconds, distance_m, calories)| ActivitySession {
                kind,
                duration_seconds,
                distance_m,
                calories,
            },
        )
        .collect())
}

/// Lengths in seconds of finished workouts that started on `date` in `tz`
pub async fn workout_seconds(
    db: &PgPool,
    user_id: Uuid,
    date: NaiveDate,
    tz: &str,
) -> sqlx::Result<Vec<f64>> {
    sqlx::query_scalar(
        "SELECT EXTRACT(EPOCH FROM ended_at - started_at)::float8 FROM workouts
         WHERE user_id = $1 AND ended_at IS NOT NULL
            AND started_at >= ($2::date)::timestamp AT TIME ZONE $3
            AND started_at < ($2::date + 1)::timestamp AT TIME ZONE $3",
    )
    .bind(user_id)
    .bind(date)
    .bind(tz)
    .fetch_all(db)
    .await
}
