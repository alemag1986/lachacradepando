use std::{path::PathBuf, sync::Arc};

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use http_body_util::BodyExt;
use sqlx::SqlitePool;
use tempfile::TempDir;
use tower::ServiceExt;
use woodshop::{AppState, config::Config, web};

struct Fixture {
    app: Router,
    pool: SqlitePool,
    _temp: TempDir,
}

async fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let db_path = temp.path().join("web.sqlite");
    let db_url = format!("sqlite://{}", db_path.display());
    let pool = woodshop::db::connect(&db_url).await.unwrap();
    let config = Config {
        app_env: "test".into(),
        app_port: 8080,
        database_url: db_url,
        telegram_bot_token: String::new(),
        telegram_webhook_secret: String::new(),
        telegram_allowed_user_ids: vec![],
        telegram_allowed_chat_ids: vec![],
        openrouter_api_key: String::new(),
        openrouter_model: String::new(),
        openrouter_fallback_model: String::new(),
        public_base_url: "https://woodshop.test".into(),
        data_dir: temp.path().display().to_string(),
    };
    let state = AppState {
        pool: pool.clone(),
        config: Arc::new(config),
        http: reqwest::Client::new(),
    };
    Fixture {
        app: web::router().with_state(state),
        pool,
        _temp: temp,
    }
}

async fn seed_customer(pool: &SqlitePool, name: Option<&str>) -> i64 {
    sqlx::query("INSERT INTO customers(name,normalized_name) VALUES(?,?)")
        .bind(name)
        .bind(name.map(str::to_lowercase))
        .execute(pool)
        .await
        .unwrap()
        .last_insert_rowid()
}

async fn seed_order(
    pool: &SqlitePool,
    customer: i64,
    status: &str,
    request: &str,
    due: Option<&str>,
) -> i64 {
    sqlx::query("INSERT INTO orders(customer_id,status,original_text,normalized_description,created_by,promised_delivery_date) VALUES(?,?,?,?,0,?)")
        .bind(customer).bind(status).bind(request).bind(request).bind(due).execute(pool).await.unwrap().last_insert_rowid()
}

async fn response(app: &Router, path: &str) -> axum::response::Response {
    app.clone()
        .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
        .await
        .unwrap()
}
async fn body_text(res: axum::response::Response) -> String {
    String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap()
}

#[tokio::test]
async fn customer_list_and_detail_escape_untrusted_content() {
    let f = fixture().await;
    let cid = seed_customer(&f.pool, Some("<script>alert(1)</script>")).await;
    let customer_res = response(&f.app, "/customers").await;
    assert_eq!(customer_res.status(), StatusCode::OK);
    let list = body_text(customer_res).await;
    assert!(list.contains("&#60;script&#62;alert(1)&#60;/script&#62;"));
    assert!(!list.contains("<script>alert(1)</script>"));
    let detail = body_text(response(&f.app, &format!("/customers/{cid}")).await).await;
    assert!(detail.contains("&#60;script&#62;alert(1)&#60;/script&#62;"));
}

#[tokio::test]
async fn invalid_ids_and_attachment_traversal_are_not_found() {
    let f = fixture().await;
    for path in [
        "/customers/0",
        "/customers/nope",
        "/orders/-1",
        "/orders/99999",
    ] {
        assert_eq!(
            response(&f.app, path).await.status(),
            StatusCode::NOT_FOUND,
            "{path}"
        );
    }
    let cid = seed_customer(&f.pool, Some("Marta")).await;
    let oid = seed_order(&f.pool, cid, "NEW", "bench", None).await;
    let aid = sqlx::query("INSERT INTO attachments(order_id,type,path,mime_type) VALUES(?,'ORDER_REFERENCE','../../etc/passwd','image/jpeg')").bind(oid).execute(&f.pool).await.unwrap().last_insert_rowid();
    assert_eq!(
        response(&f.app, &format!("/attachments/{aid}"))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn order_filters_are_bound_and_apply_customer_status_date_and_search() {
    let f = fixture().await;
    let a = seed_customer(&f.pool, Some("Ana")).await;
    let b = seed_customer(&f.pool, Some("Beto")).await;
    seed_order(&f.pool, a, "NEW", "Oak writing desk", Some("2030-04-05")).await;
    seed_order(&f.pool, b, "READY", "Walnut chair", Some("2030-04-06")).await;
    let res = response(
        &f.app,
        "/orders?status=NEW&customer=Ana&delivery_date=2030-04-05&search=writing",
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);
    let text = body_text(res).await;
    assert!(text.contains("Ana"));
    assert!(text.contains("Oak writing desk"));
    assert!(!text.contains("Walnut chair"));
    let injection = response(&f.app, "/orders?search=%27%20OR%201%3D1--").await;
    assert_eq!(injection.status(), StatusCode::OK);
    assert!(!body_text(injection).await.contains("Walnut chair"));
}

fn post(path: &str, data: &str, origin: Option<&str>) -> Request<Body> {
    let mut b = Request::builder()
        .method("POST")
        .uri(path)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
    if let Some(origin) = origin {
        b = b.header(header::ORIGIN, origin);
    }
    b.body(Body::from(data.to_owned())).unwrap()
}

#[tokio::test]
async fn management_requires_origin_and_writes_event_and_normalized_contact_atomically() {
    let f = fixture().await;
    let cid = seed_customer(&f.pool, Some("Marta Lopez")).await;
    let oid = seed_order(&f.pool, cid, "NEW", "Dining table", None).await;
    let denied = f
        .app
        .clone()
        .oneshot(post(
            &format!("/orders/{oid}/status"),
            "status=CONFIRMED",
            None,
        ))
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    let ok = f
        .app
        .clone()
        .oneshot(post(
            &format!("/orders/{oid}/status"),
            "status=CONFIRMED",
            Some("https://woodshop.test"),
        ))
        .await
        .unwrap();
    assert_eq!(ok.status(), StatusCode::SEE_OTHER);
    let status: String = sqlx::query_scalar("SELECT status FROM orders WHERE id=?")
        .bind(oid)
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(status, "CONFIRMED");
    let events: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM order_events WHERE order_id=? AND event_type='STATUS_CHANGED' AND created_by=0").bind(oid).fetch_one(&f.pool).await.unwrap();
    assert_eq!(events, 1);
    let illegal = f
        .app
        .clone()
        .oneshot(post(
            &format!("/orders/{oid}/status"),
            "status=NEW",
            Some("https://woodshop.test"),
        ))
        .await
        .unwrap();
    assert_eq!(illegal.status(), StatusCode::BAD_REQUEST);
    let date = f
        .app
        .clone()
        .oneshot(post(
            &format!("/orders/{oid}/promised-date"),
            "promised_delivery_date=2030-01-02",
            Some("https://woodshop.test"),
        ))
        .await
        .unwrap();
    assert_eq!(date.status(), StatusCode::SEE_OTHER);
    let clear_date = f
        .app
        .clone()
        .oneshot(post(
            &format!("/orders/{oid}/promised-date"),
            "promised_delivery_date=",
            Some("https://woodshop.test"),
        ))
        .await
        .unwrap();
    assert_eq!(clear_date.status(), StatusCode::SEE_OTHER);
    let date_value: Option<String> =
        sqlx::query_scalar("SELECT promised_delivery_date FROM orders WHERE id=?")
            .bind(oid)
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(date_value, None);
    let date_event_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM order_events WHERE order_id=? AND event_type='PROMISED_DATE_CHANGED'",
    )
    .bind(oid)
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(date_event_count, 2);
    let update = f
        .app
        .clone()
        .oneshot(post(
            &format!("/customers/{cid}/contact"),
            "phone=%2B1+832-555-1212&email=ME%40EXAMPLE.COM&address=&notes=shop",
            Some("https://woodshop.test"),
        ))
        .await
        .unwrap();
    assert_eq!(update.status(), StatusCode::SEE_OTHER);
    let row: (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    ) = sqlx::query_as(
        "SELECT phone,normalized_phone,email,normalized_email FROM customers WHERE id=?",
    )
    .bind(cid)
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(row.0.as_deref(), Some("+1 832-555-1212"));
    assert_eq!(row.1.as_deref(), Some("18325551212"));
    assert_eq!(row.2.as_deref(), Some("ME@EXAMPLE.COM"));
    assert_eq!(row.3.as_deref(), Some("me@example.com"));
}

#[tokio::test]
async fn delivery_history_and_photo_attachment_render_and_serve_only_contained_images() {
    let f = fixture().await;
    let cid = seed_customer(&f.pool, Some("Rosa")).await;
    let oid = seed_order(&f.pool, cid, "DELIVERED", "Bookcase", None).await;
    let delivery = sqlx::query(
        "INSERT INTO deliveries(order_id,delivered_by,notes) VALUES(?,77,'Set in place')",
    )
    .bind(oid)
    .execute(&f.pool)
    .await
    .unwrap()
    .last_insert_rowid();
    let root = PathBuf::from(&f._temp.path())
        .join("photos")
        .join(oid.to_string());
    tokio::fs::create_dir_all(&root).await.unwrap();
    tokio::fs::write(root.join("proof.png"), [137, 80, 78, 71])
        .await
        .unwrap();
    let attachment = sqlx::query("INSERT INTO attachments(order_id,delivery_id,type,path,mime_type) VALUES(?,?,'DELIVERY_PROOF',?,'image/png')").bind(oid).bind(delivery).bind(format!("{oid}/proof.png")).execute(&f.pool).await.unwrap().last_insert_rowid();
    let list = body_text(response(&f.app, "/deliveries").await).await;
    assert!(list.contains("Rosa"));
    assert!(list.contains("Set in place"));
    assert!(list.contains("1"));
    let image = response(&f.app, &format!("/attachments/{attachment}")).await;
    assert_eq!(image.status(), StatusCode::OK);
    assert_eq!(image.headers()[header::CONTENT_TYPE], "image/png");
}
