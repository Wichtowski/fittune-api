use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    error::{ApiResult, FieldErrors},
    validate,
};

/// What a new user starts with. Stored in English, the app translates names it knows
pub const DEFAULT_MEALS: [&str; 5] = ["Breakfast", "Second breakfast", "Lunch", "Snack", "Dinner"];
pub const MAX_MEALS: i64 = 10;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Meal {
    pub id: Uuid,
    pub name: String,
    pub position: i32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MealRequest {
    pub name: String,
}

impl MealRequest {
    pub fn validate(mut self) -> ApiResult<Self> {
        let mut errors = FieldErrors::default();
        self.name = validate::required_text(&mut errors, "name", &self.name, 1, 40);
        errors.into_result()?;
        Ok(self)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrderRequest {
    pub ids: Vec<Uuid>,
}
