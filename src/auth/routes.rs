use axum::{
    Router,
    extract::State,
    http::{HeaderMap, StatusCode, header::USER_AGENT},
    routing::post,
};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{
    Auth, password, session,
    signup::{self, NewUser},
};
use crate::{
    app::AppState,
    config::Registration,
    error::{ApiError, ApiResult, FieldErrors},
    extract::{ClientKey, Json},
    invites,
    users::{
        self,
        model::{Role, User},
    },
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/register", post(register))
        .route("/login", post(login))
        .route("/logout", post(logout))
}

#[derive(Debug, Serialize)]
struct AuthResponse {
    user: User,
    session: session::IssuedSession,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RegisterRequest {
    username: String,
    email: String,
    password: String,
    display_name: Option<String>,
    birthday: Option<NaiveDate>,
    invite_code: Option<String>,
}

const INVALID_INVITE: &str = "This invite code is invalid, expired or already used";

async fn register(
    State(state): State<AppState>,
    ClientKey(client): ClientKey,
    headers: HeaderMap,
    Json(request): Json<RegisterRequest>,
) -> ApiResult<(StatusCode, axum::Json<AuthResponse>)> {
    if !state.registration_attempts.try_record(&client) {
        return Err(ApiError::RateLimited);
    }
    let invite_only = state.config.registration == Registration::InviteOnly;
    if invite_only && !state.invite_attempts.allows(&client) {
        return Err(ApiError::RateLimited);
    }

    let new_user = NewUser::validate(
        &request.username,
        &request.email,
        request.password,
        request.display_name.as_deref(),
        request.birthday,
    );
    let invite_code = request
        .invite_code
        .as_deref()
        .map(str::trim)
        .filter(|code| !code.is_empty());
    if invite_only && invite_code.is_none() {
        let mut errors = match new_user {
            Err(ApiError::Validation { fields, .. }) => FieldErrors::from(fields),
            _ => FieldErrors::default(),
        };
        errors.add("invite_code", "An invite code is required");
        return Err(errors.into_error());
    }
    let new_user = new_user?;

    let mut tx = state.db.begin().await?;
    // The invite stays locked until commit, and a failed signup rolls back without using it
    let invite_id = match invite_code.filter(|_| invite_only) {
        Some(code) => {
            match invites::repo::lock_usable(&mut tx, &invites::code::hash(code)).await? {
                Some(id) => Some(id),
                None => {
                    state.invite_attempts.record(&client);
                    return Err(ApiError::validation("invite_code", INVALID_INVITE));
                }
            }
        }
        None => None,
    };

    let user_id = signup::create(&mut tx, new_user, Role::User, invite_id).await?;
    if let Some(invite_id) = invite_id {
        invites::repo::consume(&mut tx, invite_id).await?;
    }
    let session = session::create(
        &mut *tx,
        user_id,
        state.config.session_ttl,
        user_agent(&headers),
    )
    .await?;
    let user = users::repo::find_by_id(&mut *tx, user_id)
        .await?
        .ok_or(ApiError::NotFound("user"))?;
    tx.commit().await?;

    tracing::info!(user_id = %user.id, invite_id = ?invite_id, "user registered");
    Ok((
        StatusCode::CREATED,
        axum::Json(AuthResponse { user, session }),
    ))
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct LoginRequest {
    /// Username or email address.
    login: String,
    password: String,
}

async fn login(
    State(state): State<AppState>,
    ClientKey(client): ClientKey,
    headers: HeaderMap,
    Json(request): Json<LoginRequest>,
) -> ApiResult<axum::Json<AuthResponse>> {
    let login = request.login.trim();
    if login.is_empty() || request.password.is_empty() {
        let mut errors = FieldErrors::default();
        errors.ensure(!login.is_empty(), "login", "Login is required");
        errors.ensure(
            !request.password.is_empty(),
            "password",
            "Password is required",
        );
        errors.into_result()?;
    }

    // Throttle per client and per account before any hashing, so guessing and CPU abuse are both bounded
    let client_key = format!("login-ip:{client}");
    let account_key = format!("login-account:{}", login.to_lowercase());
    if !state.login_attempts.allows(&client_key) || !state.login_attempts.allows(&account_key) {
        return Err(ApiError::RateLimited);
    }

    let credentials: Option<(Uuid, String)> = sqlx::query_as(
        "SELECT id, password_hash FROM users WHERE lower(username) = lower($1) OR lower(email) = lower($1)",
    )
    .bind(login)
    .fetch_optional(&state.db)
    .await?;

    let Some((user_id, password_hash)) = credentials else {
        password::verify_dummy(request.password).await?;
        state.login_attempts.record(&client_key);
        state.login_attempts.record(&account_key);
        return Err(ApiError::InvalidCredentials);
    };
    if !password::verify(request.password, password_hash).await? {
        state.login_attempts.record(&client_key);
        state.login_attempts.record(&account_key);
        return Err(ApiError::InvalidCredentials);
    }

    let session = session::create(
        &state.db,
        user_id,
        state.config.session_ttl,
        user_agent(&headers),
    )
    .await?;
    let user = users::repo::find_by_id(&state.db, user_id)
        .await?
        .ok_or(ApiError::InvalidCredentials)?;
    Ok(axum::Json(AuthResponse { user, session }))
}

async fn logout(State(state): State<AppState>, auth: Auth) -> ApiResult<StatusCode> {
    session::revoke(&state.db, auth.0.session_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

fn user_agent(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(USER_AGENT)
        .and_then(|value| value.to_str().ok())
}
