//! Per-exercise progress: sessions grouped from raw sets, plus all-time records.

use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use super::{model::Exercise, repo::HistorySetRow};
use crate::workouts::model::SetKind;

#[derive(Debug, Serialize)]
pub struct ExerciseHistory {
    pub exercise: Exercise,
    pub records: Records,
    /// Most recent sessions first.
    pub sessions: Vec<Session>,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct HistorySet {
    pub kind: SetKind,
    pub reps: Option<i32>,
    pub weight_kg: Option<f64>,
    pub duration_seconds: Option<i32>,
    pub distance_m: Option<f64>,
    pub rpe: Option<f64>,
    pub e1rm_kg: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct Session {
    pub workout_id: Uuid,
    pub workout_title: String,
    pub started_at: DateTime<Utc>,
    pub sets: Vec<HistorySet>,
    pub working_sets: u32,
    pub total_reps: i64,
    pub volume_kg: f64,
    pub max_weight_kg: Option<f64>,
    pub best_e1rm_kg: Option<f64>,
    pub max_duration_seconds: Option<i32>,
    pub max_distance_m: Option<f64>,
}

/// A personal best and where it was achieved.
#[derive(Debug, Serialize, Clone, Copy, PartialEq)]
pub struct Record {
    pub value: f64,
    pub workout_id: Uuid,
    pub achieved_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Default, PartialEq)]
pub struct Records {
    pub max_weight_kg: Option<Record>,
    pub best_e1rm_kg: Option<Record>,
    pub max_reps: Option<Record>,
    pub best_session_volume_kg: Option<Record>,
    pub max_duration_seconds: Option<Record>,
    pub max_distance_m: Option<Record>,
}

/// Groups rows (ordered newest workout first, one workout's sets contiguous) into sessions.
/// Warm-up sets are listed but excluded from every metric and record.
pub fn build_sessions(rows: Vec<HistorySetRow>) -> Vec<Session> {
    let mut sessions: Vec<Session> = Vec::new();
    for row in rows {
        let session = match sessions.last_mut() {
            Some(session) if session.workout_id == row.workout_id => session,
            _ => {
                sessions.push(Session::new(&row));
                sessions.last_mut().expect("session was just pushed")
            }
        };
        session.add(row);
    }
    sessions
}

pub fn compute_records(sessions: &[Session]) -> Records {
    let mut records = Records::default();
    for session in sessions {
        let at = |value: f64| Record {
            value,
            workout_id: session.workout_id,
            achieved_at: session.started_at,
        };
        for set in session
            .sets
            .iter()
            .filter(|set| set.kind != SetKind::Warmup)
        {
            improve(&mut records.max_weight_kg, set.weight_kg.map(at));
            improve(&mut records.best_e1rm_kg, set.e1rm_kg.map(at));
            improve(
                &mut records.max_reps,
                set.reps.map(|reps| at(f64::from(reps))),
            );
            improve(
                &mut records.max_duration_seconds,
                set.duration_seconds.map(|d| at(f64::from(d))),
            );
            improve(&mut records.max_distance_m, set.distance_m.map(at));
        }
        if session.volume_kg > 0.0 {
            improve(
                &mut records.best_session_volume_kg,
                Some(at(session.volume_kg)),
            );
        }
    }
    records
}

/// Keeps the earliest occurrence on ties: sessions are iterated newest first, so a later
/// equal value (older session) replaces it.
fn improve(best: &mut Option<Record>, candidate: Option<Record>) {
    let Some(candidate) = candidate else { return };
    match best {
        Some(current) if candidate.value < current.value => {}
        Some(current)
            if candidate.value == current.value && candidate.achieved_at >= current.achieved_at => {
        }
        _ => *best = Some(candidate),
    }
}

impl Session {
    fn new(row: &HistorySetRow) -> Self {
        Self {
            workout_id: row.workout_id,
            workout_title: row.workout_title.clone(),
            started_at: row.started_at,
            sets: Vec::new(),
            working_sets: 0,
            total_reps: 0,
            volume_kg: 0.0,
            max_weight_kg: None,
            best_e1rm_kg: None,
            max_duration_seconds: None,
            max_distance_m: None,
        }
    }

    fn add(&mut self, row: HistorySetRow) {
        let set = HistorySet {
            kind: row.kind,
            reps: row.reps,
            weight_kg: row.weight_kg,
            duration_seconds: row.duration_seconds,
            distance_m: row.distance_m,
            rpe: row.rpe,
            e1rm_kg: row.e1rm_kg,
        };
        if set.kind != SetKind::Warmup {
            self.working_sets += 1;
            self.total_reps += i64::from(set.reps.unwrap_or(0));
            if let (Some(weight), Some(reps)) = (set.weight_kg, set.reps) {
                self.volume_kg += weight * f64::from(reps);
            }
            self.max_weight_kg = max_f64(self.max_weight_kg, set.weight_kg);
            self.best_e1rm_kg = max_f64(self.best_e1rm_kg, set.e1rm_kg);
            self.max_distance_m = max_f64(self.max_distance_m, set.distance_m);
            self.max_duration_seconds = self.max_duration_seconds.max(set.duration_seconds);
        }
        self.sets.push(set);
    }
}

fn max_f64(current: Option<f64>, candidate: Option<f64>) -> Option<f64> {
    match (current, candidate) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (a, b) => a.or(b),
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn row(workout: u128, day: u32, kind: SetKind, reps: i32, weight: f64) -> HistorySetRow {
        HistorySetRow {
            workout_id: Uuid::from_u128(workout),
            workout_title: format!("Workout {workout}"),
            started_at: Utc
                .with_ymd_and_hms(2026, 9, day, 18, 0, 0)
                .single()
                .expect("valid date"),
            kind,
            reps: Some(reps),
            weight_kg: Some(weight),
            duration_seconds: None,
            distance_m: None,
            rpe: None,
            e1rm_kg: (reps <= 12).then(|| {
                if reps == 1 {
                    weight
                } else {
                    weight * (1.0 + f64::from(reps) / 30.0)
                }
            }),
        }
    }

    #[test]
    fn groups_rows_into_sessions_and_ignores_warmups_in_metrics() {
        let sessions = build_sessions(vec![
            row(2, 20, SetKind::Warmup, 10, 60.0),
            row(2, 20, SetKind::Normal, 5, 100.0),
            row(2, 20, SetKind::Normal, 5, 100.0),
            row(1, 13, SetKind::Normal, 8, 90.0),
        ]);

        assert_eq!(sessions.len(), 2);
        let latest = &sessions[0];
        assert_eq!(latest.sets.len(), 3);
        assert_eq!(latest.working_sets, 2);
        assert_eq!(latest.total_reps, 10);
        assert_eq!(latest.volume_kg, 1000.0);
        assert_eq!(latest.max_weight_kg, Some(100.0));
    }

    #[test]
    fn records_track_best_values_and_where_they_happened() {
        let sessions = build_sessions(vec![
            row(3, 27, SetKind::Normal, 3, 110.0),
            row(2, 20, SetKind::Warmup, 20, 200.0),
            row(2, 20, SetKind::Normal, 5, 100.0),
            row(1, 13, SetKind::Normal, 12, 80.0),
        ]);
        let records = compute_records(&sessions);

        let max_weight = records.max_weight_kg.expect("max weight record");
        assert_eq!(max_weight.value, 110.0);
        assert_eq!(max_weight.workout_id, Uuid::from_u128(3));

        let max_reps = records.max_reps.expect("max reps record");
        assert_eq!(max_reps.value, 12.0, "warm-up reps must not count");

        // 110 x 3 -> 121kg beats 100 x 5 -> 116.7kg and 80 x 12 -> 112kg.
        let e1rm = records.best_e1rm_kg.expect("e1rm record");
        assert_eq!(e1rm.workout_id, Uuid::from_u128(3));
        assert!((e1rm.value - 121.0).abs() < 1e-9);

        let volume = records.best_session_volume_kg.expect("volume record");
        assert_eq!(volume.value, 960.0);
        assert_eq!(volume.workout_id, Uuid::from_u128(1));
    }

    #[test]
    fn ties_keep_the_earliest_achievement() {
        let sessions = build_sessions(vec![
            row(2, 20, SetKind::Normal, 5, 100.0),
            row(1, 13, SetKind::Normal, 5, 100.0),
        ]);
        let records = compute_records(&sessions);
        assert_eq!(
            records.max_weight_kg.map(|r| r.workout_id),
            Some(Uuid::from_u128(1))
        );
    }

    #[test]
    fn empty_history_has_no_records() {
        assert_eq!(
            compute_records(&build_sessions(Vec::new())),
            Records::default()
        );
    }
}
