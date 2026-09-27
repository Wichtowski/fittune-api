use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use super::model::{Place, PlaceRequest};

/// Active places a user may keep at once
pub const MAX_PLACES: i64 = 10;

pub enum SaveOutcome {
    Saved { created: bool, place: Place },
    NotFound,
    LimitReached,
}

pub async fn list(db: &PgPool, user_id: Uuid) -> sqlx::Result<Vec<Place>> {
    sqlx::query_as(
        "SELECT v.* FROM workout_places p
         CROSS JOIN LATERAL (SELECT * FROM workout_place_versions WHERE id = p.id ORDER BY revision DESC LIMIT 1) v
         WHERE p.user_id = $1 AND p.archived_at IS NULL ORDER BY lower(v.name), p.id",
    ).bind(user_id).fetch_all(db).await
}

pub async fn save(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    id: Uuid,
    input: &PlaceRequest,
) -> sqlx::Result<SaveOutcome> {
    // Serialises concurrent creates for one user so the limit cannot be raced past
    sqlx::query("SELECT 1 FROM users WHERE id = $1 FOR UPDATE")
        .bind(user_id)
        .execute(&mut **tx)
        .await?;
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM workout_places WHERE id = $1)")
            .bind(id)
            .fetch_one(&mut **tx)
            .await?;
    if !exists {
        let active: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM workout_places WHERE user_id = $1 AND archived_at IS NULL",
        )
        .bind(user_id)
        .fetch_one(&mut **tx)
        .await?;
        if active >= MAX_PLACES {
            return Ok(SaveOutcome::LimitReached);
        }
    }
    let created = sqlx::query(
        "INSERT INTO workout_places (id, user_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(id)
    .bind(user_id)
    .execute(&mut **tx)
    .await?
    .rows_affected()
        > 0;
    let owned: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM workout_places WHERE id = $1 AND user_id = $2 AND archived_at IS NULL FOR UPDATE",
    ).bind(id).bind(user_id).fetch_optional(&mut **tx).await?;
    if owned.is_none() {
        return Ok(SaveOutcome::NotFound);
    }

    let current: Option<Place> = sqlx::query_as(
        "SELECT * FROM workout_place_versions WHERE id = $1 ORDER BY revision DESC LIMIT 1",
    )
    .bind(id)
    .fetch_optional(&mut **tx)
    .await?;
    if let Some(current) = current
        && current.name == input.name
        && current.kind == input.kind
        && current.equipment == input.equipment
    {
        return Ok(SaveOutcome::Saved {
            created: false,
            place: current,
        });
    }
    let saved = sqlx::query_as(
        "INSERT INTO workout_place_versions (version_id, id, name, kind, equipment) VALUES ($1, $2, $3, $4, $5) RETURNING *",
    ).bind(Uuid::new_v4()).bind(id).bind(&input.name).bind(input.kind).bind(&input.equipment)
        .fetch_one(&mut **tx).await?;
    Ok(SaveOutcome::Saved {
        created,
        place: saved,
    })
}

/// Whether a place version belongs to the user. Archived places still count, so workouts
/// recorded offline before an archive can still be uploaded
pub async fn owns_version(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    version_id: Uuid,
) -> sqlx::Result<bool> {
    sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM workout_place_versions v JOIN workout_places p ON p.id = v.id
                       WHERE v.version_id = $1 AND p.user_id = $2)",
    )
    .bind(version_id)
    .bind(user_id)
    .fetch_one(&mut **tx)
    .await
}

pub async fn archive(db: &PgPool, user_id: Uuid, id: Uuid) -> sqlx::Result<bool> {
    Ok(sqlx::query("UPDATE workout_places SET archived_at = COALESCE(archived_at, now()) WHERE id = $1 AND user_id = $2")
        .bind(id).bind(user_id).execute(db).await?.rows_affected() > 0)
}
