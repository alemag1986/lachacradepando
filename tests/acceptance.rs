use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use tower::ServiceExt;
use woodshop::{AppState, config::Config, db, domain::Extraction};

#[tokio::test]
async fn order_delivery_photos_and_history_survive_reopening_database() {
    let temp = tempfile::tempdir().unwrap();
    let database_url = format!("sqlite:{}", temp.path().join("woodshop.db").display());
    let pool = db::connect(&database_url).await.unwrap();
    let extraction: Extraction =
        serde_json::from_str(include_str!("fixtures/extraction_es.json")).unwrap();
    let order_id = db::create_order(&pool, &extraction, "pedido\nMesa de roble", 42, "fixture")
        .await
        .unwrap();
    let customer_id: i64 = sqlx::query_scalar("SELECT customer_id FROM orders WHERE id=?")
        .bind(order_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    let delivery_id = db::deliver_order(&pool, order_id, 42, 42, Some("Recibido por cliente"))
        .await
        .unwrap();
    let directory = temp.path().join("photos").join(order_id.to_string());
    std::fs::create_dir_all(&directory).unwrap();
    let filename = format!("{}.jpg", uuid::Uuid::new_v4());
    image::RgbImage::from_pixel(2, 2, image::Rgb([128, 90, 54]))
        .save(directory.join(&filename))
        .unwrap();
    let photo_path = format!("{order_id}/{filename}");
    let attachment_id = db::attach_photo_with_metadata(
        &pool,
        order_id,
        Some(delivery_id),
        db::PhotoMetadata {
            kind: "DELIVERY_PROOF",
            path: &photo_path,
            file_id: "fixture-photo",
            message_id: 100,
            mime: "image/jpeg",
        },
    )
    .await
    .unwrap();
    pool.close().await;
    let pool = db::connect(&database_url).await.unwrap();
    let config = Config {
        app_env: "development".into(),
        app_port: 8080,
        database_url,
        telegram_bot_token: String::new(),
        telegram_webhook_secret: "fixture-secret".into(),
        telegram_allowed_user_ids: vec![42],
        telegram_allowed_chat_ids: vec![],
        openrouter_api_key: String::new(),
        openrouter_model: "fixture".into(),
        openrouter_fallback_model: "fixture".into(),
        public_base_url: "http://localhost:8080".into(),
        data_dir: temp.path().to_string_lossy().into_owned(),
    };
    let app = woodshop::router(AppState {
        pool: pool.clone(),
        config: Arc::new(config),
        http: reqwest::Client::new(),
    });
    for path in [
        "/health".into(),
        "/".into(),
        "/orders".into(),
        format!("/orders/{order_id}"),
        "/customers".into(),
        format!("/customers/{customer_id}"),
        "/deliveries".into(),
    ] {
        let response = app
            .clone()
            .oneshot(Request::builder().uri(&path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "failed {path}");
        assert_eq!(response.headers()["x-content-type-options"], "nosniff");
        assert_eq!(response.headers()["cache-control"], "no-store");
        assert!(
            response.headers()["content-security-policy"]
                .to_str()
                .unwrap()
                .contains("frame-ancestors 'none'")
        );
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let html = String::from_utf8(body.to_vec()).unwrap();
        if path != "/health" {
            assert!(html.contains("lang=\"es\""), "failed {path}");
            assert!(html.contains("La Chacra de Pando"));
            assert!(html.contains("Navegación principal"));
        }
        if path == "/orders" {
            assert!(html.contains(&format!("Ver detalles del pedido #{order_id}")));
            assert!(html.contains(&format!("hx-target=\"#order-preview-{order_id}\"")));
            assert!(html.contains(&format!("hx-select=\"#order-detail-{order_id}\"")));
            for (value, label) in [
                ("NEW", "Nuevo"),
                ("CONFIRMED", "Confirmado"),
                ("IN_PRODUCTION", "En producción"),
                ("READY", "Listo para entregar"),
                ("DELIVERED", "Entregado"),
                ("CANCELLED", "Cancelado"),
            ] {
                assert!(html.contains(&format!("value=\"{value}\"")));
                assert!(html.contains(label));
            }
        }
        assert!(!html.contains("Unknown customer"));
        assert!(!html.contains("Main navigation"));
        if path == format!("/orders/{order_id}") {
            assert!(html.contains(&format!("id=\"order-detail-{order_id}\"")));
            for section in [
                "Solicitud original",
                "Entrega solicitada",
                "Entrega acordada",
                "Productos",
                "Dimensiones",
                "Madera",
                "Acabado",
                "Tareas del taller",
                "Historial del pedido",
                "Galería del proyecto",
                "Ficha del cliente",
            ] {
                assert!(html.contains(section));
            }
            assert!(html.contains("Entregado"));
            assert!(html.contains("Comprobante de entrega"));
            assert!(html.contains("Pedido marcado como entregado"));
            assert!(html.contains(&format!("/attachments/{attachment_id}")));
        }
    }
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/attachments/{attachment_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "image/jpeg");
    let image_bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(image::load_from_memory(&image_bytes).unwrap().width(), 2);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM deliveries WHERE order_id=?")
        .bind(order_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
    pool.close().await;
}
