use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    error::{ApiResult, FieldErrors},
    exercises::model::{Muscle, Tracking},
    validate,
    workouts::model::{MAX_EXERCISES, MAX_SETS_PER_EXERCISE, SetKind, validate_set_values},
};

/// Planned values for one set; any measurement can be left open.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SetTarget {
    #[serde(default)]
    pub kind: SetKind,
    pub reps: Option<i32>,
    pub weight_kg: Option<f64>,
    pub duration_seconds: Option<i32>,
    pub distance_m: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub struct RoutineRequest {
    #[serde(default)]
    pub name: String,
    pub notes: Option<String>,
    #[serde(default)]
    pub exercises: Vec<RoutineExerciseRequest>,
}

#[derive(Debug, Deserialize)]
pub struct RoutineExerciseRequest {
    pub exercise_id: Uuid,
    pub rest_seconds: Option<i32>,
    pub notes: Option<String>,
    #[serde(default)]
    pub sets: Vec<SetTarget>,
}

/// A [`RoutineRequest`] that passed validation.
#[derive(Debug)]
pub struct RoutineDraft(RoutineRequest);

impl std::ops::Deref for RoutineDraft {
    type Target = RoutineRequest;

    fn deref(&self) -> &RoutineRequest {
        &self.0
    }
}

impl RoutineRequest {
    pub fn validate(mut self) -> ApiResult<RoutineDraft> {
        let mut errors = FieldErrors::default();
        self.name = validate::required_text(&mut errors, "name", &self.name, 1, 80);
        self.notes = validate::optional_text(&mut errors, "notes", self.notes.as_deref(), 2000);
        errors.ensure(
            self.exercises.len() <= MAX_EXERCISES,
            "exercises",
            format!("A routine can contain at most {MAX_EXERCISES} exercises"),
        );
        for (i, exercise) in self.exercises.iter_mut().enumerate() {
            let path = format!("exercises[{i}]");
            exercise.notes = validate::optional_text(
                &mut errors,
                &format!("{path}.notes"),
                exercise.notes.as_deref(),
                1000,
            );
            validate::in_range(
                &mut errors,
                &format!("{path}.rest_seconds"),
                exercise.rest_seconds,
                0,
                3600,
            );
            errors.ensure(
                exercise.sets.len() <= MAX_SETS_PER_EXERCISE,
                format!("{path}.sets"),
                format!("An exercise can contain at most {MAX_SETS_PER_EXERCISE} sets"),
            );
            for (j, set) in exercise.sets.iter().enumerate() {
                validate_set_values(
                    &mut errors,
                    &format!("{path}.sets[{j}]"),
                    set.reps,
                    set.weight_kg,
                    set.duration_seconds,
                    set.distance_m,
                );
            }
        }
        errors.into_result()?;
        Ok(RoutineDraft(self))
    }
}

#[derive(Debug, Serialize)]
pub struct Routine {
    pub id: Uuid,
    pub name: String,
    pub notes: Option<String>,
    pub last_performed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub exercises: Vec<RoutineExercise>,
}

#[derive(Debug, Serialize)]
pub struct RoutineExercise {
    pub id: Uuid,
    pub exercise_id: Uuid,
    pub exercise_name: String,
    pub tracking: Tracking,
    pub primary_muscle: Muscle,
    pub rest_seconds: Option<i32>,
    pub notes: Option<String>,
    pub sets: Vec<SetTarget>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_targets() {
        let request: RoutineRequest = serde_json::from_value(serde_json::json!({
            "name": "Upper A",
            "exercises": [{
                "exercise_id": "00000000-0000-0000-0000-00000000000a",
                "rest_seconds": 90,
                "sets": [{ "reps": 8, "weight_kg": 60 }, { "reps": -2 }]
            }]
        }))
        .expect("valid routine json");

        let Err(crate::error::ApiError::Validation { fields, .. }) = request.validate() else {
            panic!("expected validation error");
        };
        assert_eq!(
            fields.keys().collect::<Vec<_>>(),
            ["exercises[0].sets[1].reps"]
        );
    }
}
