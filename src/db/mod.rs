use anyhow::Context;
use sqlx::{SqlitePool, sqlite::SqliteConnectOptions};
use std::{str::FromStr, time::Duration};

use crate::domain::Extraction;

pub async fn connect(url: &str) -> anyhow::Result<SqlitePool> {
    let options = SqliteConnectOptions::from_str(url)?
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .busy_timeout(Duration::from_secs(5));
    let pool = SqlitePool::connect_with(options).await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}

pub async fn create_order(
    pool: &SqlitePool,
    extraction: &Extraction,
    original_text: &str,
    created_by: i64,
    model: &str,
) -> anyhow::Result<i64> {
    create_order_inner(pool, extraction, original_text, created_by, model, None).await
}

pub async fn create_order_for_update(
    pool: &SqlitePool,
    extraction: &Extraction,
    original_text: &str,
    user_id: i64,
    model: &str,
    update_id: i64,
) -> anyhow::Result<i64> {
    create_order_inner(
        pool,
        extraction,
        original_text,
        user_id,
        model,
        Some(update_id),
    )
    .await
}

async fn create_order_inner(
    pool: &SqlitePool,
    extraction: &Extraction,
    original_text: &str,
    created_by: i64,
    model: &str,
    update_id: Option<i64>,
) -> anyhow::Result<i64> {
    extraction.validate()?;
    let structured_result = serde_json::to_string(extraction)?;
    let mut tx = pool.begin().await?;
    if let Some(update_id) = update_id
        && let Some(result_id) = begin_idempotent_update(&mut tx, update_id).await?
    {
        tx.commit().await?;
        return Ok(result_id);
    }
    // SQLite's immediate write lock makes lookup-then-insert duplicate-safe across callers.
    sqlx::query("UPDATE customers SET updated_at = updated_at WHERE id = -1")
        .execute(&mut *tx)
        .await?;
    let phone = normalize_phone(extraction.customer.phone.as_deref());
    let email = normalize_email(extraction.customer.email.as_deref());
    let name = normalize_name(extraction.customer.name.as_deref());
    let mut existing = None;
    if let Some(value) = &phone {
        existing = sqlx::query_scalar::<_, i64>(
            "SELECT id FROM customers WHERE normalized_phone = ? ORDER BY id LIMIT 1",
        )
        .bind(value)
        .fetch_optional(&mut *tx)
        .await?;
    }
    if existing.is_none()
        && let Some(value) = &email
    {
        existing = sqlx::query_scalar::<_, i64>(
            "SELECT id FROM customers WHERE normalized_email = ? ORDER BY id LIMIT 1",
        )
        .bind(value)
        .fetch_optional(&mut *tx)
        .await?;
    }
    if existing.is_none()
        && let Some(value) = &name
    {
        let candidates: Vec<(i64, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT id, normalized_phone, normalized_email FROM customers WHERE normalized_name = ? ORDER BY id",
        )
        .bind(value)
        .fetch_all(&mut *tx)
        .await?;
        let compatible: Vec<i64> = candidates
            .into_iter()
            .filter(|(_, candidate_phone, candidate_email)| {
                contact_is_compatible(phone.as_deref(), candidate_phone.as_deref())
                    && contact_is_compatible(email.as_deref(), candidate_email.as_deref())
            })
            .map(|(id, _, _)| id)
            .collect();
        anyhow::ensure!(
            compatible.len() <= 1,
            "ambiguous customer name match; provide phone or email"
        );
        existing = compatible.first().copied();
    }
    let customer_id = if let Some(id) = existing {
        id
    } else {
        sqlx::query("INSERT INTO customers (name, phone, email, address, preferred_language, notes, normalized_phone, normalized_email, normalized_name) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(&extraction.customer.name).bind(&extraction.customer.phone).bind(&extraction.customer.email)
            .bind(&extraction.customer.address).bind(&extraction.customer.preferred_language).bind(&extraction.customer.notes)
            .bind(&phone).bind(&email).bind(&name).execute(&mut *tx).await?.last_insert_rowid()
    };
    let order_id = sqlx::query("INSERT INTO orders (customer_id, original_text, normalized_description, requested_delivery_date, created_by) VALUES (?, ?, ?, ?, ?)")
        .bind(customer_id).bind(original_text).bind(&extraction.order.description)
        .bind(&extraction.order.requested_delivery_date).bind(created_by)
        .execute(&mut *tx).await?.last_insert_rowid();
    for item in &extraction.order.items {
        sqlx::query("INSERT INTO order_items (order_id, product_type, description, quantity, length, width, height, dimension_unit, wood_species, finish, notes) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(order_id).bind(&item.product_type).bind(&item.description).bind(item.quantity)
            .bind(item.length).bind(item.width).bind(item.height).bind(&item.dimension_unit)
            .bind(&item.wood_species).bind(&item.finish).bind(&item.notes).execute(&mut *tx).await?;
    }
    sqlx::query("INSERT INTO ai_requests (original_text, structured_result, model, status, error) VALUES (?, ?, ?, 'completed', NULL)")
        .bind(original_text).bind(structured_result).bind(model).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO order_events (order_id, event_type, description, created_by) VALUES (?, 'ORDER_CREATED', 'Order created from Telegram extraction', ?)")
        .bind(order_id).bind(created_by).execute(&mut *tx).await?;
    if let Some(update_id) = update_id {
        commit_idempotent_update(&mut tx, update_id, order_id).await?;
    }
    tx.commit().await.context("committing order creation")?;
    Ok(order_id)
}

pub async fn deliver_order(
    pool: &SqlitePool,
    order_id: i64,
    user_id: i64,
    chat_id: i64,
    notes: Option<&str>,
) -> anyhow::Result<i64> {
    deliver_order_inner(pool, order_id, user_id, chat_id, notes, None).await
}

pub async fn deliver_order_for_update(
    pool: &SqlitePool,
    order_id: i64,
    user_id: i64,
    chat_id: i64,
    notes: Option<&str>,
    update_id: i64,
) -> anyhow::Result<i64> {
    deliver_order_inner(pool, order_id, user_id, chat_id, notes, Some(update_id)).await
}

async fn deliver_order_inner(
    pool: &SqlitePool,
    order_id: i64,
    user_id: i64,
    chat_id: i64,
    notes: Option<&str>,
    update_id: Option<i64>,
) -> anyhow::Result<i64> {
    let mut tx = pool.begin().await?;
    if let Some(update_id) = update_id
        && let Some(result_id) = begin_idempotent_update(&mut tx, update_id).await?
    {
        tx.commit().await?;
        return Ok(result_id);
    }
    let result = sqlx::query("UPDATE orders SET status = 'DELIVERED', updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ? AND status IN ('READY','IN_PRODUCTION','CONFIRMED','NEW')")
        .bind(order_id).execute(&mut *tx).await?;
    anyhow::ensure!(
        result.rows_affected() == 1,
        "order does not exist or cannot be delivered"
    );
    let delivery_id =
        sqlx::query("INSERT INTO deliveries (order_id, delivered_by, notes) VALUES (?, ?, ?)")
            .bind(order_id)
            .bind(user_id)
            .bind(notes)
            .execute(&mut *tx)
            .await?
            .last_insert_rowid();
    sqlx::query("DELETE FROM telegram_sessions WHERE user_id = ? AND chat_id = ?")
        .bind(user_id)
        .bind(chat_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO telegram_sessions (user_id, chat_id, order_id, delivery_id, expires_at) VALUES (?, ?, ?, ?, strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '+15 minutes'))")
        .bind(user_id).bind(chat_id).bind(order_id).bind(delivery_id).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO order_events (order_id, event_type, description, created_by) VALUES (?, 'DELIVERED', 'Order marked delivered', ?)")
        .bind(order_id).bind(user_id).execute(&mut *tx).await?;
    if let Some(update_id) = update_id {
        commit_idempotent_update(&mut tx, update_id, delivery_id).await?;
    }
    tx.commit().await?;
    Ok(delivery_id)
}

async fn begin_idempotent_update(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    update_id: i64,
) -> anyhow::Result<Option<i64>> {
    // Acquire SQLite's writer lock before checking so concurrent retries serialize.
    sqlx::query("INSERT INTO telegram_updates (update_id, status) VALUES (?, 'processing') ON CONFLICT(update_id) DO UPDATE SET status = telegram_updates.status")
        .bind(update_id)
        .execute(&mut **tx)
        .await?;
    let row: (String, Option<i64>) =
        sqlx::query_as("SELECT status, result_id FROM telegram_updates WHERE update_id = ?")
            .bind(update_id)
            .fetch_one(&mut **tx)
            .await?;
    if matches!(row.0.as_str(), "committed" | "completed") {
        return Ok(Some(row.1.ok_or_else(|| {
            anyhow::anyhow!("completed Telegram update has no result_id")
        })?));
    }
    if row.0 != "processing" {
        sqlx::query("UPDATE telegram_updates SET status = 'processing' WHERE update_id = ?")
            .bind(update_id)
            .execute(&mut **tx)
            .await?;
    }
    Ok(None)
}

async fn commit_idempotent_update(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    update_id: i64,
    result_id: i64,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE telegram_updates SET status = 'committed', result_id = ? WHERE update_id = ?",
    )
    .bind(result_id)
    .bind(update_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

#[derive(Debug, Clone, Copy)]
pub struct PhotoMetadata<'a> {
    pub kind: &'a str,
    pub path: &'a str,
    pub file_id: &'a str,
    pub message_id: i64,
    pub mime: &'a str,
}

#[allow(clippy::too_many_arguments)]
pub async fn attach_photo(
    pool: &SqlitePool,
    order_id: i64,
    delivery_id: Option<i64>,
    kind: &str,
    path: &str,
    file_id: &str,
    message_id: i64,
    mime: &str,
) -> anyhow::Result<i64> {
    attach_photo_with_metadata(
        pool,
        order_id,
        delivery_id,
        PhotoMetadata {
            kind,
            path,
            file_id,
            message_id,
            mime,
        },
    )
    .await
}

pub async fn attach_photo_with_metadata(
    pool: &SqlitePool,
    order_id: i64,
    delivery_id: Option<i64>,
    photo: PhotoMetadata<'_>,
) -> anyhow::Result<i64> {
    let PhotoMetadata {
        kind,
        path,
        file_id,
        message_id,
        mime,
    } = photo;
    anyhow::ensure!(
        matches!(mime, "image/jpeg" | "image/png" | "image/webp"),
        "unsupported image MIME type"
    );
    anyhow::ensure!(
        matches!(
            kind,
            "ORDER_REFERENCE"
                | "CUSTOMER_DRAWING"
                | "WORK_IN_PROGRESS"
                | "FINISHED_PRODUCT"
                | "DELIVERY_PROOF"
                | "OTHER"
        ),
        "invalid attachment type"
    );
    let mut tx = pool.begin().await?;
    if let Some(delivery_id) = delivery_id {
        let owns_delivery: i64 = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM deliveries WHERE id = ? AND order_id = ?)",
        )
        .bind(delivery_id)
        .bind(order_id)
        .fetch_one(&mut *tx)
        .await?;
        anyhow::ensure!(owns_delivery == 1, "delivery does not belong to order");
    } else {
        let exists: i64 = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM orders WHERE id = ?)")
            .bind(order_id)
            .fetch_one(&mut *tx)
            .await?;
        anyhow::ensure!(exists == 1, "order does not exist");
    }
    let id = sqlx::query("INSERT INTO attachments (order_id, delivery_id, type, path, telegram_file_id, telegram_message_id, mime_type) VALUES (?, ?, ?, ?, ?, ?, ?)")
        .bind(order_id).bind(delivery_id).bind(kind).bind(path).bind(file_id).bind(message_id).bind(mime)
        .execute(&mut *tx).await?.last_insert_rowid();
    tx.commit().await?;
    Ok(id)
}

pub fn normalize_name(value: Option<&str>) -> Option<String> {
    value.map(str::trim).filter(|v| !v.is_empty()).map(|v| {
        v.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    })
}

pub fn normalize_email(value: Option<&str>) -> Option<String> {
    normalize_name(value)
}

pub fn normalize_phone(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    if value.is_empty() {
        return None;
    }
    let digits: String = value.chars().filter(char::is_ascii_digit).collect();
    if digits.is_empty() {
        None
    } else {
        Some(digits)
    }
}

fn contact_is_compatible(provided: Option<&str>, stored: Option<&str>) -> bool {
    match (provided, stored) {
        (Some(provided), Some(stored)) => provided == stored,
        _ => true,
    }
}
