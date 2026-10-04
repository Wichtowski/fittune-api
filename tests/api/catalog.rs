use axum::http::{Method, StatusCode, header};
use sqlx::PgPool;

use crate::common::TestApp;

/// Rows of hasaneyldrm/exercises-dataset at the commit the import migration was generated from
const DATASET_EXERCISES: i64 = 1324;
/// Exercises of the first catalog; 44 of them merged with a dataset row
const FIRST_CATALOG: i64 = 47;
const MERGED: i64 = 44;

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn dataset_is_imported_without_replacing_the_first_catalog(pool: PgPool) {
    let (catalog, imported, polish): (i64, i64, i64) = sqlx::query_as(
        "SELECT count(*), count(catalog_ref),
                count(*) FILTER (WHERE catalog_ref IS NOT NULL AND instructions IS NOT NULL
                                 AND instructions_pl IS NOT NULL)
         FROM exercises WHERE owner_id IS NULL AND archived_at IS NULL",
    )
    .fetch_one(&pool)
    .await
    .expect("count catalog");
    assert_eq!(imported, DATASET_EXERCISES);
    assert_eq!(catalog, DATASET_EXERCISES + FIRST_CATALOG - MERGED);
    assert_eq!(polish, DATASET_EXERCISES, "instructions in both languages");

    // Ids are derived from the names the first catalog was seeded with, so an exercise that
    // kept its id kept its history, routines and the app's routine templates
    let kept: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT name, tracking, catalog_ref FROM exercises
         WHERE id = ANY (SELECT md5('fittune:exercise:' || lower(name))::uuid
                         FROM unnest($1::text[]) AS name)
         ORDER BY name",
    )
    .bind(["Barbell Bench Press", "Plank", "Pull-Up", "Treadmill Run"])
    .fetch_all(&pool)
    .await
    .expect("first catalog exercises");
    let kept: Vec<(&str, &str, Option<&str>)> = kept
        .iter()
        .map(|(name, tracking, catalog_ref)| {
            (name.as_str(), tracking.as_str(), catalog_ref.as_deref())
        })
        .collect();
    assert_eq!(
        kept,
        [
            ("Barbell Bench Press", "weight_reps", Some("gymvisual:0025")),
            ("Plank", "duration", None),
            ("Pull-Up", "reps", Some("gymvisual:0652")),
            ("Treadmill Run", "distance_duration", None),
        ]
    );
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn imported_exercises_are_matched_to_places_by_real_equipment(pool: PgPool) {
    // An exercise that needs equipment but requires nothing would show up at a place without any
    let unrestricted: Vec<(String, String)> = sqlx::query_as(
        "SELECT name, tracking FROM exercises
         WHERE owner_id IS NULL AND equipment <> 'none' AND requires = '{}'
         ORDER BY name",
    )
    .fetch_all(&pool)
    .await
    .expect("unrestricted exercises");
    assert_eq!(
        unrestricted.len(),
        6,
        "only the stretches done with a strap: {unrestricted:?}"
    );
    for (name, tracking) in &unrestricted {
        assert_eq!(tracking, "duration", "{name}");
    }

    let requires = |name: &'static str| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, Vec<String>>(
                "SELECT requires FROM exercises WHERE owner_id IS NULL AND name = $1",
            )
            .bind(name)
            .fetch_one(&pool)
            .await
            .unwrap_or_else(|err| panic!("{name}: {err}"))
        }
    };
    assert_eq!(requires("Lever Preacher Curl").await, ["strength_machines"]);
    assert_eq!(
        requires("Weighted Pull-Up").await,
        ["weight_plates", "pull_up_bar"]
    );
    assert_eq!(
        requires("Dumbbell Incline Fly").await,
        ["dumbbells", "adjustable_bench"]
    );
    assert_eq!(requires("Cable Seated Wide-Grip Row").await, ["seated_row"]);
    assert!(requires("Mountain Climber").await.is_empty());
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn exercises_carry_polish_instructions_and_the_library_is_compressed(pool: PgPool) {
    let app = TestApp::new(pool);
    let user = app.register("polyglot").await;
    let bench = app.catalog_exercise("Barbell Bench Press").await;
    let (status, exercise) = app
        .get(&format!("/api/v1/train/exercises/{bench}"), &user.token)
        .await;
    assert_eq!(status, StatusCode::OK);
    let english = exercise["instructions"].as_str().expect("instructions");
    let polish = exercise["instructions_pl"]
        .as_str()
        .expect("instructions_pl");
    assert!(english.lines().count() > 1, "one step per line: {english}");
    assert_eq!(english.lines().count(), polish.lines().count());
    assert_ne!(english, polish);

    let (status, headers, body) = app
        .raw_with_headers(
            Method::GET,
            "/api/v1/train/exercises",
            Some(&user.token),
            &[(header::ACCEPT_ENCODING, "gzip")],
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CONTENT_ENCODING], "gzip");
    assert!(
        body.len() < 600 * 1024,
        "the library is {} bytes compressed",
        body.len()
    );
}
