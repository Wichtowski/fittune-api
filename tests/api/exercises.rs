use axum::http::StatusCode;
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::common::TestApp;

fn names(body: &Value) -> Vec<&str> {
    body.as_array()
        .expect("array")
        .iter()
        .filter_map(|e| e["name"].as_str())
        .collect()
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn catalog_is_seeded_and_filterable(pool: PgPool) {
    let app = TestApp::new(pool);
    let user = app.register("lifter").await;

    let (status, all) = app.get("/api/v1/train/exercises", &user.token).await;
    assert_eq!(status, StatusCode::OK);
    assert!(all.as_array().map_or(0, Vec::len) >= 40);
    let bench = all
        .as_array()
        .and_then(|list| list.iter().find(|e| e["name"] == "Barbell Bench Press"))
        .expect("bench press in catalog");
    assert_eq!(bench["tracking"], "weight_reps");
    assert_eq!(bench["primary_muscle"], "chest");
    assert_eq!(bench["is_custom"], false);

    let (_, curls) = app
        .get(
            "/api/v1/train/exercises?q=curl&equipment=dumbbell",
            &user.token,
        )
        .await;
    assert_eq!(
        names(&curls),
        ["Hammer Curl", "Incline Dumbbell Curl", "Wrist Curl"]
    );

    let (_, triceps) = app
        .get("/api/v1/train/exercises?muscle=triceps", &user.token)
        .await;
    let triceps = names(&triceps);
    assert!(
        triceps.contains(&"Triceps Pushdown"),
        "primary muscle matches"
    );
    assert!(
        triceps.contains(&"Barbell Bench Press"),
        "secondary muscle matches"
    );

    let (status, _) = app
        .get("/api/v1/train/exercises?muscle=wings", &user.token)
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn custom_exercises_are_private_to_their_owner(pool: PgPool) {
    let app = TestApp::new(pool);
    let owner = app.register("owner").await;
    let other = app.register("other").await;

    let (status, created) = app
        .post(
            "/api/v1/train/exercises",
            Some(&owner.token),
            json!({ "name": "Sled Push", "tracking": "distance_duration", "primary_muscle": "quadriceps",
                    "equipment": "other", "secondary_muscles": ["glutes", "calves"] }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["is_custom"], true);
    let id = created["id"].as_str().expect("id");

    let (_, list) = app
        .get("/api/v1/train/exercises?q=sled", &owner.token)
        .await;
    assert_eq!(names(&list), ["Sled Push"]);
    let (_, list) = app
        .get("/api/v1/train/exercises?q=sled", &other.token)
        .await;
    assert!(names(&list).is_empty());
    assert_eq!(
        app.get(&format!("/api/v1/train/exercises/{id}"), &other.token)
            .await
            .0,
        StatusCode::NOT_FOUND
    );

    let (status, body) = app
        .post(
            "/api/v1/train/exercises",
            Some(&owner.token),
            json!({ "name": "sled push", "tracking": "duration", "primary_muscle": "quadriceps" }),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        body["fields"]["name"],
        "An exercise with this name already exists"
    );
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn owners_can_update_and_archive_custom_exercises(pool: PgPool) {
    let app = TestApp::new(pool);
    let user = app.register("owner").await;
    let (_, created) = app
        .post(
            "/api/v1/train/exercises",
            Some(&user.token),
            json!({ "name": "Landmine Press", "tracking": "weight_reps", "primary_muscle": "shoulders" }),
        )
        .await;
    let uri = format!(
        "/api/v1/train/exercises/{}",
        created["id"].as_str().expect("id")
    );

    let (status, updated) = app
        .put(
            &uri,
            &user.token,
            json!({ "name": "Half-Kneeling Landmine Press", "tracking": "weight_reps",
                    "primary_muscle": "shoulders", "equipment": "barbell", "video_id": "dQw4w9WgXcQ" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["equipment"], "barbell");
    assert_eq!(updated["video_id"], "dQw4w9WgXcQ");

    assert_eq!(
        app.delete(&uri, &user.token).await.0,
        StatusCode::NO_CONTENT
    );
    let (_, list) = app
        .get("/api/v1/train/exercises?q=landmine", &user.token)
        .await;
    assert!(
        names(&list).is_empty(),
        "archived exercises leave the library"
    );
    let (status, archived) = app.get(&uri, &user.token).await;
    assert_eq!(status, StatusCode::OK, "but remain readable for history");
    assert!(archived["archived_at"].is_string());
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn only_admins_manage_the_catalog(pool: PgPool) {
    let app = TestApp::new(pool);
    let user = app.register("user").await;
    let admin = app.register("admin").await;
    app.make_admin(&admin).await;
    let bench = app.catalog_exercise("Barbell Bench Press").await;
    let edit = json!({ "name": "Barbell Bench Press", "tracking": "weight_reps", "primary_muscle": "chest",
                       "equipment": "barbell", "difficulty": "intermediate" });

    let uri = format!("/api/v1/train/exercises/{bench}");
    assert_eq!(
        app.put(&uri, &user.token, edit.clone()).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(app.delete(&uri, &user.token).await.0, StatusCode::FORBIDDEN);
    assert_eq!(app.put(&uri, &admin.token, edit).await.0, StatusCode::OK);

    let global = json!({ "name": "Pendlay Row", "tracking": "weight_reps", "primary_muscle": "upper_back",
                         "global": true });
    let (status, _) = app
        .post("/api/v1/train/exercises", Some(&user.token), global.clone())
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, created) = app
        .post("/api/v1/train/exercises", Some(&admin.token), global)
        .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(created["is_custom"], false);

    let (_, list) = app
        .get("/api/v1/train/exercises?q=pendlay", &user.token)
        .await;
    assert_eq!(
        names(&list),
        ["Pendlay Row"],
        "catalog additions are visible to everyone"
    );
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn exercises_list_the_equipment_they_require(pool: PgPool) {
    let app = TestApp::new(pool);
    let user = app.register("gearuser").await;
    for (name, requires) in [
        (
            "Barbell Bench Press",
            json!(["barbell", "flat_bench", "squat_rack"]),
        ),
        ("Pull-Up", json!(["pull_up_bar"])),
        ("Leg Press", json!(["leg_press"])),
        ("Push-Up", json!([])),
    ] {
        let id = app.catalog_exercise(name).await;
        let (_, exercise) = app
            .get(&format!("/api/v1/train/exercises/{id}"), &user.token)
            .await;
        assert_eq!(exercise["requires"], requires, "{name}");
    }

    let (status, created) = app
        .post(
            "/api/v1/train/exercises",
            Some(&user.token),
            json!({ "name": "Landmine Row", "tracking": "weight_reps", "primary_muscle": "upper_back",
                    "equipment": "barbell", "requires": ["squat_rack", "barbell", "barbell"] }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["requires"], json!(["barbell", "squat_rack"]));

    let (status, _) = app
        .post(
            "/api/v1/train/exercises",
            Some(&user.token),
            json!({ "name": "Mystery Move", "tracking": "reps", "primary_muscle": "abs",
                    "requires": ["hoverboard"] }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
