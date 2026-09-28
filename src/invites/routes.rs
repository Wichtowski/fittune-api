use axum::{
    Router,
    extract::State,
    http::StatusCode,
    routing::{delete, get},
};
use serde::Serialize;
use uuid::Uuid;

use super::{
    code,
    model::{Invite, InviteRequest},
    repo,
};
use crate::{
    app::AppState,
    auth::Auth,
    error::{ApiError, ApiResult},
    extract::{Json, Path},
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/invites", get(list).post(create))
        .route("/admin/invites/{id}", delete(revoke))
}

#[derive(Serialize)]
struct CreatedInvite {
    invite: Invite,
    /// The only time the plaintext code is available
    code: String,
}

async fn create(
    State(state): State<AppState>,
    auth: Auth,
    Json(request): Json<InviteRequest>,
) -> ApiResult<(StatusCode, axum::Json<CreatedInvite>)> {
    auth.require_admin()?;
    let draft = request.validate()?;
    let code = code::generate();
    let invite = repo::create(&state.db, auth.user_id(), &draft, &code::hash(&code)).await?;
    // Never log the code itself
    tracing::info!(invite_id = %invite.id, admin_id = %auth.user_id(), "invite created");
    Ok((
        StatusCode::CREATED,
        axum::Json(CreatedInvite { invite, code }),
    ))
}

async fn list(State(state): State<AppState>, auth: Auth) -> ApiResult<axum::Json<Vec<Invite>>> {
    auth.require_admin()?;
    Ok(axum::Json(repo::list(&state.db).await?))
}

async fn revoke(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    auth.require_admin()?;
    if repo::revoke(&state.db, id).await? {
        tracing::info!(invite_id = %id, admin_id = %auth.user_id(), "invite revoked");
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound("invite"))
    }
}
