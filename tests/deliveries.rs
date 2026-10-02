use sqlx::SqlitePool;
use std::{ops::Deref, path::PathBuf};
use woodshop::{
    db,
    domain::{CustomerInput, Extraction, ItemInput, OrderInput},
};

struct TestDb {
    pool: SqlitePool,
    _dir: tempfile::TempDir,
}

impl Deref for TestDb {
    type Target = SqlitePool;
    fn deref(&self) -> &Self::Target {
        &self.pool
    }
}

async fn pool() -> TestDb {
    let dir = tempfile::tempdir().unwrap();
    let path: PathBuf = dir.path().join("test.db");
    let pool = db::connect(&format!("sqlite:{}", path.display()))
        .await
        .unwrap();
    TestDb { pool, _dir: dir }
}

async fn order(pool: &SqlitePool) -> i64 {
    let extraction = Extraction {
        customer: CustomerInput {
            name: Some("Rosa".into()),
            phone: None,
            email: None,
            address: None,
            preferred_language: None,
            notes: None,
        },
        order: OrderInput {
            description: "Banco".into(),
            requested_delivery_date: None,
            items: vec![ItemInput {
                product_type: None,
                description: "Banco".into(),
                quantity: 1,
                length: None,
                width: None,
                height: None,
                dimension_unit: None,
                wood_species: None,
                finish: None,
                notes: None,
            }],
        },
        missing_fields: vec![],
    };
    db::create_order(pool, &extraction, "banco", 12, "fixture")
        .await
        .unwrap()
}

#[tokio::test]
async fn delivery_transition_sets_session_and_attachment_integrity() {
    let pool = pool().await;
    let order_id = order(&pool).await;
    let delivery_id = db::deliver_order(&pool, order_id, 12, 34, Some("left at door"))
        .await
        .unwrap();
    let status: String = sqlx::query_scalar("SELECT status FROM orders WHERE id = ?")
        .bind(order_id)
        .fetch_one(&*pool)
        .await
        .unwrap();
    assert_eq!(status, "DELIVERED");
    let session = sqlx::query("SELECT user_id, chat_id, order_id, delivery_id, expires_at > strftime('%Y-%m-%dT%H:%M:%fZ', 'now') FROM telegram_sessions")
        .fetch_one(&*pool).await.unwrap();
    use sqlx::Row;
    assert_eq!(session.get::<i64, _>(0), 12);
    assert_eq!(session.get::<i64, _>(1), 34);
    assert_eq!(session.get::<i64, _>(2), order_id);
    assert_eq!(session.get::<i64, _>(3), delivery_id);
    assert_eq!(session.get::<i64, _>(4), 1);
    let attachment_id = db::attach_photo(
        &pool,
        order_id,
        Some(delivery_id),
        "DELIVERY_PROOF",
        "/data/photos/1/proof.jpg",
        "file-1",
        77,
        "image/jpeg",
    )
    .await
    .unwrap();
    let linked_order: i64 = sqlx::query_scalar("SELECT order_id FROM attachments WHERE id = ?")
        .bind(attachment_id)
        .fetch_one(&*pool)
        .await
        .unwrap();
    assert_eq!(linked_order, order_id);
    assert!(
        db::attach_photo(
            &pool,
            order_id,
            Some(delivery_id + 999),
            "DELIVERY_PROOF",
            "x",
            "x",
            78,
            "image/jpeg"
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn invalid_order_and_unsupported_mime_are_rejected() {
    let pool = pool().await;
    assert!(db::deliver_order(&pool, 999, 1, 2, None).await.is_err());
    assert!(
        db::attach_photo(&pool, 999, None, "OTHER", "x", "x", 1, "image/gif")
            .await
            .is_err()
    );
    let order_id = order(&pool).await;
    assert!(db::deliver_order(&pool, order_id, 1, 2, None).await.is_ok());
    assert!(
        db::deliver_order(&pool, order_id, 1, 2, None)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn delivery_replaces_previous_session_for_same_user_and_chat() {
    let pool = pool().await;
    let first = order(&pool).await;
    let second = order(&pool).await;
    db::deliver_order(&pool, first, 21, 31, None).await.unwrap();
    db::deliver_order(&pool, second, 21, 31, None)
        .await
        .unwrap();
    let rows: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT order_id, delivery_id FROM telegram_sessions WHERE user_id=21 AND chat_id=31",
    )
    .fetch_all(&*pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, second);
}

#[tokio::test]
async fn same_update_id_returns_existing_delivery_under_concurrency() {
    let pool = pool().await;
    let order_id = order(&pool).await;
    let (a, b) = tokio::join!(
        db::deliver_order_for_update(&pool, order_id, 21, 31, Some("nota"), 9001),
        db::deliver_order_for_update(&pool, order_id, 21, 31, Some("nota"), 9001)
    );
    let first_id = a.unwrap();
    let second_id = b.unwrap();
    assert_eq!(first_id, second_id);
    let deliveries: i64 = sqlx::query_scalar("SELECT count(*) FROM deliveries WHERE order_id=?")
        .bind(order_id)
        .fetch_one(&*pool)
        .await
        .unwrap();
    assert_eq!(deliveries, 1);
    let sessions: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM telegram_sessions WHERE user_id=21 AND chat_id=31",
    )
    .fetch_one(&*pool)
    .await
    .unwrap();
    assert_eq!(sessions, 1);
    let result_id: Option<i64> =
        sqlx::query_scalar("SELECT result_id FROM telegram_updates WHERE update_id=9001")
            .fetch_one(&*pool)
            .await
            .unwrap();
    assert_eq!(result_id, Some(first_id));
}

#[tokio::test]
async fn delivery_update_commit_failure_rolls_back_delivery_transition() {
    let pool = pool().await;
    let order_id = order(&pool).await;
    sqlx::query("CREATE TRIGGER reject_update_commit BEFORE UPDATE OF status ON telegram_updates WHEN NEW.status='committed' BEGIN SELECT RAISE(ABORT, 'commit rejected'); END")
        .execute(&*pool).await.unwrap();
    assert!(
        db::deliver_order_for_update(&pool, order_id, 21, 31, None, 9002)
            .await
            .is_err()
    );
    let status: String = sqlx::query_scalar("SELECT status FROM orders WHERE id=?")
        .bind(order_id)
        .fetch_one(&*pool)
        .await
        .unwrap();
    assert_eq!(status, "NEW");
    let deliveries: i64 = sqlx::query_scalar("SELECT count(*) FROM deliveries")
        .fetch_one(&*pool)
        .await
        .unwrap();
    let sessions: i64 = sqlx::query_scalar("SELECT count(*) FROM telegram_sessions")
        .fetch_one(&*pool)
        .await
        .unwrap();
    let updates: i64 = sqlx::query_scalar("SELECT count(*) FROM telegram_updates")
        .fetch_one(&*pool)
        .await
        .unwrap();
    assert_eq!((deliveries, sessions, updates), (0, 0, 0));
}
