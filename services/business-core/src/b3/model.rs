use crate::b2::model::DecimalString;
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PurchaseOrderLineInput {
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
    DecimalString(rust_decimal::Decimal::ZERO)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreatePurchaseOrder {
    pub legal_entity_id: Uuid,
    pub supplier_id: Uuid,
    #[serde(default)]
    pub buyer_user_id: Option<Uuid>,
    pub business_unit_id: Uuid,
    #[serde(default)]
    pub department_id: Option<Uuid>,
    #[serde(default)]
    pub brand_id: Option<Uuid>,
    pub currency: String,
    pub order_date: NaiveDate,
    #[serde(default)]
    pub expected_delivery_date: Option<NaiveDate>,
    #[serde(default)]
    pub payment_terms_days: Option<i32>,
    #[serde(default)]
    pub supplier_reference: Option<String>,
    #[serde(default)]
    pub business_note: Option<String>,
    pub lines: Vec<PurchaseOrderLineInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReplacePurchaseOrderDraft {
    pub expected_version: i64,
    #[serde(flatten)]
    pub order: CreatePurchaseOrder,
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
pub struct GoodsReceiptLineInput {
    pub purchase_order_line_id: Uuid,
    pub quantity: DecimalString,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateGoodsReceipt {
    pub purchase_order_id: Uuid,
    pub warehouse_id: Uuid,
    pub receipt_date: NaiveDate,
    pub lines: Vec<GoodsReceiptLineInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateSupplierPayment {
    pub legal_entity_id: Uuid,
    pub supplier_id: Uuid,
    pub currency: String,
    pub payment_date: NaiveDate,
    pub amount: DecimalString,
    pub payment_method: String,
    #[serde(default)]
    pub external_reference: Option<String>,
    #[serde(default)]
    pub business_note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PayableAllocationInput {
    pub payable_id: Uuid,
    pub amount: DecimalString,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApplySupplierPayment {
    pub expected_payment_version: i64,
    pub allocations: Vec<PayableAllocationInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReversePayableAllocation {
    pub expected_payment_version: i64,
    pub expected_payable_version: i64,
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
pub struct PurchaseOrderView {
    pub id: Uuid,
    pub purchase_order_number: String,
    pub legal_entity_id: Uuid,
    pub supplier_id: Uuid,
    pub currency: String,
    pub lifecycle_status: String,
    pub receiving_status: String,
    #[sqlx(try_from = "rust_decimal::Decimal")]
    pub gross_amount: DecimalString,
    pub order_date: NaiveDate,
    pub updated_at: DateTime<Utc>,
    pub version: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PurchaseOrderDraftLineView {
    pub sku_id: Uuid,
    pub warehouse_id: Uuid,
    pub unit_of_measure_id: Uuid,
    pub quantity: DecimalString,
    pub unit_price: DecimalString,
    pub discount_amount: DecimalString,
    pub tax_rate: DecimalString,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PurchaseOrderDraftView {
    pub id: Uuid,
    pub purchase_order_number: String,
    pub legal_entity_id: Uuid,
    pub supplier_id: Uuid,
    pub business_unit_id: Uuid,
    pub currency: String,
    pub order_date: NaiveDate,
    pub expected_delivery_date: Option<NaiveDate>,
    pub payment_terms_days: i32,
    pub supplier_reference: Option<String>,
    pub business_note: Option<String>,
    pub lifecycle_status: String,
    pub version: i64,
    pub lines: Vec<PurchaseOrderDraftLineView>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PurchaseOrderEntryOptions {
    pub can_create: bool,
    pub can_update: bool,
    pub data_as_of: DateTime<Utc>,
    pub draft: Option<PurchaseOrderDraftView>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PurchaseOrderConfirmationLine {
    pub line_number: i32,
    pub sku_code: String,
    pub sku_name: String,
    pub warehouse_code: String,
    pub warehouse_name: String,
    pub unit_code: String,
    pub unit_name: String,
    pub ordered_quantity: DecimalString,
    pub unit_price: DecimalString,
    pub discount_amount: DecimalString,
    pub net_amount: DecimalString,
    pub tax_rate: DecimalString,
    pub tax_amount: DecimalString,
    pub gross_amount: DecimalString,
    pub ready: bool,
    pub readiness: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PurchaseOrderConfirmationPreview {
    pub order_id: Uuid,
    pub order_number: String,
    pub supplier_code: String,
    pub supplier_name: String,
    pub currency: String,
    pub order_date: NaiveDate,
    pub expected_delivery_date: Option<NaiveDate>,
    pub payment_terms_days: i32,
    pub lifecycle_status: String,
    pub version: i64,
    pub subtotal_amount: DecimalString,
    pub discount_amount: DecimalString,
    pub net_amount: DecimalString,
    pub tax_amount: DecimalString,
    pub gross_amount: DecimalString,
    pub warehouse_count: i64,
    pub can_confirm: bool,
    pub readiness: String,
    pub checked_at: DateTime<Utc>,
    pub lines: Vec<PurchaseOrderConfirmationLine>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct GoodsReceiptView {
    pub id: Uuid,
    pub goods_receipt_number: String,
    pub purchase_order_id: Uuid,
    pub legal_entity_id: Uuid,
    pub supplier_id: Uuid,
    pub warehouse_id: Uuid,
    pub receipt_date: NaiveDate,
    pub status: String,
    pub currency: String,
    #[sqlx(try_from = "rust_decimal::Decimal")]
    pub gross_amount: DecimalString,
    #[sqlx(try_from = "rust_decimal::Decimal")]
    pub inventory_cost_amount: DecimalString,
    pub updated_at: DateTime<Utc>,
    pub version: i64,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct GoodsReceiptDraftOptionLine {
    pub order_id: Uuid,
    pub order_number: String,
    pub supplier_code: String,
    pub supplier_name: String,
    pub currency: String,
    pub warehouse_id: Uuid,
    pub warehouse_code: String,
    pub warehouse_name: String,
    pub purchase_order_line_id: Uuid,
    pub line_number: i32,
    pub sku_id: Uuid,
    pub sku_code: String,
    pub sku_name: String,
    pub unit_code: String,
    pub unit_name: String,
    #[sqlx(try_from = "rust_decimal::Decimal")]
    pub ordered_quantity: DecimalString,
    #[sqlx(try_from = "rust_decimal::Decimal")]
    pub received_quantity: DecimalString,
    #[sqlx(try_from = "rust_decimal::Decimal")]
    pub cancelled_quantity: DecimalString,
    #[sqlx(try_from = "rust_decimal::Decimal")]
    pub draft_allocated_quantity: DecimalString,
    #[sqlx(try_from = "rust_decimal::Decimal")]
    pub receivable_quantity: DecimalString,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GoodsReceiptDraftOptions {
    pub can_create: bool,
    pub data_as_of: DateTime<Utc>,
    pub items: Vec<GoodsReceiptDraftOptionLine>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GoodsReceiptConfirmationLine {
    pub purchase_order_line_id: Uuid,
    pub sku_id: Uuid,
    pub sku_code: String,
    pub sku_name: String,
    pub received_quantity: DecimalString,
    pub order_remaining_quantity: DecimalString,
    pub provisional_unit_cost: DecimalString,
    pub provisional_inventory_cost: DecimalString,
    pub current_on_hand_quantity: DecimalString,
    pub current_inventory_value: DecimalString,
    pub current_average_unit_cost: Option<DecimalString>,
    pub projected_on_hand_quantity: DecimalString,
    pub projected_inventory_value: DecimalString,
    pub projected_average_unit_cost: DecimalString,
    pub ready: bool,
    pub readiness: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GoodsReceiptConfirmationPreview {
    pub receipt_id: Uuid,
    pub receipt_number: String,
    pub purchase_order_id: Uuid,
    pub order_number: String,
    pub supplier_code: String,
    pub supplier_name: String,
    pub warehouse_code: String,
    pub warehouse_name: String,
    pub receipt_date: NaiveDate,
    pub status: String,
    pub version: i64,
    pub currency: String,
    pub expected_inventory_cost: DecimalString,
    pub expected_tax_amount: DecimalString,
    pub expected_payable_amount: DecimalString,
    pub expected_due_date: NaiveDate,
    pub can_confirm: bool,
    pub readiness: String,
    pub inventory_as_of: DateTime<Utc>,
    pub lines: Vec<GoodsReceiptConfirmationLine>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct PayableView {
    pub id: Uuid,
    pub payable_number: String,
    pub legal_entity_id: Uuid,
    pub supplier_id: Uuid,
    pub purchase_order_id: Uuid,
    pub goods_receipt_id: Uuid,
    pub currency: String,
    #[sqlx(try_from = "rust_decimal::Decimal")]
    pub original_amount: DecimalString,
    #[sqlx(try_from = "rust_decimal::Decimal")]
    pub settled_amount: DecimalString,
    #[sqlx(try_from = "rust_decimal::Decimal")]
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
pub struct SupplierPaymentView {
    pub id: Uuid,
    pub supplier_payment_number: String,
    pub legal_entity_id: Uuid,
    pub supplier_id: Uuid,
    pub currency: String,
    pub payment_date: NaiveDate,
    #[sqlx(try_from = "rust_decimal::Decimal")]
    pub amount: DecimalString,
    #[sqlx(try_from = "rust_decimal::Decimal")]
    pub allocated_amount: DecimalString,
    #[sqlx(try_from = "rust_decimal::Decimal")]
    pub unapplied_amount: DecimalString,
    pub status: String,
    pub updated_at: DateTime<Utc>,
    pub version: i64,
}
