use axum::{
    Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use chrono::{NaiveDate, Utc};
use serde::Deserialize;
use uuid::Uuid;

use super::{
    model::{AccountType, DistanceUnit, ProfileUpdate, User, WeightUnit},
    repo,
};
use crate::{
    app::AppState,
    auth::{Auth, password, session},
    error::{ApiError, ApiResult, FieldErrors},
    extract::{Json, Query, double_option},
    validate,
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/me", get(me).patch(update_me).delete(delete_me))
        .route("/me/password", post(change_password))
        .route("/users", get(list_users))
}

async fn me(State(state): State<AppState>, auth: Auth) -> ApiResult<axum::Json<User>> {
    repo::find_by_id(&state.db, auth.user_id())
        .await?
        .map(axum::Json)
        .ok_or(ApiError::Unauthorized)
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct UpdateProfileRequest {
    #[serde(deserialize_with = "double_option")]
    display_name: Option<Option<String>>,
    #[serde(deserialize_with = "double_option")]
    birthday: Option<Option<NaiveDate>>,
    #[serde(deserialize_with = "double_option")]
    account_type: Option<Option<AccountType>>,
    weight_unit: Option<WeightUnit>,
    distance_unit: Option<DistanceUnit>,
}

impl UpdateProfileRequest {
    fn validate(self) -> ApiResult<ProfileUpdate> {
        let mut errors = FieldErrors::default();
        let display_name = self
            .display_name
            .map(|name| validate::optional_text(&mut errors, "display_name", name.as_deref(), 64));
        if let Some(Some(birthday)) = self.birthday {
            errors.ensure(
                birthday <= Utc::now().date_naive(),
                "birthday",
                "Invalid birthday",
            );
        }
        errors.into_result()?;

        Ok(ProfileUpdate {
            display_name,
            birthday: self.birthday,
            account_type: self.account_type,
            weight_unit: self.weight_unit,
            distance_unit: self.distance_unit,
        })
    }
}

async fn update_me(
    State(state): State<AppState>,
    auth: Auth,
    Json(request): Json<UpdateProfileRequest>,
) -> ApiResult<axum::Json<User>> {
    let update = request.validate()?;
    let user = repo::update_profile(&state.db, auth.user_id(), update).await?;
    Ok(axum::Json(user))
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ChangePasswordRequest {
    current_password: String,
    new_password: String,
}

/// Changes the password and signs out every other session.
async fn change_password(
    State(state): State<AppState>,
    auth: Auth,
    Json(request): Json<ChangePasswordRequest>,
) -> ApiResult<StatusCode> {
    let (username, current_hash) = credentials(&state, auth.user_id()).await?;
    if !password::verify(request.current_password, current_hash).await? {
        return Err(ApiError::validation(
            "current_password",
            "Current password is incorrect",
        ));
    }
    if let Err(message) = password::check_policy(&request.new_password, &username) {
        return Err(ApiError::validation("new_password", message));
    }

    let new_hash = password::hash(request.new_password).await?;
    let mut tx = state.db.begin().await?;
    sqlx::query("UPDATE users SET password_hash = $2, updated_at = now() WHERE id = $1")
        .bind(auth.user_id())
        .bind(new_hash)
        .execute(&mut *tx)
        .await?;
    session::revoke_others(&mut *tx, auth.user_id(), auth.0.session_id).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct DeleteAccountRequest {
    password: String,
}

/// Permanently deletes the account and all of its data. Requires the password as confirmation.
async fn delete_me(
    State(state): State<AppState>,
    auth: Auth,
    Json(request): Json<DeleteAccountRequest>,
) -> ApiResult<StatusCode> {
    let (_, current_hash) = credentials(&state, auth.user_id()).await?;
    if !password::verify(request.password, current_hash).await? {
        return Err(ApiError::validation("password", "Password is incorrect"));
    }
    crate::photos::delete_all_for_user(&state, auth.user_id()).await?;
    repo::delete(&state.db, auth.user_id()).await?;
    tracing::info!(user_id = %auth.user_id(), "account deleted");
    Ok(StatusCode::NO_CONTENT)
}

async fn credentials(state: &AppState, user_id: Uuid) -> ApiResult<(String, String)> {
    sqlx::query_as("SELECT username, password_hash FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::Unauthorized)
}

#[derive(Debug, Deserialize)]
struct ListUsersQuery {
    #[serde(default = "default_limit")]
    limit: i64,
    #[serde(default)]
    offset: i64,
}

fn default_limit() -> i64 {
    50
}

/// Admin-only user directory.
async fn list_users(
    State(state): State<AppState>,
    auth: Auth,
    Query(query): Query<ListUsersQuery>,
) -> ApiResult<axum::Json<Vec<User>>> {
    auth.require_admin()?;
    let users = repo::list(&state.db, query.limit.clamp(1, 200), query.offset.max(0)).await?;
    Ok(axum::Json(users))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distinguishes_absent_and_null_fields() -> Result<(), serde_json::Error> {
        let request: UpdateProfileRequest =
            serde_json::from_str(r#"{"display_name": null, "weight_unit": "lb"}"#)?;
        assert_eq!(request.display_name, Some(None));
        assert_eq!(request.birthday, None);
        assert_eq!(request.weight_unit, Some(WeightUnit::Lb));
        Ok(())
    }

    #[test]
    fn rejects_unknown_fields() {
        assert!(serde_json::from_str::<UpdateProfileRequest>(r#"{"role": "admin"}"#).is_err());
    }
}
