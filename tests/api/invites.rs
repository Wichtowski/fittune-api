use axum::http::{Method, StatusCode};
use fittune_api::{
    auth::signup::{self, NewUser},
    users::model::Role,
};
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::common::{PASSWORD, TestApp, TestUser, uuid};

fn signup(username: &str, code: Option<&str>) -> Value {
    let mut body = json!({ "username": username, "email": format!("{username}@example.com"), "password": PASSWORD });
    if let Some(code) = code {
        body["invite_code"] = json!(code);
    }
    body
}

async fn register(app: &TestApp, username: &str, code: Option<&str>) -> (StatusCode, Value) {
    app.post("/api/v1/auth/register", None, signup(username, code))
        .await
}

/// An admin created the way an operator bootstraps a closed installation
async fn admin(app: &TestApp, pool: &PgPool) -> TestUser {
    let user = NewUser::validate("root", "root@example.com", PASSWORD.into(), None, None)
        .expect("valid admin");
    let mut tx = pool.begin().await.expect("tx");
    let id = signup::create(&mut tx, user, Role::Admin, None)
        .await
        .expect("admin created");
    tx.commit().await.expect("commit");
    let (status, body) = app
        .post(
            "/api/v1/auth/login",
            None,
            json!({ "login": "root", "password": PASSWORD }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["user"]["role"], "admin");
    TestUser {
        id: id.to_string(),
        token: body["session"]["token"].as_str().expect("token").to_owned(),
    }
}

async fn issue(app: &TestApp, admin: &TestUser, body: Value) -> (Value, String) {
    let (status, created) = app
        .post("/api/v1/admin/invites", Some(&admin.token), body)
        .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let code = created["code"].as_str().expect("code").to_owned();
    (created["invite"].clone(), code)
}

fn field_error(body: &Value, field: &str) -> String {
    body["fields"][field]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn a_fresh_installation_is_closed(pool: PgPool) {
    let app = TestApp::invite_only(pool.clone());
    let (status, body) = register(&app, "first", None).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        field_error(&body, "invite_code"),
        "An invite code is required"
    );

    let (status, body) = register(&app, "first", Some("abcd-efgh-jkmn")).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(field_error(&body, "invite_code").contains("invalid"));

    let users: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
        .fetch_one(&pool)
        .await
        .expect("count");
    assert_eq!(users, 0, "no signup, and nobody promoted to admin");
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn only_admins_manage_invites(pool: PgPool) {
    let app = TestApp::new(pool);
    let user = app.register("regular").await;
    assert_eq!(
        app.request(Method::GET, "/api/v1/admin/invites", None, None)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        app.get("/api/v1/admin/invites", &user.token).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.post("/api/v1/admin/invites", Some(&user.token), json!({}))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.delete(&format!("/api/v1/admin/invites/{}", uuid()), &user.token)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn invites_are_single_use_and_register_ordinary_users(pool: PgPool) {
    let app = TestApp::invite_only(pool.clone());
    let root = admin(&app, &pool).await;
    let (invite, code) = issue(&app, &root, json!({ "note": "for Ala" })).await;
    assert_eq!(invite["status"], "active");
    assert_eq!(invite["max_uses"], 1);
    assert_eq!(invite["created_by"], "root");
    let (_, list) = app.get("/api/v1/admin/invites", &root.token).await;
    assert!(
        !list.to_string().contains(&code),
        "listing never reveals codes"
    );

    // Codes can be typed in any case, with or without dashes, and cannot grant admin
    let mut body = signup("ala", Some(&code.to_uppercase().replace('-', " ")));
    body["role"] = json!("admin");
    let (status, created) = app.post("/api/v1/auth/register", None, body).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["user"]["role"], "user");

    let (status, body) = register(&app, "ola", Some(&code)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(field_error(&body, "invite_code").contains("already used"));
    let (_, list) = app.get("/api/v1/admin/invites", &root.token).await;
    assert_eq!(list[0]["status"], "used");
    assert_eq!(list[0]["use_count"], 1);

    let (_, code) = issue(&app, &root, json!({ "max_uses": 2 })).await;
    assert_eq!(
        register(&app, "ela", Some(&code)).await.0,
        StatusCode::CREATED
    );
    assert_eq!(
        register(&app, "iza", Some(&code)).await.0,
        StatusCode::CREATED
    );
    assert_eq!(
        register(&app, "ula", Some(&code)).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn failed_signups_do_not_use_up_the_invite(pool: PgPool) {
    let app = TestApp::invite_only(pool.clone());
    let root = admin(&app, &pool).await;
    let (invite, code) = issue(&app, &root, json!({})).await;

    // "root" is taken, so the account insert fails inside the transaction
    let (status, body) = register(&app, "root", Some(&code)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(field_error(&body, "username"), "Username already in use");
    let use_count: i32 = sqlx::query_scalar("SELECT use_count FROM invites WHERE id = $1::uuid")
        .bind(invite["id"].as_str())
        .fetch_one(&pool)
        .await
        .expect("invite");
    assert_eq!(use_count, 0);

    assert_eq!(
        register(&app, "fresh", Some(&code)).await.0,
        StatusCode::CREATED
    );
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn expired_and_revoked_invites_are_rejected(pool: PgPool) {
    let app = TestApp::invite_only(pool.clone());
    let root = admin(&app, &pool).await;

    let (expired, expired_code) = issue(&app, &root, json!({ "expires_in_days": 1 })).await;
    sqlx::query("UPDATE invites SET expires_at = now() - interval '1 minute' WHERE id = $1::uuid")
        .bind(expired["id"].as_str())
        .execute(&pool)
        .await
        .expect("expire");
    assert_eq!(
        register(&app, "late", Some(&expired_code)).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );

    let (revoked, revoked_code) = issue(&app, &root, json!({})).await;
    let uri = format!(
        "/api/v1/admin/invites/{}",
        revoked["id"].as_str().expect("id")
    );
    assert_eq!(
        app.delete(&uri, &root.token).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.delete(&uri, &root.token).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        register(&app, "gone", Some(&revoked_code)).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        app.delete(&format!("/api/v1/admin/invites/{}", uuid()), &root.token)
            .await
            .0,
        StatusCode::NOT_FOUND
    );

    let (_, list) = app.get("/api/v1/admin/invites", &root.token).await;
    let statuses: Vec<&str> = list
        .as_array()
        .expect("list")
        .iter()
        .map(|invite| invite["status"].as_str().expect("status"))
        .collect();
    assert_eq!(statuses, ["revoked", "expired"]);

    for bad in [
        json!({ "expires_in_days": 0 }),
        json!({ "expires_in_days": 91 }),
        json!({ "max_uses": 0 }),
    ] {
        assert_eq!(
            app.post("/api/v1/admin/invites", Some(&root.token), bad)
                .await
                .0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn concurrent_signups_cannot_share_a_single_use_invite(pool: PgPool) {
    let app = TestApp::invite_only(pool.clone());
    let root = admin(&app, &pool).await;
    let (_, code) = issue(&app, &root, json!({})).await;

    let (a, b, c, d) = tokio::join!(
        register(&app, "racer1", Some(&code)),
        register(&app, "racer2", Some(&code)),
        register(&app, "racer3", Some(&code)),
        register(&app, "racer4", Some(&code)),
    );
    let created = [a.0, b.0, c.0, d.0]
        .iter()
        .filter(|status| **status == StatusCode::CREATED)
        .count();
    assert_eq!(created, 1);
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn guessing_invite_codes_is_rate_limited_per_client(pool: PgPool) {
    let app = TestApp::invite_only(pool);
    let guess = |ip: &'static str, n: usize| {
        let app = &app;
        async move {
            app.request_from(
                Some(ip),
                Method::POST,
                "/api/v1/auth/register",
                None,
                Some(signup(&format!("guess{n}"), Some("zzzz-zzzz"))),
            )
            .await
            .0
        }
    };
    for n in 0..10 {
        assert_eq!(
            guess("203.0.113.7", n).await,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    assert_eq!(
        guess("203.0.113.7", 10).await,
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(
        guess("198.51.100.1", 11).await,
        StatusCode::UNPROCESSABLE_ENTITY,
        "other clients are unaffected"
    );
}
