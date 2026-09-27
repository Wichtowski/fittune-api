use axum::http::StatusCode;
use serde_json::json;
use sqlx::PgPool;

use crate::{
    activities::run,
    common::{TestApp, TestUser, uuid},
    workouts::workout,
};

/// Two finished bench workouts in consecutive weeks, one run and one unfinished workout.
async fn seed(app: &TestApp, user: &TestUser) {
    let bench = app.catalog_exercise("Barbell Bench Press").await;
    for (start, end) in [
        ("2026-09-08T17:00:00Z", Some("2026-09-08T18:00:00Z")),
        ("2026-09-15T17:00:00Z", Some("2026-09-15T18:30:00Z")),
        ("2026-09-16T17:00:00Z", None),
    ] {
        app.put(
            &format!("/api/v1/workouts/{}", uuid()),
            &user.token,
            workout(&bench, start, end, 1),
        )
        .await;
    }
    app.put(
        &format!("/api/v1/activities/{}", uuid()),
        &user.token,
        run("2026-09-17T06:00:00Z", 8000.0),
    )
    .await;
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn overview_compares_with_previous_period(pool: PgPool) {
    let app = TestApp::new(pool);
    let user = app.register("athlete").await;
    seed(&app, &user).await;

    let (status, overview) = app
        .get(
            "/api/v1/stats/overview?from=2026-09-14&to=2026-09-20&tz=Europe/Warsaw",
            &user.token,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{overview}");
    assert_eq!(
        overview["current"],
        json!({
            "workouts": 1, "workout_seconds": 5400, "sets": 2, "reps": 10, "volume_kg": 1000.0,
            "activities": 1, "activity_seconds": 1800, "activity_distance_m": 8000.0
        })
    );
    assert_eq!(overview["previous"]["workouts"], 1);
    assert_eq!(overview["previous"]["workout_seconds"], 3600);
    assert_eq!(overview["previous"]["activities"], 0);
    assert!(overview["streak_weeks"].is_u64());
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn timeline_is_zero_filled(pool: PgPool) {
    let app = TestApp::new(pool);
    let user = app.register("athlete").await;
    seed(&app, &user).await;

    let (status, points) = app
        .get(
            "/api/v1/stats/timeline?from=2026-08-31&to=2026-09-27&bucket=week&tz=UTC",
            &user.token,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{points}");
    let points = points.as_array().expect("points");
    let buckets: Vec<_> = points
        .iter()
        .map(|p| p["bucket"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(
        buckets,
        ["2026-08-31", "2026-09-07", "2026-09-14", "2026-09-21"]
    );
    let volumes: Vec<_> = points
        .iter()
        .map(|p| p["volume_kg"].as_f64().unwrap_or(-1.0))
        .collect();
    assert_eq!(volumes, [0.0, 1000.0, 1000.0, 0.0]);
    assert_eq!(points[2]["activity_distance_m"], 8000.0);

    let (_, months) = app
        .get(
            "/api/v1/stats/timeline?from=2026-08-01&to=2026-09-30&bucket=month",
            &user.token,
        )
        .await;
    assert_eq!(months[1]["workouts"], 2);
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn muscles_and_records_summarise_training(pool: PgPool) {
    let app = TestApp::new(pool);
    let user = app.register("athlete").await;
    seed(&app, &user).await;

    let (_, muscles) = app
        .get(
            "/api/v1/stats/muscles?from=2026-09-01&to=2026-09-30",
            &user.token,
        )
        .await;
    assert_eq!(
        muscles,
        json!([{ "muscle": "chest", "sets": 4, "volume_kg": 2000.0 }])
    );

    let (_, records) = app.get("/api/v1/stats/records", &user.token).await;
    assert_eq!(records.as_array().map(Vec::len), Some(1));
    assert_eq!(records[0]["exercise_name"], "Barbell Bench Press");
    assert_eq!(records[0]["max_weight_kg"], 100.0);
    assert_eq!(records[0]["sessions"], 2);
    assert_eq!(records[0]["last_performed_at"], "2026-09-15T17:00:00Z");
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn stats_validate_period_and_time_zone(pool: PgPool) {
    let app = TestApp::new(pool);
    let user = app.register("athlete").await;

    let (status, body) = app
        .get(
            "/api/v1/stats/overview?from=2026-09-20&to=2026-09-01",
            &user.token,
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(body["fields"]["from"].is_string());

    let (status, body) = app
        .get(
            "/api/v1/stats/overview?from=2026-09-01&to=2026-09-20&tz=Mars/Olympus",
            &user.token,
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["fields"]["tz"], "Unknown time zone");
}
