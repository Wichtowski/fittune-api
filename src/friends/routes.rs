use axum::{
    Router,
    extract::State,
    http::StatusCode,
    routing::{delete, get, post, put},
};
use serde::Deserialize;
use uuid::Uuid;

use super::{
    access,
    model::{
        BlockedUser, Friend, FriendRequests, PublicUser, Relationship, Sharing,
        UserWithRelationship,
    },
    progress, repo,
};
use crate::{
    app::AppState,
    auth::Auth,
    error::{ApiError, ApiResult},
    extract::{Json, Path, Query},
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/users/lookup", get(lookup))
        .route("/me/sharing", get(sharing).put(set_sharing))
        .route("/friends", get(list))
        .route("/friends/feed", get(progress::feed))
        .route("/friends/requests", get(requests).post(send_request))
        .route("/friends/requests/{user_id}", delete(delete_request))
        .route("/friends/requests/{user_id}/accept", post(accept_request))
        .route("/friends/{user_id}", get(progress::profile).delete(remove))
        .route("/friends/{user_id}/feed", get(progress::friend_feed))
        .route("/friends/{user_id}/stats/overview", get(progress::overview))
        .route("/friends/{user_id}/records", get(progress::records))
        .route("/blocks", get(blocks))
        .route("/blocks/{user_id}", put(block).delete(unblock))
}

/// Counts a lookup or request against the caller's hourly allowance
fn spend_friend_action(state: &AppState, auth: &Auth) -> ApiResult<()> {
    let key = auth.user_id().to_string();
    if !state.friend_actions.allows(&key) {
        return Err(ApiError::RateLimited);
    }
    state.friend_actions.record(&key);
    Ok(())
}

#[derive(Debug, Deserialize)]
struct LookupQuery {
    username: String,
}

/// Finds one user by exact (case-insensitive) username. There is no directory or partial
/// search, and users with a block in either direction look like they do not exist
async fn lookup(
    State(state): State<AppState>,
    auth: Auth,
    Query(query): Query<LookupQuery>,
) -> ApiResult<axum::Json<UserWithRelationship>> {
    spend_friend_action(&state, &auth)?;
    let user = visible_by_username(&state, auth.user_id(), query.username.trim()).await?;
    let relationship = repo::relationship(&state.db, auth.user_id(), user.id).await?;
    Ok(axum::Json(UserWithRelationship { user, relationship }))
}

async fn visible_by_username(
    state: &AppState,
    viewer: Uuid,
    username: &str,
) -> ApiResult<PublicUser> {
    let user = repo::find_by_username(&state.db, username)
        .await?
        .ok_or(ApiError::NotFound("user"))?;
    if user.id != viewer
        && (repo::has_blocked(&state.db, viewer, user.id).await?
            || repo::has_blocked(&state.db, user.id, viewer).await?)
    {
        return Err(ApiError::NotFound("user"));
    }
    Ok(user)
}

async fn list(State(state): State<AppState>, auth: Auth) -> ApiResult<axum::Json<Vec<Friend>>> {
    Ok(axum::Json(
        access::friends(&state.db, auth.user_id(), None).await?,
    ))
}

async fn requests(
    State(state): State<AppState>,
    auth: Auth,
) -> ApiResult<axum::Json<FriendRequests>> {
    Ok(axum::Json(FriendRequests {
        incoming: repo::incoming(&state.db, auth.user_id()).await?,
        outgoing: repo::outgoing(&state.db, auth.user_id()).await?,
    }))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SendRequest {
    username: String,
}

/// Sends a friend request. Accepts theirs instead when the other user already asked, and
/// repeats are no-ops, so the response always carries the resulting relationship
async fn send_request(
    State(state): State<AppState>,
    auth: Auth,
    Json(request): Json<SendRequest>,
) -> ApiResult<(StatusCode, axum::Json<UserWithRelationship>)> {
    spend_friend_action(&state, &auth)?;
    let me = auth.user_id();
    let user = repo::find_by_username(&state.db, request.username.trim())
        .await?
        .ok_or(ApiError::NotFound("user"))?;
    if user.id == me {
        return Err(ApiError::validation(
            "username",
            "You cannot add yourself as a friend",
        ));
    }

    let mut tx = state.db.begin().await?;
    repo::lock_pair(&mut tx, me, user.id).await?;
    if repo::has_blocked(&mut *tx, me, user.id).await? {
        return Err(ApiError::Conflict(
            "Unblock this user before sending a friend request".to_owned(),
        ));
    }
    if repo::has_blocked(&mut *tx, user.id, me).await? {
        return Err(ApiError::NotFound("user"));
    }
    let (status, relationship) = match repo::relationship(&mut *tx, me, user.id).await? {
        Relationship::None => {
            repo::insert_request(&mut tx, me, user.id).await?;
            (StatusCode::CREATED, Relationship::Outgoing)
        }
        Relationship::Incoming => {
            repo::accept(&mut *tx, user.id, me).await?;
            (StatusCode::OK, Relationship::Friends)
        }
        existing => (StatusCode::OK, existing),
    };
    tx.commit().await?;
    Ok((
        status,
        axum::Json(UserWithRelationship { user, relationship }),
    ))
}

async fn accept_request(
    State(state): State<AppState>,
    auth: Auth,
    Path(user_id): Path<Uuid>,
) -> ApiResult<axum::Json<Friend>> {
    if !repo::accept(&state.db, user_id, auth.user_id()).await? {
        return Err(ApiError::NotFound("friend request"));
    }
    Ok(axum::Json(
        access::friend(&state.db, auth.user_id(), user_id).await?,
    ))
}

/// Declines an incoming request or cancels an outgoing one
async fn delete_request(
    State(state): State<AppState>,
    auth: Auth,
    Path(user_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    if repo::delete_pending(&state.db, auth.user_id(), user_id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound("friend request"))
    }
}

async fn remove(
    State(state): State<AppState>,
    auth: Auth,
    Path(user_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    if repo::delete_friendship(&state.db, auth.user_id(), user_id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound("friend"))
    }
}

async fn blocks(
    State(state): State<AppState>,
    auth: Auth,
) -> ApiResult<axum::Json<Vec<BlockedUser>>> {
    Ok(axum::Json(
        repo::blocked_users(&state.db, auth.user_id()).await?,
    ))
}

/// Blocks a user and ends any friendship or pending request with them. Idempotent
async fn block(
    State(state): State<AppState>,
    auth: Auth,
    Path(user_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let me = auth.user_id();
    if user_id == me {
        return Err(ApiError::validation("user_id", "You cannot block yourself"));
    }
    repo::find_user(&state.db, user_id)
        .await?
        .ok_or(ApiError::NotFound("user"))?;

    let mut tx = state.db.begin().await?;
    repo::lock_pair(&mut tx, me, user_id).await?;
    repo::block(&mut tx, me, user_id).await?;
    tx.commit().await?;
    tracing::info!(blocker_id = %me, blocked_id = %user_id, "user blocked");
    Ok(StatusCode::NO_CONTENT)
}

async fn unblock(
    State(state): State<AppState>,
    auth: Auth,
    Path(user_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    if repo::unblock(&state.db, auth.user_id(), user_id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound("block"))
    }
}

async fn sharing(State(state): State<AppState>, auth: Auth) -> ApiResult<axum::Json<Sharing>> {
    Ok(axum::Json(repo::sharing(&state.db, auth.user_id()).await?))
}

async fn set_sharing(
    State(state): State<AppState>,
    auth: Auth,
    Json(sharing): Json<Sharing>,
) -> ApiResult<axum::Json<Sharing>> {
    Ok(axum::Json(
        repo::set_sharing(&state.db, auth.user_id(), sharing).await?,
    ))
}
