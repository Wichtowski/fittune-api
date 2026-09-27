use axum::http::{Method, StatusCode};
use serde_json::json;
use sqlx::PgPool;

use crate::common::{PASSWORD, TestApp};

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn register_returns_user_and_working_session(pool: PgPool) {
    let app = TestApp::new(pool);
    let (status, body) = app
        .post(
            "/api/v1/auth/register",
            None,
            json!({
                "username": "oskyy",
                "email": "Oskar@Example.com",
                "password": PASSWORD,
                "display_name": "Oskar",
                "birthday": "2002-07-02"
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["user"]["email"], "oskar@example.com");
    assert_eq!(body["user"]["role"], "user");
    assert_eq!(body["user"]["weight_unit"], "kg");
    assert!(body["user"].get("password_hash").is_none());

    let token = body["session"]["token"].as_str().expect("token");
    let (status, me) = app.get("/api/v1/me", token).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(me["username"], "oskyy");
    assert_eq!(me["birthday"], "2002-07-02");
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn register_rejects_duplicates_case_insensitively(pool: PgPool) {
    let app = TestApp::new(pool);
    app.register("lifter").await;

    let (status, body) = app
        .post(
            "/api/v1/auth/register",
            None,
            json!({ "username": "LIFTER", "email": "other@example.com", "password": PASSWORD }),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["fields"]["username"], "Username already in use");

    let (status, body) = app
        .post(
            "/api/v1/auth/register",
            None,
            json!({ "username": "someone", "email": "LIFTER@example.com", "password": PASSWORD }),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["fields"]["email"], "Email already in use");
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn register_reports_field_errors(pool: PgPool) {
    let app = TestApp::new(pool);
    let (status, body) = app
        .post(
            "/api/v1/auth/register",
            None,
            json!({ "username": "oskyy", "email": "userexample.com", "password": "password" }),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["code"], "validation_failed");
    assert_eq!(body["fields"]["email"], "Invalid email address");
    assert_eq!(
        body["fields"]["password"],
        "Password must contain at least one uppercase letter"
    );
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn login_accepts_username_or_email(pool: PgPool) {
    let app = TestApp::new(pool);
    app.register("runner").await;

    for login in ["runner", "RUNNER@example.com"] {
        let (status, body) = app
            .post(
                "/api/v1/auth/login",
                None,
                json!({ "login": login, "password": PASSWORD }),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "login with {login}");
        assert_eq!(body["user"]["username"], "runner");
    }

    let (status, body) = app
        .post(
            "/api/v1/auth/login",
            None,
            json!({ "login": "runner", "password": "Wrong#Pass1" }),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["code"], "invalid_credentials");

    let (status, _) = app
        .post(
            "/api/v1/auth/login",
            None,
            json!({ "login": "nobody", "password": PASSWORD }),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn protected_routes_require_a_valid_token(pool: PgPool) {
    let app = TestApp::new(pool);
    let (status, body) = app.request(Method::GET, "/api/v1/me", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["code"], "unauthorized");

    let (status, _) = app.get("/api/v1/me", "not-a-real-token").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn logout_revokes_only_the_current_session(pool: PgPool) {
    let app = TestApp::new(pool);
    let first = app.register("walker").await;
    let (_, second) = app
        .post(
            "/api/v1/auth/login",
            None,
            json!({ "login": "walker", "password": PASSWORD }),
        )
        .await;
    let second_token = second["session"]["token"].as_str().expect("token");

    let (status, _) = app
        .post("/api/v1/auth/logout", Some(&first.token), json!({}))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(
        app.get("/api/v1/me", &first.token).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(app.get("/api/v1/me", second_token).await.0, StatusCode::OK);
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn profile_update_supports_clearing_fields(pool: PgPool) {
    let app = TestApp::new(pool);
    let user = app.register("swimmer").await;

    let (status, body) = app
        .request(
            Method::PATCH,
            "/api/v1/me",
            Some(&user.token),
            Some(json!({ "display_name": "Swim Fan", "account_type": "professional_trainer", "weight_unit": "lb" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["display_name"], "Swim Fan");
    assert_eq!(body["account_type"], "professional_trainer");
    assert_eq!(body["weight_unit"], "lb");

    let (_, body) = app
        .request(
            Method::PATCH,
            "/api/v1/me",
            Some(&user.token),
            Some(json!({ "display_name": null })),
        )
        .await;
    assert!(body["display_name"].is_null());
    assert_eq!(
        body["account_type"], "professional_trainer",
        "untouched fields are kept"
    );
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn password_change_signs_out_other_sessions(pool: PgPool) {
    let app = TestApp::new(pool);
    let user = app.register("climber").await;
    let (_, other) = app
        .post(
            "/api/v1/auth/login",
            None,
            json!({ "login": "climber", "password": PASSWORD }),
        )
        .await;
    let other_token = other["session"]["token"].as_str().expect("token");

    let (status, body) = app
        .post(
            "/api/v1/me/password",
            Some(&user.token),
            json!({ "current_password": "Wrong#Pass1", "new_password": "New#Password9" }),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        body["fields"]["current_password"],
        "Current password is incorrect"
    );

    let (status, _) = app
        .post(
            "/api/v1/me/password",
            Some(&user.token),
            json!({ "current_password": PASSWORD, "new_password": "New#Password9" }),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(app.get("/api/v1/me", &user.token).await.0, StatusCode::OK);
    assert_eq!(
        app.get("/api/v1/me", other_token).await.0,
        StatusCode::UNAUTHORIZED
    );

    let (status, _) = app
        .post(
            "/api/v1/auth/login",
            None,
            json!({ "login": "climber", "password": "New#Password9" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn account_deletion_requires_password(pool: PgPool) {
    let app = TestApp::new(pool);
    let user = app.register("rower").await;

    let (status, _) = app
        .request(
            Method::DELETE,
            "/api/v1/me",
            Some(&user.token),
            Some(json!({ "password": "nope" })),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    let (status, _) = app
        .request(
            Method::DELETE,
            "/api/v1/me",
            Some(&user.token),
            Some(json!({ "password": PASSWORD })),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(
        app.get("/api/v1/me", &user.token).await.0,
        StatusCode::UNAUTHORIZED
    );
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn user_directory_is_admin_only(pool: PgPool) {
    let app = TestApp::new(pool);
    let user = app.register("regular").await;
    let admin = app.register("coach").await;
    app.make_admin(&admin).await;

    assert_eq!(
        app.get("/api/v1/users", &user.token).await.0,
        StatusCode::FORBIDDEN
    );
    let (status, body) = app.get("/api/v1/users", &admin.token).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.as_array().map(Vec::len), Some(2));
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn health_reports_database_status(pool: PgPool) {
    let app = TestApp::new(pool);
    let (status, body) = app.request(Method::GET, "/health", None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        json!({ "status": "ok", "version": "test", "database": "ok" })
    );

    let (status, body) = app.request(Method::GET, "/api/v1/nope", None, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], "not_found");
}
