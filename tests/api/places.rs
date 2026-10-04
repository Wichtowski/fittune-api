use axum::http::{Method, StatusCode};
use serde_json::json;
use sqlx::PgPool;

use crate::common::{TestApp, uuid};

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn places_are_private_validated_versioned_and_archivable(pool: PgPool) {
    let app = TestApp::new(pool);
    let owner = app.register("placeowner").await;
    let other = app.register("placeother").await;
    let uri = format!("/api/v1/train/places/{}", uuid());
    let input = json!({"name": " Home ", "kind": "home", "equipment": ["dumbbells", "dumbbells"]});
    assert_eq!(
        app.request(Method::GET, "/api/v1/train/places", None, None)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let (status, home) = app.put(&uri, &owner.token, input).await;
    assert_eq!(status, StatusCode::CREATED, "{home}");
    assert_eq!(home["name"], "Home");
    assert_eq!(home["equipment"], json!(["dumbbells"]));
    let same = json!({"name": "Home", "kind": "home", "equipment": ["dumbbells"]});
    assert_eq!(
        app.put(&uri, &owner.token, same.clone()).await.1["version_id"],
        home["version_id"]
    );
    assert_eq!(
        app.get("/api/v1/train/places", &other.token).await.1,
        json!([])
    );
    assert_eq!(
        app.put(&uri, &other.token, same.clone()).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.delete(&uri, &other.token).await.0,
        StatusCode::NOT_FOUND
    );
    for (bad, expected) in [
        (
            json!({"name": " ", "kind": "home", "equipment": []}),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            json!({"name": "Home", "kind": "invalid", "equipment": []}),
            StatusCode::BAD_REQUEST,
        ),
        (
            json!({"name": "Home", "kind": "home", "equipment": ["invalid"]}),
            StatusCode::BAD_REQUEST,
        ),
        (
            json!({"name": "Home", "kind": "home", "equipment": ["none"]}),
            StatusCode::BAD_REQUEST,
        ),
        (
            json!({"name": "Home", "kind": "home", "equipment": [], "user_id": other.id}),
            StatusCode::BAD_REQUEST,
        ),
    ] {
        assert_eq!(app.put(&uri, &owner.token, bad).await.0, expected);
    }
    let (_, updated) = app
        .put(
            &uri,
            &owner.token,
            json!({"name": "Bodyweight room", "kind": "custom", "equipment": []}),
        )
        .await;
    assert_ne!(updated["version_id"], home["version_id"]);
    assert_eq!(
        app.get("/api/v1/train/places", &owner.token).await.1,
        json!([updated])
    );
    assert_eq!(
        app.delete(&uri, &owner.token).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.delete(&uri, &owner.token).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.get("/api/v1/train/places", &owner.token).await.1,
        json!([])
    );
    assert_eq!(
        app.put(&uri, &owner.token, same).await.0,
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn offline_workouts_keep_owned_place_versions_after_edit_and_archive(pool: PgPool) {
    let app = TestApp::new(pool);
    let owner = app.register("offlineowner").await;
    let other = app.register("offlineother").await;
    let place_uri = format!("/api/v1/train/places/{}", uuid());
    let (_, home) = app
        .put(
            &place_uri,
            &owner.token,
            json!({"name": "Home", "kind": "home", "equipment": ["resistance_band"]}),
        )
        .await;
    let workout_uri = format!("/api/v1/train/workouts/{}", uuid());
    let mut draft = json!({"title": "Offline session", "started_at": "2026-09-20T17:00:00Z", "revision": 1, "exercises": [], "place_version_id": home["version_id"]});
    assert_eq!(
        app.put(&workout_uri, &owner.token, draft.clone()).await.1["place"],
        home
    );
    let (_, gym) = app
        .put(
            &place_uri,
            &owner.token,
            json!({"name": "Gym", "kind": "gym", "equipment": ["barbell"]}),
        )
        .await;
    app.delete(&place_uri, &owner.token).await;
    assert_eq!(app.get(&workout_uri, &owner.token).await.1["place"], home);
    draft["revision"] = json!(2);
    draft["ended_at"] = json!("2026-09-20T18:00:00Z");
    let (status, saved) = app.put(&workout_uri, &owner.token, draft.clone()).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["place"], home);
    assert_eq!(
        app.get("/api/v1/train/workouts", &owner.token).await.1["items"][0]["place"],
        home
    );
    let offline_uri = format!("/api/v1/train/workouts/{}", uuid());
    assert_eq!(
        app.put(&offline_uri, &owner.token, draft.clone()).await.1["place"],
        home
    );
    let foreign_uri = format!("/api/v1/train/workouts/{}", uuid());
    assert_eq!(
        app.put(&foreign_uri, &other.token, draft.clone()).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    draft["place_version_id"] = json!(uuid());
    assert_eq!(
        app.put(&foreign_uri, &owner.token, draft.clone()).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let mut legacy = draft.clone();
    legacy
        .as_object_mut()
        .expect("object")
        .remove("place_version_id");
    legacy["revision"] = json!(3);
    assert_eq!(
        app.put(&workout_uri, &owner.token, legacy).await.1["place"],
        home
    );
    draft["place_version_id"] = gym["version_id"].clone();
    draft["revision"] = json!(4);
    assert_eq!(
        app.put(&workout_uri, &owner.token, draft.clone()).await.1["place"],
        gym
    );
    draft["place_version_id"] = serde_json::Value::Null;
    draft["revision"] = json!(5);
    assert!(app.put(&workout_uri, &owner.token, draft).await.1["place"].is_null());
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn equipment_order_does_not_create_new_versions(pool: PgPool) {
    let app = TestApp::new(pool);
    let owner = app.register("orderowner").await;
    let uri = format!("/api/v1/train/places/{}", uuid());
    let (_, first) = app
        .put(
            &uri,
            &owner.token,
            json!({"name": "Home", "kind": "home", "equipment": ["resistance_band", "dumbbells"]}),
        )
        .await;
    assert_eq!(first["equipment"], json!(["dumbbells", "resistance_band"]));
    let (status, reordered) = app
        .put(
            &uri,
            &owner.token,
            json!({"name": "Home", "kind": "home", "equipment": ["dumbbells", "resistance_band", "resistance_band"]}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(reordered["version_id"], first["version_id"]);
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn users_keep_at_most_ten_active_places(pool: PgPool) {
    let app = TestApp::new(pool);
    let owner = app.register("limitowner").await;
    let other = app.register("limitother").await;
    let place = |n: usize| json!({"name": format!("Place {n}"), "kind": "custom", "equipment": []});
    let mut uris = Vec::new();
    for n in 0..10 {
        let uri = format!("/api/v1/train/places/{}", uuid());
        assert_eq!(
            app.put(&uri, &owner.token, place(n)).await.0,
            StatusCode::CREATED
        );
        uris.push(uri);
    }
    let extra = format!("/api/v1/train/places/{}", uuid());
    let (status, body) = app.put(&extra, &owner.token, place(10)).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");

    // Editing an existing place is still allowed at the limit, and the limit is per user
    assert_eq!(
        app.put(&uris[0], &owner.token, place(99)).await.0,
        StatusCode::OK
    );
    assert_eq!(
        app.put(&extra, &other.token, place(0)).await.0,
        StatusCode::CREATED
    );

    assert_eq!(
        app.delete(&uris[0], &owner.token).await.0,
        StatusCode::NO_CONTENT
    );
    let fresh = format!("/api/v1/train/places/{}", uuid());
    assert_eq!(
        app.put(&fresh, &owner.token, place(10)).await.0,
        StatusCode::CREATED
    );
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn places_accept_every_equipment_item(pool: PgPool) {
    let app = TestApp::new(pool);
    let owner = app.register("fullgym").await;
    let all = json!([
        "barbell",
        "ez_bar",
        "trap_bar",
        "dumbbells",
        "kettlebells",
        "weight_plates",
        "flat_bench",
        "adjustable_bench",
        "preacher_bench",
        "back_extension_bench",
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
        "strength_machines",
        "cable_station",
        "lat_pulldown",
        "seated_row",
        "treadmill",
        "rowing_machine",
        "stationary_bike",
        "cardio_machines",
        "resistance_band",
        "suspension_trainer",
        "stability_ball",
        "bosu_ball",
        "medicine_ball",
        "foam_roller",
        "plyo_box",
        "ab_wheel",
        "jump_rope",
        "battle_ropes",
        "climbing_rope",
        "sledgehammer_tire"
    ]);
    let (status, place) = app
        .put(
            &format!("/api/v1/train/places/{}", uuid()),
            &owner.token,
            json!({"name": "Full gym", "kind": "gym", "equipment": all}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{place}");
    assert_eq!(place["equipment"], all);
}
