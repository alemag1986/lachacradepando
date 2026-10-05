use std::path::{Component, Path, PathBuf};

use askama::Template;
use axum::{
    Router,
    body::Body,
    extract::{Form, Path as AxumPath, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use serde::Deserialize;
use sqlx::{QueryBuilder, Sqlite};

use crate::{
    AppState,
    db::{normalize_email, normalize_name, normalize_phone},
};

mod views;
use views::*;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(dashboard))
        .route("/orders", get(order_list))
        .route("/orders/{id}", get(order_detail))
        .route("/orders/{id}/status", post(update_status))
        .route("/orders/{id}/promised-date", post(update_promised_date))
        .route("/orders/{id}/items", post(add_work_item))
        .route("/work-items/{id}/toggle", post(toggle_work_item))
        .route("/customers", get(customer_list))
        .route("/customers/{id}", get(customer_detail))
        .route("/customers/{id}/contact", post(update_customer))
        .route("/deliveries", get(deliveries))
        .route("/attachments/{id}", get(attachment))
        .route("/static/app.css", get(stylesheet))
        .route("/static/htmx.min.js", get(htmx))
}

#[derive(Debug)]
struct WebError(StatusCode);

impl IntoResponse for WebError {
    fn into_response(self) -> Response {
        let (status, title) = match self.0 {
            StatusCode::NOT_FOUND => (StatusCode::NOT_FOUND, "Página no encontrada"),
            StatusCode::BAD_REQUEST => (StatusCode::BAD_REQUEST, "Solicitud inválida"),
            StatusCode::FORBIDDEN => (StatusCode::FORBIDDEN, "Solicitud rechazada"),
            _ => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "No se pudo completar la solicitud",
            ),
        };
        (
            status,
            Html(format!(
                "<!doctype html><html lang=\"es\"><meta charset=\"utf-8\"><title>{title} · La Chacra de Pando</title><p>{title}</p></html>"
            )),
        )
            .into_response()
    }
}

impl From<sqlx::Error> for WebError {
    fn from(_: sqlx::Error) -> Self {
        Self(StatusCode::INTERNAL_SERVER_ERROR)
    }
}
impl From<askama::Error> for WebError {
    fn from(_: askama::Error) -> Self {
        Self(StatusCode::INTERNAL_SERVER_ERROR)
    }
}

fn valid_id(id: i64) -> Result<i64, WebError> {
    if id > 0 {
        Ok(id)
    } else {
        Err(WebError(StatusCode::NOT_FOUND))
    }
}

fn render<T: Template>(template: T) -> Result<Html<String>, WebError> {
    template.render().map(Html).map_err(Into::into)
}

async fn dashboard(State(state): State<AppState>) -> Result<Html<String>, WebError> {
    let counts = sqlx::query_as::<_, DashboardCounts>(
        "SELECT\n         (SELECT COUNT(*) FROM orders WHERE status = 'NEW') AS new_count,\n         (SELECT COUNT(*) FROM orders WHERE status = 'IN_PRODUCTION') AS production_count,\n         (SELECT COUNT(*) FROM orders WHERE status = 'READY') AS ready_count,\n         (SELECT COUNT(*) FROM deliveries WHERE date(delivered_at) >= date('now','-' || ((strftime('%w','now')+6)%7) || ' days')) AS delivered_week,\n         (SELECT COUNT(*) FROM orders WHERE status NOT IN ('DELIVERED','CANCELLED') AND COALESCE(promised_delivery_date,requested_delivery_date) < date('now')) AS overdue"
    ).fetch_one(&state.pool).await?;
    let recent = sqlx::query_as::<_, OrderRow>("SELECT o.id, c.name AS customer_name, o.status, o.normalized_description AS description, o.promised_delivery_date, o.requested_delivery_date FROM orders o JOIN customers c ON c.id=o.customer_id ORDER BY o.created_at DESC LIMIT 8")
        .fetch_all(&state.pool).await?;
    render(DashboardTemplate { counts, recent })
}

#[derive(Default, Deserialize)]
pub(super) struct OrderFilters {
    pub status: Option<String>,
    pub customer: Option<String>,
    pub delivery_date: Option<String>,
    pub search: Option<String>,
}

async fn order_list(
    State(state): State<AppState>,
    Query(filters): Query<OrderFilters>,
) -> Result<Html<String>, WebError> {
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT o.id, c.name AS customer_name, o.status, o.normalized_description AS description, o.promised_delivery_date, o.requested_delivery_date FROM orders o JOIN customers c ON c.id=o.customer_id WHERE 1=1",
    );
    if let Some(status) = filters.status.as_deref().filter(|v| valid_status(v)) {
        query.push(" AND o.status = ").push_bind(status);
    }
    if let Some(customer) = filters.customer.as_deref().filter(|v| !v.trim().is_empty()) {
        query
            .push(" AND c.name LIKE ")
            .push_bind(format!("%{}%", customer.trim()));
    }
    if let Some(date) = filters.delivery_date.as_deref().filter(|v| valid_date(v)) {
        query
            .push(" AND COALESCE(o.promised_delivery_date,o.requested_delivery_date) = ")
            .push_bind(date);
    }
    if let Some(term) = filters.search.as_deref().filter(|v| !v.trim().is_empty()) {
        let pattern = format!("%{}%", term.trim());
        query
            .push(" AND (CAST(o.id AS TEXT) LIKE ")
            .push_bind(pattern.clone())
            .push(" OR c.name LIKE ")
            .push_bind(pattern.clone())
            .push(" OR o.normalized_description LIKE ")
            .push_bind(pattern)
            .push(")");
    }
    query.push(" ORDER BY COALESCE(o.promised_delivery_date,o.requested_delivery_date) IS NULL, COALESCE(o.promised_delivery_date,o.requested_delivery_date), o.id DESC");
    let orders = query
        .build_query_as::<OrderRow>()
        .fetch_all(&state.pool)
        .await?;
    render(OrderListTemplate { orders, filters })
}

async fn order_detail(
    State(state): State<AppState>,
    AxumPath(raw_id): AxumPath<String>,
) -> Result<Html<String>, WebError> {
    let id = parse_id(&raw_id)?;
    let order = sqlx::query_as::<_, OrderDetail>("SELECT o.id, o.status, o.normalized_description AS description, o.original_text, o.requested_delivery_date, o.promised_delivery_date, o.created_at, c.id AS customer_id, c.name AS customer_name, c.phone, c.email, c.address, c.notes AS customer_notes FROM orders o JOIN customers c ON c.id=o.customer_id WHERE o.id=?")
        .bind(id).fetch_optional(&state.pool).await?.ok_or(WebError(StatusCode::NOT_FOUND))?;
    let items = sqlx::query_as::<_, ItemRow>("SELECT id, product_type, description, quantity, length, width, height, dimension_unit, wood_species, finish, notes FROM order_items WHERE order_id=? ORDER BY id").bind(id).fetch_all(&state.pool).await?;
    let tasks = sqlx::query_as::<_, TaskRow>("SELECT id, description, status, sequence, assigned_to FROM work_items WHERE order_id=? ORDER BY sequence, id").bind(id).fetch_all(&state.pool).await?;
    let events = sqlx::query_as::<_, EventRow>("SELECT id, event_type, description, created_at FROM order_events WHERE order_id=? ORDER BY created_at DESC, id DESC").bind(id).fetch_all(&state.pool).await?;
    let delivery = sqlx::query_as::<_, DeliveryRow>("SELECT id, delivered_at, delivered_by, notes FROM deliveries WHERE order_id=? ORDER BY delivered_at DESC LIMIT 1").bind(id).fetch_optional(&state.pool).await?;
    let photos = sqlx::query_as::<_, AttachmentRow>("SELECT id, type, mime_type, created_at FROM attachments WHERE order_id=? ORDER BY created_at DESC").bind(id).fetch_all(&state.pool).await?;
    render(OrderDetailTemplate {
        order,
        items,
        tasks,
        events,
        delivery,
        photos,
    })
}

async fn customer_list(State(state): State<AppState>) -> Result<Html<String>, WebError> {
    let customers = sqlx::query_as::<_, CustomerRow>("SELECT c.id, c.name, c.phone, c.email, COUNT(o.id) AS order_count, MAX(o.created_at) AS latest_order FROM customers c LEFT JOIN orders o ON o.customer_id=c.id GROUP BY c.id ORDER BY c.name COLLATE NOCASE")
        .fetch_all(&state.pool).await?;
    render(CustomerListTemplate { customers })
}

async fn customer_detail(
    State(state): State<AppState>,
    AxumPath(raw_id): AxumPath<String>,
) -> Result<Html<String>, WebError> {
    let id = parse_id(&raw_id)?;
    let customer = sqlx::query_as::<_, CustomerDetail>("SELECT id, name, phone, email, address, preferred_language, notes, created_at FROM customers WHERE id=?").bind(id).fetch_optional(&state.pool).await?.ok_or(WebError(StatusCode::NOT_FOUND))?;
    let orders = sqlx::query_as::<_, OrderRow>("SELECT id, (SELECT name FROM customers WHERE id=customer_id) AS customer_name, status, normalized_description AS description, promised_delivery_date, requested_delivery_date FROM orders WHERE customer_id=? ORDER BY created_at DESC").bind(id).fetch_all(&state.pool).await?;
    let deliveries = sqlx::query_as::<_, CustomerDelivery>("SELECT d.id, d.delivered_at, d.delivered_by, d.notes, d.order_id FROM deliveries d JOIN orders o ON o.id=d.order_id WHERE o.customer_id=? ORDER BY d.delivered_at DESC").bind(id).fetch_all(&state.pool).await?;
    render(CustomerDetailTemplate {
        customer,
        orders,
        deliveries,
    })
}

async fn deliveries(State(state): State<AppState>) -> Result<Html<String>, WebError> {
    let rows = sqlx::query_as::<_, DeliveryView>("SELECT d.id, d.order_id, c.name AS customer_name, d.delivered_at, d.delivered_by, d.notes, COUNT(a.id) AS photo_count FROM deliveries d JOIN orders o ON o.id=d.order_id JOIN customers c ON c.id=o.customer_id LEFT JOIN attachments a ON a.delivery_id=d.id AND a.type='DELIVERY_PROOF' GROUP BY d.id ORDER BY d.delivered_at DESC").fetch_all(&state.pool).await?;
    render(DeliveriesTemplate { deliveries: rows })
}

async fn attachment(
    State(state): State<AppState>,
    AxumPath(raw_id): AxumPath<String>,
) -> Result<Response, WebError> {
    let id = parse_id(&raw_id)?;
    let row =
        sqlx::query_as::<_, AttachmentPath>("SELECT path, mime_type FROM attachments WHERE id=?")
            .bind(id)
            .fetch_optional(&state.pool)
            .await?
            .ok_or(WebError(StatusCode::NOT_FOUND))?;
    let data_root = PathBuf::from(&state.config.data_dir)
        .join("photos")
        .canonicalize()
        .map_err(|_| WebError(StatusCode::NOT_FOUND))?;
    let raw = Path::new(&row.path);
    if raw.components().any(|part| {
        matches!(
            part,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err(WebError(StatusCode::NOT_FOUND));
    }
    let path = data_root
        .join(raw)
        .canonicalize()
        .map_err(|_| WebError(StatusCode::NOT_FOUND))?;
    if !path.starts_with(&data_root) {
        return Err(WebError(StatusCode::NOT_FOUND));
    }
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|_| WebError(StatusCode::NOT_FOUND))?;
    if !matches!(
        row.mime_type.as_str(),
        "image/jpeg" | "image/png" | "image/webp"
    ) {
        return Err(WebError(StatusCode::NOT_FOUND));
    }
    let mut response = Response::new(Body::from(bytes));
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&row.mime_type)
            .unwrap_or(HeaderValue::from_static("application/octet-stream")),
    );
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    Ok(response)
}

#[derive(Deserialize)]
struct StatusForm {
    status: String,
}
async fn update_status(
    State(state): State<AppState>,
    AxumPath(raw_id): AxumPath<String>,
    headers: HeaderMap,
    Form(form): Form<StatusForm>,
) -> Result<Response, WebError> {
    check_origin(&headers, &state)?;
    let id = parse_id(&raw_id)?;
    let mut tx = state.pool.begin().await?;
    let current: Option<String> = sqlx::query_scalar("SELECT status FROM orders WHERE id=?")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
    let current = current.ok_or(WebError(StatusCode::NOT_FOUND))?;
    let legal = match current.as_str() {
        "NEW" => matches!(form.status.as_str(), "CONFIRMED" | "CANCELLED"),
        "CONFIRMED" => matches!(form.status.as_str(), "IN_PRODUCTION" | "CANCELLED"),
        "IN_PRODUCTION" => matches!(form.status.as_str(), "READY" | "CANCELLED"),
        "READY" => form.status == "IN_PRODUCTION",
        "DELIVERED" | "CANCELLED" => false,
        _ => false,
    };
    if !legal {
        return Err(WebError(StatusCode::BAD_REQUEST));
    }
    sqlx::query(
        "UPDATE orders SET status=?, updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?",
    )
    .bind(&form.status)
    .bind(id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO order_events(order_id,event_type,description,created_by) VALUES(?,?,?,0)",
    )
    .bind(id)
    .bind("STATUS_CHANGED")
    .bind(format!("Status changed from {current} to {}", form.status))
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(redirect(&format!("/orders/{id}")))
}

#[derive(Deserialize)]
struct DateForm {
    promised_delivery_date: String,
}
async fn update_promised_date(
    State(state): State<AppState>,
    AxumPath(raw_id): AxumPath<String>,
    headers: HeaderMap,
    Form(form): Form<DateForm>,
) -> Result<Response, WebError> {
    check_origin(&headers, &state)?;
    let id = parse_id(&raw_id)?;
    if !form.promised_delivery_date.is_empty() && !valid_date(&form.promised_delivery_date) {
        return Err(WebError(StatusCode::BAD_REQUEST));
    }
    let date = if form.promised_delivery_date.is_empty() {
        None
    } else {
        Some(form.promised_delivery_date.as_str())
    };
    let mut tx = state.pool.begin().await?;
    let old: Option<String> =
        sqlx::query_scalar("SELECT promised_delivery_date FROM orders WHERE id=?")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(WebError(StatusCode::NOT_FOUND))?;
    sqlx::query("UPDATE orders SET promised_delivery_date=?, updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?").bind(date).bind(id).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO order_events(order_id,event_type,description,created_by) VALUES(?,'PROMISED_DATE_CHANGED',?,0)").bind(id).bind(format!("Promised delivery date changed from {} to {}", old.as_deref().unwrap_or("unset"), date.unwrap_or("unset"))).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(redirect(&format!("/orders/{id}")))
}

#[derive(Deserialize)]
struct ContactForm {
    phone: String,
    email: String,
    address: String,
    notes: String,
}
async fn update_customer(
    State(state): State<AppState>,
    AxumPath(raw_id): AxumPath<String>,
    headers: HeaderMap,
    Form(form): Form<ContactForm>,
) -> Result<Response, WebError> {
    check_origin(&headers, &state)?;
    let id = parse_id(&raw_id)?;
    if let Some(phone) = empty_none(&form.phone)
        && normalize_phone(Some(phone)).is_none()
    {
        return Err(WebError(StatusCode::BAD_REQUEST));
    }
    if let Some(email) = empty_none(&form.email)
        && (normalize_email(Some(email)).is_none() || !email.contains('@'))
    {
        return Err(WebError(StatusCode::BAD_REQUEST));
    }
    let mut tx = state.pool.begin().await?;
    let name: Option<String> = sqlx::query_scalar("SELECT name FROM customers WHERE id=?")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(WebError(StatusCode::NOT_FOUND))?;
    let result = sqlx::query("UPDATE customers SET phone=?, email=?, address=?, notes=?, normalized_phone=?, normalized_email=?, normalized_name=?, updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?")
        .bind(empty_none(&form.phone)).bind(empty_none(&form.email)).bind(empty_none(&form.address)).bind(empty_none(&form.notes))
        .bind(normalize_phone(empty_none(&form.phone))).bind(normalize_email(empty_none(&form.email))).bind(normalize_name(name.as_deref())).bind(id).execute(&mut *tx).await?;
    if result.rows_affected() == 0 {
        return Err(WebError(StatusCode::NOT_FOUND));
    }
    tx.commit().await?;
    Ok(redirect(&format!("/customers/{id}")))
}

#[derive(Deserialize)]
struct WorkItemForm {
    description: String,
}
async fn add_work_item(
    State(state): State<AppState>,
    AxumPath(raw_id): AxumPath<String>,
    headers: HeaderMap,
    Form(form): Form<WorkItemForm>,
) -> Result<Response, WebError> {
    check_origin(&headers, &state)?;
    let id = parse_id(&raw_id)?;
    let description = form.description.trim();
    if description.is_empty() || description.len() > 500 {
        return Err(WebError(StatusCode::BAD_REQUEST));
    }
    let exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM orders WHERE id=?")
        .bind(id)
        .fetch_one(&state.pool)
        .await?;
    if exists == 0 {
        return Err(WebError(StatusCode::NOT_FOUND));
    }
    let mut tx = state.pool.begin().await?;
    sqlx::query("INSERT INTO work_items(order_id,description,status,sequence,created_at) VALUES (?,?,'PENDING',(SELECT COALESCE(MAX(sequence),0)+1 FROM work_items WHERE order_id=?),strftime('%Y-%m-%dT%H:%M:%fZ','now'))").bind(id).bind(description).bind(id).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO order_events(order_id,event_type,description,created_by) VALUES(?,'WORK_ITEM_ADDED',?,0)").bind(id).bind(format!("Work task added: {description}")).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(redirect(&format!("/orders/{id}")))
}

async fn toggle_work_item(
    State(state): State<AppState>,
    AxumPath(raw_id): AxumPath<String>,
    headers: HeaderMap,
) -> Result<Response, WebError> {
    check_origin(&headers, &state)?;
    let id = parse_id(&raw_id)?;
    let mut tx = state.pool.begin().await?;
    let item: Option<(i64, String, String)> =
        sqlx::query_as("SELECT order_id, description, status FROM work_items WHERE id=?")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
    let (order_id, description, current) = item.ok_or(WebError(StatusCode::NOT_FOUND))?;
    let next = if current == "DONE" { "PENDING" } else { "DONE" };
    sqlx::query("UPDATE work_items SET status=?, completed_at=CASE WHEN ?='DONE' THEN strftime('%Y-%m-%dT%H:%M:%fZ','now') ELSE NULL END WHERE id=?").bind(next).bind(next).bind(id).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO order_events(order_id,event_type,description,created_by) VALUES(?,'WORK_ITEM_STATUS_CHANGED',?,0)").bind(order_id).bind(format!("Work task {description}: {next}")).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(redirect(&format!("/orders/{order_id}")))
}

fn check_origin(headers: &HeaderMap, state: &AppState) -> Result<(), WebError> {
    let expected = state.config.public_base_url.trim_end_matches('/');
    let origin = headers
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .ok_or(WebError(StatusCode::FORBIDDEN))?;
    if origin == expected {
        Ok(())
    } else {
        Err(WebError(StatusCode::FORBIDDEN))
    }
}
fn parse_id(raw: &str) -> Result<i64, WebError> {
    raw.parse::<i64>()
        .map_err(|_| WebError(StatusCode::NOT_FOUND))
        .and_then(valid_id)
}
fn valid_status(s: &str) -> bool {
    matches!(
        s,
        "NEW" | "CONFIRMED" | "IN_PRODUCTION" | "READY" | "DELIVERED" | "CANCELLED"
    )
}
fn valid_date(s: &str) -> bool {
    s.len() == 10 && chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok()
}
fn empty_none(s: &str) -> Option<&str> {
    let t = s.trim();
    if t.is_empty() { None } else { Some(t) }
}
fn redirect(location: &str) -> Response {
    (
        StatusCode::SEE_OTHER,
        [(header::LOCATION, location.to_owned())],
    )
        .into_response()
}

async fn stylesheet() -> Response {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("../../static/app.css"),
    )
        .into_response()
}
async fn htmx() -> Response {
    (
        [(
            header::CONTENT_TYPE,
            "application/javascript; charset=utf-8",
        )],
        include_str!("../../static/htmx.min.js"),
    )
        .into_response()
}
