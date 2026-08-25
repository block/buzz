use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;
use serde::{de, Deserialize, Deserializer, Serialize, Serializer};
use std::{fmt, str::FromStr};
use uuid::Uuid;

/// Exact fixed-point value encoded as a JSON string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct DecimalString(pub Decimal);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OptionalDecimalString(pub Option<Decimal>);

impl From<Option<Decimal>> for OptionalDecimalString {
    fn from(value: Option<Decimal>) -> Self {
        Self(value)
    }
}

impl Serialize for OptionalDecimalString {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self.0 {
            Some(value) => serializer.serialize_some(&value.to_string()),
            None => serializer.serialize_none(),
        }
    }
}

impl DecimalString {
    pub fn non_negative(self, field: &'static str) -> Result<Decimal, String> {
        if self.0.is_sign_negative() {
            Err(format!("{field} must be non-negative"))
        } else {
            Ok(self.0)
        }
    }

    pub fn positive(self, field: &'static str) -> Result<Decimal, String> {
        if self.0 <= Decimal::ZERO {
            Err(format!("{field} must be greater than zero"))
        } else {
            Ok(self.0)
        }
    }
}

impl Serialize for DecimalString {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0.to_string())
    }
}

impl<'de> Deserialize<'de> for DecimalString {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct Visitor;
        impl de::Visitor<'_> for Visitor {
            type Value = DecimalString;
            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a fixed-point decimal JSON string")
            }
            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                let decimal = Decimal::from_str(value).map_err(E::custom)?;
                if decimal.scale() > 6 {
                    return Err(E::custom("decimal scale must not exceed 6"));
                }
                Ok(DecimalString(decimal))
            }
        }
        deserializer.deserialize_str(Visitor)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SalesOrderLineInput {
    pub sku_id: Uuid,
    pub warehouse_id: Uuid,
    pub unit_of_measure_id: Uuid,
    pub quantity: DecimalString,
    pub unit_price: DecimalString,
    #[serde(default = "zero")]
    pub discount_amount: DecimalString,
    #[serde(default = "zero")]
    pub tax_rate: DecimalString,
    #[serde(default)]
    pub business_unit_id: Option<Uuid>,
    #[serde(default)]
    pub department_id: Option<Uuid>,
    #[serde(default)]
    pub brand_id: Option<Uuid>,
}

fn zero() -> DecimalString {
    DecimalString(Decimal::ZERO)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateSalesOrder {
    pub legal_entity_id: Uuid,
    pub customer_id: Uuid,
    #[serde(default)]
    pub salesperson_user_id: Option<Uuid>,
    pub business_unit_id: Uuid,
    #[serde(default)]
    pub department_id: Option<Uuid>,
    #[serde(default)]
    pub brand_id: Option<Uuid>,
    pub currency: String,
    pub order_date: NaiveDate,
    #[serde(default)]
    pub requested_delivery_date: Option<NaiveDate>,
    #[serde(default)]
    pub payment_terms_days: Option<i32>,
    #[serde(default)]
    pub customer_reference: Option<String>,
    #[serde(default)]
    pub business_note: Option<String>,
    pub lines: Vec<SalesOrderLineInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReplaceSalesOrderDraft {
    pub expected_version: i64,
    pub customer_id: Uuid,
    pub business_unit_id: Uuid,
    #[serde(default)]
    pub department_id: Option<Uuid>,
    #[serde(default)]
    pub brand_id: Option<Uuid>,
    pub currency: String,
    pub order_date: NaiveDate,
    #[serde(default)]
    pub requested_delivery_date: Option<NaiveDate>,
    #[serde(default)]
    pub payment_terms_days: Option<i32>,
    #[serde(default)]
    pub customer_reference: Option<String>,
    #[serde(default)]
    pub business_note: Option<String>,
    pub lines: Vec<SalesOrderLineInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VersionCommand {
    pub expected_version: i64,
    #[serde(default)]
    pub reason_code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InventoryOpeningLineInput {
    pub warehouse_id: Uuid,
    pub sku_id: Uuid,
    pub quantity: DecimalString,
    pub unit_cost: DecimalString,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateInventoryOpening {
    pub legal_entity_id: Uuid,
    pub business_date: NaiveDate,
    pub currency: String,
    pub lines: Vec<InventoryOpeningLineInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShipmentLineInput {
    pub sales_order_line_id: Uuid,
    pub quantity: DecimalString,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateShipment {
    pub sales_order_id: Uuid,
    pub warehouse_id: Uuid,
    pub shipment_date: NaiveDate,
    pub lines: Vec<ShipmentLineInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateCustomerReceipt {
    pub legal_entity_id: Uuid,
    pub customer_id: Uuid,
    pub currency: String,
    pub receipt_date: NaiveDate,
    pub amount: DecimalString,
    pub payment_method: String,
    #[serde(default)]
    pub external_reference: Option<String>,
    #[serde(default)]
    pub business_note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AllocationInput {
    pub receivable_id: Uuid,
    pub amount: DecimalString,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApplyReceipt {
    pub expected_receipt_version: i64,
    pub allocations: Vec<AllocationInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReverseAllocation {
    pub expected_receipt_version: i64,
    pub expected_receivable_version: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandResult {
    pub id: Uuid,
    pub number: String,
    pub status: String,
    pub version: i64,
    pub trace_id: Uuid,
    pub idempotent_replay: bool,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct SalesOrderSummary {
    pub id: Uuid,
    pub order_number: String,
    pub legal_entity_id: Uuid,
    pub customer_id: Uuid,
    pub currency: String,
    pub lifecycle_status: String,
    pub hold_status: String,
    pub fulfillment_status: String,
    #[sqlx(try_from = "Decimal")]
    pub gross_amount: DecimalString,
    pub order_date: NaiveDate,
    pub updated_at: DateTime<Utc>,
    pub version: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SalesOrderConfirmationLine {
    pub sku_id: Uuid,
    pub sku_code: String,
    pub sku_name: String,
    pub warehouse_id: Uuid,
    pub warehouse_code: String,
    pub warehouse_name: String,
    pub required_quantity: DecimalString,
    pub on_hand_quantity: DecimalString,
    pub reserved_quantity: DecimalString,
    pub available_quantity: DecimalString,
    pub expected_reserved_quantity: DecimalString,
    pub shortage_quantity: DecimalString,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SalesOrderConfirmationPreview {
    pub order_id: Uuid,
    pub order_number: String,
    pub lifecycle_status: String,
    pub version: i64,
    pub can_confirm: bool,
    pub readiness: String,
    pub all_available: bool,
    pub inventory_as_of: DateTime<Utc>,
    pub lines: Vec<SalesOrderConfirmationLine>,
}

impl From<Decimal> for DecimalString {
    fn from(value: Decimal) -> Self {
        Self(value)
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InventoryBalanceView {
    pub legal_entity_id: Uuid,
    pub warehouse_id: Uuid,
    pub sku_id: Uuid,
    pub on_hand_quantity: DecimalString,
    pub reserved_quantity: DecimalString,
    pub quarantined_quantity: DecimalString,
    pub available_quantity: DecimalString,
    pub inventory_value: DecimalString,
    pub average_unit_cost: Option<DecimalString>,
    pub last_movement_id: Option<Uuid>,
    pub updated_at: DateTime<Utc>,
    pub version: i64,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ReceivableView {
    pub id: Uuid,
    pub receivable_number: String,
    pub legal_entity_id: Uuid,
    pub customer_id: Uuid,
    pub sales_order_id: Uuid,
    pub shipment_id: Uuid,
    pub currency: String,
    #[sqlx(try_from = "Decimal")]
    pub original_amount: DecimalString,
    #[sqlx(try_from = "Decimal")]
    pub settled_amount: DecimalString,
    #[sqlx(try_from = "Decimal")]
    pub open_amount: DecimalString,
    pub due_date: NaiveDate,
    pub status: String,
    pub is_overdue: bool,
    pub overdue_days: i32,
    pub updated_at: DateTime<Utc>,
    pub version: i64,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ReceiptView {
    pub id: Uuid,
    pub receipt_number: String,
    pub legal_entity_id: Uuid,
    pub customer_id: Uuid,
    pub currency: String,
    pub receipt_date: NaiveDate,
    #[sqlx(try_from = "Decimal")]
    pub amount: DecimalString,
    #[sqlx(try_from = "Decimal")]
    pub allocated_amount: DecimalString,
    #[sqlx(try_from = "Decimal")]
    pub unapplied_amount: DecimalString,
    pub status: String,
    pub updated_at: DateTime<Utc>,
    pub version: i64,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ShipmentView {
    pub id: Uuid,
    pub shipment_number: String,
    pub sales_order_id: Uuid,
    pub warehouse_id: Uuid,
    pub shipment_date: NaiveDate,
    pub status: String,
    pub confirmed_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
    pub version: i64,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ShipmentDraftOptionLine {
    pub order_id: Uuid,
    pub order_number: String,
    pub customer_code: String,
    pub customer_name: String,
    pub currency: String,
    pub warehouse_id: Uuid,
    pub warehouse_code: String,
    pub warehouse_name: String,
    pub sales_order_line_id: Uuid,
    pub line_number: i32,
    pub sku_id: Uuid,
    pub sku_code: String,
    pub sku_name: String,
    #[sqlx(try_from = "Decimal")]
    pub ordered_quantity: DecimalString,
    #[sqlx(try_from = "Decimal")]
    pub shipped_quantity: DecimalString,
    #[sqlx(try_from = "Decimal")]
    pub reservation_open_quantity: DecimalString,
    #[sqlx(try_from = "Decimal")]
    pub draft_allocated_quantity: DecimalString,
    #[sqlx(try_from = "Decimal")]
    pub shippable_quantity: DecimalString,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShipmentDraftOptions {
    pub can_create: bool,
    pub data_as_of: DateTime<Utc>,
    pub items: Vec<ShipmentDraftOptionLine>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShipmentConfirmationLine {
    pub sales_order_line_id: Uuid,
    pub sku_id: Uuid,
    pub sku_code: String,
    pub sku_name: String,
    pub quantity: DecimalString,
    pub reservation_open_quantity: DecimalString,
    pub on_hand_quantity: DecimalString,
    pub reserved_quantity: DecimalString,
    pub average_unit_cost: Option<DecimalString>,
    pub expected_cost_amount: Option<DecimalString>,
    pub ready: bool,
    pub readiness: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShipmentConfirmationPreview {
    pub shipment_id: Uuid,
    pub shipment_number: String,
    pub sales_order_id: Uuid,
    pub order_number: String,
    pub customer_code: String,
    pub customer_name: String,
    pub warehouse_code: String,
    pub warehouse_name: String,
    pub shipment_date: NaiveDate,
    pub status: String,
    pub version: i64,
    pub currency: String,
    pub sales_amount: DecimalString,
    pub expected_cost_amount: Option<DecimalString>,
    pub expected_receivable_amount: DecimalString,
    pub expected_due_date: NaiveDate,
    pub can_confirm: bool,
    pub readiness: String,
    pub inventory_as_of: DateTime<Utc>,
    pub lines: Vec<ShipmentConfirmationLine>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct InventoryOpeningView {
    pub id: Uuid,
    pub batch_number: String,
    pub legal_entity_id: Uuid,
    pub business_date: NaiveDate,
    pub currency: String,
    pub status: String,
    pub posted_at: Option<DateTime<Utc>>,
    pub reversed_at: Option<DateTime<Utc>>,
    pub version: i64,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct InventoryMovementView {
    pub id: Uuid,
    pub legal_entity_id: Uuid,
    pub warehouse_id: Uuid,
    pub sku_id: Uuid,
    pub movement_type: String,
    #[sqlx(try_from = "Decimal")]
    pub quantity: DecimalString,
    #[sqlx(try_from = "Decimal")]
    pub unit_cost: DecimalString,
    #[sqlx(try_from = "Decimal")]
    pub total_cost: DecimalString,
    pub business_date: NaiveDate,
    pub posted_at: DateTime<Utc>,
}
