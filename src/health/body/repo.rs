use chrono::NaiveDate;
use sqlx::PgPool;
use uuid::Uuid;

use super::model::{Profile, ProfileRequest, Weight};

/// Column list shared by the queries below; a macro so `concat!` can splice it into literal SQL
macro_rules! profile_columns {
    () => {
        "sex, height_cm, activity, goal, pace_kg_per_week, energy_kcal, protein_g, fat_g, carbs_g"
    };
}

pub async fn profile(db: &PgPool, user_id: Uuid) -> sqlx::Result<Option<Profile>> {
    sqlx::query_as(concat!(
        "SELECT ",
        profile_columns!(),
        " FROM body_profiles WHERE user_id = $1"
    ))
    .bind(user_id)
    .fetch_optional(db)
    .await
}

pub async fn save_profile(
    db: &PgPool,
    user_id: Uuid,
    input: &ProfileRequest,
) -> sqlx::Result<Profile> {
    sqlx::query_as(concat!(
        "INSERT INTO body_profiles (user_id, sex, height_cm, activity, goal, pace_kg_per_week,
            energy_kcal, protein_g, fat_g, carbs_g)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
         ON CONFLICT (user_id) DO UPDATE SET sex = $2, height_cm = $3, activity = $4, goal = $5,
            pace_kg_per_week = $6, energy_kcal = $7, protein_g = $8, fat_g = $9, carbs_g = $10,
            updated_at = now()
         RETURNING ",
        profile_columns!()
    ))
    .bind(user_id)
    .bind(input.sex)
    .bind(input.height_cm)
    .bind(input.activity)
    .bind(input.goal)
    .bind(input.pace_kg_per_week)
    .bind(input.energy_kcal)
    .bind(input.protein_g)
    .bind(input.fat_g)
    .bind(input.carbs_g)
    .fetch_one(db)
    .await
}

pub async fn weights(db: &PgPool, user_id: Uuid, limit: i64) -> sqlx::Result<Vec<Weight>> {
    sqlx::query_as(
        "SELECT date, weight_kg FROM body_weights WHERE user_id = $1 ORDER BY date DESC LIMIT $2",
    )
    .bind(user_id)
    .bind(limit)
    .fetch_all(db)
    .await
}

/// The weight that applies on `date`: the latest logged on or before it
pub async fn weight_on(db: &PgPool, user_id: Uuid, date: NaiveDate) -> sqlx::Result<Option<f64>> {
    sqlx::query_scalar(
        "SELECT weight_kg FROM body_weights WHERE user_id = $1 AND date <= $2 ORDER BY date DESC LIMIT 1",
    )
    .bind(user_id)
    .bind(date)
    .fetch_optional(db)
    .await
}

pub async fn save_weight(
    db: &PgPool,
    user_id: Uuid,
    date: NaiveDate,
    weight_kg: f64,
) -> sqlx::Result<Weight> {
    sqlx::query_as(
        "INSERT INTO body_weights (user_id, date, weight_kg) VALUES ($1, $2, $3)
         ON CONFLICT (user_id, date) DO UPDATE SET weight_kg = $3, updated_at = now()
         RETURNING date, weight_kg",
    )
    .bind(user_id)
    .bind(date)
    .bind(weight_kg)
    .fetch_one(db)
    .await
}

pub async fn delete_weight(db: &PgPool, user_id: Uuid, date: NaiveDate) -> sqlx::Result<bool> {
    Ok(
        sqlx::query("DELETE FROM body_weights WHERE user_id = $1 AND date = $2")
            .bind(user_id)
            .bind(date)
            .execute(db)
            .await?
            .rows_affected()
            > 0,
    )
}
