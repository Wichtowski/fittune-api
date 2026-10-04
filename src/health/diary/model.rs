use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    error::{ApiResult, FieldErrors},
    health::{
        exercise::Exercise,
        nutrients::{Nutrients, Totals, Unit},
        targets::Targets,
    },
    validate,
};

/// A logged food. The product's name and values are a snapshot taken when it was logged
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Entry {
    pub id: Uuid,
    pub date: NaiveDate,
    pub meal_id: Uuid,
    pub product_id: Option<Uuid>,
    pub amount: f64,
    /// Grams or millilitres, as the product was measured when logged
    pub unit: Unit,
    pub product_name: String,
    pub product_brand: Option<String>,
    #[sqlx(flatten)]
    pub per_100g: Nutrients,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntryRequest {
    pub date: NaiveDate,
    pub meal_id: Uuid,
    pub product_id: Uuid,
    pub amount: f64,
}

impl EntryRequest {
    pub fn validate(self) -> ApiResult<Self> {
        let mut errors = FieldErrors::default();
        validate::finite_in_range(&mut errors, "amount", Some(self.amount), 0.1, 5000.0);
        errors.into_result()?;
        Ok(self)
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct DayMealRow {
    pub id: Uuid,
    pub name: String,
    pub archived: bool,
}

#[derive(Debug, Serialize)]
pub struct DayMeal {
    pub id: Uuid,
    pub name: String,
    /// Deleted meals only show up on days that still have entries in them
    pub archived: bool,
    pub entries: Vec<Entry>,
    pub totals: Totals,
}

#[derive(Debug, Serialize)]
pub struct Day {
    pub date: NaiveDate,
    pub meals: Vec<DayMeal>,
    pub totals: Totals,
    pub targets: Option<Targets>,
    pub exercise: Exercise,
    pub weight_kg: Option<f64>,
    /// What stops targets from being calculated: `profile`, `birthday`, `weight`
    pub missing: Vec<&'static str>,
}

#[derive(Debug, Deserialize)]
pub struct DayQuery {
    #[serde(default = "utc")]
    pub tz: String,
}

fn utc() -> String {
    "UTC".to_owned()
}
