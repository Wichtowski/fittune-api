use std::io::Cursor;

use axum::http::StatusCode;
use fittune_api::health::off::import;
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::common::{TestApp, TestUser};

const HEADER: &str = "code\tlast_modified_t\tproduct_name\tbrands\tmain_category\tserving_size\tserving_quantity\tenergy_100g\tenergy-kcal_100g\tfat_100g\tsaturated-fat_100g\tcarbohydrates_100g\tsugars_100g\tfiber_100g\tproteins_100g\tsalt_100g";

async fn import_off(app: &TestApp) {
    let rows = [
        "5900259127761\t1700000000\tPłatki owsiane górskie\tMelvit\ten:oat-flakes\t40 g\t40\t\t372\t7\t1.2\t60\t1\t10\t13\t0.01",
        "4000417025005\t1700000000\tJoghurt natur\tWeihenstephan\ten:plain-yogurts\t\t\t\t66\t3.5\t2.3\t4.8\t4.8\t\t3.9\t0.12",
        // Known barcode, no nutrition: still useful for a scan, hidden from search
        "96385074\t1700000000\tMystery bar\t\t\t\t\t\t\t\t\t\t\t\t\t",
    ];
    let csv = format!("{HEADER}\n{}\n", rows.join("\n"));
    import::import_reader(&app.pool, Cursor::new(csv.into_bytes()))
        .await
        .expect("import");
}

fn product(name: &str, barcode: Option<&str>) -> Value {
    json!({
        "name": name,
        "barcode": barcode,
        "per_100g": { "energy_kcal": 60.0, "protein_g": 4.0, "fat_g": 3.0, "carbs_g": 5.0 }
    })
}

async fn lookup(app: &TestApp, user: &TestUser, code: &str) -> (StatusCode, Value) {
    app.get(
        &format!("/api/v1/health/products/barcode/{code}"),
        &user.token,
    )
    .await
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn a_barcode_resolves_to_ours_then_the_import_then_nothing(pool: PgPool) {
    let app = TestApp::new(pool);
    let user = app.register("scanner").await;
    import_off(&app).await;

    let (status, off) = lookup(&app, &user, "5900259127761").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(off["status"], "off");
    assert_eq!(off["candidate"]["name"], "Płatki owsiane górskie");
    assert_eq!(off["candidate"]["per_100g"]["energy_kcal"], 372.0);
    assert_eq!(off["candidate"]["serving_g"], 40.0);

    let (_, bare) = lookup(&app, &user, "96385074").await;
    assert_eq!(bare["status"], "off");
    assert!(bare["candidate"]["per_100g"].is_null());

    // Confirming the candidate makes it ours, and ours wins from then on
    let mut confirmed = off["candidate"].clone();
    confirmed["source"] = json!("off");
    let obj = confirmed.as_object_mut().expect("object");
    obj.remove("main_category");
    let (status, saved) = app
        .post("/api/v1/health/products", Some(&user.token), confirmed)
        .await;
    assert_eq!(status, StatusCode::CREATED, "{saved}");
    assert_eq!(saved["source"], "off");
    assert_eq!(saved["barcode"], "5900259127761");
    let (_, ours) = lookup(&app, &user, "5900259127761").await;
    assert_eq!(ours["status"], "found");
    assert_eq!(ours["product"]["id"], saved["id"]);

    let (_, none) = lookup(&app, &user, "4006381333931").await;
    assert_eq!(none, json!({ "status": "not_found" }));

    let (status, _) = lookup(&app, &user, "1234").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn upc_and_ean_forms_of_a_barcode_are_the_same_product(pool: PgPool) {
    let app = TestApp::new(pool);
    let user = app.register("scanner").await;
    let (status, saved) = app
        .post(
            "/api/v1/health/products",
            Some(&user.token),
            product("Cola", Some("036000291452")),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{saved}");
    assert_eq!(saved["barcode"], "0036000291452");
    let (_, found) = lookup(&app, &user, "0036000291452").await;
    assert_eq!(found["status"], "found");

    let (status, _) = app
        .post(
            "/api/v1/health/products",
            Some(&user.token),
            product("Cola again", Some("0036000291452")),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);

    let (status, error) = app
        .post(
            "/api/v1/health/products",
            Some(&user.token),
            product("Bad", Some("5900259127762")),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(error["fields"]["barcode"].is_string());
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn search_finds_imported_products_with_typos_and_hides_ones_already_ours(pool: PgPool) {
    let app = TestApp::new(pool);
    let user = app.register("searcher").await;
    import_off(&app).await;

    // A typo and a partial word still find it
    let (status, found) = app
        .get("/api/v1/health/products?q=plalki%20owsian", &user.token)
        .await;
    assert_eq!(status, StatusCode::OK, "{found}");
    assert_eq!(found["off"][0]["barcode"], "5900259127761");
    let (_, by_brand) = app
        .get("/api/v1/health/products?q=weihenst", &user.token)
        .await;
    assert_eq!(by_brand["off"][0]["name"], "Joghurt natur");
    // Imported products without nutrition cannot be logged, so search leaves them out
    let (_, mystery) = app
        .get("/api/v1/health/products?q=mystery", &user.token)
        .await;
    assert_eq!(mystery["off"], json!([]));

    app.post(
        "/api/v1/health/products",
        Some(&user.token),
        product("Joghurt natur", Some("4000417025005")),
    )
    .await;
    let (_, after) = app
        .get("/api/v1/health/products?q=joghurt", &user.token)
        .await;
    assert_eq!(after["products"][0]["name"], "Joghurt natur");
    assert_eq!(after["off"], json!([]));
}
