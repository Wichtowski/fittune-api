//! Account creation shared by `POST /auth/register` and the `create-admin` operator command,
//! so both apply the same validation, password policy and hashing.

use chrono::{NaiveDate, Utc};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use super::password;
use crate::{
    error::{ApiError, ApiResult, FieldErrors, unique_violation},
    users::model::Role,
    validate,
};

/// A validated new account.
#[derive(Debug)]
pub struct NewUser {
    pub username: String,
    pub email: String,
    pub password: String,
    pub display_name: Option<String>,
    pub birthday: Option<NaiveDate>,
}

impl NewUser {
    pub fn validate(
        username: &str,
        email: &str,
        password: String,
        display_name: Option<&str>,
        birthday: Option<NaiveDate>,
    ) -> ApiResult<Self> {
        let mut errors = FieldErrors::default();

        let username = validate::required_text(&mut errors, "username", username, 3, 32);
        errors.ensure(
            username
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-')),
            "username",
            "Username may only contain letters, digits, '.', '_' and '-'",
        );

        let email = email.trim().to_lowercase();
        errors.ensure(is_valid_email(&email), "email", "Invalid email address");

        if let Err(message) = password::check_policy(&password, &username) {
            errors.add("password", message);
        }

        let display_name = validate::optional_text(&mut errors, "display_name", display_name, 64);
        if let Some(birthday) = birthday {
            errors.ensure(
                birthday <= Utc::now().date_naive(),
                "birthday",
                "Invalid birthday",
            );
        }

        errors.into_result()?;
        Ok(Self {
            username,
            email,
            password,
            display_name,
            birthday,
        })
    }
}

/// Inserts the account. Taken usernames and emails become field errors
pub async fn create(
    tx: &mut Transaction<'_, Postgres>,
    user: NewUser,
    role: Role,
    invite_id: Option<Uuid>,
) -> ApiResult<Uuid> {
    let password_hash = password::hash(user.password).await?;
    let id = Uuid::new_v4();
    let inserted = sqlx::query(
        "INSERT INTO users (id, username, email, password_hash, display_name, birthday, role, invite_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(id)
    .bind(&user.username)
    .bind(&user.email)
    .bind(&password_hash)
    .bind(&user.display_name)
    .bind(user.birthday)
    .bind(role)
    .bind(invite_id)
    .execute(&mut **tx)
    .await;

    match inserted {
        Ok(_) => Ok(id),
        Err(err) => Err(match unique_violation(&err) {
            Some("users_username_key") => {
                ApiError::validation("username", "Username already in use")
            }
            Some("users_email_key") => ApiError::validation("email", "Email already in use"),
            _ => err.into(),
        }),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn valid() -> ApiResult<NewUser> {
        NewUser::validate(
            "lifter",
            "Lifter@Example.com ",
            "Deadlift#200".into(),
            Some("  "),
            NaiveDate::from_ymd_opt(1995, 5, 17),
        )
    }

    #[test]
    fn normalises_valid_registration() -> ApiResult<()> {
        let user = valid()?;
        assert_eq!(user.email, "lifter@example.com");
        assert_eq!(user.display_name, None);
        Ok(())
    }

    #[test]
    fn reports_every_invalid_field() {
        let invalid = NewUser::validate(
            "a b",
            "not-an-email",
            "short".into(),
            None,
            Some(Utc::now().date_naive() + chrono::Days::new(2)),
        );
        let Err(ApiError::Validation { fields, .. }) = invalid else {
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
