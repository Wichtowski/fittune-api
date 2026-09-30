use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    error::{ApiResult, FieldErrors},
    health::{
        barcode,
        nutrients::{Nutrients, Unit},
    },
    validate,
};

/// Where a product's values came from
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "text", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum ProductSource {
    #[default]
    Manual,
    /// Confirmed from an Open Food Facts listing
    Off,
}

/// A food in the shared database
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Product {
    pub id: Uuid,
    pub name: String,
    pub brand: Option<String>,
    pub barcode: Option<String>,
    #[sqlx(flatten)]
    pub per_100g: Nutrients,
    pub serving_amount: Option<f64>,
    pub serving_name: Option<String>,
    pub unit: Unit,
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
    pub serving_amount: Option<f64>,
    #[serde(default)]
    pub serving_name: Option<String>,
    #[serde(default)]
    pub barcode: Option<String>,
    #[serde(default)]
    pub source: ProductSource,
    #[serde(default)]
    pub unit: Unit,
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
        validate::finite_in_range(
            &mut errors,
            "serving_amount",
            self.serving_amount,
            0.1,
            2000.0,
        );
        self.per_100g.check(&mut errors, "per_100g");
        if let Some(code) = self
            .barcode
            .as_deref()
            .map(str::trim)
            .filter(|c| !c.is_empty())
        {
            self.barcode = barcode::normalize(code);
            errors.ensure(self.barcode.is_some(), "barcode", "Not a valid barcode");
        } else {
            self.barcode = None;
        }
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

/// An Open Food Facts listing that is not a FitHealth product yet; confirming it creates one.
/// `per_100g` is missing when OFF has no usable nutrition values for it
#[derive(Debug, Clone, Serialize)]
pub struct Candidate {
    pub barcode: String,
    pub name: String,
    pub brand: Option<String>,
    pub main_category: Option<String>,
    pub per_100g: Option<Nutrients>,
    pub serving_amount: Option<f64>,
    pub serving_name: Option<String>,
    pub unit: Unit,
}

#[derive(Debug, sqlx::FromRow)]
pub struct CandidateRow {
    pub barcode: String,
    pub name: String,
    pub brand: Option<String>,
    pub main_category: Option<String>,
    pub energy_kcal: Option<f64>,
    pub protein_g: Option<f64>,
    pub fat_g: Option<f64>,
    pub carbs_g: Option<f64>,
    pub saturated_fat_g: Option<f64>,
    pub sugars_g: Option<f64>,
    pub fiber_g: Option<f64>,
    pub salt_g: Option<f64>,
    pub serving_amount: Option<f64>,
    pub serving_name: Option<String>,
    pub unit: Unit,
}

impl From<CandidateRow> for Candidate {
    fn from(row: CandidateRow) -> Self {
        let per_100g = match (row.energy_kcal, row.protein_g, row.fat_g, row.carbs_g) {
            (Some(energy_kcal), Some(protein_g), Some(fat_g), Some(carbs_g)) => Some(Nutrients {
                energy_kcal,
                protein_g,
                fat_g,
                carbs_g,
                saturated_fat_g: row.saturated_fat_g,
                sugars_g: row.sugars_g,
                fiber_g: row.fiber_g,
                salt_g: row.salt_g,
            }),
            _ => None,
        };
        Self {
            barcode: row.barcode,
            name: row.name,
            brand: row.brand,
            main_category: row.main_category,
            per_100g,
            serving_amount: row.serving_amount,
            serving_name: row.serving_name,
            unit: row.unit,
        }
    }
}

/// What a scanned barcode is: a FitHealth product, an OFF listing to confirm, or unknown
#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Lookup {
    Found { product: Product },
    Off { candidate: Candidate },
    NotFound,
}

/// FitHealth products first, then OFF listings nobody has confirmed yet
#[derive(Debug, Serialize)]
pub struct SearchResults {
    pub products: Vec<Product>,
    pub off: Vec<Candidate>,
}
