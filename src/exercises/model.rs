use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    error::{ApiResult, FieldErrors},
    validate,
};

/// Which measurements a set of this exercise records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "text", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum Tracking {
    WeightReps,
    Reps,
    Duration,
    DistanceDuration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "text", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum Muscle {
    Chest,
    Lats,
    UpperBack,
    LowerBack,
    Traps,
    Shoulders,
    Biceps,
    Triceps,
    Forearms,
    Abs,
    Quadriceps,
    Hamstrings,
    Glutes,
    Calves,
    FullBody,
    Cardio,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "text", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum Equipment {
    None,
    Barbell,
    Dumbbell,
    Kettlebell,
    Machine,
    Cable,
    Band,
    Plate,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "text", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum Difficulty {
    Beginner,
    Intermediate,
    Advanced,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Exercise {
    pub id: Uuid,
    #[serde(skip)]
    pub owner_id: Option<Uuid>,
    pub name: String,
    pub tracking: Tracking,
    pub primary_muscle: Muscle,
    pub secondary_muscles: Vec<Muscle>,
    pub equipment: Equipment,
    pub difficulty: Difficulty,
    /// YouTube video id demonstrating the movement.
    pub video_id: Option<String>,
    pub instructions: Option<String>,
    pub is_custom: bool,
    pub archived_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct ExerciseRequest {
    #[serde(default)]
    pub name: String,
    pub tracking: Tracking,
    pub primary_muscle: Muscle,
    #[serde(default)]
    pub secondary_muscles: Vec<Muscle>,
    #[serde(default = "default_equipment")]
    pub equipment: Equipment,
    #[serde(default = "default_difficulty")]
    pub difficulty: Difficulty,
    pub video_id: Option<String>,
    pub instructions: Option<String>,
    /// Admins may add exercises to the shared catalog instead of their own library.
    #[serde(default)]
    pub global: bool,
}

fn default_equipment() -> Equipment {
    Equipment::None
}

fn default_difficulty() -> Difficulty {
    Difficulty::Beginner
}

/// A validated exercise definition, ready to be persisted.
#[derive(Debug)]
pub struct ExerciseDraft {
    pub name: String,
    pub tracking: Tracking,
    pub primary_muscle: Muscle,
    pub secondary_muscles: Vec<Muscle>,
    pub equipment: Equipment,
    pub difficulty: Difficulty,
    pub video_id: Option<String>,
    pub instructions: Option<String>,
}

impl ExerciseRequest {
    pub fn validate(self) -> ApiResult<ExerciseDraft> {
        let mut errors = FieldErrors::default();
        let name = validate::required_text(&mut errors, "name", &self.name, 1, 80);
        let video_id =
            validate::optional_text(&mut errors, "video_id", self.video_id.as_deref(), 11);
        if let Some(id) = &video_id {
            errors.ensure(
                id.len() == 11
                    && id
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
                "video_id",
                "Video id must be an 11 character YouTube video id",
            );
        }
        let instructions = validate::optional_text(
            &mut errors,
            "instructions",
            self.instructions.as_deref(),
            2000,
        );
        errors.into_result()?;

        let mut secondary_muscles = Vec::with_capacity(self.secondary_muscles.len());
        for muscle in self.secondary_muscles {
            if muscle != self.primary_muscle && !secondary_muscles.contains(&muscle) {
                secondary_muscles.push(muscle);
            }
        }

        Ok(ExerciseDraft {
            name,
            tracking: self.tracking,
            primary_muscle: self.primary_muscle,
            secondary_muscles,
            equipment: self.equipment,
            difficulty: self.difficulty,
            video_id,
            instructions,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(json: &str) -> ExerciseRequest {
        serde_json::from_str(json).expect("valid request json")
    }

    #[test]
    fn applies_defaults_and_dedupes_muscles() -> ApiResult<()> {
        let draft = request(
            r#"{"name": " Zercher Squat ", "tracking": "weight_reps", "primary_muscle": "quadriceps",
                "secondary_muscles": ["glutes", "quadriceps", "glutes", "abs"]}"#,
        )
        .validate()?;
        assert_eq!(draft.name, "Zercher Squat");
        assert_eq!(draft.equipment, Equipment::None);
        assert_eq!(draft.difficulty, Difficulty::Beginner);
        assert_eq!(draft.secondary_muscles, [Muscle::Glutes, Muscle::Abs]);
        Ok(())
    }

    #[test]
    fn rejects_bad_video_id() {
        let result = request(
            r#"{"name": "Squat", "tracking": "weight_reps", "primary_muscle": "quadriceps",
                "video_id": "not a video"}"#,
        )
        .validate();
        assert!(result.is_err());
    }
}
