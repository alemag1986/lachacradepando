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

fn extraction(phone: Option<&str>, quantity: i64) -> Extraction {
    Extraction {
        customer: CustomerInput {
            name: Some("Ana  Lopez".into()),
            phone: phone.map(str::to_owned),
            email: None,
            address: None,
            preferred_language: Some("es".into()),
            notes: None,
        },
        order: OrderInput {
            description: "Mesa de roble".into(),
            requested_delivery_date: None,
            items: vec![ItemInput {
                product_type: Some("table".into()),
                description: "Mesa".into(),
                quantity,
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
    }
}

#[tokio::test]
async fn exact_phone_match_reuses_customer() {
    let pool = pool().await;
    db::create_order(
        &pool,
        &extraction(Some("+1 (555) 123-4567"), 1),
        "one",
        5,
        "fixture",
    )
    .await
    .unwrap();
    db::create_order(
        &pool,
        &extraction(Some("+1 555 123 4567"), 1),
        "two",
        5,
        "fixture",
    )
    .await
    .unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM customers")
        .fetch_one(&*pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn phone_normalization_is_digits_only_without_country_inference() {
    let pool = pool().await;
    db::create_order(
        &pool,
        &extraction(Some("+1 (555) 123-4567"), 1),
        "one",
        5,
        "fixture",
    )
    .await
    .unwrap();
    db::create_order(
        &pool,
        &extraction(Some("1 555 123 4567"), 1),
        "two",
        5,
        "fixture",
    )
    .await
    .unwrap();
    db::create_order(
        &pool,
        &extraction(Some("555 123 4567"), 1),
        "three",
        5,
        "fixture",
    )
    .await
    .unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM customers")
        .fetch_one(&*pool)
        .await
        .unwrap();
    assert_eq!(
        count, 2,
        "the leading country digit is preserved, not inferred"
    );
}

#[tokio::test]
async fn lookup_falls_through_phone_to_email_then_name_in_priority_order() {
    let pool = pool().await;
    let mut first = extraction(Some("555-3000"), 1);
    first.customer.email = Some("ana@example.test".into());
    db::create_order(&pool, &first, "first", 5, "fixture")
        .await
        .unwrap();

    let mut email_match = extraction(Some("555-4000"), 1);
    email_match.customer.email = Some(" ANA@example.test ".into());
    email_match.customer.name = Some("A Different Name".into());
    db::create_order(&pool, &email_match, "email", 5, "fixture")
        .await
        .unwrap();

    let mut name_match = extraction(None, 1);
    name_match.customer.name = Some(" Ana Lopez ".into());
    db::create_order(&pool, &name_match, "name", 5, "fixture")
        .await
        .unwrap();

    let ids: Vec<i64> = sqlx::query_scalar("SELECT customer_id FROM orders ORDER BY id")
        .fetch_all(&*pool)
        .await
        .unwrap();
    assert_eq!(ids[0], ids[1], "email match is used after phone misses");
    assert_eq!(
        ids[0], ids[2],
        "name match is used after phone and email miss"
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM customers")
        .fetch_one(&*pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn name_fallback_does_not_merge_conflicting_contact_details() {
    let pool = pool().await;
    let mut first = extraction(Some("555-3101"), 1);
    first.customer.email = Some("first@example.test".into());
    db::create_order(&pool, &first, "first", 5, "fixture")
        .await
        .unwrap();
    let mut second = extraction(Some("555-3102"), 1);
    second.customer.email = Some("second@example.test".into());
    let order_id = db::create_order(&pool, &second, "second", 5, "fixture")
        .await
        .unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM customers")
        .fetch_one(&*pool)
        .await
        .unwrap();
    let customer_id: i64 = sqlx::query_scalar("SELECT customer_id FROM orders WHERE id = ?")
        .bind(order_id)
        .fetch_one(&*pool)
        .await
        .unwrap();
    assert_eq!(count, 2);
    assert_eq!(customer_id, 2);
}

#[tokio::test]
async fn ambiguous_name_fallback_fails_safely() {
    let pool = pool().await;
    let mut first = extraction(Some("555-3201"), 1);
    first.customer.email = Some("first@example.test".into());
    db::create_order(&pool, &first, "first", 5, "fixture")
        .await
        .unwrap();
    let mut second = extraction(Some("555-3202"), 1);
    second.customer.email = Some("second@example.test".into());
    db::create_order(&pool, &second, "second", 5, "fixture")
        .await
        .unwrap();
    let ambiguous = extraction(None, 1);
    assert!(
        db::create_order(&pool, &ambiguous, "ambiguous", 5, "fixture")
            .await
            .is_err()
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM orders")
        .fetch_one(&*pool)
        .await
        .unwrap();
    assert_eq!(count, 2);
}

#[tokio::test]
async fn distinct_phone_customers_can_share_a_name() {
    let pool = pool().await;
    db::create_order(&pool, &extraction(Some("555-6001"), 1), "one", 5, "fixture")
        .await
        .unwrap();
    db::create_order(&pool, &extraction(Some("555-6002"), 1), "two", 5, "fixture")
        .await
        .unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM customers")
        .fetch_one(&*pool)
        .await
        .unwrap();
    assert_eq!(count, 2);
}

#[tokio::test]
async fn duplicate_customer_prevention_under_concurrency() {
    let pool = pool().await;
    let first = extraction(Some("555-1111"), 1);
    let second = extraction(Some("555 1111"), 1);
    let (a, b) = tokio::join!(
        db::create_order(&pool, &first, "a", 5, "fixture"),
        db::create_order(&pool, &second, "b", 5, "fixture")
    );
    a.unwrap();
    b.unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM customers")
        .fetch_one(&*pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn concurrent_fallback_match_does_not_duplicate_customer() {
    let pool = pool().await;
    let mut first = extraction(None, 1);
    first.customer.email = Some("concurrent@example.test".into());
    let mut second = extraction(None, 1);
    second.customer.email = Some("CONCURRENT@example.test".into());
    let (a, b) = tokio::join!(
        db::create_order(&pool, &first, "a", 5, "fixture"),
        db::create_order(&pool, &second, "b", 5, "fixture")
    );
    a.unwrap();
    b.unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM customers")
        .fetch_one(&*pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn failed_order_transaction_leaves_no_partial_rows() {
    let pool = pool().await;
    sqlx::query("CREATE TRIGGER reject_audit BEFORE INSERT ON ai_requests BEGIN SELECT RAISE(ABORT, 'audit unavailable'); END")
        .execute(&*pool).await.unwrap();
    assert!(
        db::create_order(&pool, &extraction(Some("555-2222"), 1), "bad", 5, "fixture")
            .await
            .is_err()
    );
    for table in [
        "customers",
        "orders",
        "order_items",
        "ai_requests",
        "order_events",
    ] {
        let count: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
            .fetch_one(&*pool)
            .await
            .unwrap();
        assert_eq!(count, 0, "partial rows in {table}");
    }
}

#[tokio::test]
async fn same_update_id_returns_existing_order_atomically() {
    let pool = pool().await;
    let first = extraction(Some("555-7777"), 1);
    let retry = extraction(Some("555-7777"), 1);
    let (a, b) = tokio::join!(
        db::create_order_for_update(&pool, &first, "pedido", 5, "fixture", 7001),
        db::create_order_for_update(&pool, &retry, "pedido", 5, "fixture", 7001)
    );
    let first_id = a.unwrap();
    let second_id = b.unwrap();
    assert_eq!(first_id, second_id);
    let orders: i64 = sqlx::query_scalar("SELECT count(*) FROM orders")
        .fetch_one(&*pool)
        .await
        .unwrap();
    assert_eq!(orders, 1);
    let update: (String, Option<i64>) =
        sqlx::query_as("SELECT status, result_id FROM telegram_updates WHERE update_id=7001")
            .fetch_one(&*pool)
            .await
            .unwrap();
    assert_eq!(update, ("committed".into(), Some(first_id)));
}

#[tokio::test]
async fn update_commit_failure_rolls_back_order_and_claim() {
    let pool = pool().await;
    sqlx::query("CREATE TRIGGER reject_update_commit BEFORE UPDATE OF status ON telegram_updates WHEN NEW.status='committed' BEGIN SELECT RAISE(ABORT, 'commit rejected'); END")
        .execute(&*pool).await.unwrap();
    assert!(
        db::create_order_for_update(
            &pool,
            &extraction(Some("555-8888"), 1),
            "pedido",
            5,
            "fixture",
            8001
        )
        .await
        .is_err()
    );
    for table in [
        "customers",
        "orders",
        "order_items",
        "ai_requests",
        "order_events",
        "telegram_updates",
    ] {
        let count: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
            .fetch_one(&*pool)
            .await
            .unwrap();
        assert_eq!(count, 0, "partial rows in {table}");
    }
}

#[test]
fn customer_normalizers_are_public_and_consistent() {
    assert_eq!(
        db::normalize_phone(Some(" +1 (555) 123-4567 ")).as_deref(),
        Some("15551234567")
    );
    assert_eq!(
        db::normalize_email(Some(" Ana@Example.Test ")).as_deref(),
        Some("ana@example.test")
    );
    assert_eq!(
        db::normalize_name(Some(" Ana   Lopez ")).as_deref(),
        Some("ana lopez")
    );
}

#[test]
fn extraction_requires_strict_date_and_positive_measurements() {
    let mut value = extraction(None, 1);
    value.order.requested_delivery_date = Some("2026-2-03".into());
    assert!(value.validate().is_err());
    value.order.requested_delivery_date = Some("2026-02-30".into());
    assert!(value.validate().is_err());
    value.order.requested_delivery_date = Some("2026-02-03".into());
    value.order.items[0].length = Some(0.0);
    assert!(value.validate().is_err());
}
