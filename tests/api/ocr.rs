use axum::http::{Method, StatusCode};
use fittune_api::health::ocr::{Runtime, reserve_daily};
use serde_json::json;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

use crate::common::TestApp;

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn model_setting_requires_admin_and_persists_without_exposing_key(pool: PgPool) {
    let app = TestApp::new(pool.clone());
    let user = app.register("reader").await;
    let path = "/api/v1/admin/ocr-settings";
    assert_eq!(
        app.request(Method::GET, path, None, None).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(app.get(path, &user.token).await.0, StatusCode::FORBIDDEN);
    assert_eq!(
        app.put(path, &user.token, json!({"ocr_model":"gpt-6-astra"}))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    app.make_admin(&user).await;
    let (_, before) = app.get(path, &user.token).await;
    assert_eq!(before["ocr_model"], "gpt-6-luna");
    assert_eq!(before["ai_configured"], false);
    assert!(before.get("api_key").is_none());
    assert_eq!(
        app.put(path, &user.token, json!({"ocr_model":"arbitrary-model"}))
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        app.put(
            path,
            &user.token,
            json!({"ocr_model":"gpt-6-astra","api_key":"forbidden"})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        app.put(path, &user.token, json!({"ocr_model":"gpt-6-astra"}))
            .await
            .0,
        StatusCode::OK
    );
    let restarted = TestApp::new(pool);
    let (_, after) = restarted.get(path, &user.token).await;
    assert_eq!(after["ocr_model"], "gpt-6-astra");
    assert_eq!(after["updated_by"], user.id);
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn parser_is_authenticated_bounded_and_never_stores_results(pool: PgPool) {
    let app = TestApp::new(pool);
    let user = app.register("scanner").await;
    let path = "/api/v1/health/ocr/parse";
    let input = json!({"width":600,"height":300,"observations":[
        {"text":"per 100 ml","confidence":0.95,"bbox":[400,0,480,20]},
        {"text":"Protein","confidence":0.95,"bbox":[0,40,80,60]},
        {"text":"2,5 g","confidence":0.9,"bbox":[400,40,460,60]}
    ]});
    assert_eq!(
        app.post(path, None, input.clone()).await.0,
        StatusCode::UNAUTHORIZED
    );
    let (status, headers, bytes) = app
        .raw(
            Method::POST,
            path,
            Some(&user.token),
            Some("application/json"),
            input.to_string().into_bytes(),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["cache-control"], "no-store");
    let result: serde_json::Value = serde_json::from_slice(&bytes).expect("valid test data");
    assert_eq!(result["values"]["protein_g"], 2.5);
    assert_eq!(result["unit"], "ml");
    assert!(result["values"]["energy_kcal"].is_null());
    assert_eq!(
        app.post(
            path,
            Some(&user.token),
            json!({"width":2049,"height":1,"observations":[]})
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let mut invalid = input.clone();
    invalid["observations"][0]["bbox"][2] = json!(601);
    assert_eq!(
        app.post(path, Some(&user.token), invalid).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let (_, caps) = app
        .get("/api/v1/health/ocr/capabilities", &user.token)
        .await;
    assert_eq!(caps, json!({"ai":false,"rapid":false}));
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn daily_quota_is_atomic_and_survives_runtime_restart(pool: PgPool) {
    let app = TestApp::new(pool.clone());
    let user = app.register("quota").await;
    let id = Uuid::parse_str(&user.id).expect("valid test data");
    for _ in 0..19 {
        reserve_daily(&pool, id).await.expect("valid test data");
    }
    let (a, b) = tokio::join!(reserve_daily(&pool, id), reserve_daily(&pool, id));
    assert_ne!(a.is_ok(), b.is_ok());
    let _restart = Runtime::default();
    assert!(reserve_daily(&pool, id).await.is_err());
    let counts: Vec<i32> = sqlx::query_scalar("SELECT attempts FROM ocr_ai_daily ORDER BY subject")
        .fetch_all(&pool)
        .await
        .expect("valid test data");
    assert_eq!(counts, vec![20, 20]);
    sqlx::query("UPDATE ocr_ai_daily SET attempts=200 WHERE subject='global'")
        .execute(&pool)
        .await
        .expect("valid test data");
    let other = app.register("otherquota").await;
    assert!(
        reserve_daily(&pool, Uuid::parse_str(&other.id).expect("valid test data"))
            .await
            .is_err()
    );
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM ocr_ai_daily WHERE subject=$1")
        .bind(other.id)
        .fetch_one(&pool)
        .await
        .expect("valid test data");
    assert_eq!(n, 0);
}

#[test]
fn admission_caps_concurrency_and_releases_cancelled_work() {
    let runtime = Arc::new(Runtime::default());
    let user = Uuid::new_v4();
    let first = runtime.admit(user, true).expect("valid test data");
    assert!(runtime.admit(user, true).is_err());
    assert!(runtime.admit(user, false).is_err());
    let second = runtime
        .admit(Uuid::new_v4(), true)
        .expect("valid test data");
    assert!(runtime.admit(Uuid::new_v4(), true).is_err());
    drop(first);
    assert!(runtime.admit(user, false).is_ok());
    drop(second);
    assert!(runtime.admit(user, true).is_ok());
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn ai_upload_uses_server_model_and_key_and_counts_failed_attempts(pool: PgPool) {
    use axum::{Router, body::Bytes, http::HeaderMap, routing::post};
    use fittune_api::{config::Registration, health::ocr::model::FIELDS};
    use std::sync::atomic::{AtomicUsize, Ordering};
    let calls = Arc::new(AtomicUsize::new(0));
    let recorded = Arc::new(std::sync::Mutex::new(None));
    let captured = recorded.clone();
    let counts = calls.clone();
    let mock = Router::new().route("/responses", post(move |headers: HeaderMap, bytes: Bytes| {
        let recorded = captured.clone(); let calls = counts.clone();
        async move {
            assert_eq!(headers["authorization"], "Bearer test-provider-key");
            let input: serde_json::Value = serde_json::from_slice(&bytes).expect("request JSON");
            *recorded.lock().expect("request lock") = Some(input);
            if calls.fetch_add(1, Ordering::SeqCst) > 0 {
                return (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(json!({"error":"private provider details"})));
            }
            let read = json!({"unit":"g","basis_amount":100,
                "values":FIELDS.iter().map(|f| ((*f).to_owned(), json!(1))).collect::<serde_json::Map<_,_>>(),
                "evidence":FIELDS.iter().map(|f| ((*f).to_owned(), json!("per 100 g: 1"))).collect::<serde_json::Map<_,_>>()});
            (StatusCode::OK, axum::Json(json!({"status":"completed","output":[{"content":[{"type":"output_text","text":read.to_string()}]}]})))
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("mock bind");
    let endpoint = format!(
        "http://{}/responses",
        listener.local_addr().expect("mock address")
    );
    let server = tokio::spawn(async move {
        axum::serve(listener, mock).await.expect("mock server");
    });
    let mut config = crate::common::config(Registration::Open);
    config.openai_key = Some("test-provider-key".into());
    config.openai_endpoint = endpoint;
    let app = TestApp::with_config(pool.clone(), config);
    let user = app.register("provider").await;
    app.make_admin(&user).await;
    assert_eq!(
        app.put(
            "/api/v1/admin/ocr-settings",
            &user.token,
            json!({"ocr_model":"gpt-6-astra"})
        )
        .await
        .0,
        StatusCode::OK
    );
    let photo = image::DynamicImage::new_rgb8(24, 12);
    let mut png = std::io::Cursor::new(Vec::new());
    photo
        .write_to(&mut png, image::ImageFormat::Png)
        .expect("test photo");
    let mut multipart = b"--fixture\r\nContent-Disposition: form-data; name=\"file\"; filename=\"label.png\"\r\nContent-Type: image/png\r\n\r\n".to_vec();
    multipart.extend(png.into_inner());
    multipart.extend(b"\r\n--fixture\r\nContent-Disposition: form-data; name=\"text\"\r\n\r\nPrinted label OCR\r\n--fixture--\r\n");
    let upload = || {
        app.raw(
            Method::POST,
            "/api/v1/health/ocr/ai",
            Some(&user.token),
            Some("multipart/form-data; boundary=fixture"),
            multipart.clone(),
        )
    };
    let (status, headers, bytes) = upload().await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["cache-control"], "no-store");
    let result: serde_json::Value = serde_json::from_slice(&bytes).expect("extraction JSON");
    assert_eq!(result["source"], "ai");
    let input = recorded
        .lock()
        .expect("request lock")
        .clone()
        .expect("captured request");
    assert_eq!(input["model"], "gpt-6-astra");
    assert_eq!(input["store"], false);
    assert_eq!(input["input"][0]["content"][1]["text"], "Printed label OCR");
    assert_eq!(input["text"]["format"]["strict"], true);
    let image_url = input["input"][0]["content"][0]["image_url"]
        .as_str()
        .expect("crop URL");
    use base64::Engine;
    let sanitized = base64::engine::general_purpose::STANDARD
        .decode(
            image_url
                .strip_prefix("data:image/jpeg;base64,")
                .expect("JPEG URL"),
        )
        .expect("image data");
    let decoded = image::load_from_memory(&sanitized).expect("sanitized crop");
    assert_eq!((decoded.width(), decoded.height()), (24, 12));
    let (status, headers, bytes) = upload().await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(headers.contains_key("retry-after"));
    assert!(!String::from_utf8_lossy(&bytes).contains("private provider details"));
    let (status, headers, _) = upload().await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert!(headers.contains_key("retry-after"));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    let daily: i32 = sqlx::query_scalar("SELECT attempts FROM ocr_ai_daily WHERE subject=$1")
        .bind(user.id)
        .fetch_one(&pool)
        .await
        .expect("quota row");
    assert_eq!(daily, 2);
    server.abort();
}
