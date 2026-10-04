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
    assert!(all.as_array().map_or(0, Vec::len) >= 1300);
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
    let curl_names = names(&curls);
    for name in ["Hammer Curl", "Incline Dumbbell Curl", "Wrist Curl"] {
        assert!(curl_names.contains(&name), "{name}");
    }
    for curl in curls.as_array().expect("array") {
        assert_eq!(curl["equipment"], "dumbbell", "{curl}");
        let name = curl["name"].as_str().expect("name").to_lowercase();
        assert!(name.contains("curl"), "{name}");
    }

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
async fn created_exercises_are_shared_and_credited(pool: PgPool) {
    let app = TestApp::new(pool);
    let owner = app.register("owner").await;
    let other = app.register("other").await;

    let (status, created) = app
        .post(
            "/api/v1/train/exercises",
            Some(&owner.token),
            json!({ "name": "Prowler Push", "tracking": "distance_duration", "primary_muscle": "quadriceps",
                    "equipment": "other", "secondary_muscles": ["glutes", "calves"] }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["is_custom"], true);
    let id = created["id"].as_str().expect("id");

    let (_, list) = app
        .get("/api/v1/train/exercises?q=prowler", &owner.token)
        .await;
    assert_eq!(names(&list), ["Prowler Push"]);
    assert_eq!(created["is_own"], true);
    assert!(
        created.get("owner_id").is_none(),
        "user ids are not handed out with exercises"
    );
    assert_eq!(
        created["created_by"], "owner",
        "the username until a display name is set"
    );

    // Everyone can find and use it, and sees who made it
    app.request(
        axum::http::Method::PATCH,
        "/api/v1/me",
        Some(&owner.token),
        Some(json!({ "display_name": "Olga Owner" })),
    )
    .await;
    let (_, list) = app
        .get("/api/v1/train/exercises?q=prowler", &other.token)
        .await;
    assert_eq!(names(&list), ["Prowler Push"]);
    assert_eq!(list[0]["created_by"], "Olga Owner");
    assert_eq!(list[0]["is_own"], false);
    let uri = format!("/api/v1/train/exercises/{id}");
    let (status, seen) = app.get(&uri, &other.token).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(seen["is_custom"], true);

    // But only its owner or an admin changes it
    let edit =
        json!({ "name": "Prowler Push", "tracking": "reps", "primary_muscle": "quadriceps" });
    assert_eq!(
        app.put(&uri, &other.token, edit.clone()).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.delete(&uri, &other.token).await.0,
        StatusCode::FORBIDDEN
    );
    let admin = app.register("moderator").await;
    app.make_admin(&admin).await;
    let (status, moderated) = app.put(&uri, &admin.token, edit).await;
    assert_eq!(status, StatusCode::OK, "{moderated}");
    assert_eq!(moderated["tracking"], "reps");
    assert_eq!(moderated["is_own"], false);
    let (_, kept) = app.get(&uri, &owner.token).await;
    assert_eq!(
        kept["is_own"], true,
        "an admin's edit does not take the exercise over"
    );

    // Names are unique per owner, so someone else can have their own of the same name
    let (status, twin) = app
        .post(
            "/api/v1/train/exercises",
            Some(&other.token),
            json!({ "name": "Prowler Push", "tracking": "duration", "primary_muscle": "quadriceps" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{twin}");
    let (_, list) = app
        .get("/api/v1/train/exercises?q=prowler", &other.token)
        .await;
    assert_eq!(list.as_array().map(Vec::len), Some(2));
    let bench = app.catalog_exercise("Barbell Bench Press").await;
    let (_, catalog) = app
        .get(&format!("/api/v1/train/exercises/{bench}"), &other.token)
        .await;
    assert_eq!(catalog["is_own"], false);
    assert_eq!(catalog["created_by"], serde_json::Value::Null);

    // The owner cannot have two of the same name
    let (status, body) = app
        .post(
            "/api/v1/train/exercises",
            Some(&owner.token),
            json!({ "name": "prowler push", "tracking": "duration", "primary_muscle": "quadriceps" }),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        body["fields"]["name"],
        "An exercise with this name already exists"
    );
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn exercises_created_as_private_stay_private(pool: PgPool) {
    let app = TestApp::new(pool.clone());
    let owner = app.register("owner").await;
    let other = app.register("other").await;
    let admin = app.register("moderator").await;
    app.make_admin(&admin).await;
    let (_, created) = app
        .post(
            "/api/v1/train/exercises",
            Some(&owner.token),
            json!({ "name": "Rehab Drill", "tracking": "reps", "primary_muscle": "shoulders",
                    "instructions": "What my physio told me" }),
        )
        .await;
    let id = created["id"].as_str().expect("id");
    // What the migration does to every exercise that existed before exercises were shared
    sqlx::query("UPDATE exercises SET shared = false WHERE id = $1::uuid")
        .bind(id)
        .execute(&pool)
        .await
        .expect("mark private");

    let uri = format!("/api/v1/train/exercises/{id}");
    let search = "/api/v1/train/exercises?q=rehab";
    assert_eq!(
        names(&app.get(search, &owner.token).await.1),
        ["Rehab Drill"]
    );
    assert!(names(&app.get(search, &other.token).await.1).is_empty());
    assert_eq!(app.get(&uri, &other.token).await.0, StatusCode::NOT_FOUND);
    assert_eq!(
        app.get(&format!("{uri}/history"), &other.token).await.0,
        StatusCode::NOT_FOUND
    );
    let edit = json!({ "name": "Rehab Drill", "tracking": "reps", "primary_muscle": "shoulders" });
    assert_eq!(
        app.put(&uri, &other.token, edit).await.0,
        StatusCode::NOT_FOUND
    );
    let (status, body) = app
        .post(
            "/api/v1/train/routines",
            Some(&other.token),
            json!({ "name": "Borrowed", "exercises": [{ "exercise_id": id }] }),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");

    // Admins moderate everything users created
    assert_eq!(
        names(&app.get(search, &admin.token).await.1),
        ["Rehab Drill"]
    );
    assert_eq!(app.get(&uri, &admin.token).await.0, StatusCode::OK);
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn owners_can_update_and_archive_custom_exercises(pool: PgPool) {
    let app = TestApp::new(pool);
    let user = app.register("owner").await;
    let (_, created) = app
        .post(
            "/api/v1/train/exercises",
            Some(&user.token),
            json!({ "name": "Viking Press", "tracking": "weight_reps", "primary_muscle": "shoulders" }),
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
            json!({ "name": "Half-Kneeling Viking Press", "tracking": "weight_reps",
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
        .get("/api/v1/train/exercises?q=viking", &user.token)
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

    let global = json!({ "name": "Meadows Row", "tracking": "weight_reps", "primary_muscle": "upper_back",
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
        .get("/api/v1/train/exercises?q=meadows", &user.token)
        .await;
    assert_eq!(
        names(&list),
        ["Meadows Row"],
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
