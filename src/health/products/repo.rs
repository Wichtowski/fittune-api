use sqlx::PgPool;
use uuid::Uuid;

use super::model::{Product, ProductRequest};

/// Column list shared by the queries below; a macro so `concat!` can splice it into literal SQL
macro_rules! product_columns {
    () => {
        "p.id, p.name, p.brand, p.barcode, p.energy_kcal, p.protein_g, p.fat_g, p.carbs_g,
    p.saturated_fat_g, p.sugars_g, p.fiber_g, p.salt_g, p.serving_g, p.serving_name, p.source,
    p.created_at, p.updated_at"
    };
}

/// Name or brand contains `query`; what the caller logged most recently comes first
pub async fn search(
    db: &PgPool,
    user_id: Uuid,
    query: &str,
    limit: i64,
) -> sqlx::Result<Vec<Product>> {
    sqlx::query_as(concat!("SELECT ", product_columns!(), " FROM food_products p
         LEFT JOIN LATERAL (
            SELECT max(e.created_at) AS last_used FROM diary_entries e
            WHERE e.user_id = $1 AND e.product_id = p.id
         ) used ON true
         WHERE $2 = '' OR strpos(lower(p.name), lower($2)) > 0 OR strpos(lower(coalesce(p.brand, '')), lower($2)) > 0
         ORDER BY used.last_used DESC NULLS LAST, lower(p.name), p.id
         LIMIT $3"))
    .bind(user_id)
    .bind(query)
    .bind(limit)
    .fetch_all(db)
    .await
}

pub async fn get(db: &PgPool, id: Uuid) -> sqlx::Result<Option<Product>> {
    sqlx::query_as(concat!(
        "SELECT ",
        product_columns!(),
        " FROM food_products p WHERE p.id = $1"
    ))
    .bind(id)
    .fetch_optional(db)
    .await
}

pub async fn create(db: &PgPool, user_id: Uuid, input: &ProductRequest) -> sqlx::Result<Product> {
    let n = &input.per_100g;
    sqlx::query_as(concat!("WITH p AS (
            INSERT INTO food_products (id, name, brand, energy_kcal, protein_g, fat_g, carbs_g,
                saturated_fat_g, sugars_g, fiber_g, salt_g, serving_g, serving_name, created_by, updated_by)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $14)
            RETURNING *
         ) SELECT ", product_columns!(), " FROM p"))
    .bind(Uuid::new_v4())
    .bind(&input.name)
    .bind(&input.brand)
    .bind(n.energy_kcal)
    .bind(n.protein_g)
    .bind(n.fat_g)
    .bind(n.carbs_g)
    .bind(n.saturated_fat_g)
    .bind(n.sugars_g)
    .bind(n.fiber_g)
    .bind(n.salt_g)
    .bind(input.serving_g)
    .bind(&input.serving_name)
    .bind(user_id)
    .fetch_one(db)
    .await
}

/// Any user may correct a shared product; `updated_by` keeps who did
pub async fn update(
    db: &PgPool,
    user_id: Uuid,
    id: Uuid,
    input: &ProductRequest,
) -> sqlx::Result<Option<Product>> {
    let n = &input.per_100g;
    sqlx::query_as(concat!("WITH p AS (
            UPDATE food_products SET name = $2, brand = $3, energy_kcal = $4, protein_g = $5, fat_g = $6,
                carbs_g = $7, saturated_fat_g = $8, sugars_g = $9, fiber_g = $10, salt_g = $11,
                serving_g = $12, serving_name = $13, updated_by = $14, updated_at = now()
            WHERE id = $1
            RETURNING *
         ) SELECT ", product_columns!(), " FROM p"))
    .bind(id)
    .bind(&input.name)
    .bind(&input.brand)
    .bind(n.energy_kcal)
    .bind(n.protein_g)
    .bind(n.fat_g)
    .bind(n.carbs_g)
    .bind(n.saturated_fat_g)
    .bind(n.sugars_g)
    .bind(n.fiber_g)
    .bind(n.salt_g)
    .bind(input.serving_g)
    .bind(&input.serving_name)
    .bind(user_id)
    .fetch_optional(db)
    .await
}
