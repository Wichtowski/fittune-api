use super::model::{Column, Extraction, Observation, ParseRequest};

fn normalized(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|c| match c {
            'ą' => 'a',
            'ć' => 'c',
            'ę' => 'e',
            'ł' => 'l',
            'ń' => 'n',
            'ó' => 'o',
            'ś' => 's',
            'ź' | 'ż' => 'z',
            _ => c,
        })
        .collect()
}

fn label(text: &str) -> Option<&'static str> {
    let s = normalized(text);
    for (field, aliases) in [
        (
            "saturated_fat_g",
            &["nasycone", "saturates", "saturated"][..],
        ),
        ("sugars_g", &["cukry", "cukrow", "sugars"][..]),
        (
            "energy_kcal",
            &["energia", "energy", "energetyczna", "calories"][..],
        ),
        ("fat_g", &["tluszcz", "fat"][..]),
        (
            "carbs_g",
            &["weglowodany", "carbohydrate", "carbohydrates"][..],
        ),
        ("fiber_g", &["blonnik", "fibre", "fiber"][..]),
        ("protein_g", &["bialko", "protein"][..]),
        ("salt_g", &["sol", "salt"][..]),
    ] {
        if s.split(|c: char| !c.is_alphabetic()).any(|word| {
            aliases
                .iter()
                .any(|a| word == *a || (a.len() >= 5 && distance(word, a) <= 1))
        }) {
            return Some(field);
        }
    }
    None
}

fn distance(a: &str, b: &str) -> usize {
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.bytes().enumerate() {
        let mut previous = row[0];
        row[0] = i + 1;
        for (j, cb) in b.bytes().enumerate() {
            let old = row[j + 1];
            row[j + 1] = (row[j + 1] + 1)
                .min(row[j] + 1)
                .min(previous + usize::from(ca != cb));
            previous = old;
        }
    }
    row[b.len()]
}

fn number(text: &str) -> Option<(f64, bool)> {
    let s = text
        .trim()
        .trim_start_matches(['<', '≤', '('])
        .trim()
        .replace(',', ".");
    let token: String = s
        .chars()
        .take_while(|c| c.is_ascii_digit() || ".-OlISB".contains(*c))
        .collect();
    if !token.chars().any(|c| c.is_ascii_digit()) {
        return None;
    }
    let fixed: String = token
        .chars()
        .map(|c| match c {
            'O' => '0',
            'l' | 'I' => '1',
            'S' => '5',
            'B' => '8',
            _ => c,
        })
        .collect();
    let value = fixed.parse::<f64>().ok()?;
    value.is_finite().then_some((value, fixed != token))
}

fn split_observations(input: &[Observation]) -> Vec<Observation> {
    input
        .iter()
        .flat_map(|item| {
            let parts: Vec<_> = item
                .text
                .split(|c: char| c == '/' || c.is_whitespace())
                .filter(|part| !part.is_empty())
                .collect();
            if parts.len() < 2 {
                return vec![item.clone()];
            }
            let width = (item.bbox[2] - item.bbox[0]) / item.text.chars().count().max(1) as f64;
            let mut x = item.bbox[0];
            parts
                .into_iter()
                .map(|part| {
                    let right = (x + width * part.chars().count() as f64).min(item.bbox[2]);
                    let observation = Observation {
                        text: part.into(),
                        confidence: item.confidence.min(0.79),
                        bbox: [x, item.bbox[1], right, item.bbox[3]],
                    };
                    x = (right + width).min(item.bbox[2]);
                    observation
                })
                .collect()
        })
        .collect()
}

fn rows(observations: &[Observation]) -> Vec<Vec<&Observation>> {
    let mut sorted: Vec<_> = observations.iter().collect();
    sorted.sort_by(|a, b| a.bbox[1].total_cmp(&b.bbox[1]));
    let mut rows: Vec<Vec<&Observation>> = Vec::new();
    for item in sorted {
        if let Some(row) = rows.iter_mut().find(|r| {
            let other = r[0];
            ((item.bbox[1] + item.bbox[3] - other.bbox[1] - other.bbox[3]) / 2.0).abs()
                < 0.35 * (item.bbox[3] - item.bbox[1]).min(other.bbox[3] - other.bbox[1])
        }) {
            row.push(item);
        } else {
            rows.push(vec![item]);
        }
    }
    for row in &mut rows {
        row.sort_by(|a, b| a.bbox[0].total_cmp(&b.bbox[0]));
    }
    rows
}

pub fn parse(input: &ParseRequest) -> Extraction {
    let observations = split_observations(&input.observations);
    let rows = rows(&observations);
    let mut out = Extraction::blank("ocr");
    // Header detection is independent of row grouping because slanted tables overlap rows
    let first_nutrient = observations
        .iter()
        .filter(|o| label(&o.text).is_some())
        .map(|o| o.bbox[1])
        .min_by(f64::total_cmp)
        .unwrap_or(input.height as f64);
    for item in &observations {
        if item.bbox[1] >= first_nutrient {
            continue;
        }
        let Some((amount, _)) = number(&item.text) else {
            continue;
        };
        if item.text.contains('%') || amount <= 0.0 || amount > 2000.0 {
            continue;
        }
        let height = item.bbox[3] - item.bbox[1];
        let context = observations
            .iter()
            .filter(|o| {
                (o.bbox[1] - item.bbox[1]).abs() < height * 2.0 && o.bbox[1] < first_nutrient
            })
            .map(|o| normalized(&o.text))
            .collect::<Vec<_>>()
            .join(" ");
        if amount != 100.0
            && !["porcj", "portion", "serving"]
                .iter()
                .any(|w| context.contains(w))
        {
            continue;
        }
        let attached = normalized(&item.text)
            .trim_end_matches(|c: char| !c.is_alphanumeric())
            .to_string();
        if attached.ends_with("mg") || attached.ends_with("kg") {
            continue;
        }
        let next = observations
            .iter()
            .filter(|o| {
                o.bbox[0] >= item.bbox[2] - height * 0.4
                    && o.bbox[0] - item.bbox[2] < height
                    && (o.bbox[1] - item.bbox[1]).abs() < height * 0.6
            })
            .min_by(|a, b| a.bbox[0].total_cmp(&b.bbox[0]));
        let next = next
            .map(|o| {
                normalized(&o.text)
                    .trim_matches(|c: char| !c.is_alphabetic())
                    .to_string()
            })
            .unwrap_or_default();
        let unit = if attached.ends_with("ml") || next == "ml" {
            "ml"
        } else if attached.ends_with('g') || next == "g" {
            "g"
        } else {
            continue;
        };
        let x = (item.bbox[0] + item.bbox[2]) / 2.0;
        if !out
            .columns
            .iter()
            .any(|c| c.unit == unit && c.amount == amount && (c.x - x).abs() < height)
        {
            out.columns.push(Column {
                label: format!("{amount} {unit}"),
                unit: unit.into(),
                amount,
                x,
            });
        }
    }
    if out.columns.len() > 1
        && out
            .columns
            .iter()
            .all(|c| c.unit == out.columns[0].unit && c.amount == out.columns[0].amount)
    {
        let header = observations
            .iter()
            .map(|o| normalized(&o.text))
            .collect::<Vec<_>>()
            .join(" ");
        if !["prepared", "dry mix", "ugotowan", "przygotowan"]
            .iter()
            .any(|s| header.contains(s))
        {
            let x = out
                .columns
                .iter()
                .map(|c| c.x)
                .max_by(f64::total_cmp)
                .unwrap_or(0.0);
            out.columns.truncate(1);
            out.columns[0].x = x;
        }
    }
    out.columns.sort_by(|a, b| a.x.total_cmp(&b.x));
    out.selected_column = input.column.filter(|i| *i < out.columns.len()).or_else(|| {
        let defaults: Vec<_> = out
            .columns
            .iter()
            .enumerate()
            .filter(|(_, c)| c.amount == 100.0)
            .map(|(i, _)| i)
            .collect();
        if defaults.len() == 1 {
            Some(defaults[0])
        } else if out.columns.len() == 1 {
            Some(0)
        } else {
            None
        }
    });
    let Some(selected) = out.selected_column else {
        out.warn(
            "basis",
            "Select the labelled column, or recrop to include its per-100/portion header",
        );
        out.check();
        return out;
    };
    let column = &out.columns[selected];
    let (x, factor) = (column.x, 100.0 / column.amount);
    out.unit = Some(column.unit.clone());
    let mut previous_label = String::new();
    let mut ambiguous = std::collections::HashSet::new();
    let mut energy_is_kcal = false;
    let mut energy_from_kj: Option<f64> = None;
    let mut previous_bottom = 0.0;
    for row in &rows {
        let text = row
            .iter()
            .map(|o| o.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let clean = normalized(&text);
        if clean.contains("trans") || clean.contains("added") || clean.contains("dodane") {
            previous_label.clear();
            continue;
        }
        let own_label = label(&text);
        let numeric_only = row.iter().all(|o| {
            number(&o.text).is_some()
                || matches!(
                    normalized(&o.text).trim_matches(|c: char| !c.is_alphabetic()),
                    "g" | "mg" | "kj" | "kcal" | ""
                )
        });
        let height = row[0].bbox[3] - row[0].bbox[1];
        let field = own_label.or_else(|| {
            if numeric_only && row[0].bbox[1] - previous_bottom < height {
                label(&previous_label)
            } else {
                None
            }
        });
        if own_label.is_some() {
            previous_label = text.clone();
        }
        previous_bottom = row
            .iter()
            .map(|o| o.bbox[3])
            .max_by(f64::total_cmp)
            .unwrap_or(0.0);
        let fields: std::collections::HashSet<_> = row
            .iter()
            .filter_map(|o| label(&o.text))
            .filter(|field| !(*field == "fat_g" && own_label == Some("saturated_fat_g")))
            .collect();
        if fields.len() > 1 {
            for field in fields {
                out.warn(
                    field,
                    "Several nutrient labels share an OCR row; recrop or try AI",
                );
            }
            previous_label.clear();
            continue;
        }
        let numerics: Vec<_> = row
            .iter()
            .filter(|o| number(&o.text).is_some() && !o.text.contains('%'))
            .copied()
            .collect();
        if numerics.is_empty() {
            if own_label.is_some() {
                previous_label = text;
            } else if !numeric_only {
                previous_label.clear();
            }
            continue;
        }
        let Some(field) = field else {
            continue;
        };
        let belongs = |o: &Observation| {
            let center = (o.bbox[0] + o.bbox[2]) / 2.0;
            !out.columns
                .iter()
                .enumerate()
                .any(|(i, c)| i != selected && (center - c.x).abs() < (center - x).abs())
        };
        let energy_unit = |o: &Observation| {
            let words = normalized(
                &row.iter()
                    .filter(|unit| unit.bbox[0] >= o.bbox[0] && unit.bbox[0] <= o.bbox[2] + 30.0)
                    .map(|unit| unit.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" "),
            );
            if words.contains("kcal") {
                2
            } else if words.contains("kj") {
                1
            } else {
                0
            }
        };
        if field == "energy_kcal" {
            for numeric in numerics
                .iter()
                .filter(|o| belongs(o) && energy_unit(o) == 1)
            {
                if let Some((value, _)) = number(&numeric.text) {
                    energy_from_kj = Some(value * factor / 4.184);
                }
            }
        }
        let item = numerics
            .iter()
            .filter(|o| belongs(o))
            .max_by_key(|o| {
                if field == "energy_kcal" {
                    energy_unit(o)
                } else {
                    0
                }
            })
            .filter(|o| field == "energy_kcal" && energy_unit(o) > 0)
            .or_else(|| {
                numerics.iter().filter(|o| belongs(o)).min_by(|a, b| {
                    let center = |o: &Observation| (o.bbox[0] + o.bbox[2]) / 2.0;
                    (center(a) - x).abs().total_cmp(&(center(b) - x).abs())
                })
            });
        let Some(item) = item else {
            continue;
        };
        let center = (item.bbox[0] + item.bbox[2]) / 2.0;
        if out
            .columns
            .iter()
            .enumerate()
            .any(|(i, c)| i != selected && (center - c.x).abs() < (center - x).abs())
        {
            continue;
        }
        let Some((mut value, corrected)) = number(&item.text) else {
            continue;
        };
        out.evidence.insert(field.into(), text);
        if item.text.contains('<')
            || item.text.contains('≤')
            || row.iter().any(|o| {
                o.bbox[2] <= item.bbox[0]
                    && item.bbox[0] - o.bbox[2] < 20.0
                    && o.text.contains(['<', '≤'])
            })
        {
            out.warn(
                field,
                "Label gives an upper bound; enter an approximation explicitly",
            );
            continue;
        }
        let units = row
            .iter()
            .filter(|o| o.bbox[0] >= item.bbox[0] && o.bbox[0] - item.bbox[2] < 30.0)
            .map(|o| o.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let units = normalized(&format!("{} {units}", item.text));
        if field == "energy_kcal"
            && !units.contains("kcal")
            && !normalized(&out.evidence[field]).contains("calories")
        {
            if units.contains("kj") {
                value /= 4.184;
                out.warn(field, "Converted from kJ; check the label");
            } else {
                out.warn(
                    field,
                    "Energy unit is missing; include kcal or kJ in the crop",
                );
                continue;
            }
        } else if field != "energy_kcal" && units.contains("mg") {
            value /= 1000.0;
        }
        let kcal = field == "energy_kcal"
            && (units.contains("kcal") || normalized(&out.evidence[field]).contains("calories"));
        if field == "energy_kcal" && energy_is_kcal && !kcal {
            continue;
        }
        if ambiguous.contains(field) {
            continue;
        }
        if out.values[field].is_some() && !(field == "energy_kcal" && kcal && !energy_is_kcal) {
            if out.values[field].is_some_and(|prior| (prior - value * factor).abs() < 0.01) {
                continue;
            }
            ambiguous.insert(field);
            out.values.insert(field.into(), None);
            out.warn(field, "Multiple readings disagree; enter the label value");
            continue;
        }
        out.values.insert(field.into(), Some(value * factor));
        if kcal {
            if energy_from_kj
                .is_some_and(|kj| (kj - value * factor).abs() > (value * factor * 0.15).max(5.0))
            {
                out.warn(field, "Printed kJ and kcal disagree; check the label");
            }
            energy_is_kcal = true;
        }
        if corrected || item.confidence < 0.8 {
            out.warn(field, "Uncertain OCR reading; check the label");
        }
        if factor != 1.0 {
            out.warn(
                field,
                "Converted from a portion; confirm the portion size and unit",
            );
        }
    }
    out.check();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    fn obs(text: &str, x: f64, y: f64) -> Observation {
        Observation {
            text: text.into(),
            confidence: 0.95,
            bbox: [x, y, x + 60.0, y + 20.0],
        }
    }
    #[test]
    fn selects_header_not_first_numeric_column_and_preserves_bounds() {
        let input = ParseRequest {
            width: 600,
            height: 400,
            column: None,
            observations: vec![
                obs("per portion 30 g", 200.0, 0.0),
                obs("per 100 g", 400.0, 0.0),
                obs("Tłuszcz", 0.0, 50.0),
                obs("3,0 g", 200.0, 50.0),
                obs("10,0 g", 400.0, 50.0),
                obs("cukry", 0.0, 100.0),
                obs("<0,5 g", 400.0, 100.0),
            ],
        };
        input.validate().expect("valid observations");
        let result = parse(&input);
        assert_eq!(result.values["fat_g"], Some(10.0));
        assert_eq!(result.values["sugars_g"], None);
        assert!(result.warnings.contains_key("sugars_g"));
        assert_eq!(result.unit.as_deref(), Some("g"));
    }
    #[test]
    fn portion_calories_do_not_read_trans_fat_added_sugars_or_unrelated_rows() {
        let input = ParseRequest {
            width: 600,
            height: 500,
            column: None,
            observations: vec![
                obs("Serving size (30g)", 300.0, 0.0),
                obs("Calories", 0.0, 40.0),
                obs("90", 300.0, 40.0),
                obs("Total Fat", 0.0, 80.0),
                obs("3g", 300.0, 80.0),
                obs("Trans Fat", 0.0, 120.0),
                obs("0g", 300.0, 120.0),
                obs("Sodium", 0.0, 160.0),
                obs("300mg", 300.0, 160.0),
                obs("Sugars", 0.0, 200.0),
                obs("6g", 300.0, 200.0),
                obs("Added Sugars", 0.0, 240.0),
                obs("0g", 300.0, 240.0),
                obs("Protein", 0.0, 280.0),
                obs("2g", 300.0, 280.0),
                obs("Calcium", 0.0, 320.0),
                obs("5mg", 300.0, 320.0),
            ],
        };
        let result = parse(&input);
        assert_eq!(result.values["energy_kcal"], Some(300.0));
        assert_eq!(result.values["fat_g"], Some(10.0));
        assert_eq!(result.values["sugars_g"], Some(20.0));
        assert_eq!(result.values["protein_g"], Some(20.0 / 3.0));
        assert_eq!(result.values["salt_g"], None);
    }

    #[test]
    fn flags_inconsistent_printed_kj_and_kcal() {
        let result = parse(&ParseRequest {
            width: 600,
            height: 200,
            column: None,
            observations: vec![
                obs("per 100 g", 400.0, 0.0),
                obs("Energy", 0.0, 40.0),
                obs("1000 kJ", 300.0, 40.0),
                obs("90 kcal", 400.0, 40.0),
            ],
        });
        assert_eq!(result.values["energy_kcal"], Some(90.0));
        assert!(
            result.warnings["energy_kcal"]
                .iter()
                .any(|s| s.contains("kJ and kcal disagree"))
        );
    }

    #[test]
    fn reads_real_polish_and_english_label_observations() {
        let cases = [
            (
                "pl-09",
                include_str!("../../../tests/label-ocr/observations/pl-09.json"),
            ),
            (
                "en-01",
                include_str!("../../../tests/label-ocr/observations/en-01.json"),
            ),
        ];
        let manifest: serde_json::Value =
            serde_json::from_str(include_str!("../../../tests/label-ocr/manifest.json"))
                .expect("valid fixture manifest");
        for (id, raw) in cases {
            let input: ParseRequest =
                serde_json::from_str(raw).expect("valid fixture observations");
            input.validate().expect("bounded fixture observations");
            let result = parse(&input);
            let expected = &manifest["entries"]
                .as_array()
                .expect("manifest entries")
                .iter()
                .find(|entry| entry["id"] == id)
                .expect("fixture ID in manifest")["expected"];
            assert_eq!(result.unit.as_deref(), expected["unit"].as_str(), "{id}");
            for (field, truth) in expected["values"]
                .as_object()
                .expect("expected nutrient values")
            {
                let actual = result.values[field];
                match truth.as_f64() {
                    Some(value) => assert!(
                        actual.is_some_and(|actual| (value - actual).abs()
                            <= if field == "energy_kcal" {
                                1.0
                            } else {
                                0.01_f64.max(value * 0.02)
                            }),
                        "{id} {field}: {actual:?} != {value}"
                    ),
                    None => assert_eq!(actual, None, "{id} {field}"),
                }
            }
        }
    }

    #[test]
    fn refuses_headerless_values_and_ambiguous_bases() {
        let mut input = ParseRequest {
            width: 600,
            height: 400,
            column: None,
            observations: vec![obs("Protein", 0.0, 50.0), obs("8 g", 400.0, 50.0)],
        };
        assert_eq!(parse(&input).values["protein_g"], None);
        input
            .observations
            .extend([obs("per 100 g", 200.0, 0.0), obs("per 100 ml", 400.0, 0.0)]);
        assert!(parse(&input).selected_column.is_none());
        input.column = Some(1);
        assert_eq!(parse(&input).values["protein_g"], Some(8.0));
    }
}
