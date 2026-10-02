use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Extraction {
    pub customer: CustomerInput,
    pub order: OrderInput,
    pub missing_fields: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CustomerInput {
    pub name: Option<String>,
    pub phone: Option<String>,
    pub email: Option<String>,
    pub address: Option<String>,
    pub preferred_language: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrderInput {
    pub description: String,
    pub requested_delivery_date: Option<String>,
    pub items: Vec<ItemInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ItemInput {
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

pub type OrderItemInput = ItemInput;

impl Extraction {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.order.description.trim().is_empty(),
            "order description is required"
        );
        anyhow::ensure!(
            !self.order.items.is_empty(),
            "at least one order item is required"
        );
        if let Some(language) = &self.customer.preferred_language {
            anyhow::ensure!(
                matches!(language.as_str(), "es" | "en"),
                "preferred_language must be es or en"
            );
        }
        if let Some(date) = &self.order.requested_delivery_date {
            anyhow::ensure!(date.len() == 10, "date must use YYYY-MM-DD");
            let parsed = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")?;
            anyhow::ensure!(
                parsed.format("%Y-%m-%d").to_string() == *date,
                "date must use YYYY-MM-DD"
            );
        }
        for item in &self.order.items {
            anyhow::ensure!(
                !item.description.trim().is_empty(),
                "item description is required"
            );
            anyhow::ensure!(item.quantity > 0, "item quantity must be positive");
            if let Some(unit) = &item.dimension_unit {
                anyhow::ensure!(
                    matches!(unit.as_str(), "in" | "ft" | "cm" | "mm"),
                    "unsupported dimension_unit"
                );
            }
            for measurement in [item.length, item.width, item.height].into_iter().flatten() {
                anyhow::ensure!(
                    measurement.is_finite() && measurement > 0.0,
                    "measurements must be finite and positive"
                );
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, FromRow)]
pub struct Customer {
    pub id: i64,
    pub name: Option<String>,
    pub phone: Option<String>,
    pub email: Option<String>,
    pub address: Option<String>,
    pub preferred_language: Option<String>,
    pub notes: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, FromRow)]
pub struct Order {
    pub id: i64,
    pub customer_id: i64,
    pub status: String,
    pub original_text: String,
    pub normalized_description: String,
    pub requested_delivery_date: Option<String>,
    pub promised_delivery_date: Option<String>,
    pub created_by: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, FromRow)]
pub struct OrderItem {
    pub id: i64,
    pub order_id: i64,
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

#[derive(Debug, Clone, FromRow)]
pub struct Delivery {
    pub id: i64,
    pub order_id: i64,
    pub delivered_at: String,
    pub delivered_by: i64,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
pub struct Attachment {
    pub id: i64,
    pub order_id: i64,
    pub delivery_id: Option<i64>,
    pub r#type: String,
    pub path: String,
    pub telegram_file_id: Option<String>,
    pub telegram_message_id: Option<i64>,
    pub mime_type: String,
    pub created_at: String,
}

#[derive(Debug, Clone, FromRow)]
pub struct WorkItem {
    pub id: i64,
    pub order_id: i64,
    pub description: String,
    pub status: String,
    pub sequence: i64,
    pub assigned_to: Option<String>,
    pub created_at: String,
    pub completed_at: Option<String>,
}
