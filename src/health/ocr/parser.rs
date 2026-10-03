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
                let mut observation = item.clone();
                if let Some(part) = parts.first().filter(|part| {
                    item.text.trim_start().starts_with('/')
                        && number(part).is_some_and(|(amount, _)| amount == 100.0)
                }) {
                    observation.text = (*part).into();
                }
                return vec![observation];
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

fn text_slope(observations: &[Observation]) -> f64 {
    // Estimate text tilt from neighbouring words, without changing the captured image
    let mut slopes = Vec::new();
    for a in observations {
        let height = a.bbox[3] - a.bbox[1];
        for b in observations {
            let dx = (b.bbox[0] + b.bbox[2] - a.bbox[0] - a.bbox[2]) / 2.0;
            let dy = (b.bbox[1] + b.bbox[3] - a.bbox[1] - a.bbox[3]) / 2.0;
            let ratio = (b.bbox[3] - b.bbox[1]) / height;
            if b.bbox[0] >= a.bbox[2]
                && b.bbox[0] - a.bbox[2] < height * 2.0
                && dx > height * 1.5
                && (0.7..=1.4).contains(&ratio)
                && (dy / dx).abs() <= 0.15
            {
                slopes.push(dy / dx);
            }
        }
    }
    slopes.sort_by(f64::total_cmp);
    let slope = slopes.get(slopes.len() / 2).copied().unwrap_or(0.0);
    let mut deviations: Vec<_> = slopes.iter().map(|value| (value - slope).abs()).collect();
    deviations.sort_by(f64::total_cmp);
    let deviation = deviations.get(deviations.len() / 2).copied().unwrap_or(0.0);
    if slope.abs() > deviation * 2.0 {
        slope
    } else {
        0.0
    }
}

fn rows(observations: &[Observation], slope: f64) -> Vec<Vec<&Observation>> {
    let center = |o: &Observation| (o.bbox[1] + o.bbox[3] - slope * (o.bbox[0] + o.bbox[2])) / 2.0;
    let mut sorted: Vec<_> = observations.iter().collect();
    // Establish label anchors before assigning values that can overlap adjacent text rows
    let priority = |o: &Observation| {
        if label(&o.text).is_some() {
            0
        } else if number(&o.text).is_some() {
            2
        } else {
            1
        }
    };
    sorted.sort_by(|a, b| {
        priority(a)
            .cmp(&priority(b))
            .then_with(|| center(a).total_cmp(&center(b)))
    });
    let mut rows: Vec<Vec<&Observation>> = Vec::new();
    for item in sorted {
        if let Some(row) = rows
            .iter_mut()
            .filter(|r| {
                (center(item) - center(r[0])).abs()
                    < 0.25 * (item.bbox[3] - item.bbox[1] + r[0].bbox[3] - r[0].bbox[1])
            })
            .min_by(|a, b| {
                (center(item) - center(a[0]))
                    .abs()
                    .total_cmp(&(center(item) - center(b[0])).abs())
            })
        {
            row.push(item);
        } else {
            rows.push(vec![item]);
        }
    }
    rows.sort_by(|a, b| center(a[0]).total_cmp(&center(b[0])));
    for row in &mut rows {
        row.sort_by(|a, b| a.bbox[0].total_cmp(&b.bbox[0]));
    }
    rows
}

pub fn parse(input: &ParseRequest) -> Extraction {
    let observations = split_observations(&input.observations);
    let rows = rows(&observations, text_slope(&input.observations));
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
        let header_text = normalized(&item.text);
        let Some((amount, _)) = number(header_text.strip_prefix("per").unwrap_or(&item.text))
        else {
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
                o.bbox[1] < first_nutrient
                    && matches!(
                        normalized(&o.text).trim_matches(|c: char| !c.is_alphanumeric()),
                        "g" | "ml"
                    )
                    && ((o.bbox[0] >= item.bbox[2] - height * 0.4
                        && o.bbox[0] - item.bbox[2] < height
                        && (o.bbox[1] - item.bbox[1]).abs() < height * 0.6)
                        || (o.bbox[0] < item.bbox[2]
                            && o.bbox[2] > item.bbox[0]
                            && o.bbox[1] >= item.bbox[3]
                            && o.bbox[1] - item.bbox[3] < height * 0.6))
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
        let calorie_words = normalized(&previous_label);
        let calories_label = calorie_words
            .split(|c: char| !c.is_alphabetic())
            .any(|word| distance(word, "calories") <= 1);
        let belongs = |o: &Observation| {
            let center = (o.bbox[0] + o.bbox[2]) / 2.0;
            let after_label = own_label.is_none()
                || row
                    .iter()
                    .filter(|label_word| label(&label_word.text) == Some(field))
                    .all(|label_word| o.bbox[0] >= label_word.bbox[2] - height * 0.4);
            after_label
                && !out
                    .columns
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
            .max_by(|a, b| {
                energy_unit(a).cmp(&energy_unit(b)).then_with(|| {
                    let center = |o: &Observation| (o.bbox[0] + o.bbox[2]) / 2.0;
                    (center(b) - x).abs().total_cmp(&(center(a) - x).abs())
                })
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
        // Consume a numeric continuation once; energy can continue for its kcal reading
        if own_label.is_none() && field != "energy_kcal" {
            previous_label.clear();
        }
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
        if field == "energy_kcal" && !units.contains("kcal") && !calories_label {
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
        let kcal = field == "energy_kcal" && (units.contains("kcal") || calories_label);
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
        if corrected
            || item.confidence < 0.8
            || (calories_label
                && !calorie_words
                    .split(|c: char| !c.is_alphabetic())
                    .any(|word| word == "calories"))
        {
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
    fn recognizes_slash_attached_and_wrapped_basis_headers() {
        for header in [
            vec![obs("/100", 400.0, 0.0), obs("g", 465.0, 0.0)],
            vec![obs("100g", 400.0, 0.0)],
            vec![obs("per100", 400.0, 0.0), obs("g", 465.0, 0.0)],
            vec![obs("100", 400.0, 0.0), obs("ml", 400.0, 22.0)],
        ] {
            let mut observations = header;
            observations.extend([obs("Protein", 0.0, 80.0), obs("8g", 400.0, 80.0)]);
            let result = parse(&ParseRequest {
                width: 600,
                height: 200,
                column: None,
                observations,
            });
            assert_eq!(result.values["protein_g"], Some(8.0));
        }
    }

    #[test]
    fn energy_uses_nearest_basis_when_portion_header_is_unreadable() {
        let result = parse(&ParseRequest {
            width: 800,
            height: 200,
            column: None,
            observations: vec![
                obs("100g", 400.0, 0.0),
                obs("Energy", 0.0, 60.0),
                obs("463kcal", 400.0, 60.0),
                obs("93kcal", 600.0, 60.0),
            ],
        });
        assert_eq!(result.values["energy_kcal"], Some(463.0));
    }

    #[test]
    fn unlabelled_later_numeric_rows_do_not_inherit_protein() {
        let result = parse(&ParseRequest {
            width: 600,
            height: 200,
            column: None,
            observations: vec![
                obs("100g", 400.0, 0.0),
                obs("Protein", 0.0, 60.0),
                obs("1g", 400.0, 80.0),
                obs("0.02g", 400.0, 105.0),
            ],
        });
        assert_eq!(result.values["protein_g"], Some(1.0));
    }

    #[test]
    fn aligns_slanted_rows_without_crossing_nutrients_or_portion_columns() {
        let result = parse(&ParseRequest {
            width: 800,
            height: 300,
            column: None,
            observations: vec![
                obs("100g", 400.0, 0.0),
                obs("Total", 0.0, 60.0),
                obs("Carbohydrate", 80.0, 64.0),
                obs("47g", 400.0, 80.0),
                obs("23g", 600.0, 90.0),
                obs("Total", 0.0, 90.0),
                obs("Sugars", 80.0, 94.0),
                obs("4.2g", 400.0, 110.0),
                obs("2.1g", 600.0, 120.0),
            ],
        });
        assert_eq!(result.values["carbs_g"], Some(47.0));
        assert_eq!(result.values["sugars_g"], Some(4.2));
    }

    #[test]
    fn serving_fraction_is_not_a_wrapped_mass_header() {
        let result = parse(&ParseRequest {
            width: 800,
            height: 300,
            column: None,
            observations: vec![
                obs("Serving size", 0.0, 0.0),
                obs("1/4", 400.0, 0.0),
                obs("(28g)", 400.0, 22.0),
                obs("Protein", 0.0, 80.0),
                obs("7g", 400.0, 80.0),
            ],
        });
        assert_eq!(result.columns.len(), 1);
        assert_eq!(result.values["protein_g"], Some(25.0));
    }

    #[test]
    fn fuzzy_calories_label_supplies_kcal_but_warns_and_energy_requires_a_unit() {
        for (text, expected) in [("Galories", Some(300.0)), ("Energy", None)] {
            let result = parse(&ParseRequest {
                width: 600,
                height: 200,
                column: None,
                observations: vec![
                    obs("100g", 400.0, 0.0),
                    obs(text, 0.0, 60.0),
                    obs("300", 400.0, 80.0),
                ],
            });
            assert_eq!(result.values["energy_kcal"], expected);
            assert!(result.warnings.contains_key("energy_kcal"));
        }
    }

    #[test]
    fn recovers_slash_header_and_overlapping_rows_from_real_polish_observations() {
        let input: ParseRequest = serde_json::from_str(include_str!(
            "../../../tests/label-ocr/observations/pl-01.json"
        ))
        .expect("valid fixture observations");
        input.validate().expect("bounded fixture observations");
        let result = parse(&input);
        for (field, value) in [
            ("energy_kcal", 232.0),
            ("fat_g", 19.2),
            ("sugars_g", 1.2),
            ("fiber_g", 4.0),
            ("protein_g", 3.8),
            ("salt_g", 1.5),
        ] {
            assert_eq!(result.values[field], Some(value), "{field}");
        }
        // These values were not recognized, so neighbouring readings must not fill them
        assert_eq!(result.values["carbs_g"], None);
        assert_eq!(result.values["saturated_fat_g"], None);
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
