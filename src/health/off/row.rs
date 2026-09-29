//! One line of the Open Food Facts CSV export (tab separated, one product per line) mapped to
//! what FitHealth keeps

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};

use crate::health::barcode;

/// Positions of the columns FitHealth reads, found by name so OFF may add or reorder columns
#[derive(Debug, Clone)]
pub struct Columns {
    code: usize,
    product_name: usize,
    brands: usize,
    main_category: usize,
    energy_kcal: usize,
    energy_kj: usize,
    protein: usize,
    fat: usize,
    saturated_fat: usize,
    carbs: usize,
    sugars: usize,
    fiber: usize,
    salt: usize,
    serving_quantity: usize,
    serving_size: usize,
    last_modified: usize,
    count: usize,
}

impl Columns {
    pub fn from_header(header: &str) -> Result<Self> {
        let names: Vec<&str> = header.trim_end_matches(['\r', '\n']).split('\t').collect();
        let find = |name: &str| {
            names
                .iter()
                .position(|n| *n == name)
                .with_context(|| format!("column `{name}` missing from the header"))
        };
        Ok(Self {
            code: find("code")?,
            product_name: find("product_name")?,
            brands: find("brands")?,
            main_category: find("main_category")?,
            energy_kcal: find("energy-kcal_100g")?,
            energy_kj: find("energy_100g")?,
            protein: find("proteins_100g")?,
            fat: find("fat_100g")?,
            saturated_fat: find("saturated-fat_100g")?,
            carbs: find("carbohydrates_100g")?,
            sugars: find("sugars_100g")?,
            fiber: find("fiber_100g")?,
            salt: find("salt_100g")?,
            serving_quantity: find("serving_quantity")?,
            serving_size: find("serving_size")?,
            last_modified: find("last_modified_t")?,
            count: names.len(),
        })
    }
}

/// Values per 100 g; all four core values are present or none are
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OffNutrients {
    pub energy_kcal: f64,
    pub protein_g: f64,
    pub fat_g: f64,
    pub carbs_g: f64,
    pub saturated_fat_g: Option<f64>,
    pub sugars_g: Option<f64>,
    pub fiber_g: Option<f64>,
    pub salt_g: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OffRow {
    pub barcode: String,
    pub name: String,
    pub brand: Option<String>,
    pub main_category: Option<String>,
    pub nutrients: Option<OffNutrients>,
    pub serving_g: Option<f64>,
    pub serving_name: Option<String>,
    pub modified_at: Option<DateTime<Utc>>,
}

/// Why a line was left out
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Skip {
    Malformed,
    Barcode,
    Name,
}

pub fn parse(columns: &Columns, line: &str) -> Result<OffRow, Skip> {
    let fields: Vec<&str> = line.trim_end_matches(['\r', '\n']).split('\t').collect();
    if fields.len() != columns.count {
        return Err(Skip::Malformed);
    }
    let text = |i: usize| fields.get(i).map(|f| f.trim()).filter(|f| !f.is_empty());
    let number = |i: usize| {
        text(i)
            .and_then(|f| f.parse::<f64>().ok())
            .filter(|v| v.is_finite())
    };

    let barcode = text(columns.code)
        .and_then(barcode::normalize)
        .ok_or(Skip::Barcode)?;
    let name = text(columns.product_name)
        .map(|n| capped(n, 120))
        .ok_or(Skip::Name)?;
    // OFF lists every brand, comma separated; the first is the one on the pack
    let brand = text(columns.brands)
        .and_then(|b| b.split(',').map(str::trim).find(|b| !b.is_empty()))
        .map(|b| capped(b, 80));
    let main_category = text(columns.main_category).map(|c| capped(c, 120));

    let energy_kcal =
        number(columns.energy_kcal).or_else(|| number(columns.energy_kj).map(|kj| kj / 4.184));
    let nutrients = match (
        energy_kcal,
        number(columns.protein),
        number(columns.fat),
        number(columns.carbs),
    ) {
        (Some(energy_kcal), Some(protein_g), Some(fat_g), Some(carbs_g)) => {
            let grams = |v: f64| (0.0..=100.0).contains(&v);
            let core_fits = (0.0..=900.0).contains(&energy_kcal)
                && [protein_g, fat_g, carbs_g].into_iter().all(grams)
                && protein_g + fat_g + carbs_g <= 100.0;
            // Values that contradict the label are dropped, the product itself stays useful
            core_fits.then(|| {
                let optional = |i: usize, max: f64| number(i).filter(|v| *v >= 0.0 && *v <= max);
                OffNutrients {
                    energy_kcal,
                    protein_g,
                    fat_g,
                    carbs_g,
                    saturated_fat_g: optional(columns.saturated_fat, fat_g),
                    sugars_g: optional(columns.sugars, carbs_g),
                    fiber_g: optional(columns.fiber, 100.0),
                    salt_g: optional(columns.salt, 100.0),
                }
            })
        }
        _ => None,
    };

    let serving_g = number(columns.serving_quantity).filter(|g| *g > 0.0 && *g <= 2000.0);
    let serving_name = serving_g
        .and(text(columns.serving_size))
        .map(|s| capped(s, 40));
    let modified_at = text(columns.last_modified)
        .and_then(|t| t.parse::<i64>().ok())
        .and_then(|t| DateTime::from_timestamp(t, 0));

    Ok(OffRow {
        barcode,
        name,
        brand,
        main_category,
        nutrients,
        serving_g,
        serving_name,
        modified_at,
    })
}

fn capped(value: &str, max_chars: usize) -> String {
    value
        .chars()
        .take(max_chars)
        .collect::<String>()
        .trim()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER: &str = "code\turl\tlast_modified_t\tproduct_name\tbrands\tmain_category\tserving_size\tserving_quantity\tenergy_100g\tenergy-kcal_100g\tfat_100g\tsaturated-fat_100g\tcarbohydrates_100g\tsugars_100g\tfiber_100g\tproteins_100g\tsalt_100g";

    fn line(fields: &[(&str, &str)]) -> String {
        let names: Vec<&str> = HEADER.split('\t').collect();
        names
            .iter()
            .map(|name| {
                fields
                    .iter()
                    .find(|(n, _)| n == name)
                    .map_or("", |(_, v)| *v)
            })
            .collect::<Vec<_>>()
            .join("\t")
    }

    fn oats() -> Vec<(&'static str, &'static str)> {
        vec![
            ("code", "5900259127761"),
            ("last_modified_t", "1700000000"),
            ("product_name", " Płatki owsiane "),
            ("brands", "Melvit,Other brand"),
            ("main_category", "en:oat-flakes"),
            ("serving_size", "40 g"),
            ("serving_quantity", "40"),
            ("energy-kcal_100g", "372"),
            ("fat_100g", "7"),
            ("saturated-fat_100g", "1.2"),
            ("carbohydrates_100g", "60"),
            ("sugars_100g", "1"),
            ("fiber_100g", "10"),
            ("proteins_100g", "13"),
            ("salt_100g", "0.01"),
        ]
    }

    fn columns() -> Columns {
        Columns::from_header(HEADER).expect("header")
    }

    fn with(changes: &[(&'static str, &'static str)]) -> String {
        let mut fields = oats();
        for (name, value) in changes {
            match fields.iter_mut().find(|(n, _)| n == name) {
                Some(field) => field.1 = value,
                None => fields.push((name, value)),
            }
        }
        line(&fields)
    }

    #[test]
    fn maps_a_complete_product() {
        let row = parse(&columns(), &line(&oats())).expect("row");
        assert_eq!(row.barcode, "5900259127761");
        assert_eq!(row.name, "Płatki owsiane");
        assert_eq!(row.brand.as_deref(), Some("Melvit"));
        assert_eq!(row.main_category.as_deref(), Some("en:oat-flakes"));
        assert_eq!(row.serving_g, Some(40.0));
        assert_eq!(row.serving_name.as_deref(), Some("40 g"));
        assert_eq!(row.modified_at.map(|t| t.timestamp()), Some(1_700_000_000));
        let n = row.nutrients.expect("nutrients");
        assert_eq!(
            (n.energy_kcal, n.protein_g, n.fat_g, n.carbs_g),
            (372.0, 13.0, 7.0, 60.0)
        );
        assert_eq!(
            (n.saturated_fat_g, n.sugars_g, n.fiber_g, n.salt_g),
            (Some(1.2), Some(1.0), Some(10.0), Some(0.01))
        );
    }

    #[test]
    fn falls_back_to_kilojoules() {
        let row = parse(
            &columns(),
            &with(&[("energy-kcal_100g", ""), ("energy_100g", "1556.448")]),
        )
        .expect("row");
        assert!((row.nutrients.expect("nutrients").energy_kcal - 372.0).abs() < 0.01);
    }

    #[test]
    fn keeps_barcode_and_name_without_nutrition() {
        let row = parse(&columns(), &with(&[("proteins_100g", "")])).expect("row");
        assert_eq!(row.nutrients, None);
        assert_eq!(row.name, "Płatki owsiane");
    }

    #[test]
    fn drops_nutrition_that_breaks_label_rules() {
        let impossible = parse(&columns(), &with(&[("proteins_100g", "300")])).expect("row");
        assert_eq!(impossible.nutrients, None);
        let over_100 = parse(
            &columns(),
            &with(&[("carbohydrates_100g", "90"), ("proteins_100g", "20")]),
        )
        .expect("row");
        assert_eq!(over_100.nutrients, None);
    }

    #[test]
    fn drops_only_the_optional_value_that_does_not_fit() {
        let row = parse(
            &columns(),
            &with(&[("sugars_100g", "80"), ("saturated-fat_100g", "-1")]),
        )
        .expect("row");
        let n = row.nutrients.expect("nutrients");
        assert_eq!((n.sugars_g, n.saturated_fat_g), (None, None));
        assert_eq!(n.fiber_g, Some(10.0));
    }

    #[test]
    fn normalizes_upc_barcodes_and_skips_bad_ones() {
        assert_eq!(
            parse(&columns(), &with(&[("code", "036000291452")]))
                .expect("row")
                .barcode,
            "0036000291452"
        );
        assert_eq!(
            parse(&columns(), &with(&[("code", "5900259127762")])),
            Err(Skip::Barcode)
        );
        assert_eq!(
            parse(&columns(), &with(&[("code", "abc")])),
            Err(Skip::Barcode)
        );
    }

    #[test]
    fn skips_rows_without_a_name_or_with_the_wrong_shape() {
        assert_eq!(
            parse(&columns(), &with(&[("product_name", "  ")])),
            Err(Skip::Name)
        );
        assert_eq!(
            parse(&columns(), "5900259127761\tonly two"),
            Err(Skip::Malformed)
        );
    }

    #[test]
    fn caps_long_text_and_ignores_silly_servings() {
        let long = "x".repeat(300);
        let long: &'static str = Box::leak(long.into_boxed_str());
        let row = parse(
            &columns(),
            &with(&[("product_name", long), ("serving_quantity", "99999")]),
        )
        .expect("row");
        assert_eq!(row.name.chars().count(), 120);
        assert_eq!((row.serving_g, row.serving_name), (None, None));
    }

    #[test]
    fn a_header_without_required_columns_is_an_error() {
        assert!(Columns::from_header("code\tproduct_name").is_err());
    }
}
