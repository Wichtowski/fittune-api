use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::error::{ApiError, ApiResult};

pub const FIELDS: [&str; 8] = [
    "energy_kcal",
    "fat_g",
    "saturated_fat_g",
    "carbs_g",
    "sugars_g",
    "fiber_g",
    "protein_g",
    "salt_g",
];

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub text: String,
    pub confidence: f64,
    pub bbox: [f64; 4],
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ParseRequest {
    pub width: u32,
    pub height: u32,
    pub observations: Vec<Observation>,
    #[serde(default)]
    pub column: Option<usize>,
}

impl ParseRequest {
    pub fn validate(&self) -> ApiResult<()> {
        if self.width == 0
            || self.height == 0
            || self.width > 2048
            || self.height > 2048
            || u64::from(self.width) * u64::from(self.height) > 4_000_000
            || self.observations.len() > 1000
        {
            return Err(ApiError::validation(
                "image",
                "Invalid crop dimensions or too many OCR observations",
            ));
        }
        for o in &self.observations {
            let [x0, y0, x1, y1] = o.bbox;
            if o.text.len() > 512
                || !o.confidence.is_finite()
                || !(0.0..=1.0).contains(&o.confidence)
                || !o.bbox.iter().all(|v| v.is_finite())
                || x0 < 0.0
                || y0 < 0.0
                || x1 <= x0
                || y1 <= y0
                || x1 > f64::from(self.width)
                || y1 > f64::from(self.height)
            {
                return Err(ApiError::validation(
                    "observations",
                    "Invalid OCR text, confidence or geometry",
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Column {
    pub label: String,
    pub unit: String,
    pub amount: f64,
    pub x: f64,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Extraction {
    pub values: BTreeMap<String, Option<f64>>,
    pub warnings: BTreeMap<String, Vec<String>>,
    pub evidence: BTreeMap<String, String>,
    pub unit: Option<String>,
    pub columns: Vec<Column>,
    pub selected_column: Option<usize>,
    pub source: String,
}

impl Extraction {
    pub fn blank(source: &str) -> Self {
        Self {
            values: FIELDS.iter().map(|f| ((*f).into(), None)).collect(),
            source: source.into(),
            ..Self::default()
        }
    }

    pub fn warn(&mut self, field: &str, message: &str) {
        let list = self.warnings.entry(field.into()).or_default();
        if !list.iter().any(|s| s == message) {
            list.push(message.into());
        }
    }

    pub fn check(&mut self) {
        for field in FIELDS {
            if let Some(value) = self.values.get(field).copied().flatten() {
                let max = if field == "energy_kcal" { 900.0 } else { 100.0 };
                if !value.is_finite() || !(0.0..=max).contains(&value) {
                    self.values.insert(field.into(), None);
                    self.warn(
                        field,
                        "Outside the product's supported range; check the label",
                    );
                }
            } else {
                self.warn(field, "Not read from the label");
            }
        }
        for (child, parent) in [("sugars_g", "carbs_g"), ("saturated_fat_g", "fat_g")] {
            if let (Some(a), Some(b)) = (self.values[child], self.values[parent])
                && a > b
            {
                self.warn(child, "Exceeds the parent nutrient; check the label");
            }
        }
        if let (Some(p), Some(f), Some(c)) = (
            self.values["protein_g"],
            self.values["fat_g"],
            self.values["carbs_g"],
        ) {
            if p + f + c > 100.0 {
                for field in ["protein_g", "fat_g", "carbs_g"] {
                    self.warn(field, "Macros exceed the product's supported total");
                }
            }
            if let Some(kcal) = self.values["energy_kcal"]
                && (kcal - (4.0 * p + 9.0 * f + 4.0 * c)).abs() > (kcal * 0.15).max(10.0)
            {
                self.warn("energy_kcal", "Energy differs from the macro estimate, which omits fibre, polyols and other contributors");
            }
        }
    }
}
