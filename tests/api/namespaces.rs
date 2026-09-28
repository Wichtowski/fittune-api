use axum::http::{Method, StatusCode};
use sqlx::PgPool;

use crate::common::TestApp;

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn training_endpoints_live_under_train(pool: PgPool) {
    let app = TestApp::new(pool);
    let user = app.register("namespaced").await;

    let (status, _) = app.get("/api/v1/train/routines", &user.token).await;
    assert_eq!(status, StatusCode::OK);

    // No backward compatibility: the flat path is gone
    let (status, body) = app.get("/api/v1/routines", &user.token).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], "not_found");
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn account_endpoints_stay_shared(pool: PgPool) {
    let app = TestApp::new(pool);
    let user = app.register("shared").await;

    let (status, _) = app.get("/api/v1/me", &user.token).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = app.get("/api/v1/train/me", &user.token).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn unknown_health_routes_return_the_json_not_found_error(pool: PgPool) {
    let app = TestApp::new(pool);
    let user = app.register("nutrition").await;

    let (status, body) = app
        .request(
            Method::GET,
            "/api/v1/health/anything",
            Some(&user.token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], "not_found");
}
