use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::{
    activities::run,
    common::{PASSWORD, TestApp, TestUser, uuid},
    workouts::workout,
};

const EVERYTHING: Value = Value::Null;

fn sharing(workouts: bool, activities: bool, stats: bool, personal_records: bool) -> Value {
    json!({
        "workouts": workouts,
        "activities": activities,
        "stats": stats,
        "personal_records": personal_records,
    })
}

/// Sends `from` → `to` and accepts it, leaving them friends
async fn befriend(app: &TestApp, from: &TestUser, to: &TestUser, to_username: &str) {
    let (status, body) = app
        .post(
            "/api/v1/friends/requests",
            Some(&from.token),
            json!({ "username": to_username }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let (status, body) = app
        .post(
            &format!("/api/v1/friends/requests/{}/accept", from.id),
            Some(&to.token),
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

async fn share(app: &TestApp, user: &TestUser, settings: Value) {
    let settings = if settings == EVERYTHING {
        sharing(true, true, true, true)
    } else {
        settings
    };
    let (status, body) = app.put("/api/v1/me/sharing", &user.token, settings).await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

/// A finished bench workout and a run with private notes and health data
async fn log_training(app: &TestApp, user: &TestUser) {
    let bench = app.catalog_exercise("Barbell Bench Press").await;
    let mut finished = workout(
        &bench,
        "2026-09-20T17:00:00Z",
        Some("2026-09-20T18:00:00Z"),
        1,
    );
    finished["notes"] = json!("private workout note");
    let (status, body) = app
        .put(
            &format!("/api/v1/workouts/{}", uuid()),
            &user.token,
            finished,
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    let in_progress = workout(&bench, "2026-09-22T17:00:00Z", None, 1);
    app.put(
        &format!("/api/v1/workouts/{}", uuid()),
        &user.token,
        in_progress,
    )
    .await;

    let mut activity = run("2026-09-21T06:00:00Z", 5000.0);
    activity["notes"] = json!("private run note");
    let (status, body) = app
        .put(
            &format!("/api/v1/activities/{}", uuid()),
            &user.token,
            activity,
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
}

fn feed_types(feed: &Value) -> Vec<&str> {
    feed["items"]
        .as_array()
        .expect("feed items")
        .iter()
        .map(|item| item["type"].as_str().expect("item type"))
        .collect()
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn lookup_matches_exact_usernames_only_and_hides_private_fields(pool: PgPool) {
    let app = TestApp::new(pool);
    let alice = app.register("alice").await;
    app.register("bobby").await;

    let (status, found) = app
        .get("/api/v1/users/lookup?username=BOBBY", &alice.token)
        .await;
    assert_eq!(status, StatusCode::OK, "{found}");
    assert_eq!(found["user"]["username"], "bobby");
    assert_eq!(found["relationship"], "none");
    let fields: Vec<&String> = found["user"].as_object().expect("user").keys().collect();
    assert_eq!(fields, ["display_name", "id", "username"]);

    for query in ["bob", "bobby@example.com", ""] {
        let (status, _) = app
            .get(
                &format!("/api/v1/users/lookup?username={query}"),
                &alice.token,
            )
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{query}");
    }

    let (_, me) = app
        .get("/api/v1/users/lookup?username=alice", &alice.token)
        .await;
    assert_eq!(me["relationship"], "self");
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn requests_can_be_sent_accepted_declined_and_cancelled(pool: PgPool) {
    let app = TestApp::new(pool);
    let alice = app.register("alice").await;
    let bobby = app.register("bobby").await;
    let carol = app.register("carol").await;

    let (status, sent) = app
        .post(
            "/api/v1/friends/requests",
            Some(&alice.token),
            json!({ "username": "bobby" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{sent}");
    assert_eq!(sent["relationship"], "outgoing");

    // Repeating is a no-op
    let (status, again) = app
        .post(
            "/api/v1/friends/requests",
            Some(&alice.token),
            json!({ "username": "bobby" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(again["relationship"], "outgoing");

    let (_, requests) = app.get("/api/v1/friends/requests", &bobby.token).await;
    assert_eq!(requests["incoming"][0]["user"]["username"], "alice");
    assert_eq!(requests["outgoing"], json!([]));

    // Only the addressee can accept
    let (status, _) = app
        .post(
            &format!("/api/v1/friends/requests/{}/accept", bobby.id),
            Some(&alice.token),
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, friend) = app
        .post(
            &format!("/api/v1/friends/requests/{}/accept", alice.id),
            Some(&bobby.token),
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{friend}");
    assert_eq!(friend["user"]["username"], "alice");

    for user in [&alice, &bobby] {
        let (_, friends) = app.get("/api/v1/friends", &user.token).await;
        assert_eq!(friends.as_array().map(Vec::len), Some(1));
    }

    // Declined by the addressee
    app.post(
        "/api/v1/friends/requests",
        Some(&carol.token),
        json!({ "username": "alice" }),
    )
    .await;
    let (status, _) = app
        .delete(
            &format!("/api/v1/friends/requests/{}", carol.id),
            &alice.token,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Cancelled by the requester
    app.post(
        "/api/v1/friends/requests",
        Some(&carol.token),
        json!({ "username": "alice" }),
    )
    .await;
    let (status, _) = app
        .delete(
            &format!("/api/v1/friends/requests/{}", alice.id),
            &carol.token,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, requests) = app.get("/api/v1/friends/requests", &alice.token).await;
    assert_eq!(requests["incoming"], json!([]));

    // A pending request is not a friendship and cannot be removed as one
    let (status, _) = app
        .delete(&format!("/api/v1/friends/{}", carol.id), &alice.token)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn cannot_befriend_yourself(pool: PgPool) {
    let app = TestApp::new(pool);
    let alice = app.register("alice").await;
    let (status, body) = app
        .post(
            "/api/v1/friends/requests",
            Some(&alice.token),
            json!({ "username": "Alice" }),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn crossed_requests_become_a_friendship(pool: PgPool) {
    let app = TestApp::new(pool);
    let alice = app.register("alice").await;
    let bobby = app.register("bobby").await;

    app.post(
        "/api/v1/friends/requests",
        Some(&alice.token),
        json!({ "username": "bobby" }),
    )
    .await;
    let (status, body) = app
        .post(
            "/api/v1/friends/requests",
            Some(&bobby.token),
            json!({ "username": "alice" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["relationship"], "friends");
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn concurrent_crossed_requests_leave_one_friendship(pool: PgPool) {
    let app = TestApp::new(pool);
    // The race window is narrow, so it is run on many pairs
    for pair in 0..8 {
        let (left, right) = (format!("left{pair}"), format!("right{pair}"));
        let alice = app.register(&left).await;
        let bobby = app.register(&right).await;

        let (a, b) = tokio::join!(
            app.post(
                "/api/v1/friends/requests",
                Some(&alice.token),
                json!({ "username": right })
            ),
            app.post(
                "/api/v1/friends/requests",
                Some(&bobby.token),
                json!({ "username": left })
            ),
        );
        let mut statuses = [a.0, b.0];
        statuses.sort();
        assert_eq!(
            statuses,
            [StatusCode::OK, StatusCode::CREATED],
            "{a:?} {b:?}"
        );
    }

    let statuses: Vec<String> = sqlx::query_scalar("SELECT status FROM friendships")
        .fetch_all(&app.pool)
        .await
        .expect("friendships");
    assert_eq!(statuses, vec!["accepted"; 8]);
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn friends_see_nothing_until_sharing_is_enabled(pool: PgPool) {
    let app = TestApp::new(pool);
    let alice = app.register("alice").await;
    let bobby = app.register("bobby").await;
    befriend(&app, &alice, &bobby, "bobby").await;
    log_training(&app, &bobby).await;

    let (status, profile) = app
        .get(&format!("/api/v1/friends/{}", bobby.id), &alice.token)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(profile["sharing"], sharing(false, false, false, false));

    let (_, feed) = app.get("/api/v1/friends/feed", &alice.token).await;
    assert_eq!(feed["items"], json!([]));
    let (_, feed) = app
        .get(&format!("/api/v1/friends/{}/feed", bobby.id), &alice.token)
        .await;
    assert_eq!(feed["items"], json!([]));

    for path in ["records", "stats/overview?from=2026-09-01&to=2026-09-30"] {
        let (status, _) = app
            .get(
                &format!("/api/v1/friends/{}/{path}", bobby.id),
                &alice.token,
            )
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{path}");
    }
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn sharing_is_selective_and_strips_private_fields(pool: PgPool) {
    let app = TestApp::new(pool);
    let alice = app.register("alice").await;
    let bobby = app.register("bobby").await;
    befriend(&app, &alice, &bobby, "bobby").await;
    log_training(&app, &bobby).await;

    share(&app, &bobby, sharing(true, false, false, false)).await;
    let (_, feed) = app.get("/api/v1/friends/feed", &alice.token).await;
    // Only the finished workout, not the one in progress
    assert_eq!(feed_types(&feed), ["workout"]);
    let item = &feed["items"][0];
    assert_eq!(item["user"]["username"], "bobby");
    assert_eq!(item["volume_kg"], 1000.0);
    for private in ["notes", "place", "routine_id"] {
        assert!(item.get(private).is_none(), "{private} leaked: {item}");
    }
    assert!(item["user"].get("email").is_none());

    share(&app, &bobby, sharing(false, true, false, false)).await;
    let (_, feed) = app.get("/api/v1/friends/feed", &alice.token).await;
    assert_eq!(feed_types(&feed), ["activity"]);
    let item = &feed["items"][0];
    for private in ["notes", "avg_heart_rate", "calories", "perceived_effort"] {
        assert!(item.get(private).is_none(), "{private} leaked: {item}");
    }

    share(&app, &bobby, EVERYTHING).await;
    let (_, feed) = app.get("/api/v1/friends/feed", &alice.token).await;
    // Newest first across both kinds
    assert_eq!(feed_types(&feed), ["activity", "workout"]);

    let (status, records) = app
        .get(
            &format!("/api/v1/friends/{}/records", bobby.id),
            &alice.token,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{records}");
    assert_eq!(records[0]["exercise_name"], "Barbell Bench Press");

    let (status, overview) = app
        .get(
            &format!(
                "/api/v1/friends/{}/stats/overview?from=2026-09-01&to=2026-09-30",
                bobby.id
            ),
            &alice.token,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{overview}");
    assert_eq!(overview["current"]["workouts"], 1);

    // Sharing is one-way: Alice shares nothing with Bobby
    let (status, _) = app
        .get(
            &format!("/api/v1/friends/{}/records", alice.id),
            &bobby.token,
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn feed_pages_across_workouts_and_activities(pool: PgPool) {
    let app = TestApp::new(pool);
    let alice = app.register("alice").await;
    let bobby = app.register("bobby").await;
    befriend(&app, &alice, &bobby, "bobby").await;
    share(&app, &bobby, EVERYTHING).await;
    log_training(&app, &bobby).await;

    let (_, first) = app.get("/api/v1/friends/feed?limit=1", &alice.token).await;
    assert_eq!(feed_types(&first), ["activity"]);
    let cursor = first["next_cursor"].as_str().expect("next cursor");

    let (_, second) = app
        .get(
            &format!("/api/v1/friends/feed?limit=1&cursor={cursor}"),
            &alice.token,
        )
        .await;
    assert_eq!(feed_types(&second), ["workout"]);
    assert!(second["next_cursor"].is_null());
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn strangers_and_pending_requests_see_nothing(pool: PgPool) {
    let app = TestApp::new(pool);
    let alice = app.register("alice").await;
    let bobby = app.register("bobby").await;
    share(&app, &bobby, EVERYTHING).await;
    log_training(&app, &bobby).await;

    app.post(
        "/api/v1/friends/requests",
        Some(&alice.token),
        json!({ "username": "bobby" }),
    )
    .await;

    for path in [
        "",
        "/feed",
        "/records",
        "/stats/overview?from=2026-09-01&to=2026-09-30",
    ] {
        let (status, _) = app
            .get(&format!("/api/v1/friends/{}{path}", bobby.id), &alice.token)
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
    }
    let (_, feed) = app.get("/api/v1/friends/feed", &alice.token).await;
    assert_eq!(feed["items"], json!([]));
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn removing_a_friend_revokes_access_immediately(pool: PgPool) {
    let app = TestApp::new(pool);
    let alice = app.register("alice").await;
    let bobby = app.register("bobby").await;
    befriend(&app, &alice, &bobby, "bobby").await;
    share(&app, &bobby, EVERYTHING).await;
    log_training(&app, &bobby).await;

    let records = format!("/api/v1/friends/{}/records", bobby.id);
    assert_eq!(app.get(&records, &alice.token).await.0, StatusCode::OK);

    // Either side can remove the friendship
    let (status, _) = app
        .delete(&format!("/api/v1/friends/{}", alice.id), &bobby.token)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    assert_eq!(
        app.get(&records, &alice.token).await.0,
        StatusCode::NOT_FOUND
    );
    let (_, feed) = app.get("/api/v1/friends/feed", &alice.token).await;
    assert_eq!(feed["items"], json!([]));
    let (_, friends) = app.get("/api/v1/friends", &alice.token).await;
    assert_eq!(friends, json!([]));
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn blocking_ends_the_friendship_and_hides_both_users(pool: PgPool) {
    let app = TestApp::new(pool);
    let alice = app.register("alice").await;
    let bobby = app.register("bobby").await;
    befriend(&app, &alice, &bobby, "bobby").await;
    share(&app, &alice, EVERYTHING).await;
    share(&app, &bobby, EVERYTHING).await;

    let (status, _) = app
        .request(
            Method::PUT,
            &format!("/api/v1/blocks/{}", bobby.id),
            Some(&alice.token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    for (viewer, owner) in [(&alice, &bobby), (&bobby, &alice)] {
        let (status, _) = app
            .get(
                &format!("/api/v1/friends/{}/records", owner.id),
                &viewer.token,
            )
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (_, friends) = app.get("/api/v1/friends", &viewer.token).await;
        assert_eq!(friends, json!([]));
    }

    // The blocked user cannot find or ask the blocker, and learns nothing about the block
    let (status, _) = app
        .get("/api/v1/users/lookup?username=alice", &bobby.token)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, body) = app
        .post(
            "/api/v1/friends/requests",
            Some(&bobby.token),
            json!({ "username": "alice" }),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["message"], "user not found");

    // The blocker is told to unblock first
    let (status, _) = app
        .post(
            "/api/v1/friends/requests",
            Some(&alice.token),
            json!({ "username": "bobby" }),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);

    let (_, blocks) = app.get("/api/v1/blocks", &alice.token).await;
    assert_eq!(blocks[0]["user"]["username"], "bobby");

    let (status, _) = app
        .delete(&format!("/api/v1/blocks/{}", bobby.id), &alice.token)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    // Unblocking does not restore the friendship
    let (_, friends) = app.get("/api/v1/friends", &alice.token).await;
    assert_eq!(friends, json!([]));
    let (status, _) = app
        .post(
            "/api/v1/friends/requests",
            Some(&bobby.token),
            json!({ "username": "alice" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn blocking_cancels_pending_requests(pool: PgPool) {
    let app = TestApp::new(pool);
    let alice = app.register("alice").await;
    let bobby = app.register("bobby").await;

    app.post(
        "/api/v1/friends/requests",
        Some(&bobby.token),
        json!({ "username": "alice" }),
    )
    .await;
    app.request(
        Method::PUT,
        &format!("/api/v1/blocks/{}", bobby.id),
        Some(&alice.token),
        None,
    )
    .await;

    let (_, requests) = app.get("/api/v1/friends/requests", &alice.token).await;
    assert_eq!(requests["incoming"], json!([]));
    let (status, _) = app
        .post(
            &format!("/api/v1/friends/requests/{}/accept", bobby.id),
            Some(&alice.token),
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn deleting_an_account_removes_its_friendships(pool: PgPool) {
    let app = TestApp::new(pool);
    let alice = app.register("alice").await;
    let bobby = app.register("bobby").await;
    befriend(&app, &alice, &bobby, "bobby").await;
    share(&app, &bobby, EVERYTHING).await;

    let (status, _) = app
        .request(
            Method::DELETE,
            "/api/v1/me",
            Some(&bobby.token),
            Some(json!({ "password": PASSWORD })),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (_, friends) = app.get("/api/v1/friends", &alice.token).await;
    assert_eq!(friends, json!([]));
    let (status, _) = app
        .get(&format!("/api/v1/friends/{}", bobby.id), &alice.token)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn sharing_settings_default_to_private_and_reject_unknown_fields(pool: PgPool) {
    let app = TestApp::new(pool);
    let alice = app.register("alice").await;

    let (_, settings) = app.get("/api/v1/me/sharing", &alice.token).await;
    assert_eq!(settings, sharing(false, false, false, false));

    let mut with_photos = sharing(true, true, true, true);
    with_photos["progress_photos"] = json!(true);
    let (status, _) = app
        .put("/api/v1/me/sharing", &alice.token, with_photos)
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    share(&app, &alice, sharing(true, false, true, false)).await;
    let (_, settings) = app.get("/api/v1/me/sharing", &alice.token).await;
    assert_eq!(settings, sharing(true, false, true, false));
}
