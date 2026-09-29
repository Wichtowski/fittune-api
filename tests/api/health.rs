use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::{
    activities::run,
    common::{TestApp, TestUser, uuid},
    workouts::workout,
};

fn oats() -> Value {
    json!({
        "name": "Oat flakes",
        "brand": "Melvit",
        "per_100g": {
            "energy_kcal": 372.0, "protein_g": 13.0, "fat_g": 7.0, "carbs_g": 60.0,
            "saturated_fat_g": 1.2, "sugars_g": 1.0, "fiber_g": 10.0, "salt_g": 0.01
        },
        "serving_g": 40.0,
        "serving_name": "4 tablespoons"
    })
}

async fn create_product(app: &TestApp, user: &TestUser, body: Value) -> Value {
    let (status, product) = app
        .post("/api/v1/health/products", Some(&user.token), body)
        .await;
    assert_eq!(status, StatusCode::CREATED, "{product}");
    product
}

async fn meals(app: &TestApp, user: &TestUser) -> Vec<Value> {
    let (status, meals) = app.get("/api/v1/health/meals", &user.token).await;
    assert_eq!(status, StatusCode::OK);
    meals.as_array().expect("meals").clone()
}

fn id(value: &Value) -> &str {
    value["id"].as_str().expect("id")
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn products_are_created_validated_and_shared(pool: PgPool) {
    let app = TestApp::new(pool);
    let alice = app.register("alice").await;
    let bob = app.register("bob").await;

    let product = create_product(&app, &alice, oats()).await;
    assert_eq!(product["source"], "manual");
    assert_eq!(product["per_100g"]["protein_g"], 13.0);

    // Shared: another user finds and edits it
    let (status, found) = app.get("/api/v1/health/products?q=melv", &bob.token).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(found.as_array().expect("array").len(), 1);
    let mut edited = oats();
    edited["name"] = json!("Oat flakes, mountain");
    let uri = format!("/api/v1/health/products/{}", id(&product));
    let (status, updated) = app.put(&uri, &bob.token, edited).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["name"], "Oat flakes, mountain");

    let mut bad = oats();
    bad["per_100g"]["sugars_g"] = json!(70.0);
    bad["per_100g"]["fat_g"] = json!(50.0);
    let (status, error) = app
        .post("/api/v1/health/products", Some(&alice.token), bad)
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(error["fields"]["per_100g.sugars_g"].is_string(), "{error}");
    assert!(error["fields"]["per_100g"].is_string(), "{error}");

    let (status, _) = app
        .get(&format!("/api/v1/health/products/{}", uuid()), &alice.token)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn meals_start_with_defaults_and_can_be_edited(pool: PgPool) {
    let app = TestApp::new(pool);
    let user = app.register("eater").await;
    let other = app.register("other").await;

    let names: Vec<_> = meals(&app, &user)
        .await
        .iter()
        .map(|m| m["name"].clone())
        .collect();
    assert_eq!(
        names,
        ["Breakfast", "Second breakfast", "Lunch", "Snack", "Dinner"].map(Value::from)
    );
    // Defaults are created once
    assert_eq!(meals(&app, &user).await.len(), 5);

    let (status, supper) = app
        .post(
            "/api/v1/health/meals",
            Some(&user.token),
            json!({ "name": "  Supper " }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(supper["name"], "Supper");

    let uri = format!("/api/v1/health/meals/{}", id(&supper));
    let (status, renamed) = app
        .request(
            Method::PATCH,
            &uri,
            Some(&user.token),
            Some(json!({ "name": "Late snack" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(renamed["name"], "Late snack");
    let (status, _) = app
        .request(
            Method::PATCH,
            &uri,
            Some(&other.token),
            Some(json!({ "name": "Mine" })),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let mut ids: Vec<String> = meals(&app, &user)
        .await
        .iter()
        .map(|m| id(m).to_owned())
        .collect();
    ids.reverse();
    let (status, reordered) = app
        .put(
            "/api/v1/health/meals/order",
            &user.token,
            json!({ "ids": ids }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(reordered[0]["name"], "Late snack");
    let (status, _) = app
        .put(
            "/api/v1/health/meals/order",
            &user.token,
            json!({ "ids": [ids[0]] }),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    for meal in meals(&app, &user).await.iter().skip(1) {
        let (status, _) = app
            .delete(&format!("/api/v1/health/meals/{}", id(meal)), &user.token)
            .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
    }
    let last = meals(&app, &user).await;
    assert_eq!(last.len(), 1);
    let (status, _) = app
        .delete(
            &format!("/api/v1/health/meals/{}", id(&last[0])),
            &user.token,
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn entries_snapshot_the_product_and_stay_private(pool: PgPool) {
    let app = TestApp::new(pool);
    let user = app.register("eater").await;
    let other = app.register("other").await;
    let product = create_product(&app, &user, oats()).await;
    let breakfast = meals(&app, &user).await[0].clone();

    let uri = format!("/api/v1/health/entries/{}", uuid());
    let body = json!({ "date": "2026-09-29", "meal_id": id(&breakfast), "product_id": id(&product), "grams": 50.0 });
    let (status, entry) = app.put(&uri, &user.token, body.clone()).await;
    assert_eq!(status, StatusCode::CREATED, "{entry}");
    assert_eq!(entry["product_name"], "Oat flakes");

    let mut more = body.clone();
    more["grams"] = json!(80.0);
    let (status, entry) = app.put(&uri, &user.token, more).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(entry["grams"], 80.0);

    // Editing the shared product does not rewrite the entry
    let mut changed = oats();
    changed["name"] = json!("Renamed oats");
    changed["per_100g"]["energy_kcal"] = json!(100.0);
    app.put(
        &format!("/api/v1/health/products/{}", id(&product)),
        &user.token,
        changed,
    )
    .await;
    let (_, day) = app
        .get("/api/v1/health/days/2026-09-29?tz=UTC", &user.token)
        .await;
    let logged = &day["meals"][0]["entries"][0];
    assert_eq!(logged["product_name"], "Oat flakes");
    assert_eq!(logged["per_100g"]["energy_kcal"], 372.0);
    assert!((day["totals"]["energy_kcal"].as_f64().expect("number") - 297.6).abs() < 0.01);

    // Another user can neither see, change nor log into someone else's meal
    let (status, _) = app.put(&uri, &other.token, body.clone()).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = app
        .put(
            &format!("/api/v1/health/entries/{}", uuid()),
            &other.token,
            body,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (_, their_day) = app
        .get("/api/v1/health/days/2026-09-29?tz=UTC", &other.token)
        .await;
    assert_eq!(their_day["totals"]["energy_kcal"], 0.0);

    let missing = json!({ "date": "2026-09-29", "meal_id": id(&breakfast), "product_id": uuid(), "grams": 50.0 });
    let (status, _) = app
        .put(
            &format!("/api/v1/health/entries/{}", uuid()),
            &user.token,
            missing,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _) = app.delete(&uri, &user.token).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, day) = app
        .get("/api/v1/health/days/2026-09-29?tz=UTC", &user.token)
        .await;
    assert_eq!(day["totals"]["energy_kcal"], 0.0);
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn search_puts_the_callers_recent_products_first(pool: PgPool) {
    let app = TestApp::new(pool);
    let user = app.register("eater").await;
    let mut milk = oats();
    milk["name"] = json!("Milk 2%");
    milk["brand"] = Value::Null;
    let milk = create_product(&app, &user, milk).await;
    let mut almond = oats();
    almond["name"] = json!("Almond milk");
    almond["brand"] = Value::Null;
    create_product(&app, &user, almond).await;

    let (_, before) = app.get("/api/v1/health/products?q=milk", &user.token).await;
    assert_eq!(before[0]["name"], "Almond milk");

    let meal = meals(&app, &user).await[0].clone();
    let body = json!({ "date": "2026-09-29", "meal_id": id(&meal), "product_id": id(&milk), "grams": 200.0 });
    app.put(
        &format!("/api/v1/health/entries/{}", uuid()),
        &user.token,
        body,
    )
    .await;
    let (_, after) = app.get("/api/v1/health/products?q=milk", &user.token).await;
    assert_eq!(after[0]["name"], "Milk 2%");
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn a_day_without_body_data_says_what_is_missing(pool: PgPool) {
    let app = TestApp::new(pool);
    let user = app.register("newbie").await;
    let (status, day) = app
        .get("/api/v1/health/days/2026-09-29?tz=UTC", &user.token)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(day["targets"].is_null());
    assert_eq!(day["missing"], json!(["profile", "birthday", "weight"]));
    assert_eq!(day["meals"].as_array().expect("array").len(), 5);

    let (status, _) = app
        .get("/api/v1/health/days/2026-09-29?tz=Mars/Base", &user.token)
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn targets_follow_the_body_and_the_days_training(pool: PgPool) {
    let app = TestApp::new(pool);
    let user = app.register("athlete").await;
    app.request(
        Method::PATCH,
        "/api/v1/me",
        Some(&user.token),
        Some(json!({ "birthday": "1996-09-01" })),
    )
    .await;
    let (status, profile) = app
        .put(
            "/api/v1/health/profile",
            &user.token,
            json!({ "sex": "male", "height_cm": 180.0, "activity": "moderate", "goal": "maintain", "pace_kg_per_week": 0.0 }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{profile}");
    app.put(
        "/api/v1/health/weights/2026-09-20",
        &user.token,
        json!({ "weight_kg": 80.0 }),
    )
    .await;
    // A later weight must not change an earlier day
    app.put(
        "/api/v1/health/weights/2026-09-30",
        &user.token,
        json!({ "weight_kg": 90.0 }),
    )
    .await;

    let bench = app.catalog_exercise("Barbell Bench Press").await;
    // 19:00-20:00 in Warsaw on the 28th
    app.put(
        &format!("/api/v1/train/workouts/{}", uuid()),
        &user.token,
        workout(
            &bench,
            "2026-09-28T17:00:00Z",
            Some("2026-09-28T18:00:00Z"),
            1,
        ),
    )
    .await;
    // 30 minutes, 8 km: 16 km/h
    app.put(
        &format!("/api/v1/train/activities/{}", uuid()),
        &user.token,
        run("2026-09-28T06:00:00Z", 8000.0),
    )
    .await;
    // 00:30 on the 29th in Warsaw, still the 28th in UTC
    app.put(
        &format!("/api/v1/train/workouts/{}", uuid()),
        &user.token,
        workout(
            &bench,
            "2026-09-28T22:30:00Z",
            Some("2026-09-28T23:30:00Z"),
            1,
        ),
    )
    .await;

    let (status, day) = app
        .get(
            "/api/v1/health/days/2026-09-28?tz=Europe/Warsaw",
            &user.token,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{day}");
    assert_eq!(day["missing"], json!([]));
    assert_eq!(day["exercise"]["workouts"], 1);
    assert_eq!(day["exercise"]["activities"], 1);
    // Workout 4 * 80 * 1 h, run (16 - 1) * 80 * 0.5 h
    let training = 320.0 + 600.0;
    assert!(
        (day["exercise"]["energy_kcal"].as_f64().expect("number") - training).abs() < 0.01,
        "{day}"
    );
    // Age 30: 10 * 80 + 6.25 * 180 - 150 + 5 = 1780, moderate 1.55
    let expected = 1780.0 * 1.55 + training;
    assert!(
        (day["targets"]["energy_kcal"].as_f64().expect("number") - expected).abs() < 0.01,
        "{day}"
    );
    assert_eq!(day["weight_kg"], 80.0);

    let (_, next) = app
        .get(
            "/api/v1/health/days/2026-09-29?tz=Europe/Warsaw",
            &user.token,
        )
        .await;
    assert_eq!(next["exercise"]["workouts"], 1);

    // Overrides replace the calculated energy but keep the training bonus
    let (status, _) = app
        .put(
            "/api/v1/health/profile",
            &user.token,
            json!({ "sex": "male", "height_cm": 180.0, "activity": "moderate", "goal": "maintain", "pace_kg_per_week": 0.0, "energy_kcal": 2500.0 }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (_, day) = app
        .get(
            "/api/v1/health/days/2026-09-28?tz=Europe/Warsaw",
            &user.token,
        )
        .await;
    assert!(
        (day["targets"]["energy_kcal"].as_f64().expect("number") - (2500.0 + training)).abs()
            < 0.01
    );
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn profile_and_weights_are_validated_and_private(pool: PgPool) {
    let app = TestApp::new(pool);
    let user = app.register("body").await;
    let other = app.register("other").await;

    let (status, empty) = app.get("/api/v1/health/profile", &user.token).await;
    assert_eq!(status, StatusCode::OK);
    assert!(empty["sex"].is_null());

    let (status, error) = app
        .put(
            "/api/v1/health/profile",
            &user.token,
            json!({ "sex": "male", "height_cm": 20.0, "activity": "moderate", "goal": "lose", "pace_kg_per_week": 3.0 }),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(error["fields"]["height_cm"].is_string());
    assert!(error["fields"]["pace_kg_per_week"].is_string());

    for (date, kg) in [("2026-09-01", 82.0), ("2026-09-15", 81.0)] {
        let (status, _) = app
            .put(
                &format!("/api/v1/health/weights/{date}"),
                &user.token,
                json!({ "weight_kg": kg }),
            )
            .await;
        assert_eq!(status, StatusCode::OK);
    }
    let (status, _) = app
        .put(
            "/api/v1/health/weights/2026-09-16",
            &user.token,
            json!({ "weight_kg": 900.0 }),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    let (_, weights) = app.get("/api/v1/health/weights", &user.token).await;
    assert_eq!(weights[0]["date"], "2026-09-15");
    assert_eq!(weights.as_array().expect("array").len(), 2);
    let (_, theirs) = app.get("/api/v1/health/weights", &other.token).await;
    assert_eq!(theirs, json!([]));

    let (status, _) = app
        .delete("/api/v1/health/weights/2026-09-15", &user.token)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = app
        .delete("/api/v1/health/weights/2026-09-15", &user.token)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn deleting_an_account_removes_its_diary_but_keeps_shared_products(pool: PgPool) {
    let app = TestApp::new(pool);
    let user = app.register("leaver").await;
    let stayer = app.register("stayer").await;
    let product = create_product(&app, &user, oats()).await;
    let meal = meals(&app, &user).await[0].clone();
    let body = json!({ "date": "2026-09-29", "meal_id": id(&meal), "product_id": id(&product), "grams": 50.0 });
    app.put(
        &format!("/api/v1/health/entries/{}", uuid()),
        &user.token,
        body,
    )
    .await;
    app.put(
        "/api/v1/health/weights/2026-09-29",
        &user.token,
        json!({ "weight_kg": 70.0 }),
    )
    .await;

    let (status, _) = app
        .request(
            Method::DELETE,
            "/api/v1/me",
            Some(&user.token),
            Some(json!({ "password": crate::common::PASSWORD })),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, kept) = app
        .get(
            &format!("/api/v1/health/products/{}", id(&product)),
            &stayer.token,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(kept["name"], "Oat flakes");
}
