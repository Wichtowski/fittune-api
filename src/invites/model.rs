use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    error::{ApiResult, FieldErrors},
    validate,
};

/// Invite metadata for admins. The code itself is only returned once, when it is created
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Invite {
    pub id: Uuid,
    pub note: Option<String>,
    pub status: InviteStatus,
    pub max_uses: i32,
    pub use_count: i32,
    pub expires_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub created_by: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, sqlx::Type)]
#[sqlx(type_name = "text", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum InviteStatus {
    Active,
    Used,
    Expired,
    Revoked,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InviteRequest {
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default = "default_expiry_days")]
    pub expires_in_days: i32,
    #[serde(default = "default_max_uses")]
    pub max_uses: i32,
}

fn default_expiry_days() -> i32 {
    7
}

fn default_max_uses() -> i32 {
    1
}

pub struct InviteDraft {
    pub note: Option<String>,
    pub expires_in_days: i32,
    pub max_uses: i32,
}

impl InviteRequest {
    pub fn validate(self) -> ApiResult<InviteDraft> {
        let mut errors = FieldErrors::default();
        let note = validate::optional_text(&mut errors, "note", self.note.as_deref(), 120);
        errors.ensure(
            (1..=90).contains(&self.expires_in_days),
            "expires_in_days",
            "Invites expire after 1 to 90 days",
        );
        errors.ensure(
            (1..=50).contains(&self.max_uses),
            "max_uses",
            "An invite can be used 1 to 50 times",
        );
        errors.into_result()?;
        Ok(InviteDraft {
            note,
            expires_in_days: self.expires_in_days,
            max_uses: self.max_uses,
        })
    }
}
