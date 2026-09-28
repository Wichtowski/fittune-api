//! The one place that decides what a user may see of another user. Every friend read goes
//! through [`friends`] or [`can_view`], so a removed friendship or a block revokes access on the
//! next request

use sqlx::PgPool;
use uuid::Uuid;

use super::model::{Friend, Resource};
use crate::error::{ApiError, ApiResult};

/// Accepted friends of `viewer` with no block in either direction, each with what they share.
/// `only` narrows the result to one user
pub async fn friends(db: &PgPool, viewer: Uuid, only: Option<Uuid>) -> sqlx::Result<Vec<Friend>> {
    sqlx::query_as(
        "SELECT u.id, u.username, u.display_name, f.responded_at AS since,
                COALESCE(s.workouts, false) AS workouts,
                COALESCE(s.activities, false) AS activities,
                COALESCE(s.stats, false) AS stats,
                COALESCE(s.personal_records, false) AS personal_records
         FROM friendships f
         JOIN users u ON u.id = CASE WHEN f.requester_id = $1 THEN f.addressee_id ELSE f.requester_id END
         LEFT JOIN friend_sharing s ON s.user_id = u.id
         WHERE (f.requester_id = $1 OR f.addressee_id = $1)
           AND f.status = 'accepted'
           AND ($2::uuid IS NULL OR u.id = $2)
           AND NOT EXISTS (
               SELECT 1 FROM user_blocks b
               WHERE (b.blocker_id = $1 AND b.blocked_id = u.id)
                  OR (b.blocker_id = u.id AND b.blocked_id = $1)
           )
         ORDER BY lower(u.username)",
    )
    .bind(viewer)
    .bind(only)
    .fetch_all(db)
    .await
}

/// The friend `owner` as seen by `viewer`, or `404` when they are not friends
pub async fn friend(db: &PgPool, viewer: Uuid, owner: Uuid) -> ApiResult<Friend> {
    friends(db, viewer, Some(owner))
        .await?
        .into_iter()
        .next()
        .ok_or(ApiError::NotFound("friend"))
}

/// `404` when `owner` is not a friend of `viewer`, `403` when they do not share `resource`
pub async fn can_view(
    db: &PgPool,
    viewer: Uuid,
    owner: Uuid,
    resource: Resource,
) -> ApiResult<Friend> {
    let friend = friend(db, viewer, owner).await?;
    if friend.sharing.allows(resource) {
        Ok(friend)
    } else {
        Err(ApiError::Forbidden)
    }
}
