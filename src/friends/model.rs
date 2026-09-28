use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{activities::model::Activity, workouts::model::WorkoutSummary};

/// The only profile fields other users ever see
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct PublicUser {
    pub id: Uuid,
    pub username: String,
    pub display_name: Option<String>,
}

/// What a user lets friends see. Everything is private until the user opts in
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
#[serde(deny_unknown_fields)]
pub struct Sharing {
    pub workouts: bool,
    pub activities: bool,
    pub stats: bool,
    pub personal_records: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resource {
    Workouts,
    Activities,
    Stats,
    PersonalRecords,
}

impl Sharing {
    pub fn allows(&self, resource: Resource) -> bool {
        match resource {
            Resource::Workouts => self.workouts,
            Resource::Activities => self.activities,
            Resource::Stats => self.stats,
            Resource::PersonalRecords => self.personal_records,
        }
    }
}

/// An accepted friend and what they share with the viewer
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Friend {
    #[sqlx(flatten)]
    pub user: PublicUser,
    pub since: DateTime<Utc>,
    #[sqlx(flatten)]
    pub sharing: Sharing,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct FriendRequest {
    #[sqlx(flatten)]
    pub user: PublicUser,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct FriendRequests {
    pub incoming: Vec<FriendRequest>,
    pub outgoing: Vec<FriendRequest>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct BlockedUser {
    #[sqlx(flatten)]
    pub user: PublicUser,
    pub created_at: DateTime<Utc>,
}

/// How the viewer relates to another user
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Relationship {
    #[serde(rename = "self")]
    Myself,
    None,
    /// The viewer sent a request that is still pending
    Outgoing,
    /// The other user sent a request that is still pending
    Incoming,
    Friends,
}

#[derive(Debug, Serialize)]
pub struct UserWithRelationship {
    pub user: PublicUser,
    pub relationship: Relationship,
}

/// A finished workout as friends see it: no place, routine or notes
#[derive(Debug, Serialize)]
pub struct FriendWorkout {
    pub id: Uuid,
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

impl From<WorkoutSummary> for FriendWorkout {
    fn from(w: WorkoutSummary) -> Self {
        Self {
            id: w.id,
            title: w.title,
            started_at: w.started_at,
            ended_at: w.ended_at,
            duration_seconds: w.duration_seconds,
            exercise_count: w.exercise_count,
            set_count: w.set_count,
            total_reps: w.total_reps,
            volume_kg: w.volume_kg,
            exercise_names: w.exercise_names,
        }
    }
}

/// An activity as friends see it: no notes or health data
#[derive(Debug, Serialize)]
pub struct FriendActivity {
    pub id: Uuid,
    pub kind: crate::activities::model::ActivityKind,
    pub title: String,
    pub started_at: DateTime<Utc>,
    pub duration_seconds: i32,
    pub distance_m: Option<f64>,
    pub elevation_gain_m: Option<f64>,
}

impl From<Activity> for FriendActivity {
    fn from(a: Activity) -> Self {
        Self {
            id: a.id,
            kind: a.kind,
            title: a.title,
            started_at: a.started_at,
            duration_seconds: a.duration_seconds,
            distance_m: a.distance_m,
            elevation_gain_m: a.elevation_gain_m,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FeedItem {
    Workout(FriendWorkout),
    Activity(FriendActivity),
}

impl FeedItem {
    pub fn key(&self) -> (DateTime<Utc>, Uuid) {
        match self {
            Self::Workout(w) => (w.started_at, w.id),
            Self::Activity(a) => (a.started_at, a.id),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct FeedEntry {
    pub user: PublicUser,
    #[serde(flatten)]
    pub item: FeedItem,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sharing_defaults_to_private() {
        let sharing = Sharing::default();
        for resource in [
            Resource::Workouts,
            Resource::Activities,
            Resource::Stats,
            Resource::PersonalRecords,
        ] {
            assert!(!sharing.allows(resource));
        }
    }

    #[test]
    fn sharing_rejects_unknown_fields() {
        let json = r#"{"workouts":true,"activities":false,"stats":false,"personal_records":false,"photos":true}"#;
        assert!(serde_json::from_str::<Sharing>(json).is_err());
    }
}
