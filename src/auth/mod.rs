pub mod password;
mod routes;
pub mod session;
pub mod signup;

use axum::{
    extract::FromRequestParts,
    http::{header::AUTHORIZATION, request::Parts},
};

pub use routes::router;
pub use session::Principal;

use crate::{app::AppState, error::ApiError, users::model::Role};

/// Extracts the authenticated principal from an `Authorization: Bearer <token>` header.
#[derive(Debug, Clone, Copy)]
pub struct Auth(pub Principal);

impl Auth {
    pub fn user_id(&self) -> uuid::Uuid {
        self.0.user_id
    }

    /// What decides which exercises this user sees
    pub fn viewer(&self) -> crate::exercises::repo::Viewer {
        crate::exercises::repo::Viewer {
            user_id: self.user_id(),
            admin: self.is_admin(),
        }
    }

    pub fn is_admin(&self) -> bool {
        self.0.role == Role::Admin
    }

    pub fn require_admin(&self) -> Result<(), ApiError> {
        if self.is_admin() {
            Ok(())
        } else {
            Err(ApiError::Forbidden)
        }
    }
}

impl FromRequestParts<AppState> for Auth {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(bearer_token)
            .ok_or(ApiError::Unauthorized)?;

        session::authenticate(&state.db, token, state.config.session_ttl)
            .await?
            .map(Auth)
            .ok_or(ApiError::Unauthorized)
    }
}

fn bearer_token(header: &str) -> Option<&str> {
    let (scheme, token) = header.split_once(' ')?;
    let token = token.trim();
    (scheme.eq_ignore_ascii_case("bearer") && !token.is_empty()).then_some(token)
}

#[cfg(test)]
mod tests {
    use super::bearer_token;

    #[test]
    fn parses_bearer_header() {
        assert_eq!(bearer_token("Bearer abc"), Some("abc"));
        assert_eq!(bearer_token("bearer  abc "), Some("abc"));
        assert_eq!(bearer_token("Basic abc"), None);
        assert_eq!(bearer_token("Bearer "), None);
        assert_eq!(bearer_token("Bearer"), None);
    }
}
