//! Label values per 100 g and the totals built from them

use std::ops::Add;

use serde::{Deserialize, Serialize};

use crate::{error::FieldErrors, validate};

/// Values per 100 g, as on an EU nutrition label
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, sqlx::FromRow)]
#[serde(deny_unknown_fields)]
pub struct Nutrients {
    pub energy_kcal: f64,
    pub protein_g: f64,
    pub fat_g: f64,
    pub carbs_g: f64,
    #[serde(default)]
    pub saturated_fat_g: Option<f64>,
    #[serde(default)]
    pub sugars_g: Option<f64>,
    #[serde(default)]
    pub fiber_g: Option<f64>,
    #[serde(default)]
    pub salt_g: Option<f64>,
}

impl Nutrients {
    /// Records errors as `<prefix>.<field>`, and label-wide problems under `prefix`
    pub fn check(&self, errors: &mut FieldErrors, prefix: &str) {
        let field = |name: &str| format!("{prefix}.{name}");
        validate::finite_in_range(
            errors,
            &field("energy_kcal"),
            Some(self.energy_kcal),
            0.0,
            900.0,
        );
        for (name, value) in [
            ("protein_g", Some(self.protein_g)),
            ("fat_g", Some(self.fat_g)),
            ("carbs_g", Some(self.carbs_g)),
            ("saturated_fat_g", self.saturated_fat_g),
            ("sugars_g", self.sugars_g),
            ("fiber_g", self.fiber_g),
            ("salt_g", self.salt_g),
        ] {
            validate::finite_in_range(errors, &field(name), value, 0.0, 100.0);
        }
        if let Some(saturated) = self.saturated_fat_g {
            errors.ensure(
                saturated <= self.fat_g,
                field("saturated_fat_g"),
                "Saturates cannot exceed fat",
            );
        }
        if let Some(sugars) = self.sugars_g {
            errors.ensure(
                sugars <= self.carbs_g,
                field("sugars_g"),
                "Sugars cannot exceed carbohydrate",
            );
        }
        errors.ensure(
            self.protein_g + self.fat_g + self.carbs_g <= 100.0,
            prefix,
            "Protein, fat and carbohydrate add up to more than 100 g",
        );
    }
}

/// Summed amounts; missing optional values count as zero
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct Totals {
    pub energy_kcal: f64,
    pub protein_g: f64,
    pub fat_g: f64,
    pub carbs_g: f64,
    pub saturated_fat_g: f64,
    pub sugars_g: f64,
    pub fiber_g: f64,
    pub salt_g: f64,
}

impl Totals {
    /// What `grams` of a food with `per_100g` contains
    pub fn of(per_100g: &Nutrients, grams: f64) -> Self {
        let f = grams / 100.0;
        Self {
            energy_kcal: per_100g.energy_kcal * f,
            protein_g: per_100g.protein_g * f,
            fat_g: per_100g.fat_g * f,
            carbs_g: per_100g.carbs_g * f,
            saturated_fat_g: per_100g.saturated_fat_g.unwrap_or(0.0) * f,
            sugars_g: per_100g.sugars_g.unwrap_or(0.0) * f,
            fiber_g: per_100g.fiber_g.unwrap_or(0.0) * f,
            salt_g: per_100g.salt_g.unwrap_or(0.0) * f,
        }
    }
}

impl Add for Totals {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self {
            energy_kcal: self.energy_kcal + other.energy_kcal,
            protein_g: self.protein_g + other.protein_g,
            fat_g: self.fat_g + other.fat_g,
            carbs_g: self.carbs_g + other.carbs_g,
            saturated_fat_g: self.saturated_fat_g + other.saturated_fat_g,
            sugars_g: self.sugars_g + other.sugars_g,
            fiber_g: self.fiber_g + other.fiber_g,
            salt_g: self.salt_g + other.salt_g,
        }
    }
}
