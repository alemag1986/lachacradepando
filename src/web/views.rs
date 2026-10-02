use askama::Template;
use sqlx::FromRow;

#[derive(FromRow)]
pub struct DashboardCounts {
    pub new_count: i64,
    pub production_count: i64,
    pub ready_count: i64,
    pub delivered_week: i64,
    pub overdue: i64,
}
#[derive(FromRow)]
pub struct OrderRow {
    pub id: i64,
    pub customer_name: Option<String>,
    pub status: String,
    pub description: Option<String>,
    pub promised_delivery_date: Option<String>,
    pub requested_delivery_date: Option<String>,
}
#[derive(FromRow)]
pub struct OrderDetail {
    pub id: i64,
    pub status: String,
    pub description: Option<String>,
    pub original_text: Option<String>,
    pub requested_delivery_date: Option<String>,
    pub promised_delivery_date: Option<String>,
    pub created_at: String,
    pub customer_id: i64,
    pub customer_name: Option<String>,
    pub phone: Option<String>,
    pub email: Option<String>,
    pub address: Option<String>,
    pub customer_notes: Option<String>,
}
#[derive(FromRow)]
pub struct ItemRow {
    pub product_type: Option<String>,
    pub description: String,
    pub quantity: i64,
    pub length: Option<f64>,
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub dimension_unit: Option<String>,
    pub wood_species: Option<String>,
    pub finish: Option<String>,
    pub notes: Option<String>,
}
#[derive(FromRow)]
pub struct TaskRow {
    pub id: i64,
    pub description: String,
    pub status: String,
    pub assigned_to: Option<String>,
}
#[derive(FromRow)]
pub struct EventRow {
    pub event_type: String,
    pub description: Option<String>,
    pub created_at: String,
}
#[derive(FromRow)]
pub struct DeliveryRow {
    pub delivered_at: String,
    pub delivered_by: i64,
    pub notes: Option<String>,
}
#[derive(FromRow)]
pub struct AttachmentRow {
    pub id: i64,
    #[sqlx(rename = "type")]
    pub kind: String,
}
#[derive(FromRow)]
pub struct CustomerRow {
    pub id: i64,
    pub name: Option<String>,
    pub phone: Option<String>,
    pub email: Option<String>,
    pub order_count: i64,
    pub latest_order: Option<String>,
}
#[derive(FromRow)]
pub struct CustomerDetail {
    pub id: i64,
    pub name: Option<String>,
    pub phone: Option<String>,
    pub email: Option<String>,
    pub address: Option<String>,
    pub preferred_language: Option<String>,
    pub notes: Option<String>,
    pub created_at: String,
}
#[derive(FromRow)]
pub struct CustomerDelivery {
    pub delivered_at: String,
    pub delivered_by: i64,
    pub notes: Option<String>,
    pub order_id: i64,
}
#[derive(FromRow)]
pub struct DeliveryView {
    pub order_id: i64,
    pub customer_name: Option<String>,
    pub delivered_at: String,
    pub delivered_by: i64,
    pub notes: Option<String>,
    pub photo_count: i64,
}
#[derive(FromRow)]
pub struct AttachmentPath {
    pub path: String,
    pub mime_type: String,
}

impl ItemRow {
    pub fn dimensions(&self) -> String {
        format!(
            "{} × {} × {} {}",
            dim(self.length),
            dim(self.width),
            dim(self.height),
            self.dimension_unit.as_deref().unwrap_or("")
        )
    }
}
fn dim(value: Option<f64>) -> String {
    match value {
        Some(v) => v.to_string(),
        None => "—".to_owned(),
    }
}
impl OrderRow {
    pub fn due_date(&self) -> &str {
        self.promised_delivery_date
            .as_deref()
            .or(self.requested_delivery_date.as_deref())
            .unwrap_or("—")
    }
    pub fn customer(&self) -> &str {
        self.customer_name.as_deref().unwrap_or("Unknown customer")
    }
}
impl OrderDetail {
    pub fn customer(&self) -> &str {
        self.customer_name.as_deref().unwrap_or("Unknown customer")
    }
}
impl CustomerRow {
    pub fn display_name(&self) -> &str {
        self.name.as_deref().unwrap_or("Unnamed customer")
    }
}
impl CustomerDetail {
    pub fn display_name(&self) -> &str {
        self.name.as_deref().unwrap_or("Unnamed customer")
    }
}
impl DeliveryView {
    pub fn customer(&self) -> &str {
        self.customer_name.as_deref().unwrap_or("Unknown customer")
    }
}
impl OrderFilters {
    pub fn selected_status(&self, status: &str) -> bool {
        self.status.as_deref() == Some(status)
    }
    pub fn search_value(&self) -> &str {
        self.search.as_deref().unwrap_or("")
    }
    pub fn customer_value(&self) -> &str {
        self.customer.as_deref().unwrap_or("")
    }
    pub fn date_value(&self) -> &str {
        self.delivery_date.as_deref().unwrap_or("")
    }
}
use super::OrderFilters;

#[derive(Template)]
#[template(path = "dashboard.html")]
pub struct DashboardTemplate {
    pub counts: DashboardCounts,
    pub recent: Vec<OrderRow>,
}
#[derive(Template)]
#[template(path = "orders.html")]
pub struct OrderListTemplate {
    pub orders: Vec<OrderRow>,
    pub filters: super::OrderFilters,
}
#[derive(Template)]
#[template(path = "order_detail.html")]
pub struct OrderDetailTemplate {
    pub order: OrderDetail,
    pub items: Vec<ItemRow>,
    pub tasks: Vec<TaskRow>,
    pub events: Vec<EventRow>,
    pub delivery: Option<DeliveryRow>,
    pub photos: Vec<AttachmentRow>,
}
#[derive(Template)]
#[template(path = "customers.html")]
pub struct CustomerListTemplate {
    pub customers: Vec<CustomerRow>,
}
#[derive(Template)]
#[template(path = "customer_detail.html")]
pub struct CustomerDetailTemplate {
    pub customer: CustomerDetail,
    pub orders: Vec<OrderRow>,
    pub deliveries: Vec<CustomerDelivery>,
}
#[derive(Template)]
#[template(path = "deliveries.html")]
pub struct DeliveriesTemplate {
    pub deliveries: Vec<DeliveryView>,
}
