#![forbid(unsafe_code)]

use chrono::{DateTime, NaiveDate, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use uuid::Uuid;

pub const DEFAULT_LIMIT: u32 = 20;
pub const MAX_LIMIT: u32 = 100;
pub const DEFAULT_PERIOD_DAYS: i64 = 90;
pub const MAX_PERIOD_DAYS: i64 = 366;
pub const MAX_TEXT_CHARS: usize = 128;
pub const MAX_CURSOR_CHARS: usize = 512;

pub const SALES_ORDER_READ: &str = "sales_order:read";
pub const PURCHASE_ORDER_READ: &str = "purchase_order:read";
pub const INVENTORY_READ: &str = "inventory:read";
pub const RECEIVABLE_READ: &str = "receivable:read";
pub const PAYABLE_READ: &str = "payable:read";
pub const ORDER_PROFIT_READ: &str = "order_profit:read";

pub const READ_SCOPES: [&str; 6] = [
    SALES_ORDER_READ,
    PURCHASE_ORDER_READ,
    INVENTORY_READ,
    RECEIVABLE_READ,
    PAYABLE_READ,
    ORDER_PROFIT_READ,
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BusinessToolStatus {
    Ok,
    Partial,
    MissingContext,
    NotFoundOrForbidden,
    InvalidFilter,
    UpstreamUnavailable,
    RateLimited,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Money {
    pub amount: String,
    pub currency: String,
}

impl Money {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if !valid_decimal(&self.amount) {
            return Err(ValidationError::InvalidDecimal);
        }
        if self.currency.len() != 3 || !self.currency.bytes().all(|b| b.is_ascii_uppercase()) {
            return Err(ValidationError::InvalidCurrency);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScopeSummary {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub legal_entity_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Pagination {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    pub has_more: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResourceRef {
    pub r#type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub title: String,
    pub biz_uri: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Evidence {
    pub source_system: String,
    pub object_type: String,
    pub object_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BusinessToolResult<T> {
    pub schema_version: u8,
    pub status: BusinessToolStatus,
    pub as_of: DateTime<Utc>,
    pub scope_summary: ScopeSummary,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub summary: BTreeMap<String, Value>,
    pub items: Vec<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pagination: Option<Pagination>,
    pub resource_refs: Vec<ResourceRef>,
    pub evidence: Vec<Evidence>,
    pub warnings: Vec<String>,
    pub trace_id: Uuid,
}

impl<T> BusinessToolResult<T> {
    pub fn empty(status: BusinessToolStatus, trace_id: Uuid) -> Self {
        Self {
            schema_version: 1,
            status,
            as_of: Utc::now(),
            scope_summary: ScopeSummary::default(),
            summary: BTreeMap::new(),
            items: Vec::new(),
            pagination: None,
            resource_refs: Vec::new(),
            evidence: Vec::new(),
            warnings: Vec::new(),
            trace_id,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DateRange {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<NaiveDate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<NaiveDate>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PageInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetSalesOrderInput {
    pub order_id: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SearchSalesOrdersInput {
    pub order_number: Option<String>,
    pub status: Option<SalesOrderStatus>,
    pub customer_ids: Option<Vec<String>>,
    pub legal_entity_ids: Option<Vec<String>>,
    pub salesperson_ids: Option<Vec<String>>,
    pub brand_ids: Option<Vec<String>>,
    pub date_range: Option<DateRange>,
    #[serde(flatten)]
    pub page: PageInput,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetPurchaseOrderInput {
    pub order_id: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SearchPurchaseOrdersInput {
    pub order_number: Option<String>,
    pub status: Option<PurchaseOrderStatus>,
    pub supplier_ids: Option<Vec<String>>,
    pub legal_entity_ids: Option<Vec<String>>,
    pub date_range: Option<DateRange>,
    #[serde(flatten)]
    pub page: PageInput,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InventoryBalanceInput {
    pub product_ids: Option<Vec<String>>,
    pub warehouse_ids: Option<Vec<String>>,
    pub legal_entity_ids: Option<Vec<String>>,
    #[serde(flatten)]
    pub page: PageInput,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReceivablesInput {
    pub customer_ids: Option<Vec<String>>,
    pub legal_entity_ids: Option<Vec<String>>,
    pub overdue_days_min: Option<u16>,
    pub due_date_range: Option<DateRange>,
    #[serde(flatten)]
    pub page: PageInput,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PayablesInput {
    pub supplier_ids: Option<Vec<String>>,
    pub legal_entity_ids: Option<Vec<String>>,
    pub overdue_days_min: Option<u16>,
    pub due_date_range: Option<DateRange>,
    #[serde(flatten)]
    pub page: PageInput,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OrderProfitInput {
    pub order_id: Option<String>,
    pub management_period: Option<String>,
    pub legal_entity_ids: Option<Vec<String>>,
    pub date_range: Option<DateRange>,
    pub loss_only: Option<bool>,
    #[serde(flatten)]
    pub page: PageInput,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProfitabilityInput {
    pub management_period: String,
    pub currency: String,
    pub dimension_one: String,
    pub dimension_two: Option<String>,
    pub legal_entity_ids: Option<Vec<String>>,
    #[serde(flatten)]
    pub page: PageInput,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagementProfitReportInput {
    pub management_period: String,
    pub currency: String,
    pub legal_entity_ids: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperatingDashboardInput {
    pub management_period: String,
    pub currency: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DataQualityInput {}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagementReportSnapshotInput {
    pub snapshot_id: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProfitEvidenceInput {
    pub order_id: String,
    #[serde(flatten)]
    pub page: PageInput,
}

pub trait ValidateInput {
    fn validate_and_normalize(&mut self, today: NaiveDate) -> Result<(), ValidationError>;
}

macro_rules! validate_id_input {
    ($type:ty, $field:ident) => {
        impl ValidateInput for $type {
            fn validate_and_normalize(&mut self, _today: NaiveDate) -> Result<(), ValidationError> {
                normalize_text(&mut self.$field, MAX_TEXT_CHARS)?;
                Ok(())
            }
        }
    };
}

validate_id_input!(GetSalesOrderInput, order_id);
validate_id_input!(GetPurchaseOrderInput, order_id);

impl ValidateInput for SearchSalesOrdersInput {
    fn validate_and_normalize(&mut self, today: NaiveDate) -> Result<(), ValidationError> {
        normalize_optional(&mut self.order_number)?;
        validate_ids(&mut self.customer_ids, 50)?;
        validate_ids(&mut self.legal_entity_ids, 50)?;
        validate_ids(&mut self.salesperson_ids, 50)?;
        validate_ids(&mut self.brand_ids, 50)?;
        normalize_range(&mut self.date_range, today)?;
        normalize_page(&mut self.page)
    }
}

impl ValidateInput for SearchPurchaseOrdersInput {
    fn validate_and_normalize(&mut self, today: NaiveDate) -> Result<(), ValidationError> {
        normalize_optional(&mut self.order_number)?;
        validate_ids(&mut self.supplier_ids, 50)?;
        validate_ids(&mut self.legal_entity_ids, 50)?;
        normalize_range(&mut self.date_range, today)?;
        normalize_page(&mut self.page)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SalesOrderStatus {
    Draft,
    Approved,
    Processing,
    PartiallyShipped,
    Completed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PurchaseOrderStatus {
    Draft,
    Approved,
    Processing,
    PartiallyReceived,
    Completed,
    Cancelled,
}

impl ValidateInput for InventoryBalanceInput {
    fn validate_and_normalize(&mut self, _today: NaiveDate) -> Result<(), ValidationError> {
        validate_ids(&mut self.product_ids, 50)?;
        validate_ids(&mut self.warehouse_ids, 20)?;
        validate_ids(&mut self.legal_entity_ids, 50)?;
        normalize_page(&mut self.page)
    }
}

impl ValidateInput for ReceivablesInput {
    fn validate_and_normalize(&mut self, today: NaiveDate) -> Result<(), ValidationError> {
        validate_ids(&mut self.customer_ids, 50)?;
        validate_ids(&mut self.legal_entity_ids, 50)?;
        normalize_range(&mut self.due_date_range, today)?;
        normalize_page(&mut self.page)
    }
}

impl ValidateInput for PayablesInput {
    fn validate_and_normalize(&mut self, today: NaiveDate) -> Result<(), ValidationError> {
        validate_ids(&mut self.supplier_ids, 50)?;
        validate_ids(&mut self.legal_entity_ids, 50)?;
        normalize_range(&mut self.due_date_range, today)?;
        normalize_page(&mut self.page)
    }
}

impl ValidateInput for OrderProfitInput {
    fn validate_and_normalize(&mut self, today: NaiveDate) -> Result<(), ValidationError> {
        normalize_optional(&mut self.order_id)?;
        validate_ids(&mut self.legal_entity_ids, 50)?;
        if self.order_id.is_none() && self.date_range.is_none() && self.management_period.is_none()
        {
            return Err(ValidationError::MissingContext);
        }
        if let Some(period) = &self.management_period {
            validate_profit_context(period, "CNY")?;
        }
        if self.date_range.is_some() {
            normalize_range(&mut self.date_range, today)?;
        }
        normalize_page(&mut self.page)
    }
}

fn validate_profit_context(period: &str, currency: &str) -> Result<(), ValidationError> {
    if period.len() != 7
        || period.as_bytes().get(4) != Some(&b'-')
        || !period[..4].bytes().all(|byte| byte.is_ascii_digit())
        || !matches!(
            &period[5..],
            "01" | "02" | "03" | "04" | "05" | "06" | "07" | "08" | "09" | "10" | "11" | "12"
        )
    {
        return Err(ValidationError::InvalidDateRange);
    }
    if currency.len() != 3 || !currency.bytes().all(|byte| byte.is_ascii_uppercase()) {
        return Err(ValidationError::InvalidCurrency);
    }
    Ok(())
}

impl ValidateInput for ProfitabilityInput {
    fn validate_and_normalize(&mut self, _today: NaiveDate) -> Result<(), ValidationError> {
        validate_profit_context(&self.management_period, &self.currency)?;
        let valid = |value: &str| {
            matches!(
                value,
                "group"
                    | "legal_entity"
                    | "customer"
                    | "sku"
                    | "product_category"
                    | "brand"
                    | "salesperson"
                    | "business_unit"
                    | "department"
                    | "warehouse"
                    | "sales_order"
            )
        };
        if !valid(&self.dimension_one)
            || self
                .dimension_two
                .as_deref()
                .is_some_and(|value| !valid(value) || value == self.dimension_one)
        {
            return Err(ValidationError::UnsafeText);
        }
        validate_ids(&mut self.legal_entity_ids, 50)?;
        normalize_page(&mut self.page)
    }
}

impl ValidateInput for ManagementProfitReportInput {
    fn validate_and_normalize(&mut self, _today: NaiveDate) -> Result<(), ValidationError> {
        validate_profit_context(&self.management_period, &self.currency)?;
        validate_ids(&mut self.legal_entity_ids, 50)
    }
}

impl ValidateInput for OperatingDashboardInput {
    fn validate_and_normalize(&mut self, _today: NaiveDate) -> Result<(), ValidationError> {
        validate_profit_context(&self.management_period, &self.currency)
    }
}

impl ValidateInput for DataQualityInput {
    fn validate_and_normalize(&mut self, _today: NaiveDate) -> Result<(), ValidationError> {
        Ok(())
    }
}

impl ValidateInput for ManagementReportSnapshotInput {
    fn validate_and_normalize(&mut self, _today: NaiveDate) -> Result<(), ValidationError> {
        normalize_text(&mut self.snapshot_id, MAX_TEXT_CHARS)
    }
}

impl ValidateInput for ProfitEvidenceInput {
    fn validate_and_normalize(&mut self, _today: NaiveDate) -> Result<(), ValidationError> {
        normalize_text(&mut self.order_id, MAX_TEXT_CHARS)?;
        normalize_page(&mut self.page)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ValidationError {
    #[error("INVALID_FILTER: text is empty, too long, or contains unsafe characters")]
    UnsafeText,
    #[error("INVALID_FILTER: too many values")]
    TooManyValues,
    #[error("INVALID_FILTER: limit exceeds the configured maximum")]
    LimitExceeded,
    #[error("INVALID_FILTER: cursor is invalid")]
    InvalidCursor,
    #[error("INVALID_FILTER: date range exceeds 366 days or is reversed")]
    InvalidDateRange,
    #[error("MISSING_CONTEXT: an order id or date range is required")]
    MissingContext,
    #[error("INVALID_FILTER: amount must be a decimal string")]
    InvalidDecimal,
    #[error("INVALID_FILTER: currency must be ISO-style uppercase code")]
    InvalidCurrency,
}

pub fn valid_biz_uri(value: &str) -> bool {
    let Some(rest) = value.strip_prefix("biz://") else {
        return false;
    };
    let mut segments = rest.split('/');
    let Some(kind) = segments.next() else {
        return false;
    };
    if matches!(
        kind,
        "operations-dashboard" | "data-quality" | "operating-incidents" | "operating-trends"
    ) {
        return segments.next().is_none();
    }
    let Some(id) = segments.next() else {
        return false;
    };
    let suffix = segments.next();
    if kind == "profitability" {
        let Some(entity_id) = suffix else {
            return false;
        };
        let Some(period) = segments.next() else {
            return false;
        };
        return matches!(id, "customer" | "sku" | "brand" | "salesperson")
            && valid_identifier(entity_id)
            && period.len() == 7
            && period.as_bytes().get(4) == Some(&b'-')
            && period[..4].bytes().all(|byte| byte.is_ascii_digit())
            && matches!(
                &period[5..],
                "01" | "02" | "03" | "04" | "05" | "06" | "07" | "08" | "09" | "10" | "11" | "12"
            )
            && segments.next().is_none();
    }
    let shape_allowed = match (kind, suffix) {
        ("customer", Some("receivables")) | ("supplier", Some("payables")) => true,
        (_, None) => matches!(
            kind,
            "agent-query"
                | "sales-order"
                | "shipment"
                | "customer-receipt"
                | "purchase-order"
                | "order-profit"
                | "profit-adjustment"
                | "management-report"
                | "inventory"
                | "customer"
                | "supplier"
        ),
        _ => false,
    };
    segments.next().is_none() && shape_allowed && valid_identifier(id)
}

fn normalize_page(page: &mut PageInput) -> Result<(), ValidationError> {
    page.limit = Some(page.limit.unwrap_or(DEFAULT_LIMIT));
    if page
        .limit
        .is_some_and(|limit| limit == 0 || limit > MAX_LIMIT)
    {
        return Err(ValidationError::LimitExceeded);
    }
    if let Some(cursor) = page.cursor.as_mut() {
        let trimmed = cursor.trim();
        if trimmed.is_empty()
            || trimmed.len() > MAX_CURSOR_CHARS
            || !trimmed
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'=' | b'.'))
        {
            return Err(ValidationError::InvalidCursor);
        }
        *cursor = trimmed.to_string();
    }
    Ok(())
}

fn normalize_range(range: &mut Option<DateRange>, today: NaiveDate) -> Result<(), ValidationError> {
    let value = range.get_or_insert_with(|| DateRange {
        from: today.checked_sub_days(chrono::Days::new(DEFAULT_PERIOD_DAYS as u64)),
        to: Some(today),
    });
    let to = value.to.unwrap_or(today);
    let from = value
        .from
        .or_else(|| to.checked_sub_days(chrono::Days::new(DEFAULT_PERIOD_DAYS as u64)))
        .ok_or(ValidationError::InvalidDateRange)?;
    let days = to.signed_duration_since(from).num_days();
    if !(0..=MAX_PERIOD_DAYS).contains(&days) {
        return Err(ValidationError::InvalidDateRange);
    }
    value.from = Some(from);
    value.to = Some(to);
    Ok(())
}

fn validate_ids(values: &mut Option<Vec<String>>, max: usize) -> Result<(), ValidationError> {
    let Some(values) = values else {
        return Ok(());
    };
    if values.len() > max {
        return Err(ValidationError::TooManyValues);
    }
    for value in values {
        normalize_text(value, MAX_TEXT_CHARS)?;
    }
    Ok(())
}

fn normalize_optional(value: &mut Option<String>) -> Result<(), ValidationError> {
    if let Some(value) = value {
        normalize_text(value, MAX_TEXT_CHARS)?;
    }
    Ok(())
}

fn normalize_text(value: &mut String, max: usize) -> Result<(), ValidationError> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed.chars().count() > max
        || trimmed.chars().any(char::is_control)
        || !trimmed
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | ':' | '@' | ' '))
    {
        return Err(ValidationError::UnsafeText);
    }
    *value = trimmed.to_string();
    Ok(())
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_TEXT_CHARS
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':' | b'@'))
}

fn valid_decimal(value: &str) -> bool {
    let value = value.strip_prefix('-').unwrap_or(value);
    let mut parts = value.split('.');
    let whole = parts.next().unwrap_or_default();
    let fraction = parts.next();
    !whole.is_empty()
        && whole.bytes().all(|b| b.is_ascii_digit())
        && fraction.is_none_or(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
        && parts.next().is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 8, 20).expect("fixed date")
    }

    #[test]
    fn unknown_sql_and_url_fields_are_rejected() {
        let sql =
            serde_json::from_value::<SearchSalesOrdersInput>(serde_json::json!({"sql":"select *"}));
        let url = serde_json::from_value::<SearchSalesOrdersInput>(
            serde_json::json!({"rawUrl":"https://evil.invalid"}),
        );
        assert!(sql.is_err());
        assert!(url.is_err());
    }

    #[test]
    fn illegal_status_enum_is_rejected() {
        let input = serde_json::from_value::<SearchSalesOrdersInput>(
            serde_json::json!({"status":"drop_table"}),
        );
        assert!(input.is_err());
    }

    #[test]
    fn default_period_and_limit_are_bounded() {
        let mut input = SearchSalesOrdersInput::default();
        input.validate_and_normalize(today()).expect("valid");
        assert_eq!(input.page.limit, Some(20));
        let range = input.date_range.expect("default range");
        assert_eq!(range.to, Some(today()));
        assert_eq!(range.from, NaiveDate::from_ymd_opt(2026, 5, 22));
    }

    #[test]
    fn excessive_range_and_limit_are_rejected() {
        let mut input = SearchSalesOrdersInput {
            date_range: Some(DateRange {
                from: NaiveDate::from_ymd_opt(2025, 1, 1),
                to: Some(today()),
            }),
            page: PageInput {
                limit: Some(101),
                cursor: None,
            },
            ..Default::default()
        };
        assert_eq!(
            input.validate_and_normalize(today()),
            Err(ValidationError::InvalidDateRange)
        );
    }

    #[test]
    fn money_is_decimal_string_not_float() {
        assert!(Money {
            amount: "125430.50".into(),
            currency: "CNY".into()
        }
        .validate()
        .is_ok());
        assert_eq!(
            Money {
                amount: "12.3.4".into(),
                currency: "CNY".into()
            }
            .validate(),
            Err(ValidationError::InvalidDecimal)
        );
        assert!(serde_json::from_value::<Money>(
            serde_json::json!({"amount":12.3,"currency":"CNY"})
        )
        .is_err());
    }

    #[test]
    fn only_allowlisted_resource_links_pass() {
        assert!(valid_biz_uri(
            "biz://agent-query/fc84644d-43ac-462f-8a30-456e04a2e9a3"
        ));
        assert!(valid_biz_uri("biz://sales-order/SO-001"));
        assert!(valid_biz_uri("biz://order-profit/SO-001"));
        assert!(valid_biz_uri(
            "biz://profitability/customer/CUST-001/2026-08"
        ));
        assert!(valid_biz_uri("biz://management-report/MGR-001"));
        assert!(valid_biz_uri("biz://profit-adjustment/ADJ-001"));
        assert!(valid_biz_uri("biz://shipment/SHP-001"));
        assert!(valid_biz_uri("biz://customer-receipt/RCPT-001"));
        assert!(valid_biz_uri("biz://customer/CUST-001"));
        assert!(valid_biz_uri("biz://customer/CUST-001/receivables"));
        assert!(valid_biz_uri("biz://operations-dashboard"));
        assert!(valid_biz_uri("biz://data-quality"));
        assert!(valid_biz_uri("biz://operating-incidents"));
        assert!(valid_biz_uri("biz://operating-trends"));
        assert!(!valid_biz_uri("biz://data-quality/extra"));
        assert!(!valid_biz_uri("https://evil.invalid/SO-001"));
        assert!(!valid_biz_uri("biz://sales-order/../../admin"));
    }
}
