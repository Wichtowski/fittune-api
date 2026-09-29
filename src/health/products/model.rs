use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    error::{ApiResult, FieldErrors},
    health::nutrients::Nutrients,
    validate,
};

/// A food in the shared database
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Product {
    pub id: Uuid,
    pub name: String,
    pub brand: Option<String>,
    pub barcode: Option<String>,
    #[sqlx(flatten)]
    pub per_100g: Nutrients,
    pub serving_g: Option<f64>,
    pub serving_name: Option<String>,
    pub source: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProductRequest {
    pub name: String,
    #[serde(default)]
    pub brand: Option<String>,
    pub per_100g: Nutrients,
    #[serde(default)]
    pub serving_g: Option<f64>,
    #[serde(default)]
    pub serving_name: Option<String>,
}

impl ProductRequest {
    pub fn validate(mut self) -> ApiResult<Self> {
        let mut errors = FieldErrors::default();
        self.name = validate::required_text(&mut errors, "name", &self.name, 1, 120);
        self.brand = validate::optional_text(&mut errors, "brand", self.brand.as_deref(), 80);
        self.serving_name = validate::optional_text(
            &mut errors,
            "serving_name",
            self.serving_name.as_deref(),
            40,
        );
        validate::finite_in_range(&mut errors, "serving_g", self.serving_g, 0.1, 2000.0);
        self.per_100g.check(&mut errors, "per_100g");
        errors.into_result()?;
        Ok(self)
    }
}

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    #[serde(default)]
    pub q: String,
    #[serde(default = "default_limit")]
    pub limit: i64,
}

fn default_limit() -> i64 {
    20
}
