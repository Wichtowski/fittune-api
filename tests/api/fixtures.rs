//! `seed-dev` fixtures, so they break here rather than on a developer's machine when the schema
//! or an API contract changes.

use axum::http::StatusCode;
use chrono::Utc;
use fittune_api::{AppState, config::Registration, fixtures};
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::common::{TestApp, config};

type Counts = (i64, i64, i64, i64, i64, i64, i64, i64);

async fn counts(pool: &PgPool) -> Counts {
    sqlx::query_as(
        "SELECT (SELECT count(*) FROM users), (SELECT count(*) FROM exercises),
                (SELECT count(*) FROM routines), (SELECT count(*) FROM workouts),
                (SELECT count(*) FROM workout_sets), (SELECT count(*) FROM activities),
                (SELECT count(*) FROM workout_place_versions), (SELECT count(*) FROM invites)",
    )
    .fetch_one(pool)
    .await
    .expect("counts")
}

async fn sign_in(app: &TestApp, username: &str) -> String {
    let (status, body) = app
        .post(
            "/api/v1/auth/login",
            None,
            json!({ "login": username, "password": fixtures::PASSWORD }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{username} cannot sign in: {body}");
    body["session"]["token"].as_str().expect("token").to_owned()
}

fn items(page: &Value) -> &Vec<Value> {
    page["items"].as_array().expect("paged items")
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn seeds_a_fresh_database_and_reruns_without_duplicates(pool: PgPool) {
    let state = AppState::new(pool.clone(), config(Registration::InviteOnly));
    let now = Utc::now();
    let clock = fixtures::Clock::new(now.date_naive(), now).expect("clock");

    let first = fixtures::seed(&state, clock).await.expect("first seed");
    assert_eq!(first.accounts_created, fixtures::ACCOUNTS.len());
    assert!(first.active_invite_code.is_some());
    assert!(first.kept_workouts.is_empty());
    let after_first = counts(&pool).await;

    let second = fixtures::seed(&state, clock).await.expect("second seed");
    assert_eq!(second.accounts_created, 0);
    assert_eq!(second.workouts, first.workouts);
    assert_eq!(second.active_invite_code, None);
    assert_eq!(counts(&pool).await, after_first);

    let app = TestApp::new(pool.clone());
    let demo = sign_in(&app, "demo").await;
    let (_, records) = app.get("/api/v1/train/stats/records", &demo).await;
    assert!(!records.as_array().expect("records").is_empty());
    let (_, history) = app
        .get("/api/v1/train/workouts?status=completed&limit=100", &demo)
        .await;
    assert!(items(&history).len() > 40);

    let newbie = sign_in(&app, "newbie").await;
    let (_, empty) = app.get("/api/v1/train/workouts", &newbie).await;
    assert!(items(&empty).is_empty());

    let admin = sign_in(&app, "admin").await;
    let (_, invites) = app.get("/api/v1/admin/invites", &admin).await;
    let mut statuses: Vec<_> = invites
        .as_array()
        .expect("invites")
        .iter()
        .map(|invite| invite["status"].as_str().expect("status").to_owned())
        .collect();
    statuses.sort_unstable();
    statuses.dedup();
    assert_eq!(statuses, ["active", "expired", "revoked", "used"]);

    // Continuing the in-progress workout in the app makes it newer than the fixture
    let casual = sign_in(&app, "casual").await;
    let (_, open) = app
        .get("/api/v1/train/workouts?status=in_progress", &casual)
        .await;
    let open = &items(&open)[0];
    let uri = format!(
        "/api/v1/train/workouts/{}",
        open["id"].as_str().expect("id")
    );
    let (_, mut workout) = app.get(&uri, &casual).await;
    workout["revision"] = json!(5);
    workout["place_version_id"] = workout["place"]["version_id"].clone();
    let (status, body) = app.put(&uri, &casual, workout).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let third = fixtures::seed(&state, clock).await.expect("third seed");
    assert_eq!(third.kept_workouts, ["casual/in-progress"]);
    let (_, kept) = app.get(&uri, &casual).await;
    assert_eq!(kept["revision"], 5);
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn reset_removes_only_fixture_accounts(pool: PgPool) {
    let state = AppState::new(pool.clone(), config(Registration::InviteOnly));
    let now = Utc::now();
    let clock = fixtures::Clock::new(now.date_naive(), now).expect("clock");
    fixtures::seed(&state, clock).await.expect("seed");

    let app = TestApp::new(pool.clone());
    let someone = app.register("someone").await;
    let (status, _) = app
        .put(
            &format!("/api/v1/train/activities/{}", crate::common::uuid()),
            &someone.token,
            json!({ "kind": "run", "title": "Mine", "started_at": now, "duration_seconds": 600 }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);

    let removed = fixtures::reset(&state).await.expect("reset");
    assert_eq!(removed, fixtures::ACCOUNTS.len());
    let (users, _, routines, workouts, _, activities, _, invites) = counts(&pool).await;
    assert_eq!(
        (users, routines, workouts, activities, invites),
        (1, 0, 0, 1, 0)
    );

    let again = fixtures::seed(&state, clock)
        .await
        .expect("seed after reset");
    assert_eq!(again.accounts_created, fixtures::ACCOUNTS.len());
}
