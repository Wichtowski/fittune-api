//! What the fixtures contain. Everything here is a pure function of the base date, so every run
//! produces the same documents with the same ids.

use anyhow::{Result, bail};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Dev-only password shared by every fixture account
pub const PASSWORD: &str = "FitTune#Dev1";
/// Reserved TLD, so fixture addresses can never belong to a real person
pub const EMAIL_DOMAIN: &str = "fittune.test";

/// Stable id for a fixture document, derived from its key like the catalog seed derives ids
/// from exercise names.
pub fn fixture_id(key: &str) -> Uuid {
    let digest = Sha256::digest(format!("fittune:fixture:{key}").as_bytes());
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    uuid::Builder::from_custom_bytes(bytes).into_uuid()
}

/// Places fixture history relative to a base date, so recent-history screens stay populated
#[derive(Debug, Clone, Copy)]
pub struct Clock {
    pub base_date: NaiveDate,
    pub now: DateTime<Utc>,
}

impl Clock {
    pub fn new(base_date: NaiveDate, now: DateTime<Utc>) -> Result<Self> {
        if base_date > now.date_naive() {
            bail!("the fixture base date {base_date} is in the future");
        }
        Ok(Self { base_date, now })
    }

    /// `hour:minute` UTC, `days_ago` days before the base date.
    pub fn at(&self, days_ago: i64, hour: u32, minute: u32) -> DateTime<Utc> {
        self.base_date
            .and_hms_opt(hour, minute, 0)
            .unwrap_or_default()
            .and_utc()
            - Duration::days(days_ago)
    }

    /// "Right now" in fixture time: the actual time on the base date, or its evening for a
    /// base date in the past.
    pub fn anchor(&self) -> DateTime<Utc> {
        self.now.min(self.at(0, 19, 0))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Admin,
    /// Main account: months of history, routines, places, custom exercises and activities
    Demo,
    /// Imperial units, light history and a workout left in progress on "another device"
    Casual,
    /// Just registered, nothing logged yet
    Newbie,
}

pub struct Account {
    pub kind: Kind,
    pub username: &'static str,
    pub profile: fn() -> Value,
    pub summary: &'static str,
}

impl Account {
    pub fn email(&self) -> String {
        format!("{}@{EMAIL_DOMAIN}", self.username)
    }
}

/// The admin comes first: it issues the invites the others register with
pub const ACCOUNTS: [Account; 4] = [
    Account {
        kind: Kind::Admin,
        username: "admin",
        profile: || json!({ "display_name": "Fixture Admin" }),
        summary: "admin: invites in every state, user list, catalog editing",
    },
    Account {
        kind: Kind::Demo,
        username: "demo",
        profile: || {
            json!({
                "display_name": "Alex Demo",
                "birthday": "1994-05-17",
                "account_type": "gym_enthusiast",
                "weight_unit": "kg",
                "distance_unit": "km",
            })
        },
        summary: "12 weeks of push/pull/legs plus a session today, records, routines, 2 places, custom exercises, runs and rides, friends with casual, a friend request from newbie",
    },
    Account {
        kind: Kind::Casual,
        username: "casual",
        profile: || json!({ "display_name": "Sam Casual", "weight_unit": "lb", "distance_unit": "mi" }),
        summary: "lb/mi units, 6 light weeks, a workout in progress to continue from the Workout tab, friends with demo sharing sessions only",
    },
    Account {
        kind: Kind::Newbie,
        username: "newbie",
        profile: || json!({ "display_name": "Nia Newbie" }),
        summary: "new account: no places, routines or history, a pending friend request to demo",
    },
];

pub struct Place {
    pub key: &'static str,
    pub name: &'static str,
    pub kind: &'static str,
    pub equipment: &'static [&'static str],
}

const EVERY_ITEM: &[&str] = &[
    "barbell",
    "ez_bar",
    "dumbbells",
    "kettlebells",
    "flat_bench",
    "adjustable_bench",
    "squat_rack",
    "pull_up_bar",
    "dip_station",
    "leg_press",
    "leg_extension",
    "leg_curl",
    "calf_raise_machine",
    "smith_machine",
    "chest_press_machine",
    "pec_deck",
    "shoulder_press_machine",
    "assisted_pull_up_machine",
    "cable_station",
    "lat_pulldown",
    "seated_row",
    "treadmill",
    "rowing_machine",
    "stationary_bike",
    "resistance_band",
    "ab_wheel",
    "jump_rope",
];

pub fn places(kind: Kind) -> &'static [Place] {
    match kind {
        Kind::Demo => &[
            Place {
                key: "gym",
                name: "Iron Temple Gym",
                kind: "gym",
                equipment: EVERY_ITEM,
            },
            Place {
                key: "home",
                name: "Home garage",
                kind: "home",
                equipment: &[
                    "barbell",
                    "dumbbells",
                    "kettlebells",
                    "adjustable_bench",
                    "squat_rack",
                    "pull_up_bar",
                    "rowing_machine",
                    "resistance_band",
                    "ab_wheel",
                    "jump_rope",
                ],
            },
        ],
        Kind::Casual => &[Place {
            key: "gym",
            name: "Downtown Fitness",
            kind: "gym",
            equipment: &[
                "dumbbells",
                "flat_bench",
                "adjustable_bench",
                "leg_press",
                "chest_press_machine",
                "shoulder_press_machine",
                "cable_station",
                "lat_pulldown",
                "seated_row",
                "treadmill",
                "stationary_bike",
            ],
        }],
        Kind::Admin | Kind::Newbie => &[],
    }
}

/// Custom exercises covering the reps and duration tracking modes as well as weight
pub fn custom_exercises(kind: Kind) -> Vec<Value> {
    if kind != Kind::Demo {
        return Vec::new();
    }
    vec![
        json!({
            "name": "Zercher Squat",
            "tracking": "weight_reps",
            "primary_muscle": "quadriceps",
            "secondary_muscles": ["glutes", "abs", "upper_back"],
            "equipment": "barbell",
            "requires": ["barbell", "squat_rack"],
            "difficulty": "advanced",
            "instructions": "Hold the bar in the crook of your elbows, brace, and squat to depth.",
        }),
        json!({
            "name": "Banded Pull-Apart",
            "tracking": "reps",
            "primary_muscle": "shoulders",
            "secondary_muscles": ["upper_back"],
            "equipment": "band",
            "requires": ["resistance_band"],
        }),
        json!({
            "name": "Dead Hang",
            "tracking": "duration",
            "primary_muscle": "forearms",
            "secondary_muscles": ["lats"],
            "requires": ["pull_up_bar"],
        }),
    ]
}

#[derive(Debug, Clone, Copy)]
enum Finisher {
    Drop,
    Failure,
}

#[derive(Debug, Clone, Copy)]
enum Dose {
    /// Weight that climbs by `step` every week, with ramp-up sets at a share of it
    Weight {
        start: f64,
        step: f64,
        sets: usize,
        reps: i32,
        warmups: &'static [(f64, i32)],
        finisher: Option<Finisher>,
    },
    /// Bodyweight reps, one more every `every` weeks
    Reps { sets: usize, start: i32, every: i32 },
    /// Holds that grow by `step` seconds a week
    Hold { sets: usize, start: i32, step: i32 },
    /// A fixed distance done `step` seconds faster every week
    Cardio {
        distance_m: f64,
        start: i32,
        step: i32,
    },
}

#[derive(Debug, Clone, Copy)]
struct Slot {
    exercise: &'static str,
    rest: i32,
    dose: Dose,
}

pub struct Template {
    pub routine: &'static str,
    pub notes: &'static str,
    title: &'static str,
    place: &'static str,
    minutes: i64,
    slots: &'static [Slot],
}

const fn weight(
    exercise: &'static str,
    rest: i32,
    (start, step): (f64, f64),
    (sets, reps): (usize, i32),
    warmups: &'static [(f64, i32)],
    finisher: Option<Finisher>,
) -> Slot {
    Slot {
        exercise,
        rest,
        dose: Dose::Weight {
            start,
            step,
            sets,
            reps,
            warmups,
            finisher,
        },
    }
}

const fn other(exercise: &'static str, rest: i32, dose: Dose) -> Slot {
    Slot {
        exercise,
        rest,
        dose,
    }
}

const PUSH: Template = Template {
    routine: "Push",
    notes: "Chest, shoulders and triceps. Add weight when every working set hits its reps.",
    title: "Push day",
    place: "gym",
    minutes: 65,
    slots: &[
        weight(
            "Barbell Bench Press",
            180,
            (70.0, 2.5),
            (3, 5),
            &[(0.5, 10), (0.75, 5)],
            None,
        ),
        weight(
            "Overhead Press",
            150,
            (40.0, 1.25),
            (3, 6),
            &[(0.6, 8)],
            None,
        ),
        weight(
            "Incline Dumbbell Press",
            90,
            (22.0, 1.0),
            (3, 10),
            &[],
            None,
        ),
        weight(
            "Lateral Raise",
            60,
            (8.0, 0.5),
            (3, 15),
            &[],
            Some(Finisher::Drop),
        ),
        weight("Triceps Pushdown", 60, (25.0, 1.25), (3, 12), &[], None),
        other(
            "Plank",
            60,
            Dose::Hold {
                sets: 2,
                start: 60,
                step: 5,
            },
        ),
    ],
};

const PULL: Template = Template {
    routine: "Pull",
    notes: "Back and biceps. One heavy deadlift block, then volume.",
    title: "Pull day",
    place: "gym",
    minutes: 60,
    slots: &[
        weight(
            "Conventional Deadlift",
            240,
            (110.0, 5.0),
            (2, 5),
            &[(0.4, 8), (0.6, 5), (0.8, 3)],
            None,
        ),
        other(
            "Pull-Up",
            120,
            Dose::Reps {
                sets: 3,
                start: 6,
                every: 3,
            },
        ),
        weight("Barbell Row", 120, (60.0, 2.5), (3, 8), &[(0.6, 8)], None),
        weight("Face Pull", 60, (20.0, 1.0), (3, 15), &[], None),
        weight(
            "Barbell Curl",
            60,
            (30.0, 1.25),
            (3, 10),
            &[],
            Some(Finisher::Failure),
        ),
        other(
            "Banded Pull-Apart",
            45,
            Dose::Reps {
                sets: 2,
                start: 20,
                every: 4,
            },
        ),
    ],
};

const LEGS: Template = Template {
    routine: "Legs",
    notes: "Squat focus. Keep the RDLs strict.",
    title: "Leg day",
    place: "gym",
    minutes: 70,
    slots: &[
        weight(
            "Barbell Back Squat",
            210,
            (90.0, 2.5),
            (3, 5),
            &[(0.45, 10), (0.7, 5), (0.85, 2)],
            None,
        ),
        weight(
            "Romanian Deadlift",
            150,
            (80.0, 2.5),
            (3, 8),
            &[(0.6, 8)],
            None,
        ),
        weight("Leg Press", 120, (140.0, 5.0), (3, 12), &[], None),
        weight("Lying Leg Curl", 75, (35.0, 1.25), (3, 12), &[], None),
        weight("Standing Calf Raise", 60, (60.0, 2.5), (4, 15), &[], None),
        other(
            "Hanging Leg Raise",
            60,
            Dose::Reps {
                sets: 3,
                start: 10,
                every: 3,
            },
        ),
    ],
};

const HOME: Template = Template {
    routine: "Full Body (Home)",
    notes: "Short full-body session for weekends at home.",
    title: "Home full body",
    place: "home",
    minutes: 50,
    slots: &[
        weight("Zercher Squat", 150, (50.0, 2.5), (3, 8), &[(0.6, 6)], None),
        weight("One-Arm Dumbbell Row", 75, (26.0, 1.0), (3, 10), &[], None),
        other(
            "Push-Up",
            60,
            Dose::Reps {
                sets: 3,
                start: 18,
                every: 2,
            },
        ),
        weight("Kettlebell Swing", 60, (24.0, 0.0), (3, 15), &[], None),
        other(
            "Dead Hang",
            60,
            Dose::Hold {
                sets: 2,
                start: 35,
                step: 5,
            },
        ),
        other(
            "Rowing Machine",
            0,
            Dose::Cardio {
                distance_m: 2000.0,
                start: 510,
                step: 3,
            },
        ),
    ],
};

const CASUAL_A: Template = Template {
    routine: "Full Body A",
    notes: "Machines and a treadmill finisher.",
    title: "Full Body A",
    place: "gym",
    minutes: 45,
    slots: &[
        weight("Leg Press", 90, (80.0, 5.0), (3, 10), &[], None),
        weight("Lat Pulldown", 90, (40.0, 2.5), (3, 10), &[], None),
        weight(
            "Seated Dumbbell Shoulder Press",
            75,
            (12.0, 1.0),
            (3, 10),
            &[],
            None,
        ),
        other(
            "Plank",
            45,
            Dose::Hold {
                sets: 2,
                start: 30,
                step: 5,
            },
        ),
        other(
            "Treadmill Run",
            0,
            Dose::Cardio {
                distance_m: 1600.0,
                start: 660,
                step: 6,
            },
        ),
    ],
};

const CASUAL_B: Template = Template {
    routine: "Full Body B",
    notes: "Lunges, rows and a bike finisher.",
    title: "Full Body B",
    place: "gym",
    minutes: 45,
    slots: &[
        weight("Walking Lunge", 75, (10.0, 1.0), (3, 12), &[], None),
        weight("Seated Cable Row", 90, (35.0, 2.5), (3, 10), &[], None),
        other(
            "Push-Up",
            60,
            Dose::Reps {
                sets: 3,
                start: 8,
                every: 2,
            },
        ),
        weight("Hammer Curl", 60, (10.0, 0.5), (2, 12), &[], None),
        other(
            "Stationary Bike",
            0,
            Dose::Cardio {
                distance_m: 8000.0,
                start: 1080,
                step: 10,
            },
        ),
    ],
};

pub fn routines(kind: Kind) -> &'static [Template] {
    match kind {
        Kind::Demo => &[PUSH, PULL, LEGS, HOME],
        Kind::Casual => &[CASUAL_A, CASUAL_B],
        Kind::Admin | Kind::Newbie => &[],
    }
}

/// A logged set; routine targets use the same values without `rpe` and `completed`
#[derive(Debug, Clone, PartialEq)]
pub struct Set {
    pub kind: &'static str,
    pub reps: Option<i32>,
    pub weight_kg: Option<f64>,
    pub duration_seconds: Option<i32>,
    pub distance_m: Option<f64>,
    pub rpe: Option<f64>,
    pub completed: bool,
}

impl Set {
    fn new(kind: &'static str) -> Self {
        Self {
            kind,
            reps: None,
            weight_kg: None,
            duration_seconds: None,
            distance_m: None,
            rpe: None,
            completed: true,
        }
    }

    pub fn target(&self) -> Value {
        json!({
            "kind": self.kind,
            "reps": self.reps,
            "weight_kg": self.weight_kg,
            "duration_seconds": self.duration_seconds,
            "distance_m": self.distance_m,
        })
    }
}

/// Rounded to the 0.25 kg steps gyms actually have
fn plates(kg: f64) -> f64 {
    (kg * 4.0).round() / 4.0
}

fn sets(dose: Dose, week: i32, deload: bool) -> Vec<Set> {
    match dose {
        Dose::Weight {
            start,
            step,
            sets,
            reps,
            warmups,
            finisher,
        } => {
            let load = start + step * f64::from(week);
            let load = plates(if deload { load * 0.85 } else { load });
            let mut out: Vec<Set> = warmups
                .iter()
                .map(|&(share, reps)| Set {
                    reps: Some(reps),
                    weight_kg: Some(plates(load * share)),
                    ..Set::new("warmup")
                })
                .collect();
            for i in 0..sets {
                let last = i + 1 == sets;
                // Heavier weeks cost a rep on the last set now and then
                let missed = i32::from(last && !deload && (week + 1) % 3 == 0);
                out.push(Set {
                    reps: Some(reps - missed),
                    weight_kg: Some(load),
                    rpe: last.then_some(if deload {
                        6.0
                    } else {
                        7.0 + f64::from(week % 4) * 0.5
                    }),
                    ..Set::new("normal")
                });
            }
            match finisher {
                Some(Finisher::Drop) => out.push(Set {
                    reps: Some(reps + 4),
                    weight_kg: Some(plates(load * 0.6)),
                    rpe: Some(9.5),
                    ..Set::new("drop")
                }),
                Some(Finisher::Failure) => out.push(Set {
                    reps: Some(reps / 2 + 2),
                    weight_kg: Some(load),
                    rpe: Some(10.0),
                    ..Set::new("failure")
                }),
                None => {}
            }
            out
        }
        Dose::Reps { sets, start, every } => (0..sets)
            .map(|_| Set {
                reps: Some(start + week / every),
                ..Set::new("normal")
            })
            .collect(),
        Dose::Hold { sets, start, step } => (0..sets)
            .map(|_| Set {
                duration_seconds: Some(start + step * week),
                ..Set::new("normal")
            })
            .collect(),
        Dose::Cardio {
            distance_m,
            start,
            step,
        } => vec![Set {
            distance_m: Some(distance_m),
            duration_seconds: Some(start - step * week),
            ..Set::new("normal")
        }],
    }
}

pub struct PlannedExercise {
    pub exercise: &'static str,
    pub rest_seconds: i32,
    pub sets: Vec<Set>,
}

impl Template {
    /// The routine's targets: week one of the program.
    pub fn exercises(&self) -> Vec<PlannedExercise> {
        self.session(0, false)
    }

    fn session(&self, week: i32, deload: bool) -> Vec<PlannedExercise> {
        self.slots
            .iter()
            .map(|slot| PlannedExercise {
                exercise: slot.exercise,
                rest_seconds: slot.rest,
                sets: sets(slot.dose, week, deload),
            })
            .collect()
    }
}

pub struct PlannedWorkout {
    /// Unique per account; the workout, exercise and set ids derive from it
    pub key: String,
    pub title: &'static str,
    pub notes: Option<&'static str>,
    pub routine: &'static str,
    pub place: &'static str,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub exercises: Vec<PlannedExercise>,
}

struct Session<'a> {
    template: &'a Template,
    week: i32,
    deload: bool,
}

impl Session<'_> {
    fn at(&self, key: String, started_at: DateTime<Utc>) -> PlannedWorkout {
        // Sessions run a little longer as the weights go up
        let minutes = self.template.minutes + i64::from(self.week % 3) * 4;
        PlannedWorkout {
            key,
            title: self.template.title,
            notes: self
                .deload
                .then_some("Deload week: lighter weights, focus on technique."),
            routine: self.template.routine,
            place: self.template.place,
            started_at,
            ended_at: Some(started_at + Duration::minutes(minutes)),
            exercises: self.template.session(self.week, self.deload),
        }
    }
}

const DEMO_WEEKS: i32 = 12;
const DEMO_DELOAD_WEEK: i32 = 7;
const DEMO_MISSED_LEGS_WEEK: i32 = 4;
const CASUAL_WEEKS: i32 = 6;

/// Days before the base date of weekday slot `day` (1 = the most recent) in `week` of `weeks`.
fn days_ago(weeks: i32, week: i32, day: i64) -> i64 {
    i64::from(weeks - 1 - week) * 7 + day
}

pub fn workouts(kind: Kind, clock: &Clock) -> Vec<PlannedWorkout> {
    let mut out = Vec::new();
    match kind {
        Kind::Demo => {
            for week in 0..DEMO_WEEKS {
                let deload = week == DEMO_DELOAD_WEEK;
                let day = |slot| days_ago(DEMO_WEEKS, week, slot);
                let session = |template| Session {
                    template,
                    week,
                    deload,
                };
                out.push(session(&PUSH).at(format!("w{week}-push"), clock.at(day(7), 7, 0)));
                out.push(session(&PULL).at(format!("w{week}-pull"), clock.at(day(5), 18, 0)));
                if week != DEMO_MISSED_LEGS_WEEK {
                    out.push(session(&LEGS).at(format!("w{week}-legs"), clock.at(day(3), 18, 30)));
                }
                if week % 2 == 1 {
                    let home = Session {
                        template: &HOME,
                        week: week / 2,
                        deload,
                    };
                    out.push(home.at(format!("w{week}-home"), clock.at(day(2), 9, 0)));
                }
            }
            // Earlier today, so "this week" on the dashboard is never empty and the latest
            // session sets new records
            let today = Session {
                template: &PUSH,
                week: DEMO_WEEKS,
                deload: false,
            };
            out.push(today.at("today-push".to_owned(), clock.anchor() - Duration::hours(3)));
        }
        Kind::Casual => {
            for week in 0..CASUAL_WEEKS {
                let day = |slot| days_ago(CASUAL_WEEKS, week, slot);
                let session = |template| Session {
                    template,
                    week,
                    deload: false,
                };
                out.push(session(&CASUAL_A).at(format!("w{week}-a"), clock.at(day(6), 17, 30)));
                out.push(session(&CASUAL_B).at(format!("w{week}-b"), clock.at(day(3), 17, 30)));
            }
            out.push(in_progress(clock));
        }
        Kind::Admin | Kind::Newbie => {}
    }
    out
}

/// Started "on another device" a little over half an hour ago: two sets done, the rest to go.
fn in_progress(clock: &Clock) -> PlannedWorkout {
    let mut exercises = CASUAL_A.session(CASUAL_WEEKS, false);
    exercises.truncate(3);
    for (i, set) in exercises
        .iter_mut()
        .flat_map(|e| e.sets.iter_mut())
        .enumerate()
    {
        set.completed = i < 2;
        if !set.completed {
            set.rpe = None;
        }
    }
    PlannedWorkout {
        key: "in-progress".to_owned(),
        title: CASUAL_A.title,
        notes: None,
        routine: CASUAL_A.routine,
        place: CASUAL_A.place,
        started_at: clock.anchor() - Duration::minutes(35),
        ended_at: None,
        exercises,
    }
}

pub struct PlannedActivity {
    pub key: String,
    pub body: Value,
}

struct Effort {
    avg_heart_rate: i32,
    perceived: i32,
}

fn activity(
    key: String,
    (kind, title): (&str, &str),
    started_at: DateTime<Utc>,
    (duration_seconds, distance_m, elevation_gain_m): (i32, f64, Option<f64>),
    effort: Effort,
) -> PlannedActivity {
    let calories = duration_seconds / 60 * (4 + effort.perceived);
    PlannedActivity {
        key,
        body: json!({
            "kind": kind,
            "title": title,
            "started_at": started_at,
            "duration_seconds": duration_seconds,
            "distance_m": distance_m,
            "elevation_gain_m": elevation_gain_m,
            "avg_heart_rate": effort.avg_heart_rate,
            "calories": calories,
            "perceived_effort": effort.perceived,
        }),
    }
}

const fn effort(avg_heart_rate: i32, perceived: i32) -> Effort {
    Effort {
        avg_heart_rate,
        perceived,
    }
}

#[allow(clippy::cast_possible_truncation)]
pub fn activities(kind: Kind, clock: &Clock) -> Vec<PlannedActivity> {
    let mut out = Vec::new();
    match kind {
        Kind::Demo => {
            for week in 0..DEMO_WEEKS {
                let day = |slot| days_ago(DEMO_WEEKS, week, slot);
                // Easy runs get longer and a little faster
                let km = 5.0 + 0.25 * f64::from(week);
                let seconds = (km * f64::from(330 - week)) as i32;
                out.push(activity(
                    format!("w{week}-easy-run"),
                    ("run", "Easy run"),
                    clock.at(day(6), 6, 30),
                    (seconds, km * 1000.0, Some(35.0)),
                    effort(146, 4),
                ));
                if week % 2 == 0 {
                    out.push(activity(
                        format!("w{week}-tempo-run"),
                        ("run", "Tempo run"),
                        clock.at(day(4), 6, 30),
                        (6 * (295 - week), 6000.0, Some(20.0)),
                        effort(164, 7),
                    ));
                }
                if week % 3 == 1 {
                    out.push(activity(
                        format!("w{week}-swim"),
                        ("swim", "Pool swim"),
                        clock.at(day(4), 12, 15),
                        (38 * 60, 1500.0, None),
                        effort(132, 5),
                    ));
                }
                if week % 4 == 3 {
                    out.push(activity(
                        format!("w{week}-hike"),
                        ("hike", "Ridge hike"),
                        clock.at(day(1), 8, 0),
                        (3 * 3600 + 25 * 60, 12_400.0, Some(760.0)),
                        effort(121, 5),
                    ));
                } else {
                    let km = 35.0 + 1.5 * f64::from(week);
                    out.push(activity(
                        format!("w{week}-ride"),
                        ("ride", "Sunday ride"),
                        clock.at(day(1), 9, 30),
                        (
                            (km / 27.0 * 3600.0) as i32,
                            km * 1000.0,
                            Some(300.0 + 20.0 * f64::from(week)),
                        ),
                        effort(138, 6),
                    ));
                }
            }
        }
        Kind::Casual => {
            for week in 0..CASUAL_WEEKS {
                out.push(activity(
                    format!("w{week}-walk"),
                    ("walk", "Evening walk"),
                    clock.at(days_ago(CASUAL_WEEKS, week, 2), 19, 0),
                    (45 * 60, 4800.0, None),
                    effort(104, 2),
                ));
            }
        }
        Kind::Admin | Kind::Newbie => {}
    }
    out
}

/// Invites for the admin's Invites screen, besides the used ones fixture accounts registered with
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InviteState {
    Active,
    Revoked,
    Expired,
}

pub const INVITES: [(&str, InviteState); 3] = [
    ("Fixture: open invite", InviteState::Active),
    ("Fixture: revoked invite", InviteState::Revoked),
    ("Fixture: expired invite", InviteState::Expired),
];

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn clock() -> Clock {
        let now = Utc
            .with_ymd_and_hms(2026, 9, 28, 8, 0, 0)
            .single()
            .expect("valid date");
        Clock::new(now.date_naive(), now).expect("valid clock")
    }

    #[test]
    fn ids_are_stable_and_distinct() {
        assert_eq!(
            fixture_id("demo/workout/w0-push"),
            fixture_id("demo/workout/w0-push")
        );
        assert_ne!(
            fixture_id("demo/workout/w0-push"),
            fixture_id("demo/workout/w0-pull")
        );
        assert_eq!(fixture_id("a").get_version_num(), 8);
    }

    #[test]
    fn history_stays_in_the_past_and_is_unique() {
        let clock = clock();
        for account in &ACCOUNTS {
            let workouts = workouts(account.kind, &clock);
            let mut keys: Vec<_> = workouts.iter().map(|w| w.key.as_str()).collect();
            keys.sort_unstable();
            keys.dedup();
            assert_eq!(keys.len(), workouts.len(), "{}", account.username);
            for workout in &workouts {
                assert!(workout.started_at <= clock.now, "{}", workout.key);
                assert!(
                    workout
                        .ended_at
                        .is_none_or(|end| end <= clock.now && end > workout.started_at),
                    "{}",
                    workout.key
                );
            }
            for activity in activities(account.kind, &clock) {
                let started: DateTime<Utc> =
                    serde_json::from_value(activity.body["started_at"].clone()).expect("timestamp");
                assert!(started < clock.now, "{}", activity.key);
            }
        }
    }

    #[test]
    fn demo_history_covers_the_program() {
        let clock = clock();
        let workouts = workouts(Kind::Demo, &clock);
        assert_eq!(workouts.len(), 12 * 3 - 1 + 6 + 1);
        let oldest = workouts
            .iter()
            .map(|w| w.started_at)
            .min()
            .expect("workouts");
        assert!(clock.now - oldest > Duration::days(80));

        let kinds: Vec<_> = workouts
            .iter()
            .flat_map(|w| &w.exercises)
            .flat_map(|e| &e.sets)
            .map(|s| s.kind)
            .collect();
        for kind in ["warmup", "normal", "drop", "failure"] {
            assert!(kinds.contains(&kind), "{kind}");
        }
    }

    #[test]
    fn weights_progress_except_on_deload() {
        let bench = |week, deload| sets(PUSH.slots[0].dose, week, deload)[2].weight_kg;
        assert_eq!(bench(0, false), Some(70.0));
        assert_eq!(bench(4, false), Some(80.0));
        assert!(bench(7, true) < bench(6, false));
    }

    #[test]
    fn in_progress_workout_is_open_and_partly_done() {
        let clock = clock();
        let workout = in_progress(&clock);
        assert!(workout.ended_at.is_none());
        assert!(workout.started_at < clock.now);
        let done = workout
            .exercises
            .iter()
            .flat_map(|e| &e.sets)
            .filter(|s| s.completed)
            .count();
        assert_eq!(done, 2);
    }

    #[test]
    fn past_base_date_anchors_in_the_evening() -> Result<()> {
        let now = clock().now;
        let past = Clock::new(NaiveDate::from_ymd_opt(2026, 6, 1).unwrap_or_default(), now)?;
        assert_eq!(past.anchor(), past.at(0, 19, 0));
        assert!(Clock::new(now.date_naive() + Duration::days(1), now).is_err());
        Ok(())
    }
}
