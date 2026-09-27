//! Small normalisation helpers shared by request validation.

use crate::error::FieldErrors;

/// Trims `value` and checks its length (in characters) is within `min..=max`.
pub fn required_text(
    errors: &mut FieldErrors,
    field: &str,
    value: &str,
    min: usize,
    max: usize,
) -> String {
    let trimmed = value.trim();
    let len = trimmed.chars().count();
    if len == 0 {
        errors.add(field, format!("{} is required", label(field)));
    } else if len < min {
        errors.add(
            field,
            format!("{} must be at least {min} characters long", label(field)),
        );
    } else if len > max {
        errors.add(
            field,
            format!("{} must be at most {max} characters long", label(field)),
        );
    }
    trimmed.to_owned()
}

/// Trims an optional text value, mapping blank strings to `None`.
pub fn optional_text(
    errors: &mut FieldErrors,
    field: &str,
    value: Option<&str>,
    max: usize,
) -> Option<String> {
    let trimmed = value.map(str::trim).filter(|v| !v.is_empty())?;
    if trimmed.chars().count() > max {
        errors.add(
            field,
            format!("{} must be at most {max} characters long", label(field)),
        );
    }
    Some(trimmed.to_owned())
}

pub fn in_range<T: PartialOrd + Copy + std::fmt::Display>(
    errors: &mut FieldErrors,
    field: &str,
    value: Option<T>,
    min: T,
    max: T,
) {
    if let Some(v) = value {
        errors.ensure(
            v >= min && v <= max,
            field,
            format!("{} must be between {min} and {max}", label(field)),
        );
    }
}

/// `f64` counterpart of [`in_range`] that also rejects NaN and infinities.
pub fn finite_in_range(
    errors: &mut FieldErrors,
    field: &str,
    value: Option<f64>,
    min: f64,
    max: f64,
) {
    if let Some(v) = value {
        errors.ensure(
            v.is_finite() && v >= min && v <= max,
            field,
            format!("{} must be between {min} and {max}", label(field)),
        );
    }
}

/// Turns `weight_kg` or `exercises[0].sets[1].reps` into "Weight kg" / "Reps".
fn label(field: &str) -> String {
    let last = field.rsplit('.').next().unwrap_or(field);
    let words = last.replace('_', " ");
    let mut chars = words.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn required_text_trims_and_checks_bounds() {
        let mut errors = FieldErrors::default();
        assert_eq!(
            required_text(&mut errors, "name", "  Push day ", 1, 20),
            "Push day"
        );
        assert!(errors.is_empty());

        required_text(&mut errors, "name", "   ", 1, 20);
        required_text(&mut errors, "username", "ab", 3, 20);
        let Err(crate::error::ApiError::Validation { fields, .. }) = errors.into_result() else {
            panic!("expected validation error");
        };
        assert_eq!(fields["name"], "Name is required");
        assert_eq!(
            fields["username"],
            "Username must be at least 3 characters long"
        );
    }

    #[test]
    fn optional_text_maps_blank_to_none() {
        let mut errors = FieldErrors::default();
        assert_eq!(optional_text(&mut errors, "notes", Some("   "), 10), None);
        assert_eq!(
            optional_text(&mut errors, "notes", Some(" hi "), 10),
            Some("hi".into())
        );
        assert!(errors.is_empty());
    }

    #[test]
    fn finite_in_range_rejects_nan() {
        let mut errors = FieldErrors::default();
        finite_in_range(
            &mut errors,
            "exercises[0].sets[0].weight_kg",
            Some(f64::NAN),
            0.0,
            10.0,
        );
        let Err(crate::error::ApiError::Validation { message, .. }) = errors.into_result() else {
            panic!("expected validation error");
        };
        assert_eq!(message, "Weight kg must be between 0 and 10");
    }
}
