use sqlx::{PgExecutor, PgPool, Postgres, Transaction};
use uuid::Uuid;

use super::model::{Invite, InviteDraft};

/// Columns matching [`Invite`], with the status derived from the other fields
macro_rules! invite_columns {
    () => {
        "i.id, i.note, i.max_uses, i.use_count, i.expires_at, i.revoked_at, i.created_at,
         u.username AS created_by,
         CASE WHEN i.revoked_at IS NOT NULL THEN 'revoked'
              WHEN i.use_count >= i.max_uses THEN 'used'
              WHEN i.expires_at <= now() THEN 'expired'
              ELSE 'active' END AS status"
    };
}

pub async fn create(
    db: &PgPool,
    created_by: Uuid,
    draft: &InviteDraft,
    code_hash: &[u8],
) -> sqlx::Result<Invite> {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO invites (id, code_hash, note, max_uses, expires_at, created_by)
         VALUES ($1, $2, $3, $4, now() + make_interval(days => $5), $6)",
    )
    .bind(id)
    .bind(code_hash)
    .bind(&draft.note)
    .bind(draft.max_uses)
    .bind(draft.expires_in_days)
    .bind(created_by)
    .execute(db)
    .await?;
    find(db, id).await?.ok_or_else(|| sqlx::Error::RowNotFound)
}

pub async fn find(db: impl PgExecutor<'_>, id: Uuid) -> sqlx::Result<Option<Invite>> {
    sqlx::query_as(concat!(
        "SELECT ",
        invite_columns!(),
        " FROM invites i LEFT JOIN users u ON u.id = i.created_by WHERE i.id = $1"
    ))
    .bind(id)
    .fetch_optional(db)
    .await
}

/// Newest first; admins rarely need more than the latest few hundred
pub async fn list(db: &PgPool) -> sqlx::Result<Vec<Invite>> {
    sqlx::query_as(concat!(
        "SELECT ",
        invite_columns!(),
        " FROM invites i LEFT JOIN users u ON u.id = i.created_by
          ORDER BY i.created_at DESC, i.id LIMIT 200"
    ))
    .fetch_all(db)
    .await
}

pub async fn revoke(db: &PgPool, id: Uuid) -> sqlx::Result<bool> {
    let result =
        sqlx::query("UPDATE invites SET revoked_at = COALESCE(revoked_at, now()) WHERE id = $1")
            .bind(id)
            .execute(db)
            .await?;
    Ok(result.rows_affected() > 0)
}

/// Locks a usable invite for the rest of the transaction, so concurrent registrations
/// cannot both use its last remaining use
pub async fn lock_usable(
    tx: &mut Transaction<'_, Postgres>,
    code_hash: &[u8],
) -> sqlx::Result<Option<Uuid>> {
    sqlx::query_scalar(
        "SELECT id FROM invites
         WHERE code_hash = $1 AND revoked_at IS NULL AND expires_at > now() AND use_count < max_uses
         FOR UPDATE",
    )
    .bind(code_hash)
    .fetch_optional(&mut **tx)
    .await
}

pub async fn consume(tx: &mut Transaction<'_, Postgres>, id: Uuid) -> sqlx::Result<()> {
    sqlx::query("UPDATE invites SET use_count = use_count + 1 WHERE id = $1")
        .bind(id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}
