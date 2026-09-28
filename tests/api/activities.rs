use axum::http::StatusCode;
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::common::{TestApp, uuid};

pub fn run(started_at: &str, distance_m: f64) -> Value {
    json!({
        "kind": "run",
        "title": "Easy run",
        "started_at": started_at,
        "duration_seconds": 1800,
        "distance_m": distance_m,
        "avg_heart_rate": 145,
        "perceived_effort": 4
    })
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn put_creates_and_updates_activities(pool: PgPool) {
    let app = TestApp::new(pool);
    let user = app.register("runner").await;
    let uri = format!("/api/v1/train/activities/{}", uuid());

    let (status, created) = app
        .put(&uri, &user.token, run("2026-09-26T06:00:00Z", 5000.0))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["kind"], "run");
    assert_eq!(created["distance_m"], 5000.0);

    let mut edited = run("2026-09-26T06:00:00Z", 5200.0);
    edited["title"] = json!("Tempo run");
    let (status, updated) = app.put(&uri, &user.token, edited).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["title"], "Tempo run");
    assert_eq!(updated["created_at"], created["created_at"]);

    let (status, fetched) = app.get(&uri, &user.token).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(fetched["distance_m"], 5200.0);
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn activities_validate_and_stay_private(pool: PgPool) {
    let app = TestApp::new(pool);
    let owner = app.register("owner").await;
    let other = app.register("other").await;
    let uri = format!("/api/v1/train/activities/{}", uuid());

    let mut invalid = run("2026-09-26T06:00:00Z", -1.0);
    invalid["duration_seconds"] = json!(0);
    let (status, body) = app.put(&uri, &owner.token, invalid).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(body["fields"]["distance_m"].is_string());
    assert!(body["fields"]["duration_seconds"].is_string());

    app.put(&uri, &owner.token, run("2026-09-26T06:00:00Z", 5000.0))
        .await;
    assert_eq!(app.get(&uri, &other.token).await.0, StatusCode::NOT_FOUND);
    let (status, _) = app
        .put(&uri, &other.token, run("2026-09-26T06:00:00Z", 1.0))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(app.get(&uri, &owner.token).await.1["distance_m"], 5000.0);

    assert_eq!(
        app.delete(&uri, &other.token).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.delete(&uri, &owner.token).await.0,
        StatusCode::NO_CONTENT
    );
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn list_filters_by_kind_and_paginates(pool: PgPool) {
    let app = TestApp::new(pool);
    let user = app.register("runner").await;
    for day in 1..=3 {
        let start = format!("2026-09-{day:02}T06:00:00Z");
        app.put(
            &format!("/api/v1/train/activities/{}", uuid()),
            &user.token,
            run(&start, 5000.0),
        )
        .await;
    }
    let mut ride = run("2026-09-04T06:00:00Z", 30_000.0);
    ride["kind"] = json!("ride");
    app.put(
        &format!("/api/v1/train/activities/{}", uuid()),
        &user.token,
        ride,
    )
    .await;

    let (_, all) = app.get("/api/v1/train/activities", &user.token).await;
    assert_eq!(all["items"].as_array().map(Vec::len), Some(4));
    assert_eq!(all["items"][0]["kind"], "ride");

    let (_, runs) = app
        .get("/api/v1/train/activities?kind=run&limit=2", &user.token)
        .await;
    assert_eq!(runs["items"].as_array().map(Vec::len), Some(2));
    let cursor = runs["next_cursor"].as_str().expect("next page");
    let (_, rest) = app
        .get(
            &format!("/api/v1/train/activities?kind=run&limit=2&cursor={cursor}"),
            &user.token,
        )
        .await;
    assert_eq!(rest["items"][0]["started_at"], "2026-09-01T06:00:00Z");
    assert!(rest["next_cursor"].is_null());
}
