use std::collections::BTreeMap;

use axum::{
    Json,
    extract::rejection::{JsonRejection, PathRejection, QueryRejection},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;

/// Every error the HTTP layer can return. Converted into a JSON body of the shape
/// `{ "code": "...", "message": "...", "fields": { ... } }`.
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("{0}")]
    BadRequest(String),
    #[error("{message}")]
    Validation {
        message: String,
        fields: BTreeMap<String, String>,
    },
    #[error("authentication required")]
    Unauthorized,
    #[error("invalid login or password")]
    InvalidCredentials,
    #[error("you do not have permission to perform this action")]
    Forbidden,
    #[error("{0} not found")]
    NotFound(&'static str),
    #[error("{0}")]
    Conflict(String),
    #[error("photo storage is unavailable")]
    StorageUnavailable,
    #[error("photo is too large")]
    PayloadTooLarge,
    #[error("too many attempts, try again later")]
    RateLimited,
    #[error("database error")]
    Database(#[from] sqlx::Error),
    #[error("internal error")]
    Internal(#[from] anyhow::Error),
}

pub type ApiResult<T> = Result<T, ApiError>;

#[derive(Serialize)]
struct ErrorBody<'a> {
    code: &'a str,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    fields: Option<&'a BTreeMap<String, String>>,
}

impl ApiError {
    pub fn validation(field: &str, message: impl Into<String>) -> Self {
        let mut errors = FieldErrors::default();
        errors.add(field, message);
        errors.into_error()
    }

    fn status_and_code(&self) -> (StatusCode, &'static str) {
        match self {
            Self::BadRequest(_) => (StatusCode::BAD_REQUEST, "bad_request"),
            Self::Validation { .. } => (StatusCode::UNPROCESSABLE_ENTITY, "validation_failed"),
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized"),
            Self::InvalidCredentials => (StatusCode::UNAUTHORIZED, "invalid_credentials"),
            Self::Forbidden => (StatusCode::FORBIDDEN, "forbidden"),
            Self::NotFound(_) => (StatusCode::NOT_FOUND, "not_found"),
            Self::Conflict(_) => (StatusCode::CONFLICT, "conflict"),
            Self::StorageUnavailable => (StatusCode::SERVICE_UNAVAILABLE, "storage_unavailable"),
            Self::PayloadTooLarge => (StatusCode::PAYLOAD_TOO_LARGE, "payload_too_large"),
            Self::RateLimited => (StatusCode::TOO_MANY_REQUESTS, "rate_limited"),
            Self::Database(_) | Self::Internal(_) => {
                (StatusCode::INTERNAL_SERVER_ERROR, "internal_error")
            }
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code) = self.status_and_code();
        match &self {
            Self::Database(err) => tracing::error!(error = %err, "database error"),
            Self::Internal(err) => tracing::error!(error = ?err, "internal error"),
            _ => {}
        }

        let fields = match &self {
            Self::Validation { fields, .. } if !fields.is_empty() => Some(fields),
            _ => None,
        };
        let body = ErrorBody {
            code,
            message: self.to_string(),
            fields,
        };
        (status, Json(body)).into_response()
    }
}

impl From<JsonRejection> for ApiError {
    fn from(rejection: JsonRejection) -> Self {
        Self::BadRequest(rejection.body_text())
    }
}

impl From<QueryRejection> for ApiError {
    fn from(rejection: QueryRejection) -> Self {
        Self::BadRequest(rejection.body_text())
    }
}

impl From<PathRejection> for ApiError {
    fn from(rejection: PathRejection) -> Self {
        Self::BadRequest(rejection.body_text())
    }
}

/// Accumulates per-field validation messages so clients can show every problem at once.
#[derive(Debug, Default)]
pub struct FieldErrors(BTreeMap<String, String>);

impl From<BTreeMap<String, String>> for FieldErrors {
    fn from(fields: BTreeMap<String, String>) -> Self {
        Self(fields)
    }
}

impl FieldErrors {
    /// Records `message` for `field`, keeping the first message if the field already failed.
    pub fn add(&mut self, field: impl Into<String>, message: impl Into<String>) {
        self.0.entry(field.into()).or_insert_with(|| message.into());
    }

    pub fn ensure(
        &mut self,
        condition: bool,
        field: impl Into<String>,
        message: impl Into<String>,
    ) {
        if !condition {
            self.add(field, message);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn into_result(self) -> ApiResult<()> {
        if self.is_empty() {
            Ok(())
        } else {
            Err(self.into_error())
        }
    }

    pub(crate) fn into_error(self) -> ApiError {
        let message = self
            .0
            .values()
            .next()
            .cloned()
            .unwrap_or_else(|| "Invalid request".to_owned());
        ApiError::Validation {
            message,
            fields: self.0,
        }
    }
}

/// Returns the violated constraint name when `err` is a unique-constraint violation.
pub fn unique_violation(err: &sqlx::Error) -> Option<&str> {
    match err {
        sqlx::Error::Database(db) if db.is_unique_violation() => db.constraint(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_errors_keep_first_message_per_field() {
        let mut errors = FieldErrors::default();
        errors.add("password", "too short");
        errors.add("password", "missing uppercase");
        errors.ensure(true, "email", "never recorded");

        let Err(ApiError::Validation { message, fields }) = errors.into_result() else {
            panic!("expected a validation error");
        };
        assert_eq!(message, "too short");
        assert_eq!(fields.len(), 1);
    }

    #[test]
    fn empty_field_errors_are_ok() {
        assert!(FieldErrors::default().into_result().is_ok());
    }
}
