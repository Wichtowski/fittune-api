//! Opaque bearer sessions. Clients receive a random token; only its SHA-256 digest
//! is stored, so a database leak does not expose usable credentials.

use std::time::Duration;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::{PgExecutor, PgPool};
use uuid::Uuid;

use crate::users::model::Role;

/// Sessions are extended at most this often, to avoid a write on every request.
const TOUCH_INTERVAL_SECS: i64 = 10 * 60;

#[derive(Debug, Clone, Serialize)]
pub struct IssuedSession {
    pub token: String,
    pub expires_at: DateTime<Utc>,
}

/// The authenticated principal behind a request.
#[derive(Debug, Clone, Copy)]
pub struct Principal {
    pub user_id: Uuid,
    pub role: Role,
    pub session_id: Uuid,
}

/// Issues a random bearer token and stores only its digest
pub async fn create(
    db: impl PgExecutor<'_>,
    user_id: Uuid,
    ttl: Duration,
    user_agent: Option<&str>,
) -> sqlx::Result<IssuedSession> {
    let token = generate_token();
    let expires_at = sqlx::query_scalar(
        "INSERT INTO sessions (id, user_id, token_hash, user_agent, expires_at)
         VALUES ($1, $2, $3, $4, now() + make_interval(secs => $5))
         RETURNING expires_at",
    )
    .bind(Uuid::new_v4())
    .bind(user_id)
    .bind(hash_token(&token))
    .bind(user_agent.map(|ua| ua.chars().take(256).collect::<String>()))
    .bind(ttl.as_secs_f64())
    .fetch_one(db)
    .await?;
    Ok(IssuedSession { token, expires_at })
}

/// Resolves a bearer token to its principal, sliding the expiry forward on use.
pub async fn authenticate(
    db: &PgPool,
    token: &str,
    ttl: Duration,
) -> sqlx::Result<Option<Principal>> {
    let row: Option<(Uuid, Uuid, Role, bool)> = sqlx::query_as(
        "SELECT s.id, u.id, u.role, s.last_used_at < now() - make_interval(secs => $2)
         FROM sessions s JOIN users u ON u.id = s.user_id
         WHERE s.token_hash = $1 AND s.expires_at > now()",
    )
    .bind(hash_token(token))
    .bind(TOUCH_INTERVAL_SECS as f64)
    .fetch_optional(db)
    .await?;

    let Some((session_id, user_id, role, stale)) = row else {
        return Ok(None);
    };
    if stale {
        sqlx::query(
            "UPDATE sessions SET last_used_at = now(), expires_at = now() + make_interval(secs => $2)
             WHERE id = $1",
        )
        .bind(session_id)
        .bind(ttl.as_secs_f64())
        .execute(db)
        .await?;
    }
    Ok(Some(Principal {
        user_id,
        role,
        session_id,
    }))
}

/// Deletes one session, immediately invalidating its token
pub async fn revoke(db: impl PgExecutor<'_>, session_id: Uuid) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM sessions WHERE id = $1")
        .bind(session_id)
        .execute(db)
        .await?;
    Ok(())
}

/// Signs the user out everywhere except the session identified by `keep`.
pub async fn revoke_others(db: impl PgExecutor<'_>, user_id: Uuid, keep: Uuid) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM sessions WHERE user_id = $1 AND id <> $2")
        .bind(user_id)
        .bind(keep)
        .execute(db)
        .await?;
    Ok(())
}

/// Removes sessions whose expiry has passed
pub async fn purge_expired(db: &PgPool) -> sqlx::Result<u64> {
    let result = sqlx::query("DELETE FROM sessions WHERE expires_at <= now()")
        .execute(db)
        .await?;
    Ok(result.rows_affected())
}

fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    rand::fill(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn hash_token(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_unique_and_url_safe() {
        let a = generate_token();
        let b = generate_token();
        assert_ne!(a, b);
        assert_eq!(a.len(), 43);
        assert!(
            a.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        );
    }

    #[test]
    fn token_hash_is_stable_sha256() {
        assert_eq!(hash_token("abc"), hash_token("abc"));
        assert_eq!(hash_token("abc").len(), 32);
        assert_ne!(hash_token("abc"), hash_token("abd"));
    }
}
