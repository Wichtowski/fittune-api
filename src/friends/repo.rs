use sqlx::{PgExecutor, PgPool, Postgres, Transaction};
use uuid::Uuid;

use super::model::{BlockedUser, FriendRequest, PublicUser, Relationship, Sharing};

pub async fn find_by_username(
    db: impl PgExecutor<'_>,
    username: &str,
) -> sqlx::Result<Option<PublicUser>> {
    sqlx::query_as("SELECT id, username, display_name FROM users WHERE lower(username) = lower($1)")
        .bind(username)
        .fetch_optional(db)
        .await
}

pub async fn find_user(db: impl PgExecutor<'_>, id: Uuid) -> sqlx::Result<Option<PublicUser>> {
    sqlx::query_as("SELECT id, username, display_name FROM users WHERE id = $1")
        .bind(id)
        .fetch_optional(db)
        .await
}

/// Serialises every write on the unordered pair `(a, b)` until the transaction ends, so crossed
/// requests and a block racing a request cannot interleave
pub async fn lock_pair(tx: &mut Transaction<'_, Postgres>, a: Uuid, b: Uuid) -> sqlx::Result<()> {
    let (low, high) = if a < b { (a, b) } else { (b, a) };
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('friends:' || $1 || ':' || $2, 0))")
        .bind(low.to_string())
        .bind(high.to_string())
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Whether `blocker` has blocked `blocked`
pub async fn has_blocked(
    db: impl PgExecutor<'_>,
    blocker: Uuid,
    blocked: Uuid,
) -> sqlx::Result<bool> {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM user_blocks WHERE blocker_id = $1 AND blocked_id = $2)",
    )
    .bind(blocker)
    .bind(blocked)
    .fetch_one(db)
    .await
}

/// How `viewer` relates to `other`, ignoring blocks
pub async fn relationship(
    db: impl PgExecutor<'_>,
    viewer: Uuid,
    other: Uuid,
) -> sqlx::Result<Relationship> {
    if viewer == other {
        return Ok(Relationship::Myself);
    }
    let row: Option<(Uuid, String)> = sqlx::query_as(
        "SELECT requester_id, status FROM friendships
         WHERE LEAST(requester_id, addressee_id) = LEAST($1::uuid, $2::uuid)
           AND GREATEST(requester_id, addressee_id) = GREATEST($1::uuid, $2::uuid)",
    )
    .bind(viewer)
    .bind(other)
    .fetch_optional(db)
    .await?;
    Ok(match row {
        None => Relationship::None,
        Some((_, status)) if status == "accepted" => Relationship::Friends,
        Some((requester, _)) if requester == viewer => Relationship::Outgoing,
        Some(_) => Relationship::Incoming,
    })
}

pub async fn insert_request(
    tx: &mut Transaction<'_, Postgres>,
    requester: Uuid,
    addressee: Uuid,
) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO friendships (requester_id, addressee_id, status) VALUES ($1, $2, 'pending')",
    )
    .bind(requester)
    .bind(addressee)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Accepts the pending request `requester` sent to `addressee`; `false` when there is none
pub async fn accept(
    db: impl PgExecutor<'_>,
    requester: Uuid,
    addressee: Uuid,
) -> sqlx::Result<bool> {
    let result = sqlx::query(
        "UPDATE friendships SET status = 'accepted', responded_at = now()
         WHERE requester_id = $1 AND addressee_id = $2 AND status = 'pending'",
    )
    .bind(requester)
    .bind(addressee)
    .execute(db)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// Declines or cancels a pending request between the two users, whoever sent it
pub async fn delete_pending(db: &PgPool, a: Uuid, b: Uuid) -> sqlx::Result<bool> {
    delete_pair(db, a, b, Some("pending")).await
}

pub async fn delete_friendship(db: &PgPool, a: Uuid, b: Uuid) -> sqlx::Result<bool> {
    delete_pair(db, a, b, Some("accepted")).await
}

/// Deletes the pair's row in `status`, or in any status when `None`
async fn delete_pair(
    db: impl PgExecutor<'_>,
    a: Uuid,
    b: Uuid,
    status: Option<&str>,
) -> sqlx::Result<bool> {
    let result = sqlx::query(
        "DELETE FROM friendships
         WHERE LEAST(requester_id, addressee_id) = LEAST($1::uuid, $2::uuid)
           AND GREATEST(requester_id, addressee_id) = GREATEST($1::uuid, $2::uuid)
           AND ($3::text IS NULL OR status = $3)",
    )
    .bind(a)
    .bind(b)
    .bind(status)
    .execute(db)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// Blocks `blocked` and ends any friendship or pending request between the two
pub async fn block(
    tx: &mut Transaction<'_, Postgres>,
    blocker: Uuid,
    blocked: Uuid,
) -> sqlx::Result<()> {
    delete_pair(&mut **tx, blocker, blocked, None).await?;
    sqlx::query(
        "INSERT INTO user_blocks (blocker_id, blocked_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(blocker)
    .bind(blocked)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub async fn unblock(db: &PgPool, blocker: Uuid, blocked: Uuid) -> sqlx::Result<bool> {
    let result = sqlx::query("DELETE FROM user_blocks WHERE blocker_id = $1 AND blocked_id = $2")
        .bind(blocker)
        .bind(blocked)
        .execute(db)
        .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn blocked_users(db: &PgPool, blocker: Uuid) -> sqlx::Result<Vec<BlockedUser>> {
    sqlx::query_as(
        "SELECT u.id, u.username, u.display_name, b.created_at
         FROM user_blocks b JOIN users u ON u.id = b.blocked_id
         WHERE b.blocker_id = $1
         ORDER BY lower(u.username)",
    )
    .bind(blocker)
    .fetch_all(db)
    .await
}

/// Pending requests sent to `user_id`. A block always deletes the request, so none are hidden
pub async fn incoming(db: &PgPool, user_id: Uuid) -> sqlx::Result<Vec<FriendRequest>> {
    sqlx::query_as(
        "SELECT u.id, u.username, u.display_name, f.created_at
         FROM friendships f JOIN users u ON u.id = f.requester_id
         WHERE f.addressee_id = $1 AND f.status = 'pending'
         ORDER BY f.created_at DESC",
    )
    .bind(user_id)
    .fetch_all(db)
    .await
}

pub async fn outgoing(db: &PgPool, user_id: Uuid) -> sqlx::Result<Vec<FriendRequest>> {
    sqlx::query_as(
        "SELECT u.id, u.username, u.display_name, f.created_at
         FROM friendships f JOIN users u ON u.id = f.addressee_id
         WHERE f.requester_id = $1 AND f.status = 'pending'
         ORDER BY f.created_at DESC",
    )
    .bind(user_id)
    .fetch_all(db)
    .await
}

pub async fn sharing(db: &PgPool, user_id: Uuid) -> sqlx::Result<Sharing> {
    let sharing: Option<Sharing> = sqlx::query_as(
        "SELECT workouts, activities, stats, personal_records FROM friend_sharing WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_optional(db)
    .await?;
    Ok(sharing.unwrap_or_default())
}

pub async fn set_sharing(db: &PgPool, user_id: Uuid, sharing: Sharing) -> sqlx::Result<Sharing> {
    sqlx::query_as(
        "INSERT INTO friend_sharing (user_id, workouts, activities, stats, personal_records)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (user_id) DO UPDATE SET
            workouts = EXCLUDED.workouts,
            activities = EXCLUDED.activities,
            stats = EXCLUDED.stats,
            personal_records = EXCLUDED.personal_records,
            updated_at = now()
         RETURNING workouts, activities, stats, personal_records",
    )
    .bind(user_id)
    .bind(sharing.workouts)
    .bind(sharing.activities)
    .bind(sharing.stats)
    .bind(sharing.personal_records)
    .fetch_one(db)
    .await
}
