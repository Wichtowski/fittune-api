use axum::http::StatusCode;
use serde_json::json;
use sqlx::PgPool;

use crate::{
    common::{TestApp, uuid},
    workouts::workout,
};

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn routine_crud_round_trip(pool: PgPool) {
    let app = TestApp::new(pool);
    let user = app.register("planner").await;
    let squat = app.catalog_exercise("Barbell Back Squat").await;
    let plank = app.catalog_exercise("Plank").await;

    let (status, routine) = app
        .post(
            "/api/v1/routines",
            Some(&user.token),
            json!({
                "name": "Leg Day",
                "notes": "Brace hard",
                "exercises": [
                    { "exercise_id": squat, "rest_seconds": 180,
                      "sets": [{ "kind": "warmup", "reps": 8, "weight_kg": 60 }, { "reps": 5, "weight_kg": 120 }] },
                    { "exercise_id": plank, "sets": [{ "duration_seconds": 60 }] }
                ]
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{routine}");
    assert_eq!(
        routine["exercises"][0]["exercise_name"],
        "Barbell Back Squat"
    );
    assert_eq!(routine["exercises"][0]["sets"][0]["kind"], "warmup");
    assert_eq!(routine["exercises"][0]["sets"][1]["kind"], "normal");
    assert_eq!(routine["exercises"][1]["tracking"], "duration");
    assert!(routine["last_performed_at"].is_null());
    let id = routine["id"].as_str().expect("id");
    let uri = format!("/api/v1/routines/{id}");

    let (status, updated) = app
        .put(
            &uri,
            &user.token,
            json!({ "name": "Leg Day B", "exercises": [{ "exercise_id": plank }] }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["name"], "Leg Day B");
    assert_eq!(updated["exercises"].as_array().map(Vec::len), Some(1));

    // Finishing a workout started from the routine records when it was last performed.
    let mut session = workout(
        &squat,
        "2026-09-22T07:00:00Z",
        Some("2026-09-22T08:00:00Z"),
        1,
    );
    session["routine_id"] = json!(id);
    app.put(
        &format!("/api/v1/workouts/{}", uuid()),
        &user.token,
        session,
    )
    .await;

    let (_, list) = app.get("/api/v1/routines", &user.token).await;
    assert_eq!(list[0]["last_performed_at"], "2026-09-22T07:00:00Z");

    assert_eq!(
        app.delete(&uri, &user.token).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(app.get(&uri, &user.token).await.0, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn routines_are_private(pool: PgPool) {
    let app = TestApp::new(pool);
    let owner = app.register("owner").await;
    let other = app.register("other").await;
    let (_, routine) = app
        .post(
            "/api/v1/routines",
            Some(&owner.token),
            json!({ "name": "Mine" }),
        )
        .await;
    let uri = format!("/api/v1/routines/{}", routine["id"].as_str().expect("id"));

    assert_eq!(app.get(&uri, &other.token).await.0, StatusCode::NOT_FOUND);
    assert_eq!(
        app.put(&uri, &other.token, json!({ "name": "Stolen" }))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.delete(&uri, &other.token).await.0,
        StatusCode::NOT_FOUND
    );
    let (_, list) = app.get("/api/v1/routines", &other.token).await;
    assert_eq!(list, json!([]));
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn routines_validate_their_contents(pool: PgPool) {
    let app = TestApp::new(pool);
    let user = app.register("planner").await;

    let (status, body) = app
        .post(
            "/api/v1/routines",
            Some(&user.token),
            json!({ "name": " ", "exercises": [{ "exercise_id": uuid() }] }),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["fields"]["name"], "Name is required");

    let (status, body) = app
        .post(
            "/api/v1/routines",
            Some(&user.token),
            json!({ "name": "Ghost", "exercises": [{ "exercise_id": uuid() }] }),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        body["fields"]["exercises"],
        "Routine references an unknown exercise"
    );
}
