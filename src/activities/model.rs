use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    error::{ApiResult, FieldErrors},
    validate,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "text", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum ActivityKind {
    Run,
    Ride,
    Walk,
    Hike,
    Swim,
    Row,
    Other,
}

/// Cardio / endurance session such as a run or ride. Written whole with a client id.
#[derive(Debug, Deserialize)]
pub struct ActivityRequest {
    pub kind: ActivityKind,
    #[serde(default)]
    pub title: String,
    pub notes: Option<String>,
    pub started_at: DateTime<Utc>,
    pub duration_seconds: i32,
    pub distance_m: Option<f64>,
    pub elevation_gain_m: Option<f64>,
    pub avg_heart_rate: Option<i32>,
    pub calories: Option<i32>,
    /// Session RPE on a 1-10 scale.
    pub perceived_effort: Option<i32>,
}

/// An [`ActivityRequest`] that passed validation.
#[derive(Debug)]
pub struct ActivityDraft(ActivityRequest);

impl std::ops::Deref for ActivityDraft {
    type Target = ActivityRequest;

    fn deref(&self) -> &ActivityRequest {
        &self.0
    }
}

impl ActivityRequest {
    pub fn validate(mut self, now: DateTime<Utc>) -> ApiResult<ActivityDraft> {
        let mut errors = FieldErrors::default();
        self.title = validate::required_text(&mut errors, "title", &self.title, 1, 120);
        self.notes = validate::optional_text(&mut errors, "notes", self.notes.as_deref(), 2000);
        errors.ensure(
            self.started_at <= now + Duration::days(1),
            "started_at",
            "Activity cannot start in the future",
        );
        validate::in_range(
            &mut errors,
            "duration_seconds",
            Some(self.duration_seconds),
            1,
            7 * 86_400,
        );
        validate::finite_in_range(&mut errors, "distance_m", self.distance_m, 0.0, 2_000_000.0);
        validate::finite_in_range(
            &mut errors,
            "elevation_gain_m",
            self.elevation_gain_m,
            0.0,
            20_000.0,
        );
        validate::in_range(&mut errors, "avg_heart_rate", self.avg_heart_rate, 30, 250);
        validate::in_range(&mut errors, "calories", self.calories, 0, 50_000);
        validate::in_range(
            &mut errors,
            "perceived_effort",
            self.perceived_effort,
            1,
            10,
        );
        errors.into_result()?;
        Ok(ActivityDraft(self))
    }
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Activity {
    pub id: Uuid,
    pub kind: ActivityKind,
    pub title: String,
    pub notes: Option<String>,
    pub started_at: DateTime<Utc>,
    pub duration_seconds: i32,
    pub distance_m: Option<f64>,
    pub elevation_gain_m: Option<f64>,
    pub avg_heart_rate: Option<i32>,
    pub calories: Option<i32>,
    pub perceived_effort: Option<i32>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(overrides: serde_json::Value) -> ActivityRequest {
        let mut base = serde_json::json!({
            "kind": "run",
            "title": "Morning run",
            "started_at": "2026-09-27T06:30:00Z",
            "duration_seconds": 1800,
            "distance_m": 5000.0
        });
        if let (Some(base), Some(extra)) = (base.as_object_mut(), overrides.as_object()) {
            base.extend(extra.clone());
        }
        serde_json::from_value(base).expect("valid activity json")
    }

    #[test]
    fn accepts_valid_activity() {
        assert!(request(serde_json::json!({})).validate(Utc::now()).is_ok());
    }

    #[test]
    fn rejects_zero_duration_and_bad_heart_rate() {
        let result = request(serde_json::json!({ "duration_seconds": 0, "avg_heart_rate": 400 }))
            .validate(Utc::now());
        let Err(crate::error::ApiError::Validation { fields, .. }) = result else {
            panic!("expected validation error");
        };
        assert_eq!(
            fields.keys().collect::<Vec<_>>(),
            ["avg_heart_rate", "duration_seconds"]
        );
    }
}
