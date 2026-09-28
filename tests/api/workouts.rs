use axum::http::StatusCode;
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::common::{TestApp, uuid};

/// A finished workout with one bench press exercise: a warm-up and two working sets.
pub fn workout(
    exercise_id: &str,
    started_at: &str,
    ended_at: Option<&str>,
    revision: i64,
) -> Value {
    json!({
        "title": "Push Day",
        "started_at": started_at,
        "ended_at": ended_at,
        "revision": revision,
        "exercises": [{
            "id": uuid(),
            "exercise_id": exercise_id,
            "rest_seconds": 120,
            "sets": [
                { "id": uuid(), "kind": "warmup", "reps": 10, "weight_kg": 40.0, "completed": true },
                { "id": uuid(), "reps": 5, "weight_kg": 100.0, "rpe": 8.0, "completed": true },
                { "id": uuid(), "reps": 5, "weight_kg": 100.0, "completed": true },
                { "id": uuid(), "reps": 5, "weight_kg": 100.0, "completed": false }
            ]
        }]
    })
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn put_creates_then_replaces_a_workout(pool: PgPool) {
    let app = TestApp::new(pool);
    let user = app.register("lifter").await;
    let bench = app.catalog_exercise("Barbell Bench Press").await;
    let id = uuid();
    let uri = format!("/api/v1/train/workouts/{id}");

    let draft = workout(&bench, "2026-09-20T17:00:00Z", None, 1);
    let (status, created) = app.put(&uri, &user.token, draft.clone()).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["id"], id.as_str());
    assert!(created["ended_at"].is_null());
    assert_eq!(
        created["exercises"][0]["exercise_name"],
        "Barbell Bench Press"
    );
    assert_eq!(
        created["exercises"][0]["sets"].as_array().map(Vec::len),
        Some(4)
    );
    assert_eq!(created["exercises"][0]["sets"][0]["kind"], "warmup");

    // Replaying the same revision is idempotent.
    let (status, _) = app.put(&uri, &user.token, draft).await;
    assert_eq!(status, StatusCode::OK);

    // A newer revision replaces the whole document.
    let mut finished = workout(
        &bench,
        "2026-09-20T17:00:00Z",
        Some("2026-09-20T18:05:00Z"),
        5,
    );
    finished["title"] = json!("Heavy Push");
    let (status, updated) = app.put(&uri, &user.token, finished).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["title"], "Heavy Push");
    assert_eq!(updated["revision"], 5);
    assert_eq!(updated["ended_at"], "2026-09-20T18:05:00Z");

    let (status, fetched) = app.get(&uri, &user.token).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(fetched, updated);
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn stale_revisions_are_rejected(pool: PgPool) {
    let app = TestApp::new(pool);
    let user = app.register("lifter").await;
    let bench = app.catalog_exercise("Barbell Bench Press").await;
    let uri = format!("/api/v1/train/workouts/{}", uuid());

    app.put(
        &uri,
        &user.token,
        workout(&bench, "2026-09-20T17:00:00Z", None, 7),
    )
    .await;
    let (status, body) = app
        .put(
            &uri,
            &user.token,
            workout(&bench, "2026-09-20T17:00:00Z", None, 6),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "conflict");

    let (_, stored) = app.get(&uri, &user.token).await;
    assert_eq!(stored["revision"], 7);
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn workouts_are_isolated_between_users(pool: PgPool) {
    let app = TestApp::new(pool);
    let owner = app.register("owner").await;
    let intruder = app.register("intruder").await;
    let bench = app.catalog_exercise("Barbell Bench Press").await;
    let uri = format!("/api/v1/train/workouts/{}", uuid());

    app.put(
        &uri,
        &owner.token,
        workout(&bench, "2026-09-20T17:00:00Z", None, 1),
    )
    .await;

    assert_eq!(
        app.get(&uri, &intruder.token).await.0,
        StatusCode::NOT_FOUND
    );
    let overwrite = workout(&bench, "2026-09-20T17:00:00Z", None, 99);
    assert_eq!(
        app.put(&uri, &intruder.token, overwrite).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.delete(&uri, &intruder.token).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(app.get(&uri, &owner.token).await.1["revision"], 1);
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn workouts_cannot_reference_other_users_exercises(pool: PgPool) {
    let app = TestApp::new(pool);
    let owner = app.register("owner").await;
    let other = app.register("other").await;
    let (_, custom) = app
        .post(
            "/api/v1/train/exercises",
            Some(&owner.token),
            json!({ "name": "Secret Lift", "tracking": "weight_reps", "primary_muscle": "chest" }),
        )
        .await;
    let custom_id = custom["id"].as_str().expect("id");

    let uri = format!("/api/v1/train/workouts/{}", uuid());
    let (status, body) = app
        .put(
            &uri,
            &other.token,
            workout(custom_id, "2026-09-20T17:00:00Z", None, 1),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        body["fields"]["exercises"],
        "Workout references an unknown exercise"
    );
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn list_paginates_and_filters_by_status(pool: PgPool) {
    let app = TestApp::new(pool);
    let user = app.register("lifter").await;
    let bench = app.catalog_exercise("Barbell Bench Press").await;

    for day in 1..=5 {
        let start = format!("2026-09-{day:02}T17:00:00Z");
        let end = format!("2026-09-{day:02}T18:00:00Z");
        let ended = (day != 5).then_some(end.as_str());
        app.put(
            &format!("/api/v1/train/workouts/{}", uuid()),
            &user.token,
            workout(&bench, &start, ended, 1),
        )
        .await;
    }

    let (status, page) = app
        .get(
            "/api/v1/train/workouts?status=completed&limit=3",
            &user.token,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let items = page["items"].as_array().expect("items");
    assert_eq!(items.len(), 3);
    assert_eq!(items[0]["started_at"], "2026-09-04T17:00:00Z");
    assert_eq!(items[0]["duration_seconds"], 3600);
    assert_eq!(items[0]["exercise_names"], json!(["Barbell Bench Press"]));
    assert_eq!(
        items[0]["set_count"], 2,
        "warm-ups and incomplete sets are excluded"
    );
    assert_eq!(items[0]["total_reps"], 10);
    assert_eq!(items[0]["volume_kg"], 1000.0);

    let cursor = page["next_cursor"].as_str().expect("more pages");
    let (_, next) = app
        .get(
            &format!("/api/v1/train/workouts?status=completed&limit=3&cursor={cursor}"),
            &user.token,
        )
        .await;
    assert_eq!(next["items"].as_array().map(Vec::len), Some(1));
    assert!(next["next_cursor"].is_null());

    let (_, active) = app
        .get("/api/v1/train/workouts?status=in_progress", &user.token)
        .await;
    assert_eq!(active["items"].as_array().map(Vec::len), Some(1));
    assert_eq!(active["items"][0]["started_at"], "2026-09-05T17:00:00Z");
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn delete_removes_workout(pool: PgPool) {
    let app = TestApp::new(pool);
    let user = app.register("lifter").await;
    let bench = app.catalog_exercise("Barbell Bench Press").await;
    let uri = format!("/api/v1/train/workouts/{}", uuid());
    app.put(
        &uri,
        &user.token,
        workout(&bench, "2026-09-20T17:00:00Z", None, 1),
    )
    .await;

    assert_eq!(
        app.delete(&uri, &user.token).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(app.get(&uri, &user.token).await.0, StatusCode::NOT_FOUND);
    assert_eq!(app.delete(&uri, &user.token).await.0, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn exercise_history_reports_sessions_and_records(pool: PgPool) {
    let app = TestApp::new(pool);
    let user = app.register("lifter").await;
    let bench = app.catalog_exercise("Barbell Bench Press").await;

    app.put(
        &format!("/api/v1/train/workouts/{}", uuid()),
        &user.token,
        workout(
            &bench,
            "2026-09-10T17:00:00Z",
            Some("2026-09-10T18:00:00Z"),
            1,
        ),
    )
    .await;
    let mut heavier = workout(
        &bench,
        "2026-09-17T17:00:00Z",
        Some("2026-09-17T18:00:00Z"),
        1,
    );
    heavier["exercises"][0]["sets"][1]["weight_kg"] = json!(110.0);
    heavier["exercises"][0]["sets"][1]["reps"] = json!(3);
    let heavier_id = uuid();
    app.put(
        &format!("/api/v1/train/workouts/{heavier_id}"),
        &user.token,
        heavier,
    )
    .await;
    // In-progress workouts never count towards history.
    app.put(
        &format!("/api/v1/train/workouts/{}", uuid()),
        &user.token,
        workout(&bench, "2026-09-24T17:00:00Z", None, 1),
    )
    .await;

    let (status, history) = app
        .get(
            &format!("/api/v1/train/exercises/{bench}/history"),
            &user.token,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(history["exercise"]["name"], "Barbell Bench Press");
    let sessions = history["sessions"].as_array().expect("sessions");
    assert_eq!(sessions.len(), 2);
    assert_eq!(sessions[0]["workout_id"], heavier_id.as_str());
    assert_eq!(
        sessions[0]["sets"].as_array().map(Vec::len),
        Some(3),
        "only completed sets"
    );
    assert_eq!(sessions[1]["volume_kg"], 1000.0);

    let records = &history["records"];
    assert_eq!(records["max_weight_kg"]["value"], 110.0);
    assert_eq!(records["max_weight_kg"]["workout_id"], heavier_id.as_str());
    assert_eq!(records["best_session_volume_kg"]["value"], 1000.0);
    let e1rm = records["best_e1rm_kg"]["value"].as_f64().expect("e1rm");
    assert!((e1rm - 121.0).abs() < 1e-9, "110 x 3 -> 121kg, got {e1rm}");
}
