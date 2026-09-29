//! Calories burned in FitTune training, net of rest so they add cleanly to the resting and
//! daily-life energy in the targets

use serde::Serialize;

use crate::activities::model::ActivityKind;

/// A finished strength session counts as vigorous effort for at most this long
const MAX_WORKOUT_HOURS: f64 = 3.0;
const STRENGTH_MET: f64 = 5.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ActivitySession {
    pub kind: ActivityKind,
    pub duration_seconds: i32,
    pub distance_m: Option<f64>,
    /// Calories the user recorded, trusted over any estimate
    pub calories: Option<i32>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct Exercise {
    pub workouts: u32,
    pub activities: u32,
    pub energy_kcal: f64,
    /// False when estimates were skipped because no weight is logged yet
    pub estimated: bool,
}

fn met(kind: ActivityKind, hours: f64, distance_m: Option<f64>) -> f64 {
    match kind {
        // Running cost tracks speed closely: about one MET per km/h
        ActivityKind::Run => distance_m
            .filter(|m| *m > 0.0 && hours > 0.0)
            .map_or(9.8, |m| (m / 1000.0 / hours).clamp(6.0, 18.0)),
        ActivityKind::Ride => 7.5,
        ActivityKind::Walk => 3.5,
        ActivityKind::Hike => 6.0,
        ActivityKind::Swim => 7.0,
        ActivityKind::Row => 7.0,
        ActivityKind::Other => 5.0,
    }
}

/// Net calories of one activity; `None` when it needs a weight and there is none
pub fn activity_kcal(session: &ActivitySession, weight_kg: Option<f64>) -> Option<f64> {
    if let Some(calories) = session.calories {
        return Some(f64::from(calories));
    }
    let hours = f64::from(session.duration_seconds) / 3600.0;
    let met = met(session.kind, hours, session.distance_m);
    weight_kg.map(|kg| (met - 1.0) * kg * hours)
}

/// Net calories of a finished strength workout lasting `seconds`
pub fn workout_kcal(seconds: f64, weight_kg: Option<f64>) -> Option<f64> {
    let hours = (seconds / 3600.0).clamp(0.0, MAX_WORKOUT_HOURS);
    weight_kg.map(|kg| (STRENGTH_MET - 1.0) * kg * hours)
}

/// Everything trained on one day
pub fn day(
    activities: &[ActivitySession],
    workout_seconds: &[f64],
    weight_kg: Option<f64>,
) -> Exercise {
    let activity_kcal: f64 = activities
        .iter()
        .filter_map(|session| activity_kcal(session, weight_kg))
        .sum();
    let workout_kcal: f64 = workout_seconds
        .iter()
        .filter_map(|seconds| workout_kcal(*seconds, weight_kg))
        .sum();
    Exercise {
        workouts: u32::try_from(workout_seconds.len()).unwrap_or(u32::MAX),
        activities: u32::try_from(activities.len()).unwrap_or(u32::MAX),
        energy_kcal: activity_kcal + workout_kcal,
        estimated: weight_kg.is_some(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 0.01
    }

    fn session(kind: ActivityKind, minutes: i32, km: Option<f64>) -> ActivitySession {
        ActivitySession {
            kind,
            duration_seconds: minutes * 60,
            distance_m: km.map(|k| k * 1000.0),
            calories: None,
        }
    }

    #[test]
    fn recorded_calories_win_even_without_a_weight() {
        let run = ActivitySession {
            calories: Some(512),
            ..session(ActivityKind::Run, 45, Some(8.0))
        };
        assert_eq!(activity_kcal(&run, None), Some(512.0));
    }

    #[test]
    fn running_uses_speed() {
        // 10 km in an hour is about 10 MET, net 9
        let run = session(ActivityKind::Run, 60, Some(10.0));
        assert!(close(
            activity_kcal(&run, Some(70.0)).expect("estimate"),
            9.0 * 70.0
        ));
        // Speed is clamped, a GPS glitch cannot claim 40 km/h
        let glitch = session(ActivityKind::Run, 30, Some(20.0));
        assert!(close(
            activity_kcal(&glitch, Some(70.0)).expect("estimate"),
            17.0 * 70.0 * 0.5
        ));
        let no_distance = session(ActivityKind::Run, 60, None);
        assert!(close(
            activity_kcal(&no_distance, Some(70.0)).expect("estimate"),
            8.8 * 70.0
        ));
    }

    #[test]
    fn other_kinds_use_their_met() {
        for (kind, met) in [
            (ActivityKind::Ride, 7.5),
            (ActivityKind::Walk, 3.5),
            (ActivityKind::Hike, 6.0),
            (ActivityKind::Swim, 7.0),
            (ActivityKind::Row, 7.0),
            (ActivityKind::Other, 5.0),
        ] {
            let kcal = activity_kcal(&session(kind, 30, Some(5.0)), Some(80.0)).expect("estimate");
            assert!(close(kcal, (met - 1.0) * 80.0 * 0.5), "{kind:?}");
        }
    }

    #[test]
    fn estimates_need_a_weight() {
        assert_eq!(
            activity_kcal(&session(ActivityKind::Walk, 30, None), None),
            None
        );
        assert_eq!(workout_kcal(3600.0, None), None);
    }

    #[test]
    fn workouts_count_as_vigorous_effort_capped_at_three_hours() {
        assert!(close(
            workout_kcal(3600.0, Some(80.0)).expect("estimate"),
            4.0 * 80.0
        ));
        assert!(close(
            workout_kcal(5.0 * 3600.0, Some(80.0)).expect("estimate"),
            4.0 * 80.0 * 3.0
        ));
    }

    #[test]
    fn a_day_adds_everything_up() {
        let walk = session(ActivityKind::Walk, 60, None);
        let logged = ActivitySession {
            calories: Some(300),
            ..session(ActivityKind::Ride, 60, None)
        };
        let day = day(&[walk, logged], &[1800.0], Some(80.0));
        assert_eq!((day.workouts, day.activities), (1, 2));
        assert!(close(
            day.energy_kcal,
            2.5 * 80.0 + 300.0 + 4.0 * 80.0 * 0.5
        ));
        assert!(day.estimated);
    }

    #[test]
    fn a_day_without_weight_keeps_recorded_calories_only() {
        let walk = session(ActivityKind::Walk, 60, None);
        let logged = ActivitySession {
            calories: Some(300),
            ..session(ActivityKind::Ride, 60, None)
        };
        let day = day(&[walk, logged], &[1800.0], None);
        assert!(close(day.energy_kcal, 300.0));
        assert!(!day.estimated);
    }
}
