use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use super::model::{Candidate, CandidateRow, Product, ProductRequest};

/// Column list shared by the queries below; a macro so `concat!` can splice it into literal SQL
macro_rules! product_columns {
    () => {
        "p.id, p.name, p.brand, p.barcode, p.energy_kcal, p.protein_g, p.fat_g, p.carbs_g,
    p.saturated_fat_g, p.sugars_g, p.fiber_g, p.salt_g, p.serving_amount, p.serving_name, p.unit, p.source,
    p.created_at, p.updated_at"
    };
}

/// Column list for [`CandidateRow`]
macro_rules! candidate_columns {
    () => {
        "o.barcode, o.name, o.brand, o.main_category, o.energy_kcal, o.protein_g, o.fat_g, o.carbs_g,
        o.saturated_fat_g, o.sugars_g, o.fiber_g, o.salt_g, o.serving_amount, o.serving_name, o.unit"
    };
}

/// How loosely a typed word may match a word in a name; lower than pg_trgm's 0.6 so a typo or a
/// missing Polish letter ("platki" for "płatki") still matches
const WORD_SIMILARITY: &str = "SET LOCAL pg_trgm.word_similarity_threshold = 0.4";

/// Name or brand contains `query` or nearly matches it; what the caller logged most recently
/// comes first, then the closest match
pub async fn search(
    conn: &mut PgConnection,
    user_id: Uuid,
    query: &str,
    limit: i64,
) -> sqlx::Result<Vec<Product>> {
    sqlx::query(WORD_SIMILARITY).execute(&mut *conn).await?;
    sqlx::query_as(concat!(
        "SELECT ", product_columns!(), " FROM food_products p
         LEFT JOIN LATERAL (
            SELECT max(e.created_at) AS last_used FROM diary_entries e
            WHERE e.user_id = $1 AND e.product_id = p.id
         ) used ON true
         WHERE $2 = ''
            OR strpos(lower(p.name), lower($2)) > 0 OR strpos(lower(coalesce(p.brand, '')), lower($2)) > 0
            OR lower($2) <% lower(p.name) OR lower($2) <% lower(p.brand)
         ORDER BY used.last_used DESC NULLS LAST,
            greatest(word_similarity(lower($2), lower(p.name)), word_similarity(lower($2), lower(coalesce(p.brand, '')))) DESC,
            lower(p.name), p.id
         LIMIT $3"
    ))
    .bind(user_id)
    .bind(query)
    .bind(limit)
    .fetch_all(&mut *conn)
    .await
}

/// Imported OFF listings that can be logged (they have nutrition) and are not FitHealth
/// products yet, closest match first
pub async fn search_off(
    conn: &mut PgConnection,
    query: &str,
    limit: i64,
) -> sqlx::Result<Vec<Candidate>> {
    sqlx::query(WORD_SIMILARITY).execute(&mut *conn).await?;
    let rows: Vec<CandidateRow> = sqlx::query_as(concat!(
        "SELECT ", candidate_columns!(), " FROM off_products o
         WHERE o.energy_kcal IS NOT NULL
            AND (lower($1) <% lower(o.name) OR lower($1) <% lower(o.brand))
            AND NOT EXISTS (SELECT 1 FROM food_products f WHERE f.barcode = o.barcode)
         ORDER BY greatest(word_similarity(lower($1), lower(o.name)), word_similarity(lower($1), lower(coalesce(o.brand, '')))) DESC,
            lower(o.name), o.barcode
         LIMIT $2"
    ))
    .bind(query)
    .bind(limit)
    .fetch_all(conn)
    .await?;
    Ok(rows.into_iter().map(Candidate::from).collect())
}

pub async fn by_barcode(db: &PgPool, barcode: &str) -> sqlx::Result<Option<Product>> {
    sqlx::query_as(concat!(
        "SELECT ",
        product_columns!(),
        " FROM food_products p WHERE p.barcode = $1"
    ))
    .bind(barcode)
    .fetch_optional(db)
    .await
}

pub async fn off_by_barcode(db: &PgPool, barcode: &str) -> sqlx::Result<Option<Candidate>> {
    let row: Option<CandidateRow> = sqlx::query_as(concat!(
        "SELECT ",
        candidate_columns!(),
        " FROM off_products o WHERE o.barcode = $1"
    ))
    .bind(barcode)
    .fetch_optional(db)
    .await?;
    Ok(row.map(Candidate::from))
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
                saturated_fat_g, sugars_g, fiber_g, salt_g, serving_amount, serving_name, created_by, updated_by,
                barcode, source, unit)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $14, $15, $16, $17)
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
    .bind(input.serving_amount)
    .bind(&input.serving_name)
    .bind(user_id)
    .bind(&input.barcode)
    .bind(input.source)
    .bind(input.unit)
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
                serving_amount = $12, serving_name = $13, updated_by = $14, updated_at = now(),
                barcode = COALESCE($15, barcode), unit = $16
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
    .bind(input.serving_amount)
    .bind(&input.serving_name)
    .bind(user_id)
    .bind(&input.barcode)
    .bind(input.unit)
    .fetch_optional(db)
    .await
}
