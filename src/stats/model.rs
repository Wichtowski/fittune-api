use chrono::{DateTime, Days, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    error::{ApiResult, FieldErrors},
    exercises::model::{Muscle, Tracking},
};

/// Longest period any stats query may cover.
const MAX_PERIOD_DAYS: i64 = 3 * 366;

/// Inclusive calendar-date range interpreted in the caller's IANA time zone.
#[derive(Debug, Deserialize)]
pub struct PeriodQuery {
    pub from: NaiveDate,
    pub to: NaiveDate,
    #[serde(default = "utc")]
    pub tz: String,
}

fn utc() -> String {
    "UTC".to_owned()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Period {
    pub from: NaiveDate,
    pub to: NaiveDate,
}

impl PeriodQuery {
    pub fn validate(&self) -> ApiResult<Period> {
        let mut errors = FieldErrors::default();
        errors.ensure(
            self.from <= self.to,
            "from",
            "Period start must not be after its end",
        );
        errors.ensure(
            (self.to - self.from).num_days() < MAX_PERIOD_DAYS,
            "to",
            "Period must not be longer than three years",
        );
        errors.ensure(
            !self.tz.is_empty() && self.tz.len() <= 64,
            "tz",
            "Unknown time zone",
        );
        errors.into_result()?;
        Ok(Period {
            from: self.from,
            to: self.to,
        })
    }
}

impl Period {
    pub fn days(self) -> u64 {
        u64::try_from((self.to - self.from).num_days() + 1).unwrap_or(1)
    }

    /// The equally long period that ends the day before this one starts.
    pub fn previous(self) -> Self {
        let to = self.from - Days::new(1);
        Self {
            from: to - Days::new(self.days() - 1),
            to,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Bucket {
    Week,
    Month,
}

impl Bucket {
    pub fn as_sql(self) -> &'static str {
        match self {
            Self::Week => "week",
            Self::Month => "month",
        }
    }
}

#[derive(Debug, Default, Serialize, sqlx::FromRow, PartialEq)]
pub struct Totals {
    pub workouts: i64,
    pub workout_seconds: i64,
    pub sets: i64,
    pub reps: i64,
    pub volume_kg: f64,
    pub activities: i64,
    pub activity_seconds: i64,
    pub activity_distance_m: f64,
}

#[derive(Debug, Serialize)]
pub struct Overview {
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub current: Totals,
    /// Same-length period immediately before `from`, for comparison.
    pub previous: Totals,
    /// Consecutive weeks (Monday-based) with at least one workout or activity.
    pub streak_weeks: u32,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct TimelinePoint {
    pub bucket: NaiveDate,
    pub workouts: i64,
    pub workout_seconds: i64,
    pub sets: i64,
    pub reps: i64,
    pub volume_kg: f64,
    pub activities: i64,
    pub activity_seconds: i64,
    pub activity_distance_m: f64,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct MuscleVolume {
    pub muscle: Muscle,
    pub sets: i64,
    pub volume_kg: f64,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct ExerciseRecord {
    pub exercise_id: Uuid,
    pub exercise_name: String,
    pub tracking: Tracking,
    pub primary_muscle: Muscle,
    pub max_weight_kg: Option<f64>,
    pub best_e1rm_kg: Option<f64>,
    pub max_reps: Option<i32>,
    pub max_duration_seconds: Option<i32>,
    pub max_distance_m: Option<f64>,
    pub sessions: i64,
    pub last_performed_at: DateTime<Utc>,
}

/// Counts consecutive active weeks ending at `current_week`, or at the week before it when the
/// current week has no activity yet (a streak is not broken until a whole week is missed).
/// `active_weeks` must be distinct Monday dates sorted newest first.
pub fn week_streak(active_weeks: &[NaiveDate], current_week: NaiveDate) -> u32 {
    let mut expected = match active_weeks.first() {
        Some(&week) if week == current_week => current_week,
        Some(&week) if week == current_week - Days::new(7) => week,
        _ => return 0,
    };
    let mut streak = 0;
    for &week in active_weeks {
        if week != expected {
            break;
        }
        streak += 1;
        expected = expected - Days::new(7);
    }
    streak
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(s: &str) -> NaiveDate {
        s.parse().expect("valid date")
    }

    #[test]
    fn previous_period_has_same_length() {
        let period = Period {
            from: date("2026-09-21"),
            to: date("2026-09-27"),
        };
        assert_eq!(period.days(), 7);
        assert_eq!(
            period.previous(),
            Period {
                from: date("2026-09-14"),
                to: date("2026-09-20")
            }
        );
    }

    #[test]
    fn rejects_inverted_or_huge_periods() {
        let inverted = PeriodQuery {
            from: date("2026-09-27"),
            to: date("2026-09-01"),
            tz: utc(),
        };
        assert!(inverted.validate().is_err());
        let huge = PeriodQuery {
            from: date("2020-01-01"),
            to: date("2026-01-01"),
            tz: utc(),
        };
        assert!(huge.validate().is_err());
    }

    #[test]
    fn streak_counts_consecutive_weeks() {
        let current = date("2026-09-21");
        let weeks = [
            date("2026-09-21"),
            date("2026-09-14"),
            date("2026-09-07"),
            date("2026-08-24"),
        ];
        assert_eq!(week_streak(&weeks, current), 3);
    }

    #[test]
    fn streak_survives_until_a_full_week_is_missed() {
        let current = date("2026-09-21");
        assert_eq!(
            week_streak(&[date("2026-09-14"), date("2026-09-07")], current),
            2
        );
        assert_eq!(week_streak(&[date("2026-09-07")], current), 0);
        assert_eq!(week_streak(&[], current), 0);
    }
}
