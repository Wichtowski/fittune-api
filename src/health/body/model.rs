use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::{
    error::{ApiResult, FieldErrors},
    health::targets::{Activity, Goal, Overrides, Sex},
    validate,
};

/// Everything is `None` until the user sets their profile up
#[derive(Debug, Clone, Default, Serialize, sqlx::FromRow)]
pub struct Profile {
    pub sex: Option<Sex>,
    pub height_cm: Option<f64>,
    pub activity: Option<Activity>,
    pub goal: Option<Goal>,
    pub pace_kg_per_week: Option<f64>,
    pub energy_kcal: Option<f64>,
    pub protein_g: Option<f64>,
    pub fat_g: Option<f64>,
    pub carbs_g: Option<f64>,
}

impl Profile {
    pub fn overrides(&self) -> Overrides {
        Overrides {
            energy_kcal: self.energy_kcal,
            protein_g: self.protein_g,
            fat_g: self.fat_g,
            carbs_g: self.carbs_g,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileRequest {
    pub sex: Sex,
    pub height_cm: f64,
    pub activity: Activity,
    pub goal: Goal,
    pub pace_kg_per_week: f64,
    #[serde(default)]
    pub energy_kcal: Option<f64>,
    #[serde(default)]
    pub protein_g: Option<f64>,
    #[serde(default)]
    pub fat_g: Option<f64>,
    #[serde(default)]
    pub carbs_g: Option<f64>,
}

impl ProfileRequest {
    pub fn validate(mut self) -> ApiResult<Self> {
        let mut errors = FieldErrors::default();
        validate::finite_in_range(&mut errors, "height_cm", Some(self.height_cm), 100.0, 250.0);
        validate::finite_in_range(
            &mut errors,
            "pace_kg_per_week",
            Some(self.pace_kg_per_week),
            0.0,
            1.0,
        );
        validate::finite_in_range(&mut errors, "energy_kcal", self.energy_kcal, 800.0, 6000.0);
        validate::finite_in_range(&mut errors, "protein_g", self.protein_g, 0.0, 500.0);
        validate::finite_in_range(&mut errors, "fat_g", self.fat_g, 0.0, 400.0);
        validate::finite_in_range(&mut errors, "carbs_g", self.carbs_g, 0.0, 1000.0);
        errors.into_result()?;
        // Keeping weight has no pace
        if self.goal == Goal::Maintain {
            self.pace_kg_per_week = 0.0;
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Weight {
    pub date: NaiveDate,
    pub weight_kg: f64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeightRequest {
    pub weight_kg: f64,
}

impl WeightRequest {
    pub fn validate(self) -> ApiResult<Self> {
        let mut errors = FieldErrors::default();
        validate::finite_in_range(&mut errors, "weight_kg", Some(self.weight_kg), 25.0, 400.0);
        errors.into_result()?;
        Ok(self)
    }
}

#[derive(Debug, Deserialize)]
pub struct WeightsQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
}

fn default_limit() -> i64 {
    30
}
