#![forbid(unsafe_code)]

use business_query_contracts::{DateRange, Money, PageInput, ResourceRef, ValidationError};
use chrono::{DateTime, NaiveDate, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

pub const BUSINESS_ANOMALY_READ: &str = "business_anomaly:read";
pub const RULE_SET_VERSION: &str = "trade-risk-v1.0";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AnomalyStatus {
    Ok,
    Partial,
    MissingContext,
    NotFoundOrForbidden,
    UpstreamUnavailable,
    DataQualityBlocked,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DataQualityCode {
    MissingCost,
    MissingFreight,
    MissingCommission,
    MissingRebate,
    MissingCurrency,
    MissingRelation,
    StaleData,
    DuplicateSource,
    InvalidAmount,
    InconsistentStatus,
    PartialSync,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FactIdentity {
    pub source_system: String,
    pub object_type: String,
    pub object_id: String,
    pub object_version: String,
    pub legal_entity_id: String,
    pub updated_at: DateTime<Utc>,
    pub data_as_of: DateTime<Utc>,
    pub source_sync_status: String,
    pub trace_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SalesOrderFact {
    #[serde(flatten)]
    pub identity: FactIdentity,
    pub customer_id: String,
    pub warehouse_id: String,
    pub brand_id: String,
    pub business_unit_id: String,
    pub salesperson_id: String,
    pub sku_id: String,
    pub ordered_at: NaiveDate,
    pub revenue: Money,
    pub shipped_amount: Money,
    pub invoiced_amount: Money,
    pub received_amount: Money,
    pub open_order_demand_qty: String,
    pub recent_shipment_or_open_order: bool,
    pub days_since_outbound: u32,
    pub days_since_due: u32,
    pub status: String,
    pub outbound_status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PurchaseOrderFact {
    #[serde(flatten)]
    pub identity: FactIdentity,
    pub supplier_id: String,
    pub warehouse_id: String,
    pub brand_id: String,
    pub sku_id: String,
    pub ordered_at: NaiveDate,
    pub unit_price: Money,
    pub quantity: String,
    pub unit: String,
    pub received_amount: Money,
    pub invoiced_amount: Money,
    pub paid_amount: Money,
    pub receipt_rate: String,
    pub payment_rate: String,
    pub days_since_receipt: u32,
    pub in_transit_qty: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InventoryPosition {
    #[serde(flatten)]
    pub identity: FactIdentity,
    pub warehouse_id: String,
    pub sku_id: String,
    pub brand_id: String,
    pub on_hand_qty: String,
    pub available_qty: String,
    pub inventory_age_days: u32,
    pub sales_qty_last_90_days: String,
    pub in_transit_purchase_qty: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReceivablePosition {
    #[serde(flatten)]
    pub identity: FactIdentity,
    pub customer_id: String,
    pub outstanding_amount: Money,
    pub overdue_amount: Money,
    pub overdue_days: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PayablePosition {
    #[serde(flatten)]
    pub identity: FactIdentity,
    pub supplier_id: String,
    pub outstanding_amount: Money,
    pub overdue_amount: Money,
    pub overdue_days: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OrderProfitFact {
    #[serde(flatten)]
    pub identity: FactIdentity,
    pub sales_order_id: String,
    pub customer_id: String,
    pub brand_id: String,
    pub salesperson_id: String,
    pub revenue: Money,
    pub product_cost: Option<Money>,
    pub freight: Option<Money>,
    pub commission: Option<Money>,
    pub discount: Option<Money>,
    pub customer_rebate: Option<Money>,
    pub supplier_rebate: Option<Money>,
    pub platform_fee: Option<Money>,
    pub contribution_profit: Option<Money>,
    pub contribution_margin_rate: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BusinessDataQuality {
    pub code: DataQualityCode,
    pub object_type: String,
    pub object_id: String,
    pub field: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FindingEvidence {
    pub source_system: String,
    pub object_type: String,
    pub object_id: String,
    pub object_version: String,
    pub updated_at: DateTime<Utc>,
    pub field: String,
    pub observed_value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub threshold: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FindingRule {
    pub id: String,
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub threshold: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BusinessAnomaly {
    pub id: Uuid,
    pub r#type: String,
    pub severity: Severity,
    pub confidence: Confidence,
    pub title: String,
    pub summary_code: String,
    pub primary_resource: ResourceRef,
    pub related_resources: Vec<ResourceRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub impact: Option<Money>,
    pub rule: FindingRule,
    pub evidence: Vec<FindingEvidence>,
    pub data_as_of: DateTime<Utc>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AnomalyTotals {
    pub finding_count: usize,
    pub impact_by_currency: Vec<Money>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BusinessAnomalyResult {
    pub schema_version: u8,
    pub status: AnomalyStatus,
    pub run_id: Uuid,
    pub rule_set_version: String,
    pub data_as_of: DateTime<Utc>,
    pub scope_summary: BTreeMap<String, serde_json::Value>,
    pub totals: AnomalyTotals,
    pub findings: Vec<BusinessAnomaly>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pagination: Option<business_query_contracts::Pagination>,
    pub warnings: Vec<String>,
    pub trace_id: Uuid,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AnomalyFilterInput {
    pub anomaly_types: Option<Vec<String>>,
    pub severities: Option<Vec<Severity>>,
    pub legal_entity_ids: Option<Vec<String>>,
    pub customer_ids: Option<Vec<String>>,
    pub supplier_ids: Option<Vec<String>>,
    pub warehouse_ids: Option<Vec<String>>,
    pub sku_ids: Option<Vec<String>>,
    pub brand_ids: Option<Vec<String>>,
    pub active: Option<bool>,
    pub date_range: Option<DateRange>,
    #[serde(flatten)]
    pub page: PageInput,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetAnomalyInput {
    pub finding_id: Uuid,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProfitRiskInput {
    pub legal_entity_ids: Option<Vec<String>>,
    pub customer_ids: Option<Vec<String>>,
    pub brand_ids: Option<Vec<String>>,
    pub salesperson_ids: Option<Vec<String>>,
    pub date_range: Option<DateRange>,
    #[serde(flatten)]
    pub page: PageInput,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReceivableRiskInput {
    pub legal_entity_ids: Option<Vec<String>>,
    pub customer_ids: Option<Vec<String>>,
    pub overdue_days_min: Option<u16>,
    pub date_range: Option<DateRange>,
    #[serde(flatten)]
    pub page: PageInput,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InventoryRiskInput {
    pub legal_entity_ids: Option<Vec<String>>,
    pub warehouse_ids: Option<Vec<String>>,
    pub sku_ids: Option<Vec<String>>,
    pub brand_ids: Option<Vec<String>>,
    pub date_range: Option<DateRange>,
    #[serde(flatten)]
    pub page: PageInput,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PurchaseRiskInput {
    pub legal_entity_ids: Option<Vec<String>>,
    pub supplier_ids: Option<Vec<String>>,
    pub sku_ids: Option<Vec<String>>,
    pub date_range: Option<DateRange>,
    #[serde(flatten)]
    pub page: PageInput,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CrossDomainRiskInput {
    pub legal_entity_ids: Option<Vec<String>>,
    pub customer_ids: Option<Vec<String>>,
    pub warehouse_ids: Option<Vec<String>>,
    pub sku_ids: Option<Vec<String>>,
    pub date_range: Option<DateRange>,
    #[serde(flatten)]
    pub page: PageInput,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProfitChangeInput {
    pub base_period: DateRange,
    pub comparison_period: DateRange,
    pub legal_entity_ids: Option<Vec<String>>,
    pub customer_ids: Option<Vec<String>>,
    pub brand_ids: Option<Vec<String>>,
}

pub trait ValidateAnomalyInput {
    fn validate(&mut self, today: NaiveDate) -> Result<(), ValidationError>;
}

fn validate_page(page: &mut PageInput) -> Result<(), ValidationError> {
    page.limit = Some(page.limit.unwrap_or(20));
    if page.limit.is_some_and(|value| value == 0 || value > 100) {
        return Err(ValidationError::LimitExceeded);
    }
    if page.cursor.as_ref().is_some_and(|value| value.len() > 512) {
        return Err(ValidationError::InvalidCursor);
    }
    Ok(())
}

fn validate_ids(values: &Option<Vec<String>>, max: usize) -> Result<(), ValidationError> {
    if values.as_ref().is_some_and(|items| {
        items.len() > max
            || items.iter().any(|item| {
                item.is_empty()
                    || item.len() > 128
                    || !item.bytes().all(|b| {
                        b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':')
                    })
            })
    }) {
        return Err(ValidationError::UnsafeText);
    }
    Ok(())
}

fn validate_range(range: &Option<DateRange>, today: NaiveDate) -> Result<(), ValidationError> {
    let Some(range) = range else {
        return Ok(());
    };
    let to = range.to.unwrap_or(today);
    let from = range.from.unwrap_or(to);
    if !(0..=366).contains(&to.signed_duration_since(from).num_days()) {
        return Err(ValidationError::InvalidDateRange);
    }
    Ok(())
}

macro_rules! validate_risk {
    ($type:ty, $($field:ident),* $(,)?) => {
        impl ValidateAnomalyInput for $type {
            fn validate(&mut self, today: NaiveDate) -> Result<(), ValidationError> {
                $(validate_ids(&self.$field, 50)?;)*
                validate_range(&self.date_range, today)?;
                validate_page(&mut self.page)
            }
        }
    };
}

validate_risk!(
    ProfitRiskInput,
    legal_entity_ids,
    customer_ids,
    brand_ids,
    salesperson_ids
);
validate_risk!(ReceivableRiskInput, legal_entity_ids, customer_ids);
validate_risk!(
    InventoryRiskInput,
    legal_entity_ids,
    warehouse_ids,
    sku_ids,
    brand_ids
);
validate_risk!(PurchaseRiskInput, legal_entity_ids, supplier_ids, sku_ids);
validate_risk!(
    CrossDomainRiskInput,
    legal_entity_ids,
    customer_ids,
    warehouse_ids,
    sku_ids
);
validate_risk!(
    AnomalyFilterInput,
    legal_entity_ids,
    customer_ids,
    supplier_ids,
    warehouse_ids,
    sku_ids,
    brand_ids
);

impl ValidateAnomalyInput for GetAnomalyInput {
    fn validate(&mut self, _today: NaiveDate) -> Result<(), ValidationError> {
        if self.finding_id.is_nil() {
            Err(ValidationError::UnsafeText)
        } else {
            Ok(())
        }
    }
}

impl ValidateAnomalyInput for ProfitChangeInput {
    fn validate(&mut self, today: NaiveDate) -> Result<(), ValidationError> {
        validate_range(&Some(self.base_period.clone()), today)?;
        validate_range(&Some(self.comparison_period.clone()), today)?;
        validate_ids(&self.legal_entity_ids, 50)?;
        validate_ids(&self.customer_ids, 50)?;
        validate_ids(&self.brand_ids, 50)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn dangerous_generic_fields_are_rejected() {
        for value in [
            json!({"sql":"select *"}),
            json!({"url":"https://evil.invalid"}),
            json!({"join":"customers"}),
            json!({"formula":"1+1"}),
        ] {
            assert!(serde_json::from_value::<CrossDomainRiskInput>(value).is_err());
        }
    }

    #[test]
    fn limits_and_ids_are_bounded() {
        let mut input = InventoryRiskInput {
            page: PageInput {
                limit: Some(101),
                cursor: None,
            },
            ..Default::default()
        };
        assert_eq!(
            input.validate(Utc::now().date_naive()),
            Err(ValidationError::LimitExceeded)
        );
        input.page.limit = Some(20);
        input.sku_ids = Some(vec!["../secret".into()]);
        assert_eq!(
            input.validate(Utc::now().date_naive()),
            Err(ValidationError::UnsafeText)
        );
    }
}
