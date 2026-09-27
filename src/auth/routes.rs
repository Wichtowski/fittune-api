use axum::{
    Router,
    extract::State,
    http::{HeaderMap, StatusCode, header::USER_AGENT},
    routing::post,
};
use chrono::{NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{Auth, password, session};
use crate::{
    app::AppState,
    error::{ApiError, ApiResult, FieldErrors, unique_violation},
    extract::Json,
    users::{self, model::User},
    validate,
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
}

struct NewUser {
    username: String,
    email: String,
    password: String,
    display_name: Option<String>,
    birthday: Option<NaiveDate>,
}

impl RegisterRequest {
    fn validate(self) -> ApiResult<NewUser> {
        let mut errors = FieldErrors::default();

        let username = validate::required_text(&mut errors, "username", &self.username, 3, 32);
        errors.ensure(
            username
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-')),
            "username",
            "Username may only contain letters, digits, '.', '_' and '-'",
        );

        let email = self.email.trim().to_lowercase();
        errors.ensure(is_valid_email(&email), "email", "Invalid email address");

        if let Err(message) = password::check_policy(&self.password, &username) {
            errors.add("password", message);
        }

        let display_name = validate::optional_text(
            &mut errors,
            "display_name",
            self.display_name.as_deref(),
            64,
        );
        if let Some(birthday) = self.birthday {
            errors.ensure(
                birthday <= Utc::now().date_naive(),
                "birthday",
                "Invalid birthday",
            );
        }

        errors.into_result()?;
        Ok(NewUser {
            username,
            email,
            password: self.password,
            display_name,
            birthday: self.birthday,
        })
    }
}

fn is_valid_email(email: &str) -> bool {
    let Some((local, domain)) = email.split_once('@') else {
        return false;
    };
    email.len() <= 254
        && !local.is_empty()
        && !domain.contains('@')
        && !email.chars().any(char::is_whitespace)
        && domain.split('.').count() >= 2
        && domain.split('.').all(|label| !label.is_empty())
}

async fn register(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<RegisterRequest>,
) -> ApiResult<(StatusCode, axum::Json<AuthResponse>)> {
    let new_user = request.validate()?;
    let password_hash = password::hash(new_user.password).await?;

    let mut tx = state.db.begin().await?;
    let user_id = Uuid::new_v4();
    let inserted = sqlx::query(
        "INSERT INTO users (id, username, email, password_hash, display_name, birthday)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(user_id)
    .bind(&new_user.username)
    .bind(&new_user.email)
    .bind(&password_hash)
    .bind(&new_user.display_name)
    .bind(new_user.birthday)
    .execute(&mut *tx)
    .await;

    if let Err(err) = inserted {
        return Err(match unique_violation(&err) {
            Some("users_username_key") => {
                ApiError::validation("username", "Username already in use")
            }
            Some("users_email_key") => ApiError::validation("email", "Email already in use"),
            _ => err.into(),
        });
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

    tracing::info!(user_id = %user.id, "user registered");
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

    let credentials: Option<(Uuid, String)> = sqlx::query_as(
        "SELECT id, password_hash FROM users WHERE lower(username) = lower($1) OR lower(email) = lower($1)",
    )
    .bind(login)
    .fetch_optional(&state.db)
    .await?;

    let Some((user_id, password_hash)) = credentials else {
        password::verify_dummy(request.password).await?;
        return Err(ApiError::InvalidCredentials);
    };
    if !password::verify(request.password, password_hash).await? {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> RegisterRequest {
        RegisterRequest {
            username: "lifter".into(),
            email: "Lifter@Example.com ".into(),
            password: "Deadlift#200".into(),
            display_name: Some("  ".into()),
            birthday: NaiveDate::from_ymd_opt(1995, 5, 17),
        }
    }

    #[test]
    fn normalises_valid_registration() -> ApiResult<()> {
        let user = request().validate()?;
        assert_eq!(user.email, "lifter@example.com");
        assert_eq!(user.display_name, None);
        Ok(())
    }

    #[test]
    fn reports_every_invalid_field() {
        let invalid = RegisterRequest {
            username: "a b".into(),
            email: "not-an-email".into(),
            password: "short".into(),
            birthday: Some(Utc::now().date_naive() + chrono::Days::new(2)),
            ..request()
        };
        let Err(ApiError::Validation { fields, .. }) = invalid.validate() else {
            panic!("expected validation error");
        };
        let keys: Vec<_> = fields.keys().map(String::as_str).collect();
        assert_eq!(keys, ["birthday", "email", "password", "username"]);
    }

    #[test]
    fn email_validation() {
        for valid in ["a@b.co", "first.last+tag@sub.example.org"] {
            assert!(is_valid_email(valid), "{valid}");
        }
        for invalid in [
            "",
            "userexample.com",
            "@example.com",
            "a@b",
            "a@b..com",
            "a b@c.com",
            "a@b@c.com",
        ] {
            assert!(!is_valid_email(invalid), "{invalid}");
        }
    }
}
