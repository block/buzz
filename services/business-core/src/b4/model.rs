use crate::b2::model::DecimalString;
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FixedWeightInput {
    pub sales_order_id: Uuid,
    pub weight: DecimalString,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AdjustmentLineInput {
    pub metric_type: String,
    pub amount: DecimalString,
    pub business_date: NaiveDate,
    pub allocation_basis: String,
    #[serde(default)]
    pub direct_sales_order_id: Option<Uuid>,
    #[serde(default)]
    pub customer_id: Option<Uuid>,
    #[serde(default)]
    pub sku_id: Option<Uuid>,
    #[serde(default)]
    pub brand_id: Option<Uuid>,
    #[serde(default)]
    pub salesperson_user_id: Option<Uuid>,
    #[serde(default)]
    pub business_unit_id: Option<Uuid>,
    #[serde(default)]
    pub department_id: Option<Uuid>,
    #[serde(default)]
    pub warehouse_id: Option<Uuid>,
    #[serde(default)]
    pub sales_order_ids: Vec<Uuid>,
    #[serde(default)]
    pub fixed_weights: Vec<FixedWeightInput>,
    pub reason_code: String,
    #[serde(default)]
    pub source_reference: Option<String>,
    #[serde(default)]
    pub business_note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateAdjustmentBatch {
    pub legal_entity_id: Uuid,
    pub currency: String,
    pub management_period: String,
    pub lines: Vec<AdjustmentLineInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReplaceAdjustmentDraft {
    pub expected_version: i64,
    #[serde(flatten)]
    pub batch: CreateAdjustmentBatch,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VersionCommand {
    pub expected_version: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PostAdjustment {
    pub expected_version: i64,
    pub preview_id: Uuid,
    pub preview_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GenerateReportSnapshot {
    pub report_type: String,
    pub management_period: String,
    pub currency: String,
    #[serde(default)]
    pub legal_entity_ids: Vec<Uuid>,
    #[serde(default)]
    pub supersedes_snapshot_id: Option<Uuid>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewResult {
    pub preview_id: Uuid,
    pub batch_id: Uuid,
    pub preview_hash: String,
    pub source_hash: String,
    pub source_watermark: i64,
    pub batch_version: i64,
    pub total_amount: DecimalString,
    pub allocated_amount: DecimalString,
    pub unallocated_amount: DecimalString,
    pub allocations: Value,
    pub data_as_of: DateTime<Utc>,
    pub trace_id: Uuid,
    pub idempotent_replay: bool,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct OrderProfitView {
    pub sales_order_id: Uuid,
    pub order_number: String,
    pub legal_entity_id: Uuid,
    pub customer_id: Uuid,
    pub brand_id: Option<Uuid>,
    pub business_unit_id: Uuid,
    pub salesperson_user_id: Uuid,
    pub currency: String,
    #[sqlx(try_from = "rust_decimal::Decimal")]
    pub net_revenue: DecimalString,
    #[sqlx(try_from = "rust_decimal::Decimal")]
    pub product_cost: DecimalString,
    #[sqlx(try_from = "rust_decimal::Decimal")]
    pub gross_profit: DecimalString,
    #[sqlx(try_from = "rust_decimal::Decimal")]
    pub outbound_freight: DecimalString,
    #[sqlx(try_from = "rust_decimal::Decimal")]
    pub sales_commission: DecimalString,
    #[sqlx(try_from = "rust_decimal::Decimal")]
    pub platform_fee: DecimalString,
    #[sqlx(try_from = "rust_decimal::Decimal")]
    pub customer_rebate: DecimalString,
    #[sqlx(try_from = "rust_decimal::Decimal")]
    pub supplier_rebate: DecimalString,
    #[sqlx(try_from = "rust_decimal::Decimal")]
    pub other_direct_cost: DecimalString,
    #[sqlx(try_from = "rust_decimal::Decimal")]
    pub contribution_profit: DecimalString,
    #[sqlx(try_from = "rust_decimal::Decimal")]
    pub allocated_operating_expense: DecimalString,
    #[sqlx(try_from = "rust_decimal::Decimal")]
    pub management_operating_profit: DecimalString,
    pub gross_margin_rate: Option<rust_decimal::Decimal>,
    pub contribution_margin_rate: Option<rust_decimal::Decimal>,
    pub management_operating_margin_rate: Option<rust_decimal::Decimal>,
    pub data_quality_status: String,
    pub data_as_of: DateTime<Utc>,
    pub last_fact_sequence: i64,
}
