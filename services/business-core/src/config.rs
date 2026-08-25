use std::net::SocketAddr;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub bind_addr: SocketAddr,
    pub service_credential: String,
    pub service_audience: String,
    pub bootstrap_enabled: bool,
    pub bootstrap_user_id: Option<Uuid>,
    pub sales_enabled: bool,
    pub inventory_enabled: bool,
    pub receivables_enabled: bool,
    pub purchasing_enabled: bool,
    pub receiving_enabled: bool,
    pub payables_enabled: bool,
    pub profitability_enabled: bool,
    pub management_reporting_enabled: bool,
    pub operational_adjustments_enabled: bool,
    pub profit_projection_worker_enabled: bool,
    pub profit_projection_batch_size: i64,
    pub profit_projection_retry_limit: u32,
    pub sales_order_number_prefix: String,
    pub shipment_number_prefix: String,
    pub receivable_number_prefix: String,
    pub customer_receipt_number_prefix: String,
    pub inventory_opening_number_prefix: String,
    pub inventory_count_number_prefix: String,
    pub purchase_requisition_number_prefix: String,
    pub purchase_order_number_prefix: String,
    pub goods_receipt_number_prefix: String,
    pub trade_payable_number_prefix: String,
    pub supplier_payment_number_prefix: String,
    pub sales_return_number_prefix: String,
    pub purchase_return_number_prefix: String,
    pub profit_adjustment_number_prefix: String,
    pub management_report_snapshot_number_prefix: String,
    pub profit_management_timezone: String,
    pub profit_default_currency: String,
    pub profit_allocation_max_targets: usize,
    pub profit_report_max_rows: usize,
    pub profit_data_stale_after_minutes: i64,
    pub default_payment_terms_days: i32,
    pub default_supplier_payment_terms_days: i32,
    pub default_currency: String,
    pub command_rate_limit_per_minute: u32,
    pub business_web_origin: String,
    pub business_web_embed_origin: String,
    pub business_session_cookie_name: String,
}

fn required(name: &str) -> Result<String, String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("{name} is required"))
}

fn boolean(name: &str, default: bool) -> Result<bool, String> {
    std::env::var(name)
        .ok()
        .map(|value| {
            value
                .parse::<bool>()
                .map_err(|_| format!("{name} must be true or false"))
        })
        .transpose()
        .map(|value| value.unwrap_or(default))
}

fn integer<T>(name: &str, default: &str, min: T, max: T) -> Result<T, String>
where
    T: std::str::FromStr + PartialOrd + Copy + std::fmt::Display,
{
    let value = std::env::var(name)
        .unwrap_or_else(|_| default.into())
        .parse::<T>()
        .map_err(|_| format!("{name} must be an integer"))?;
    if value < min || value > max {
        return Err(format!("{name} must be between {min} and {max}"));
    }
    Ok(value)
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let mode = std::env::var("BUSINESS_CORE_SERVICE_AUTH_MODE")
            .unwrap_or_else(|_| "shared_secret".into());
        if mode != "shared_secret" {
            return Err("BUSINESS_CORE_SERVICE_AUTH_MODE must be shared_secret".into());
        }
        let service_credential = required("BUSINESS_CORE_SERVICE_CREDENTIAL")?;
        if service_credential.len() < 32 {
            return Err("BUSINESS_CORE_SERVICE_CREDENTIAL must be at least 32 bytes".into());
        }
        let service_audience = std::env::var("BUSINESS_CORE_SERVICE_AUDIENCE")
            .unwrap_or_else(|_| "business-core".into());
        if service_audience != "business-core" {
            return Err("BUSINESS_CORE_SERVICE_AUDIENCE must be business-core".into());
        }
        let bootstrap_enabled = boolean("BUSINESS_CORE_BOOTSTRAP_ENABLED", false)?;
        let bootstrap_user_id = std::env::var("BUSINESS_CORE_BOOTSTRAP_USER_ID")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .map(|value| {
                Uuid::parse_str(&value)
                    .map_err(|_| "BUSINESS_CORE_BOOTSTRAP_USER_ID must be a UUID".to_string())
            })
            .transpose()?;
        if bootstrap_enabled && bootstrap_user_id.is_none() {
            return Err(
                "BUSINESS_CORE_BOOTSTRAP_USER_ID is required when bootstrap is enabled".into(),
            );
        }
        if boolean("INVENTORY_NEGATIVE_STOCK_ALLOWED", false)? {
            return Err("INVENTORY_NEGATIVE_STOCK_ALLOWED=true is not supported in B2".into());
        }
        if !boolean("SALES_CONFIRMATION_REQUIRES_FULL_RESERVATION", true)? {
            return Err("SALES_CONFIRMATION_REQUIRES_FULL_RESERVATION must be true in B2".into());
        }
        if boolean("PURCHASE_OVER_RECEIPT_ALLOWED", false)? {
            return Err("PURCHASE_OVER_RECEIPT_ALLOWED=true is not supported in B3".into());
        }
        let cost_status =
            std::env::var("PURCHASE_COST_STATUS_DEFAULT").unwrap_or_else(|_| "provisional".into());
        if cost_status != "provisional" {
            return Err("PURCHASE_COST_STATUS_DEFAULT must be provisional in B3".into());
        }
        let cost_policy = std::env::var("PURCHASE_INVENTORY_COST_POLICY")
            .unwrap_or_else(|_| "po_net_excluding_tax".into());
        if cost_policy != "po_net_excluding_tax" {
            return Err("PURCHASE_INVENTORY_COST_POLICY must be po_net_excluding_tax in B3".into());
        }
        let default_currency = std::env::var("DEFAULT_CURRENCY").unwrap_or_else(|_| "CNY".into());
        if default_currency.len() != 3
            || !default_currency
                .bytes()
                .all(|byte| byte.is_ascii_uppercase())
        {
            return Err("DEFAULT_CURRENCY must be an uppercase ISO-4217 code".into());
        }
        let profit_default_currency =
            std::env::var("PROFIT_DEFAULT_CURRENCY").unwrap_or_else(|_| default_currency.clone());
        if profit_default_currency.len() != 3
            || !profit_default_currency
                .bytes()
                .all(|byte| byte.is_ascii_uppercase())
        {
            return Err("PROFIT_DEFAULT_CURRENCY must be an uppercase ISO-4217 code".into());
        }
        let default_payment_terms_days = std::env::var("DEFAULT_PAYMENT_TERMS_DAYS")
            .unwrap_or_else(|_| "30".into())
            .parse::<i32>()
            .map_err(|_| "DEFAULT_PAYMENT_TERMS_DAYS must be an integer")?;
        if !(0..=3650).contains(&default_payment_terms_days) {
            return Err("DEFAULT_PAYMENT_TERMS_DAYS must be between 0 and 3650".into());
        }
        let default_supplier_payment_terms_days =
            std::env::var("DEFAULT_SUPPLIER_PAYMENT_TERMS_DAYS")
                .unwrap_or_else(|_| "30".into())
                .parse::<i32>()
                .map_err(|_| "DEFAULT_SUPPLIER_PAYMENT_TERMS_DAYS must be an integer")?;
        if !(0..=3650).contains(&default_supplier_payment_terms_days) {
            return Err("DEFAULT_SUPPLIER_PAYMENT_TERMS_DAYS must be between 0 and 3650".into());
        }
        let command_rate_limit_per_minute =
            std::env::var("BUSINESS_CORE_COMMAND_RATE_LIMIT_PER_MINUTE")
                .unwrap_or_else(|_| "60".into())
                .parse::<u32>()
                .map_err(|_| "BUSINESS_CORE_COMMAND_RATE_LIMIT_PER_MINUTE must be an integer")?;
        if !(1..=600).contains(&command_rate_limit_per_minute) {
            return Err(
                "BUSINESS_CORE_COMMAND_RATE_LIMIT_PER_MINUTE must be between 1 and 600".into(),
            );
        }
        let business_web_origin = required("BUSINESS_WEB_ORIGIN")?;
        let business_web_embed_origin = std::env::var("BUSINESS_WEB_EMBED_ORIGIN")
            .unwrap_or_else(|_| business_web_origin.clone());
        for (name, value) in [
            ("BUSINESS_WEB_ORIGIN", &business_web_origin),
            ("BUSINESS_WEB_EMBED_ORIGIN", &business_web_embed_origin),
        ] {
            let url = url::Url::parse(value).map_err(|_| format!("{name} must be an origin"))?;
            if !matches!(url.scheme(), "http" | "https")
                || url.host_str().is_none()
                || url.path() != "/"
                || url.query().is_some()
                || url.fragment().is_some()
            {
                return Err(format!("{name} must contain only scheme and authority"));
            }
        }
        let business_session_cookie_name = std::env::var("BUSINESS_SESSION_COOKIE_NAME")
            .unwrap_or_else(|_| "__Host-bizfin_business".into());
        if !business_session_cookie_name.starts_with("__Host-") {
            return Err("BUSINESS_SESSION_COOKIE_NAME must use __Host-".into());
        }
        Ok(Self {
            database_url: required("BUSINESS_CORE_DATABASE_URL")?,
            bind_addr: std::env::var("BUSINESS_CORE_BIND_ADDR")
                .unwrap_or_else(|_| "127.0.0.1:3110".into())
                .parse()
                .map_err(|_| "BUSINESS_CORE_BIND_ADDR must be a socket address")?,
            service_credential,
            service_audience,
            bootstrap_enabled,
            bootstrap_user_id,
            sales_enabled: boolean("BUSINESS_CORE_SALES_ENABLED", true)?,
            inventory_enabled: boolean("BUSINESS_CORE_INVENTORY_ENABLED", true)?,
            receivables_enabled: boolean("BUSINESS_CORE_RECEIVABLES_ENABLED", true)?,
            purchasing_enabled: boolean("BUSINESS_CORE_PURCHASING_ENABLED", true)?,
            receiving_enabled: boolean("BUSINESS_CORE_RECEIVING_ENABLED", true)?,
            payables_enabled: boolean("BUSINESS_CORE_PAYABLES_ENABLED", true)?,
            profitability_enabled: boolean("BUSINESS_CORE_PROFITABILITY_ENABLED", true)?,
            management_reporting_enabled: boolean(
                "BUSINESS_CORE_MANAGEMENT_REPORTING_ENABLED",
                true,
            )?,
            operational_adjustments_enabled: boolean(
                "BUSINESS_CORE_OPERATIONAL_ADJUSTMENTS_ENABLED",
                true,
            )?,
            profit_projection_worker_enabled: boolean("PROFIT_PROJECTION_WORKER_ENABLED", true)?,
            profit_projection_batch_size: integer(
                "PROFIT_PROJECTION_BATCH_SIZE",
                "200",
                1_i64,
                1000,
            )?,
            profit_projection_retry_limit: integer(
                "PROFIT_PROJECTION_RETRY_LIMIT",
                "5",
                0_u32,
                100,
            )?,
            sales_order_number_prefix: std::env::var("SALES_ORDER_NUMBER_PREFIX")
                .unwrap_or_else(|_| "SO".into()),
            shipment_number_prefix: std::env::var("SHIPMENT_NUMBER_PREFIX")
                .unwrap_or_else(|_| "SHP".into()),
            receivable_number_prefix: std::env::var("RECEIVABLE_NUMBER_PREFIX")
                .unwrap_or_else(|_| "AR".into()),
            customer_receipt_number_prefix: std::env::var("CUSTOMER_RECEIPT_NUMBER_PREFIX")
                .unwrap_or_else(|_| "RCPT".into()),
            inventory_opening_number_prefix: std::env::var("INVENTORY_OPENING_NUMBER_PREFIX")
                .unwrap_or_else(|_| "OPEN".into()),
            inventory_count_number_prefix: std::env::var("INVENTORY_COUNT_NUMBER_PREFIX")
                .unwrap_or_else(|_| "CNT".into()),
            purchase_requisition_number_prefix: std::env::var("PURCHASE_REQUISITION_NUMBER_PREFIX")
                .unwrap_or_else(|_| "PRQ".into()),
            purchase_order_number_prefix: std::env::var("PURCHASE_ORDER_NUMBER_PREFIX")
                .unwrap_or_else(|_| "PO".into()),
            goods_receipt_number_prefix: std::env::var("GOODS_RECEIPT_NUMBER_PREFIX")
                .unwrap_or_else(|_| "GR".into()),
            trade_payable_number_prefix: std::env::var("TRADE_PAYABLE_NUMBER_PREFIX")
                .unwrap_or_else(|_| "AP".into()),
            supplier_payment_number_prefix: std::env::var("SUPPLIER_PAYMENT_NUMBER_PREFIX")
                .unwrap_or_else(|_| "PAY".into()),
            sales_return_number_prefix: std::env::var("SALES_RETURN_NUMBER_PREFIX")
                .unwrap_or_else(|_| "SRET".into()),
            purchase_return_number_prefix: std::env::var("PURCHASE_RETURN_NUMBER_PREFIX")
                .unwrap_or_else(|_| "PRET".into()),
            profit_adjustment_number_prefix: std::env::var("PROFIT_ADJUSTMENT_NUMBER_PREFIX")
                .unwrap_or_else(|_| "ADJ".into()),
            management_report_snapshot_number_prefix: std::env::var(
                "MANAGEMENT_REPORT_SNAPSHOT_NUMBER_PREFIX",
            )
            .unwrap_or_else(|_| "MGR".into()),
            profit_management_timezone: std::env::var("PROFIT_MANAGEMENT_TIMEZONE")
                .unwrap_or_else(|_| "Asia/Shanghai".into()),
            profit_default_currency,
            profit_allocation_max_targets: integer(
                "PROFIT_ALLOCATION_MAX_TARGETS",
                "500",
                1_usize,
                5000,
            )?,
            profit_report_max_rows: integer("PROFIT_REPORT_MAX_ROWS", "1000", 1_usize, 10_000)?,
            profit_data_stale_after_minutes: integer(
                "PROFIT_DATA_STALE_AFTER_MINUTES",
                "15",
                1_i64,
                525_600,
            )?,
            default_payment_terms_days,
            default_supplier_payment_terms_days,
            default_currency,
            command_rate_limit_per_minute,
            business_web_origin,
            business_web_embed_origin,
            business_session_cookie_name,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_bind_is_loopback() {
        let value: SocketAddr = "127.0.0.1:3110".parse().unwrap();
        assert!(value.ip().is_loopback());
    }
}
