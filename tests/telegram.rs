use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

use axum::{
    Json, Router,
    body::Body,
    extract::{Path, State},
    http::{Request, StatusCode, header},
    response::IntoResponse,
    routing::{get, post},
};
use tower::ServiceExt;
use woodshop::{
    AppState,
    ai::parse_extraction,
    config::Config,
    db,
    telegram::{
        ApiEndpoints, Command, parse_command, router_with_api_endpoints, validate_photo,
        validate_telegram_file_path,
    },
};

#[test]
fn parses_pedido_and_entrega_commands() {
    assert_eq!(
        parse_command("pedido\nJuan\nMesa"),
        Command::Pedido("Juan\nMesa".into())
    );
    assert_eq!(parse_command("ENTREGA 1042"), Command::Entrega(1042));
    assert_eq!(parse_command("entrega 0"), Command::Other);
    assert_eq!(parse_command("entrega 4 extra"), Command::Other);
}

#[test]
fn validates_spanish_and_english_ai_fixtures() {
    for fixture in [
        include_str!("fixtures/extraction_es.json"),
        include_str!("fixtures/extraction_en.json"),
    ] {
        parse_extraction(fixture).expect("fixture should match Extraction and domain validation");
    }
}

#[test]
fn rejects_omitted_optional_keys_instead_of_inventing_nulls() {
    let incomplete = r#"{"customer":{"name":null,"phone":null,"email":null,"address":null,"preferred_language":null,"notes":null},"order":{"description":"Bench","requested_delivery_date":null,"items":[{"product_type":null,"description":"Bench","quantity":1,"length":null,"width":null,"height":null,"dimension_unit":null,"wood_species":null,"finish":null}]},"missing_fields":[]}"#;
    assert!(parse_extraction(incomplete).is_err());
}

#[test]
fn rejects_unknown_or_truncated_photo_bytes() {
    assert!(validate_photo(b"not an image").is_err());
    assert!(validate_photo(&[]).is_err());
    assert!(validate_telegram_file_path("../../etc/passwd").is_err());
    assert!(validate_telegram_file_path("photos/ref.jpg").is_ok());
}

#[derive(Clone)]
struct MockCounts {
    ai: Arc<AtomicUsize>,
    telegram: Arc<AtomicUsize>,
    file: Arc<AtomicUsize>,
    fail_file_once: Arc<AtomicBool>,
    fail_send_once: Arc<AtomicBool>,
    invalid_first_ai: Arc<AtomicBool>,
    invalid_all_ai: Arc<AtomicBool>,
    models: Arc<tokio::sync::Mutex<Vec<String>>>,
    photo: Arc<Vec<u8>>,
}

async fn mock_openrouter(
    State(counts): State<MockCounts>,
    Json(body): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    let attempt = counts.ai.fetch_add(1, Ordering::SeqCst);
    counts
        .models
        .lock()
        .await
        .push(body["model"].as_str().unwrap_or_default().to_owned());
    if counts.invalid_all_ai.load(Ordering::SeqCst)
        || (attempt == 0 && counts.invalid_first_ai.swap(false, Ordering::SeqCst))
    {
        return Json(serde_json::json!({"choices":[{"message":{"content":"not json"}}]}));
    }
    Json(
        serde_json::json!({"choices":[{"message":{"content":include_str!("fixtures/extraction_es.json")}}]}),
    )
}
async fn mock_send(
    State(counts): State<MockCounts>,
    Json(_body): Json<serde_json::Value>,
) -> axum::response::Response {
    counts.telegram.fetch_add(1, Ordering::SeqCst);
    if counts.fail_send_once.swap(false, Ordering::SeqCst) {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"ok":false})),
        )
            .into_response();
    }
    Json(serde_json::json!({"ok":true,"result":{"message_id":1}})).into_response()
}
async fn mock_get_file(State(counts): State<MockCounts>) -> Json<serde_json::Value> {
    Json(
        serde_json::json!({"ok":true,"result":{"file_path":"photos/ref.png","file_size":counts.photo.len()}}),
    )
}
async fn mock_file(
    State(counts): State<MockCounts>,
    Path(_path): Path<String>,
) -> axum::response::Response {
    if counts.fail_file_once.swap(false, Ordering::SeqCst) {
        return (StatusCode::SERVICE_UNAVAILABLE, Vec::new()).into_response();
    }
    counts.file.fetch_add(1, Ordering::SeqCst);
    (
        [(header::CONTENT_TYPE, "image/png")],
        counts.photo.as_ref().clone(),
    )
        .into_response()
}

async fn mock_state() -> (
    AppState,
    String,
    MockCounts,
    tempfile::TempDir,
    tokio::task::JoinHandle<()>,
) {
    let dir = tempfile::tempdir().unwrap();
    let pool = db::connect(&format!("sqlite:{}", dir.path().join("bot.db").display()))
        .await
        .unwrap();
    let config = Config {
        app_env: "test".into(),
        app_port: 0,
        database_url: format!("sqlite:{}", dir.path().join("bot.db").display()),
        telegram_bot_token: "test-token".into(),
        telegram_webhook_secret: "test-secret".into(),
        telegram_allowed_user_ids: vec![42],
        telegram_allowed_chat_ids: vec![],
        openrouter_api_key: "test-key".into(),
        openrouter_model: "fixture-model".into(),
        openrouter_fallback_model: "fixture-fallback".into(),
        public_base_url: "http://localhost".into(),
        data_dir: dir.path().to_string_lossy().into_owned(),
    };
    let state = AppState {
        pool,
        config: Arc::new(config),
        http: reqwest::Client::new(),
    };
    let mut photo = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
        2,
        2,
        image::Rgb([80, 100, 120]),
    ))
    .write_to(&mut photo, image::ImageFormat::Png)
    .unwrap();
    let counts = MockCounts {
        ai: Arc::new(AtomicUsize::new(0)),
        telegram: Arc::new(AtomicUsize::new(0)),
        file: Arc::new(AtomicUsize::new(0)),
        fail_file_once: Arc::new(AtomicBool::new(false)),
        fail_send_once: Arc::new(AtomicBool::new(false)),
        invalid_first_ai: Arc::new(AtomicBool::new(false)),
        invalid_all_ai: Arc::new(AtomicBool::new(false)),
        models: Arc::new(tokio::sync::Mutex::new(Vec::new())),
        photo: Arc::new(photo.into_inner()),
    };
    let app = Router::new()
        .route("/openrouter/chat/completions", post(mock_openrouter))
        .route("/bottest-token/sendMessage", post(mock_send))
        .route("/bottest-token/getFile", get(mock_get_file))
        .route("/file/bottest-token/{*path}", get(mock_file))
        .with_state(counts.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (state, url, counts, dir, task)
}

#[tokio::test]
async fn authenticated_pedido_entrega_and_delivery_photo_use_mock_services() {
    let (state, base, _counts, _dir, server) = mock_state().await;
    let endpoints = ApiEndpoints::for_test(
        base.clone(),
        format!("{base}/file"),
        format!("{base}/openrouter/chat/completions"),
    );
    let app = router_with_api_endpoints(endpoints).with_state(state.clone());
    let send = |body: serde_json::Value| {
        Request::builder()
            .method("POST")
            .uri("/telegram/webhook")
            .header("x-telegram-bot-api-secret-token", "test-secret")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    };
    let pedido = serde_json::json!({"update_id":710,"message":{"message_id":70,"from":{"id":42},"chat":{"id":42,"type":"private"},"caption":"pedido\nJuan Perez\nMesa de roble","photo":[{"file_id":"order-photo","width":2,"height":2}]}});
    let response = app.clone().oneshot(send(pedido)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let order_id: i64 = sqlx::query_scalar("SELECT id FROM orders LIMIT 1")
        .fetch_one(&state.pool)
        .await
        .unwrap();
    let entrega = serde_json::json!({"update_id":711,"message":{"message_id":71,"from":{"id":42},"chat":{"id":42,"type":"private"},"text":format!("entrega {order_id}")}});
    assert_eq!(
        app.clone().oneshot(send(entrega)).await.unwrap().status(),
        StatusCode::OK
    );
    let album_one = serde_json::json!({"update_id":712,"message":{"message_id":72,"media_group_id":"delivery-album-72","from":{"id":42},"chat":{"id":42,"type":"private"},"photo":[{"file_id":"delivery-photo-a","width":2,"height":2}]}});
    let album_two = serde_json::json!({"update_id":713,"message":{"message_id":73,"media_group_id":"delivery-album-72","from":{"id":42},"chat":{"id":42,"type":"private"},"photo":[{"file_id":"delivery-photo-b","width":2,"height":2}]}});
    let (one, two) = tokio::join!(
        app.clone().oneshot(send(album_one)),
        app.oneshot(send(album_two))
    );
    assert_eq!(one.unwrap().status(), StatusCode::OK);
    assert_eq!(two.unwrap().status(), StatusCode::OK);
    let order_photos: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM attachments WHERE order_id=? AND type='ORDER_REFERENCE'",
    )
    .bind(order_id)
    .fetch_one(&state.pool)
    .await
    .unwrap();
    let delivery_photos: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM attachments WHERE order_id=? AND type='DELIVERY_PROOF'",
    )
    .bind(order_id)
    .fetch_one(&state.pool)
    .await
    .unwrap();
    assert_eq!(order_photos, 1);
    assert_eq!(delivery_photos, 2);
    server.abort();
}

#[tokio::test]
async fn committed_order_retry_resumes_photo_without_second_ai_call() {
    let (state, base, counts, _dir, server) = mock_state().await;
    sqlx::query("CREATE TRIGGER reject_first_photo BEFORE INSERT ON attachments BEGIN SELECT RAISE(ABORT,'temporary attachment failure'); END")
        .execute(&state.pool).await.unwrap();
    let endpoints = ApiEndpoints::for_test(
        base.clone(),
        format!("{base}/file"),
        format!("{base}/openrouter/chat/completions"),
    );
    let app = router_with_api_endpoints(endpoints).with_state(state.clone());
    let payload = serde_json::json!({"update_id":820,"message":{"message_id":82,"from":{"id":42},"chat":{"id":42,"type":"private"},"caption":"pedido\nJuan Perez\nMesa de roble","photo":[{"file_id":"retry-photo","width":2,"height":2}]}}).to_string();
    let send = || {
        Request::builder()
            .method("POST")
            .uri("/telegram/webhook")
            .header("x-telegram-bot-api-secret-token", "test-secret")
            .header("content-type", "application/json")
            .body(Body::from(payload.clone()))
            .unwrap()
    };
    assert_eq!(
        app.clone().oneshot(send()).await.unwrap().status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    sqlx::query("DROP TRIGGER reject_first_photo")
        .execute(&state.pool)
        .await
        .unwrap();
    assert_eq!(app.oneshot(send()).await.unwrap().status(), StatusCode::OK);
    let orders: i64 = sqlx::query_scalar("SELECT count(*) FROM orders")
        .fetch_one(&state.pool)
        .await
        .unwrap();
    let attachments: i64 =
        sqlx::query_scalar("SELECT count(*) FROM attachments WHERE type='ORDER_REFERENCE'")
            .fetch_one(&state.pool)
            .await
            .unwrap();
    assert_eq!(orders, 1);
    assert_eq!(attachments, 1);
    assert_eq!(counts.ai.load(Ordering::SeqCst), 1);
    server.abort();
}

#[tokio::test]
async fn failed_reply_retries_without_recreating_order_or_calling_ai() {
    let (state, base, counts, _dir, server) = mock_state().await;
    counts.fail_send_once.store(true, Ordering::SeqCst);
    let endpoints = ApiEndpoints::for_test(
        base.clone(),
        format!("{base}/file"),
        format!("{base}/openrouter/chat/completions"),
    );
    let app = router_with_api_endpoints(endpoints).with_state(state.clone());
    let payload = update(825, 42, "pedido\nMesa de roble").to_string();
    let send = || {
        Request::builder()
            .method("POST")
            .uri("/telegram/webhook")
            .header("x-telegram-bot-api-secret-token", "test-secret")
            .header("content-type", "application/json")
            .body(Body::from(payload.clone()))
            .unwrap()
    };
    assert_eq!(
        app.clone().oneshot(send()).await.unwrap().status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(app.oneshot(send()).await.unwrap().status(), StatusCode::OK);
    let orders: i64 = sqlx::query_scalar("SELECT count(*) FROM orders")
        .fetch_one(&state.pool)
        .await
        .unwrap();
    assert_eq!(orders, 1);
    assert_eq!(counts.ai.load(Ordering::SeqCst), 1);
    assert_eq!(counts.telegram.load(Ordering::SeqCst), 2);
    server.abort();
}

#[tokio::test]
async fn pedido_album_caption_does_not_lose_companion_photo() {
    let (state, base, _counts, _dir, server) = mock_state().await;
    let endpoints = ApiEndpoints::for_test(
        base.clone(),
        format!("{base}/file"),
        format!("{base}/openrouter/chat/completions"),
    );
    let app = router_with_api_endpoints(endpoints).with_state(state.clone());
    let send = |body: serde_json::Value| {
        let app = app.clone();
        async move {
            app.oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/telegram/webhook")
                    .header("x-telegram-bot-api-secret-token", "test-secret")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap()
        }
    };
    let leader = serde_json::json!({"update_id":830,"message":{"message_id":83,"media_group_id":"album-83","from":{"id":42},"chat":{"id":42,"type":"private"},"caption":"pedido\nJuan Perez\nMesa de roble","photo":[{"file_id":"album-a","width":2,"height":2}]}});
    let companion = serde_json::json!({"update_id":831,"message":{"message_id":84,"media_group_id":"album-83","from":{"id":42},"chat":{"id":42,"type":"private"},"photo":[{"file_id":"album-b","width":2,"height":2}]}});
    let (first, second) = tokio::join!(send(leader), send(companion));
    assert_eq!(first.status(), StatusCode::OK);
    assert_eq!(second.status(), StatusCode::OK);
    let orders: i64 = sqlx::query_scalar("SELECT count(*) FROM orders")
        .fetch_one(&state.pool)
        .await
        .unwrap();
    let photos: i64 =
        sqlx::query_scalar("SELECT count(*) FROM attachments WHERE type='ORDER_REFERENCE'")
            .fetch_one(&state.pool)
            .await
            .unwrap();
    assert_eq!(orders, 1);
    assert_eq!(photos, 2);
    server.abort();
}

#[tokio::test]
async fn persisted_album_member_survives_router_restart_before_caption_arrives() {
    let (state, base, _counts, _dir, server) = mock_state().await;
    let endpoints = ApiEndpoints::for_test(
        base.clone(),
        format!("{base}/file"),
        format!("{base}/openrouter/chat/completions"),
    );
    let app = router_with_api_endpoints(endpoints.clone()).with_state(state.clone());
    let companion = serde_json::json!({"update_id":840,"message":{"message_id":84,"media_group_id":"restart-album","from":{"id":42},"chat":{"id":42,"type":"private"},"photo":[{"file_id":"persisted-b","width":2,"height":2}]}});
    let send = |app: axum::Router, body: serde_json::Value| async move {
        app.oneshot(
            Request::builder()
                .method("POST")
                .uri("/telegram/webhook")
                .header("x-telegram-bot-api-secret-token", "test-secret")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
    };
    assert_eq!(send(app, companion).await.status(), StatusCode::OK);
    state.pool.close().await;
    let pool = db::connect(&state.config.database_url).await.unwrap();
    let restarted = AppState {
        pool: pool.clone(),
        config: state.config.clone(),
        http: state.http.clone(),
    };
    let app = router_with_api_endpoints(endpoints).with_state(restarted);
    let caption = serde_json::json!({"update_id":841,"message":{"message_id":85,"media_group_id":"restart-album","from":{"id":42},"chat":{"id":42,"type":"private"},"caption":"pedido\nJuan Perez\nMesa","photo":[{"file_id":"persisted-a","width":2,"height":2}]}});
    assert_eq!(send(app, caption).await.status(), StatusCode::OK);
    let photos: i64 =
        sqlx::query_scalar("SELECT count(*) FROM attachments WHERE type='ORDER_REFERENCE'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(photos, 2);
    server.abort();
}

#[tokio::test]
async fn late_pedido_caption_reclassifies_companion_from_delivery_to_reference() {
    let (state, base, _counts, _dir, server) = mock_state().await;
    let existing: woodshop::domain::Extraction =
        serde_json::from_str(include_str!("fixtures/extraction_es.json")).unwrap();
    let delivered_order = db::create_order(&state.pool, &existing, "existing order", 42, "fixture")
        .await
        .unwrap();
    db::deliver_order(&state.pool, delivered_order, 42, 42, None)
        .await
        .unwrap();
    let endpoints = ApiEndpoints::for_test(
        base.clone(),
        format!("{base}/file"),
        format!("{base}/openrouter/chat/completions"),
    );
    let app = router_with_api_endpoints(endpoints).with_state(state.clone());
    let send = |body: serde_json::Value| {
        let app = app.clone();
        async move {
            app.oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/telegram/webhook")
                    .header("x-telegram-bot-api-secret-token", "test-secret")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap()
        }
    };
    let companion = serde_json::json!({"update_id":850,"message":{"message_id":85,"media_group_id":"late-caption","from":{"id":42},"chat":{"id":42,"type":"private"},"photo":[{"file_id":"late-b","width":2,"height":2}]}});
    assert_eq!(send(companion).await.status(), StatusCode::OK);
    let mistakenly_delivery: i64 =
        sqlx::query_scalar("SELECT count(*) FROM attachments WHERE type='DELIVERY_PROOF'")
            .fetch_one(&state.pool)
            .await
            .unwrap();
    assert_eq!(mistakenly_delivery, 1);
    let caption = serde_json::json!({"update_id":851,"message":{"message_id":86,"media_group_id":"late-caption","from":{"id":42},"chat":{"id":42,"type":"private"},"caption":"pedido\nJuan Perez\nMesa nueva","photo":[{"file_id":"late-a","width":2,"height":2}]}});
    assert_eq!(send(caption).await.status(), StatusCode::OK);
    let delivery: i64 =
        sqlx::query_scalar("SELECT count(*) FROM attachments WHERE type='DELIVERY_PROOF'")
            .fetch_one(&state.pool)
            .await
            .unwrap();
    let references: i64 =
        sqlx::query_scalar("SELECT count(*) FROM attachments WHERE type='ORDER_REFERENCE'")
            .fetch_one(&state.pool)
            .await
            .unwrap();
    assert_eq!(delivery, 0);
    assert_eq!(references, 2);
    server.abort();
}

#[tokio::test]
async fn late_album_companion_after_pedido_commit_attaches_to_existing_order() {
    let (state, base, counts, _dir, server) = mock_state().await;
    let endpoints = ApiEndpoints::for_test(
        base.clone(),
        format!("{base}/file"),
        format!("{base}/openrouter/chat/completions"),
    );
    let app = router_with_api_endpoints(endpoints).with_state(state.clone());
    let send = |body: serde_json::Value| {
        let app = app.clone();
        async move {
            app.oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/telegram/webhook")
                    .header("x-telegram-bot-api-secret-token", "test-secret")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap()
        }
    };
    let caption = serde_json::json!({"update_id":860,"message":{"message_id":86,"media_group_id":"caption-first","from":{"id":42},"chat":{"id":42,"type":"private"},"caption":"pedido\nMesa nueva","photo":[{"file_id":"caption-first-a","width":2,"height":2}]}});
    assert_eq!(send(caption).await.status(), StatusCode::OK);
    let order_id: i64 = sqlx::query_scalar("SELECT id FROM orders")
        .fetch_one(&state.pool)
        .await
        .unwrap();
    let late = serde_json::json!({"update_id":861,"message":{"message_id":87,"media_group_id":"caption-first","from":{"id":42},"chat":{"id":42,"type":"private"},"photo":[{"file_id":"caption-first-b","width":2,"height":2}]}});
    assert_eq!(send(late).await.status(), StatusCode::OK);
    let references: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM attachments WHERE order_id=? AND type='ORDER_REFERENCE'",
    )
    .bind(order_id)
    .fetch_one(&state.pool)
    .await
    .unwrap();
    let orders: i64 = sqlx::query_scalar("SELECT count(*) FROM orders")
        .fetch_one(&state.pool)
        .await
        .unwrap();
    assert_eq!(references, 2);
    assert_eq!(orders, 1);
    assert_eq!(counts.ai.load(Ordering::SeqCst), 1);
    server.abort();
}

#[tokio::test]
async fn duplicate_delivery_photo_is_serialized_and_session_is_user_chat_expiry_scoped() {
    let (state, base, _counts, _dir, server) = mock_state().await;
    let extraction: woodshop::domain::Extraction =
        serde_json::from_str(include_str!("fixtures/extraction_es.json")).unwrap();
    let order = db::create_order(&state.pool, &extraction, "session test", 42, "fixture")
        .await
        .unwrap();
    db::deliver_order(&state.pool, order, 42, 42, None)
        .await
        .unwrap();
    let endpoints = ApiEndpoints::for_test(
        base.clone(),
        format!("{base}/file"),
        format!("{base}/openrouter/chat/completions"),
    );
    let app = router_with_api_endpoints(endpoints).with_state(state.clone());
    let body=serde_json::json!({"update_id":870,"message":{"message_id":87,"from":{"id":42},"chat":{"id":42,"type":"private"},"photo":[{"file_id":"same-delivery-photo","width":2,"height":2}]}}).to_string();
    let send = || {
        Request::builder()
            .method("POST")
            .uri("/telegram/webhook")
            .header("x-telegram-bot-api-secret-token", "test-secret")
            .header("content-type", "application/json")
            .body(Body::from(body.clone()))
            .unwrap()
    };
    let (a, b) = tokio::join!(app.clone().oneshot(send()), app.clone().oneshot(send()));
    assert_eq!(a.unwrap().status(), StatusCode::OK);
    assert_eq!(b.unwrap().status(), StatusCode::OK);
    let initial: i64 =
        sqlx::query_scalar("SELECT count(*) FROM attachments WHERE type='DELIVERY_PROOF'")
            .fetch_one(&state.pool)
            .await
            .unwrap();
    assert_eq!(initial, 1);
    sqlx::query("UPDATE telegram_sessions SET expires_at=strftime('%Y-%m-%dT%H:%M:%fZ','now','-1 minute') WHERE user_id=42 AND chat_id=42").execute(&state.pool).await.unwrap();
    let expired = serde_json::json!({"update_id":871,"message":{"message_id":88,"from":{"id":42},"chat":{"id":42,"type":"private"},"photo":[{"file_id":"expired-photo","width":2,"height":2}]}});
    let expired_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/telegram/webhook")
                .header("x-telegram-bot-api-secret-token", "test-secret")
                .header("content-type", "application/json")
                .body(Body::from(expired.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(expired_response.status(), StatusCode::OK);
    db::deliver_order(
        &state.pool,
        db::create_order(&state.pool, &extraction, "other chat", 42, "fixture")
            .await
            .unwrap(),
        42,
        777,
        None,
    )
    .await
    .unwrap();
    let wrong_chat = serde_json::json!({"update_id":872,"message":{"message_id":89,"from":{"id":42},"chat":{"id":42,"type":"private"},"photo":[{"file_id":"wrong-chat-photo","width":2,"height":2}]}});
    assert_eq!(
        app.oneshot(
            Request::builder()
                .method("POST")
                .uri("/telegram/webhook")
                .header("x-telegram-bot-api-secret-token", "test-secret")
                .header("content-type", "application/json")
                .body(Body::from(wrong_chat.to_string()))
                .unwrap()
        )
        .await
        .unwrap()
        .status(),
        StatusCode::OK
    );
    let final_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM attachments WHERE type='DELIVERY_PROOF'")
            .fetch_one(&state.pool)
            .await
            .unwrap();
    assert_eq!(final_count, 1);
    server.abort();
}

fn update(update_id: i64, user_id: i64, text: &str) -> serde_json::Value {
    serde_json::json!({"update_id":update_id,"message":{"message_id":update_id,"from":{"id":user_id},"chat":{"id":user_id,"type":"private"},"text":text}})
}

#[tokio::test]
async fn unauthorized_webhook_is_not_persisted_and_pedido_retry_is_idempotent() {
    let (state, base, counts, _dir, server) = mock_state().await;
    let endpoints = ApiEndpoints::for_test(
        base.clone(),
        format!("{base}/file"),
        format!("{base}/openrouter/chat/completions"),
    );
    let app = router_with_api_endpoints(endpoints).with_state(state.clone());
    let unauthorized = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/telegram/webhook")
                .header("x-telegram-bot-api-secret-token", "test-secret")
                .header("content-type", "application/json")
                .body(Body::from(update(500, 99, "pedido\nMesa").to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::OK);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM telegram_updates")
        .fetch_one(&state.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    let request = || {
        Request::builder()
            .method("POST")
            .uri("/telegram/webhook")
            .header("x-telegram-bot-api-secret-token", "test-secret")
            .header("content-type", "application/json")
            .body(Body::from(
                update(
                    501,
                    42,
                    "pedido\nJuan Perez\n8325551212\nMesa rustica de roble",
                )
                .to_string(),
            ))
            .unwrap()
    };
    let (a, b) = tokio::join!(
        app.clone().oneshot(request()),
        app.clone().oneshot(request())
    );
    assert_eq!(a.unwrap().status(), StatusCode::OK);
    assert_eq!(b.unwrap().status(), StatusCode::OK);
    let orders: i64 = sqlx::query_scalar("SELECT count(*) FROM orders")
        .fetch_one(&state.pool)
        .await
        .unwrap();
    assert_eq!(orders, 1);
    assert_eq!(counts.ai.load(Ordering::SeqCst), 1);
    assert_eq!(counts.telegram.load(Ordering::SeqCst), 1);
    server.abort();
}

#[tokio::test]
async fn invalid_model_json_uses_one_fallback_attempt_only() {
    let (state, base, counts, _dir, server) = mock_state().await;
    counts.invalid_first_ai.store(true, Ordering::SeqCst);
    let endpoints = ApiEndpoints::for_test(
        base.clone(),
        format!("{base}/file"),
        format!("{base}/openrouter/chat/completions"),
    );
    let app = router_with_api_endpoints(endpoints).with_state(state.clone());
    let body = update(880, 42, "pedido\nMesa de roble");
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/telegram/webhook")
                .header("x-telegram-bot-api-secret-token", "test-secret")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(counts.ai.load(Ordering::SeqCst), 2);
    assert_eq!(
        *counts.models.lock().await,
        vec!["fixture-model", "fixture-fallback"]
    );
    let orders: i64 = sqlx::query_scalar("SELECT count(*) FROM orders")
        .fetch_one(&state.pool)
        .await
        .unwrap();
    assert_eq!(orders, 1);
    server.abort();
}

#[tokio::test]
async fn failed_extraction_is_audited_with_the_last_attempted_model() {
    let (state, base, counts, _dir, server) = mock_state().await;
    counts.invalid_all_ai.store(true, Ordering::SeqCst);
    let endpoints = ApiEndpoints::for_test(
        base.clone(),
        format!("{base}/file"),
        format!("{base}/openrouter/chat/completions"),
    );
    let app = router_with_api_endpoints(endpoints).with_state(state.clone());
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/telegram/webhook")
                .header("x-telegram-bot-api-secret-token", "test-secret")
                .header("content-type", "application/json")
                .body(Body::from(
                    update(881, 42, "pedido\nMesa sin detalles").to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(counts.ai.load(Ordering::SeqCst), 2);
    let audit: (String, String, String) =
        sqlx::query_as("SELECT original_text,model,status FROM ai_requests")
            .fetch_one(&state.pool)
            .await
            .unwrap();
    assert_eq!(audit.0, "pedido\nMesa sin detalles");
    assert_eq!(audit.1, "fixture-fallback");
    assert_eq!(audit.2, "failed");
    let orders: i64 = sqlx::query_scalar("SELECT count(*) FROM orders")
        .fetch_one(&state.pool)
        .await
        .unwrap();
    assert_eq!(orders, 0);
    server.abort();
}
