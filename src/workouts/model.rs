use std::collections::HashSet;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    error::{ApiResult, FieldErrors},
    exercises::model::{Muscle, Tracking},
    validate,
};

pub const MAX_EXERCISES: usize = 60;
pub const MAX_SETS_PER_EXERCISE: usize = 60;
pub const MAX_DURATION_HOURS: i64 = 24;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "text", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum SetKind {
    Warmup,
    #[default]
    Normal,
    Drop,
    Failure,
}

/// Full workout document sent by clients. Workouts are always written whole, which makes
/// every write idempotent and lets an offline client simply replay its latest snapshot.
#[derive(Debug, Deserialize)]
pub struct WorkoutRequest {
    #[serde(default)]
    pub title: String,
    pub notes: Option<String>,
    pub routine_id: Option<Uuid>,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    /// Client-side edit counter; writes older than the stored revision are rejected.
    pub revision: i64,
    #[serde(default)]
    pub exercises: Vec<WorkoutExerciseRequest>,
}

#[derive(Debug, Deserialize)]
pub struct WorkoutExerciseRequest {
    pub id: Uuid,
    pub exercise_id: Uuid,
    pub notes: Option<String>,
    pub rest_seconds: Option<i32>,
    #[serde(default)]
    pub sets: Vec<SetRequest>,
}

#[derive(Debug, Deserialize)]
pub struct SetRequest {
    pub id: Uuid,
    #[serde(default)]
    pub kind: SetKind,
    pub reps: Option<i32>,
    pub weight_kg: Option<f64>,
    pub duration_seconds: Option<i32>,
    pub distance_m: Option<f64>,
    pub rpe: Option<f64>,
    #[serde(default)]
    pub completed: bool,
}

/// A [`WorkoutRequest`] that passed validation, with text fields normalised.
#[derive(Debug)]
pub struct WorkoutDraft(WorkoutRequest);

impl std::ops::Deref for WorkoutDraft {
    type Target = WorkoutRequest;

    fn deref(&self) -> &WorkoutRequest {
        &self.0
    }
}

impl WorkoutRequest {
    pub fn validate(mut self, now: DateTime<Utc>) -> ApiResult<WorkoutDraft> {
        let mut errors = FieldErrors::default();

        self.title = validate::required_text(&mut errors, "title", &self.title, 1, 120);
        self.notes = validate::optional_text(&mut errors, "notes", self.notes.as_deref(), 2000);
        errors.ensure(
            self.revision >= 0,
            "revision",
            "Revision must not be negative",
        );
        errors.ensure(
            self.started_at <= now + Duration::days(1),
            "started_at",
            "Workout cannot start in the future",
        );
        if let Some(ended_at) = self.ended_at {
            errors.ensure(
                ended_at >= self.started_at,
                "ended_at",
                "Workout cannot end before it starts",
            );
            errors.ensure(
                ended_at - self.started_at <= Duration::hours(MAX_DURATION_HOURS),
                "ended_at",
                "Workout cannot last longer than 24 hours",
            );
        }
        errors.ensure(
            self.exercises.len() <= MAX_EXERCISES,
            "exercises",
            format!("A workout can contain at most {MAX_EXERCISES} exercises"),
        );

        let mut ids = HashSet::new();
        for (i, exercise) in self.exercises.iter_mut().enumerate() {
            let path = format!("exercises[{i}]");
            errors.ensure(
                ids.insert(exercise.id),
                format!("{path}.id"),
                "Ids must be unique",
            );
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
                let path = format!("{path}.sets[{j}]");
                errors.ensure(
                    ids.insert(set.id),
                    format!("{path}.id"),
                    "Ids must be unique",
                );
                validate_set_values(
                    &mut errors,
                    &path,
                    set.reps,
                    set.weight_kg,
                    set.duration_seconds,
                    set.distance_m,
                );
                validate::finite_in_range(&mut errors, &format!("{path}.rpe"), set.rpe, 1.0, 10.0);
            }
        }

        errors.into_result()?;
        Ok(WorkoutDraft(self))
    }
}

/// Bounds shared by logged sets and routine targets.
pub fn validate_set_values(
    errors: &mut FieldErrors,
    path: &str,
    reps: Option<i32>,
    weight_kg: Option<f64>,
    duration_seconds: Option<i32>,
    distance_m: Option<f64>,
) {
    validate::in_range(errors, &format!("{path}.reps"), reps, 0, 1000);
    validate::finite_in_range(errors, &format!("{path}.weight_kg"), weight_kg, 0.0, 1000.0);
    validate::in_range(
        errors,
        &format!("{path}.duration_seconds"),
        duration_seconds,
        0,
        86_400,
    );
    validate::finite_in_range(
        errors,
        &format!("{path}.distance_m"),
        distance_m,
        0.0,
        1_000_000.0,
    );
}

#[derive(Debug, Serialize)]
pub struct Workout {
    pub id: Uuid,
    pub routine_id: Option<Uuid>,
    pub title: String,
    pub notes: Option<String>,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub revision: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub exercises: Vec<WorkoutExercise>,
}

#[derive(Debug, Serialize)]
pub struct WorkoutExercise {
    pub id: Uuid,
    pub exercise_id: Uuid,
    pub exercise_name: String,
    pub tracking: Tracking,
    pub primary_muscle: Muscle,
    pub notes: Option<String>,
    pub rest_seconds: Option<i32>,
    pub sets: Vec<WorkoutSet>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct WorkoutSet {
    pub id: Uuid,
    #[serde(skip)]
    pub workout_exercise_id: Uuid,
    pub kind: SetKind,
    pub reps: Option<i32>,
    pub weight_kg: Option<f64>,
    pub duration_seconds: Option<i32>,
    pub distance_m: Option<f64>,
    pub rpe: Option<f64>,
    pub completed: bool,
}

/// List item with aggregates over completed working sets.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct WorkoutSummary {
    pub id: Uuid,
    pub routine_id: Option<Uuid>,
    pub title: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub duration_seconds: Option<i64>,
    pub exercise_count: i64,
    pub set_count: i64,
    pub total_reps: i64,
    pub volume_kg: f64,
    pub exercise_names: Vec<String>,
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;
    use crate::error::ApiError;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 27, 12, 0, 0)
            .single()
            .expect("valid date")
    }

    fn request() -> WorkoutRequest {
        serde_json::from_value(serde_json::json!({
            "title": "  Push Day ",
            "started_at": "2026-09-27T10:00:00Z",
            "ended_at": "2026-09-27T11:00:00Z",
            "revision": 3,
            "exercises": [{
                "id": "00000000-0000-0000-0000-000000000001",
                "exercise_id": "00000000-0000-0000-0000-00000000000a",
                "sets": [
                    { "id": "00000000-0000-0000-0000-000000000002", "reps": 5, "weight_kg": 100.0, "completed": true },
                    { "id": "00000000-0000-0000-0000-000000000003", "kind": "warmup", "reps": 10 }
                ]
            }]
        }))
        .expect("valid workout json")
    }

    fn field_errors(result: ApiResult<WorkoutDraft>) -> Vec<String> {
        match result {
            Err(ApiError::Validation { fields, .. }) => fields.into_keys().collect(),
            other => panic!("expected validation error, got {other:?}"),
        }
    }

    #[test]
    fn accepts_and_normalises_valid_workout() -> ApiResult<()> {
        let draft = request().validate(now())?;
        assert_eq!(draft.title, "Push Day");
        assert_eq!(draft.exercises[0].sets[0].kind, SetKind::Normal);
        assert_eq!(draft.exercises[0].sets[1].kind, SetKind::Warmup);
        Ok(())
    }

    #[test]
    fn rejects_inconsistent_times() {
        let mut workout = request();
        workout.ended_at = Some(workout.started_at - Duration::minutes(1));
        assert_eq!(field_errors(workout.validate(now())), ["ended_at"]);

        let mut workout = request();
        workout.ended_at = Some(workout.started_at + Duration::hours(25));
        assert_eq!(field_errors(workout.validate(now())), ["ended_at"]);

        let mut workout = request();
        workout.started_at = now() + Duration::days(2);
        workout.ended_at = None;
        assert_eq!(field_errors(workout.validate(now())), ["started_at"]);
    }

    #[test]
    fn rejects_duplicate_ids_and_out_of_range_values() {
        let mut workout = request();
        let sets = &mut workout.exercises[0].sets;
        sets[1].id = sets[0].id;
        sets[0].reps = Some(-1);
        sets[0].weight_kg = Some(f64::INFINITY);
        sets[0].rpe = Some(11.0);
        assert_eq!(
            field_errors(workout.validate(now())),
            [
                "exercises[0].sets[0].reps",
                "exercises[0].sets[0].rpe",
                "exercises[0].sets[0].weight_kg",
                "exercises[0].sets[1].id",
            ]
        );
    }
}
