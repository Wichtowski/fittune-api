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

    fn repair_lost_decimal(&mut self) {
        if self.source != "ocr" || !matches!(self.unit.as_deref(), Some("g" | "ml")) {
            return;
        }
        let Some(kcal) = self.values["energy_kcal"]
            .filter(|value| value.is_finite() && *value > 0.0 && *value <= 900.0)
        else {
            return;
        };
        if self.warnings.get("energy_kcal").is_some_and(|warnings| {
            warnings.iter().any(|warning| {
                !matches!(
                    warning.as_str(),
                    "Converted from kJ; check the label"
                        | "Converted from a portion; confirm the portion size and unit"
                )
            })
        }) {
            return;
        }
        let fields = ["protein_g", "fat_g", "carbs_g"];
        let [Some(protein), Some(fat), Some(carbs)] = fields.map(|field| self.values[field]) else {
            return;
        };
        let macros = [protein, fat, carbs];
        if macros
            .iter()
            .any(|value| !value.is_finite() || *value < 0.0)
        {
            return;
        }
        let estimate = 4.0 * protein + 9.0 * fat + 4.0 * carbs;
        let tolerance = (kcal * 0.15).max(10.0);
        if kcal - estimate > tolerance {
            self.warn(
                "energy_kcal",
                "Detected kcal is above the macro estimate; automatic macro correction skipped",
            );
            return;
        }
        if estimate - kcal <= tolerance {
            return;
        }
        // A calorie equation cannot identify three unknowns, so require one unique decimal-shift candidate
        let mut candidates = Vec::new();
        for (index, value) in macros.iter().enumerate() {
            for divisor in [10.0, 100.0] {
                let mut candidate = macros;
                candidate[index] = value / divisor;
                if candidate.iter().any(|value| *value > 100.0)
                    || candidate.iter().sum::<f64>() > 100.0
                    || self.values["saturated_fat_g"].is_some_and(|value| value > candidate[1])
                    || self.values["sugars_g"].is_some_and(|value| value > candidate[2])
                {
                    continue;
                }
                let corrected_estimate =
                    4.0 * candidate[0] + 9.0 * candidate[1] + 4.0 * candidate[2];
                if (corrected_estimate - kcal).abs() <= tolerance {
                    candidates.push((index, candidate[index]));
                }
            }
        }
        if let [(index, value)] = candidates.as_slice() {
            let field = fields[*index];
            self.values.insert(field.into(), Some(*value));
            self.evidence
                .entry(field.into())
                .or_default()
                .push_str(&format!(" [{} -> {} g]", macros[*index], value));
            self.warn(field, "Possible lost decimal point; adjusted using the other macros and detected kcal. Confirm the label");
        }
    }

    pub fn check(&mut self) {
        self.repair_lost_decimal();
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

#[cfg(test)]
mod tests {
    use super::*;

    fn reading(kcal: f64, protein: f64, fat: f64, carbs: f64) -> Extraction {
        let mut out = Extraction::blank("ocr");
        out.unit = Some("g".into());
        for (field, value) in [
            ("energy_kcal", kcal),
            ("protein_g", protein),
            ("fat_g", fat),
            ("carbs_g", carbs),
        ] {
            out.values.insert(field.into(), Some(value));
        }
        out
    }

    #[test]
    fn repairs_only_unique_decimal_errors_with_trusted_energy() {
        for (kcal, protein, fat, carbs, field, expected) in [
            (107.0, 19.0, 25.0, 2.2, "fat_g", 2.5),
            (107.0, 190.0, 2.5, 2.2, "protein_g", 19.0),
            (314.5, 2.5, 0.5, 750.0, "carbs_g", 75.0),
            (107.0, 19.0, 250.0, 2.2, "fat_g", 2.5),
        ] {
            let mut out = reading(kcal, protein, fat, carbs);
            out.evidence
                .insert(field.into(), "Original OCR text".into());
            out.check();
            assert_eq!(out.values[field], Some(expected));
            assert!(
                out.warnings[field]
                    .iter()
                    .any(|warning| warning.contains("Possible lost decimal point"))
            );
            assert!(out.evidence[field].starts_with("Original OCR text ["));
        }
        let mut high_energy = reading(500.0, 19.0, 2.5, 2.2);
        high_energy.check();
        assert_eq!(high_energy.values["fat_g"], Some(2.5));
        assert!(
            high_energy.warnings["energy_kcal"]
                .iter()
                .any(|warning| warning.contains("correction skipped"))
        );

        let mut ambiguous = reading(50.0, 10.0, 0.0, 10.0);
        ambiguous.check();
        assert_eq!(ambiguous.values["protein_g"], Some(10.0));
        assert_eq!(ambiguous.values["carbs_g"], Some(10.0));

        for scenario in ["missing", "uncertain", "parent", "ai", "invalid_energy"] {
            let mut out = reading(107.0, 19.0, 25.0, 2.2);
            match scenario {
                "missing" => {
                    out.values.insert("protein_g".into(), None);
                }
                "uncertain" => out.warn("energy_kcal", "Uncertain OCR reading; check the label"),
                "parent" => {
                    out.values.insert("saturated_fat_g".into(), Some(3.0));
                }
                "ai" => out.source = "ai".into(),
                _ => {
                    out.values.insert("energy_kcal".into(), Some(1070.0));
                }
            }
            out.check();
            assert_eq!(out.values["fat_g"], Some(25.0), "{scenario}");
        }
    }
}
