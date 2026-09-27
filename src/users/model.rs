use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "text", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Admin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "text", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum AccountType {
    GymEnthusiast,
    ProfessionalTrainer,
    Nutritionist,
    Psychologist,
    PhysicalTherapist,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "text", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum WeightUnit {
    Kg,
    Lb,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "text", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum DistanceUnit {
    Km,
    Mi,
}

/// A user as exposed by the API. Credentials never leave the `auth` module.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct User {
    pub id: Uuid,
    pub username: String,
    pub email: String,
    pub display_name: Option<String>,
    pub birthday: Option<NaiveDate>,
    pub role: Role,
    pub account_type: Option<AccountType>,
    pub weight_unit: WeightUnit,
    pub distance_unit: DistanceUnit,
    pub created_at: DateTime<Utc>,
}

/// Validated profile changes; `None` leaves a field untouched, `Some(None)` clears it.
#[derive(Debug, Default)]
pub struct ProfileUpdate {
    pub display_name: Option<Option<String>>,
    pub birthday: Option<Option<NaiveDate>>,
    pub account_type: Option<Option<AccountType>>,
    pub weight_unit: Option<WeightUnit>,
    pub distance_unit: Option<DistanceUnit>,
}
