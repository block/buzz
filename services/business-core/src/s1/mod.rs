//! Business Core S1 stability and operating-report read models.

pub mod api;
mod incidents;
mod trends;

pub use incidents::IncidentCommand;
pub use trends::{CreateSubscription, GenerateOperatingSnapshot, SubscriptionCommand};

use crate::{
    b2::{common::authorize, DomainError},
    store::PgStore,
};
use chrono::{Months, NaiveDate, Utc};
use rust_decimal::Decimal;
use serde::Serialize;
use serde_json::{json, Value};
use sqlx::Row;
use std::time::Instant;
use uuid::Uuid;

const DASHBOARD_TARGET_MS: f64 = 500.0;
const DATA_QUALITY_TARGET_MS: f64 = 2_000.0;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct QueryStage {
    name: &'static str,
    duration_ms: f64,
}

#[derive(Clone)]
pub struct OperationsService {
    store: PgStore,
    projection_worker_enabled: bool,
    projection_stale_after_minutes: i64,
}

impl OperationsService {
    pub fn new(
        store: PgStore,
        projection_worker_enabled: bool,
        projection_stale_after_minutes: i64,
    ) -> Self {
        Self {
            store,
            projection_worker_enabled,
            projection_stale_after_minutes,
        }
    }

    pub async fn data_quality(&self, actor: Uuid) -> Result<Value, DomainError> {
        let overall_started = Instant::now();
        let mut stages = Vec::with_capacity(8);
        let stage_started = Instant::now();
        let auth = authorize(
            &self.store,
            actor,
            "management_report:read",
            None,
            None,
            None,
            None,
            None,
        )
        .await?;
        record_stage(&mut stages, "authorization", stage_started);
        let legal_entities = auth.scopes.legal_entity_ids.into_iter().collect::<Vec<_>>();
        let warehouses = auth.scopes.warehouse_ids.into_iter().collect::<Vec<_>>();
        let customers = auth.scopes.customer_ids.into_iter().collect::<Vec<_>>();
        let suppliers = auth.scopes.supplier_ids.into_iter().collect::<Vec<_>>();
        let brands = auth.scopes.brand_ids.into_iter().collect::<Vec<_>>();
        let business_units = auth
            .scopes
            .business_unit_ids
            .into_iter()
            .collect::<Vec<_>>();

        let stage_started = Instant::now();
        let inventory: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM inventory_balance_reconciliation WHERE legal_entity_id=ANY($1) AND warehouse_id=ANY($2) AND (on_hand_difference<>0 OR reserved_difference<>0 OR value_difference<>0)",
        )
        .bind(&legal_entities)
        .bind(&warehouses)
        .fetch_one(self.store.pool())
        .await?;
        record_stage(&mut stages, "inventoryReconciliation", stage_started);
        let stage_started = Instant::now();
        let receivables: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM receivable_balance_reconciliation r JOIN trade_receivables t ON t.id=r.receivable_id WHERE t.legal_entity_id=ANY($1) AND t.customer_id=ANY($2) AND (r.settled_difference<>0 OR r.open_difference<>0)",
        )
        .bind(&legal_entities)
        .bind(&customers)
        .fetch_one(self.store.pool())
        .await?;
        record_stage(&mut stages, "receivablesReconciliation", stage_started);
        let stage_started = Instant::now();
        let payables: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM payable_balance_reconciliation r JOIN trade_payables p ON p.id=r.payable_id WHERE p.legal_entity_id=ANY($1) AND p.supplier_id=ANY($2) AND (r.settled_difference<>0 OR r.open_difference<>0)",
        )
        .bind(&legal_entities)
        .bind(&suppliers)
        .fetch_one(self.store.pool())
        .await?;
        record_stage(&mut stages, "payablesReconciliation", stage_started);
        let stage_started = Instant::now();
        let profit: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM profit_projection_reconciliation r JOIN shipments s ON s.id=r.shipment_id JOIN sales_orders o ON o.id=r.sales_order_id WHERE s.legal_entity_id=ANY($1) AND s.customer_id=ANY($2) AND s.warehouse_id=ANY($3) AND (o.brand_id IS NULL OR o.brand_id=ANY($4)) AND o.business_unit_id=ANY($5) AND (r.revenue_difference<>0 OR r.cost_difference<>0)",
        )
        .bind(&legal_entities)
        .bind(&customers)
        .bind(&warehouses)
        .bind(&brands)
        .bind(&business_units)
        .fetch_one(self.store.pool())
        .await?;
        record_stage(&mut stages, "profitReconciliation", stage_started);
        let stage_started = Instant::now();
        let pending_failures: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM profit_projection_failures f JOIN shipments s ON s.id=f.aggregate_id JOIN sales_orders o ON o.id=s.sales_order_id WHERE f.status='pending' AND s.legal_entity_id=ANY($1) AND s.customer_id=ANY($2) AND s.warehouse_id=ANY($3) AND (o.brand_id IS NULL OR o.brand_id=ANY($4)) AND o.business_unit_id=ANY($5)",
        )
        .bind(&legal_entities)
        .bind(&customers)
        .bind(&warehouses)
        .bind(&brands)
        .bind(&business_units)
        .fetch_one(self.store.pool())
        .await?;
        record_stage(&mut stages, "projectionFailures", stage_started);
        let stage_started = Instant::now();
        let projection = sqlx::query(
            "SELECT o.last_outbox_created_at,o.last_fact_sequence,o.updated_at,COALESCE((SELECT count(*) FROM business_core_outbox e WHERE e.topic IN ('shipment_confirmed','shipment_reversed','sales_return_confirmed') AND (o.last_outbox_created_at IS NULL OR (e.created_at,e.id)>(o.last_outbox_created_at,o.last_outbox_event_id))),0) pending_events FROM profit_projection_offsets o WHERE o.consumer_name='profit_projection_v1'",
        )
        .fetch_optional(self.store.pool())
        .await?;
        record_stage(&mut stages, "projectionWatermark", stage_started);
        let pending_events = projection
            .as_ref()
            .map_or(0, |row| row.get::<i64, _>("pending_events"));
        let updated_at = projection
            .as_ref()
            .map(|row| row.get::<chrono::DateTime<Utc>, _>("updated_at"));
        let now = Utc::now();
        let freshness_age_seconds =
            updated_at.map(|value| now.signed_duration_since(value).num_seconds().max(0));
        let stale_after_seconds = self.projection_stale_after_minutes.saturating_mul(60);
        let projection_fresh = self.projection_worker_enabled
            && freshness_age_seconds.is_some_and(|value| value <= stale_after_seconds);
        let difference_count = inventory + receivables + payables + profit;
        let status = if difference_count > 0 || pending_failures > 0 {
            "blocked"
        } else if pending_events > 0 || !projection_fresh {
            "partial"
        } else {
            "complete"
        };
        let duration_ms = elapsed_ms(overall_started);
        let mut alerts = Vec::new();
        for (domain, count, evidence_path) in [
            ("inventory", inventory, "/api/v1/reconciliation/inventory"),
            (
                "receivables",
                receivables,
                "/api/v1/reconciliation/receivables",
            ),
            ("payables", payables, "/api/v1/reconciliation/payables"),
            ("profitFacts", profit, "/api/v1/reconciliation/profit-facts"),
        ] {
            if count > 0 {
                alerts.push(alert(
                    "RECONCILIATION_DIFFERENCE",
                    "critical",
                    &format!("{domain} 存在 {count} 条对账差异"),
                    evidence_path,
                ));
            }
        }
        append_projection_alerts(
            &mut alerts,
            self.projection_worker_enabled,
            projection_fresh,
            pending_events,
            pending_failures,
        );
        if duration_ms > DATA_QUALITY_TARGET_MS {
            alerts.push(alert(
                "SLOW_REPORT_READ",
                "warning",
                "数据质量聚合超过 2 秒目标",
                "/api/v1/operations/data-quality",
            ));
        }
        Ok(json!({
            "status": status,
            "differenceCount": difference_count,
            "checks": [
                check("inventory", inventory, "/api/v1/reconciliation/inventory"),
                check("receivables", receivables, "/api/v1/reconciliation/receivables"),
                check("payables", payables, "/api/v1/reconciliation/payables"),
                check("profitFacts", profit, "/api/v1/reconciliation/profit-facts")
            ],
            "projection": {
                "workerEnabled": self.projection_worker_enabled,
                "fresh": projection_fresh,
                "pendingEvents": pending_events,
                "pendingFailures": pending_failures,
                "freshnessAgeSeconds": freshness_age_seconds,
                "staleAfterSeconds": stale_after_seconds,
                "lastOutboxCreatedAt": projection.as_ref().and_then(|row| row.get::<Option<chrono::DateTime<Utc>>,_>("last_outbox_created_at")),
                "lastFactSequence": projection.as_ref().and_then(|row| row.get::<Option<i64>,_>("last_fact_sequence")),
                "updatedAt": updated_at
            },
            "alerts": alerts,
            "diagnostics": read_diagnostics(stages, duration_ms, DATA_QUALITY_TARGET_MS),
            "scopeVersion": auth.scope_version,
            "effectiveScopeHash": auth.effective_scope_hash,
            "dataAsOf": Utc::now(),
            "repairPolicy": "inspect_evidence_then_run_scoped_reconciliation_or_idempotent_projection_replay",
            "boundary": "business_operations_only_not_financial_accounting"
        }))
    }

    pub async fn dashboard(
        &self,
        actor: Uuid,
        management_period: &str,
        currency: &str,
    ) -> Result<Value, DomainError> {
        let overall_started = Instant::now();
        let mut stages = Vec::with_capacity(8);
        validate_period(management_period)?;
        crate::b2::common::validate_currency(currency)?;
        let (period_start, period_end) = period_bounds(management_period)?;
        let stage_started = Instant::now();
        let auth = authorize(
            &self.store,
            actor,
            "management_report:read",
            None,
            None,
            None,
            None,
            None,
        )
        .await?;
        record_stage(&mut stages, "authorization", stage_started);
        let le = auth.scopes.legal_entity_ids.into_iter().collect::<Vec<_>>();
        let wh = auth.scopes.warehouse_ids.into_iter().collect::<Vec<_>>();
        let customer = auth.scopes.customer_ids.into_iter().collect::<Vec<_>>();
        let supplier = auth.scopes.supplier_ids.into_iter().collect::<Vec<_>>();
        let brand = auth.scopes.brand_ids.into_iter().collect::<Vec<_>>();
        let bu = auth
            .scopes
            .business_unit_ids
            .into_iter()
            .collect::<Vec<_>>();

        let stage_started = Instant::now();
        let sales = sqlx::query(
            "SELECT count(*) order_count,COALESCE(sum(gross_amount),0)::numeric(24,6) order_amount,count(*) FILTER(WHERE lifecycle_status IN ('confirmed','completed')) committed_count,count(*) FILTER(WHERE fulfillment_status='shipped') shipped_count,count(*) FILTER(WHERE hold_status='manual_review_hold') hold_count FROM sales_orders WHERE order_date>=$1 AND order_date<$2 AND currency=$3 AND legal_entity_id=ANY($4) AND customer_id=ANY($5) AND (brand_id IS NULL OR brand_id=ANY($6)) AND business_unit_id=ANY($7)",
        )
        .bind(period_start).bind(period_end).bind(currency).bind(&le).bind(&customer).bind(&brand).bind(&bu)
        .fetch_one(self.store.pool()).await?;
        record_stage(&mut stages, "salesOrders", stage_started);
        let stage_started = Instant::now();
        let shipments = sqlx::query(
            "SELECT count(*) FILTER(WHERE s.status='confirmed') shipment_count,COALESCE(sum(s.sales_amount) FILTER(WHERE s.status='confirmed'),0)::numeric(24,6) shipped_revenue,COALESCE(sum(s.cost_amount) FILTER(WHERE s.status='confirmed'),0)::numeric(24,6) shipped_cost FROM shipments s JOIN sales_orders o ON o.id=s.sales_order_id WHERE s.shipment_date>=$1 AND s.shipment_date<$2 AND s.currency=$3 AND s.legal_entity_id=ANY($4) AND s.customer_id=ANY($5) AND s.warehouse_id=ANY($6) AND (o.brand_id IS NULL OR o.brand_id=ANY($7)) AND o.business_unit_id=ANY($8)",
        )
        .bind(period_start).bind(period_end).bind(currency).bind(&le).bind(&customer).bind(&wh).bind(&brand).bind(&bu)
        .fetch_one(self.store.pool()).await?;
        record_stage(&mut stages, "shipments", stage_started);
        let stage_started = Instant::now();
        let purchasing = sqlx::query(
            "WITH scoped AS (SELECT p.id,p.gross_amount,p.receiving_status,count(*) line_count,count(*) FILTER(WHERE l.received_quantity+l.cancelled_quantity=l.ordered_quantity) received_line_count FROM purchase_orders p JOIN purchase_order_lines l ON l.purchase_order_id=p.id WHERE p.order_date>=$1 AND p.order_date<$2 AND p.currency=$3 AND p.legal_entity_id=ANY($4) AND p.supplier_id=ANY($5) AND (p.brand_id IS NULL OR p.brand_id=ANY($6)) AND p.business_unit_id=ANY($7) AND l.warehouse_id=ANY($8) GROUP BY p.id) SELECT count(*) purchase_order_count,COALESCE(sum(gross_amount),0)::numeric(24,6) purchase_order_amount,COALESCE(sum(line_count),0)::bigint line_count,COALESCE(sum(received_line_count),0)::bigint received_line_count,count(*) FILTER(WHERE receiving_status='received') received_order_count FROM scoped",
        )
        .bind(period_start).bind(period_end).bind(currency).bind(&le).bind(&supplier).bind(&brand).bind(&bu).bind(&wh)
        .fetch_one(self.store.pool()).await?;
        record_stage(&mut stages, "purchaseOrders", stage_started);
        let stage_started = Instant::now();
        let inventory = sqlx::query(
            "SELECT count(*) sku_location_count,count(*) FILTER(WHERE b.on_hand_quantity>0) stocked_location_count,count(*) FILTER(WHERE b.reserved_quantity>0) reserved_location_count,COALESCE(sum(b.inventory_value),0)::numeric(24,6) inventory_value,count(*) FILTER(WHERE b.on_hand_quantity-b.reserved_quantity=0 AND b.reserved_quantity>0) stockout_count FROM inventory_balances b JOIN business_legal_entities e ON e.id=b.legal_entity_id WHERE e.functional_currency=$1 AND b.legal_entity_id=ANY($2) AND b.warehouse_id=ANY($3)",
        )
        .bind(currency).bind(&le).bind(&wh).fetch_one(self.store.pool()).await?;
        record_stage(&mut stages, "inventoryBalances", stage_started);
        let stage_started = Instant::now();
        let profit = sqlx::query(
            "SELECT COALESCE(sum(CASE direction WHEN 'normal' THEN amount ELSE -amount END) FILTER(WHERE metric_type='net_revenue'),0)::numeric(24,6) revenue,COALESCE(sum(CASE direction WHEN 'normal' THEN amount ELSE -amount END) FILTER(WHERE metric_type='product_cost'),0)::numeric(24,6) product_cost,COALESCE(sum(CASE direction WHEN 'normal' THEN amount ELSE -amount END) FILTER(WHERE metric_type IN ('outbound_freight','sales_commission','platform_fee','customer_rebate','other_direct_cost','allocated_operating_expense')),0)::numeric(24,6) operating_cost,COALESCE(sum(CASE direction WHEN 'normal' THEN amount ELSE -amount END) FILTER(WHERE metric_type='supplier_rebate'),0)::numeric(24,6) supplier_rebate,COALESCE(max(fact_sequence),0) source_watermark FROM profit_facts WHERE management_period=$1 AND currency=$2 AND legal_entity_id=ANY($3) AND customer_id=ANY($4) AND (brand_id IS NULL OR brand_id=ANY($5)) AND business_unit_id=ANY($6) AND (warehouse_id IS NULL OR warehouse_id=ANY($7))",
        )
        .bind(management_period).bind(currency).bind(&le).bind(&customer).bind(&brand).bind(&bu).bind(&wh)
        .fetch_one(self.store.pool()).await?;
        record_stage(&mut stages, "profitFacts", stage_started);
        let stage_started = Instant::now();
        let projection = sqlx::query(
            "SELECT o.updated_at,COALESCE((SELECT count(*) FROM business_core_outbox e WHERE e.topic IN ('shipment_confirmed','shipment_reversed','sales_return_confirmed') AND (o.last_outbox_created_at IS NULL OR (e.created_at,e.id)>(o.last_outbox_created_at,o.last_outbox_event_id))),0) pending_events,(SELECT count(*) FROM profit_projection_failures f WHERE f.status='pending' AND (EXISTS(SELECT 1 FROM shipments s JOIN sales_orders so ON so.id=s.sales_order_id WHERE s.id=f.aggregate_id AND s.legal_entity_id=ANY($1) AND s.customer_id=ANY($2) AND s.warehouse_id=ANY($3) AND (so.brand_id IS NULL OR so.brand_id=ANY($4)) AND so.business_unit_id=ANY($5)) OR EXISTS(SELECT 1 FROM sales_returns r JOIN sales_orders so ON so.id=r.sales_order_id WHERE r.id=f.aggregate_id AND r.legal_entity_id=ANY($1) AND r.customer_id=ANY($2) AND r.warehouse_id=ANY($3) AND (so.brand_id IS NULL OR so.brand_id=ANY($4)) AND so.business_unit_id=ANY($5)))) pending_failures FROM profit_projection_offsets o WHERE o.consumer_name='profit_projection_v1'",
        )
        .bind(&le).bind(&customer).bind(&wh).bind(&brand).bind(&bu)
        .fetch_optional(self.store.pool()).await?;
        record_stage(&mut stages, "projectionHealth", stage_started);

        let revenue: Decimal = profit.get("revenue");
        let product_cost: Decimal = profit.get("product_cost");
        let operating_cost: Decimal = profit.get("operating_cost");
        let supplier_rebate: Decimal = profit.get("supplier_rebate");
        let operating_profit = revenue - product_cost - operating_cost + supplier_rebate;
        let committed = sales.get::<i64, _>("committed_count");
        let shipped = sales.get::<i64, _>("shipped_count");
        let line_count = purchasing.get::<i64, _>("line_count");
        let received_line_count = purchasing.get::<i64, _>("received_line_count");
        let updated_at = projection
            .as_ref()
            .map(|row| row.get::<chrono::DateTime<Utc>, _>("updated_at"));
        let now = Utc::now();
        let freshness_age_seconds =
            updated_at.map(|value| now.signed_duration_since(value).num_seconds().max(0));
        let stale_after_seconds = self.projection_stale_after_minutes.saturating_mul(60);
        let projection_fresh = self.projection_worker_enabled
            && freshness_age_seconds.is_some_and(|value| value <= stale_after_seconds);
        let pending_events = projection
            .as_ref()
            .map_or(0, |row| row.get::<i64, _>("pending_events"));
        let pending_failures = projection
            .as_ref()
            .map_or(0, |row| row.get::<i64, _>("pending_failures"));
        let report_status = if pending_failures > 0 {
            "blocked"
        } else if pending_events > 0 || !projection_fresh {
            "partial"
        } else {
            "complete"
        };
        let duration_ms = elapsed_ms(overall_started);
        let mut alerts = Vec::new();
        append_projection_alerts(
            &mut alerts,
            self.projection_worker_enabled,
            projection_fresh,
            pending_events,
            pending_failures,
        );
        if duration_ms > DASHBOARD_TARGET_MS {
            alerts.push(alert(
                "SLOW_REPORT_READ",
                "warning",
                "经营驾驶舱读取超过 500 毫秒目标",
                "/api/v1/operations/dashboard",
            ));
        }
        Ok(json!({
            "managementPeriod": management_period,
            "currency": currency,
            "sourceWatermark": profit.get::<i64,_>("source_watermark"),
            "sales": {
                "orderCount": sales.get::<i64,_>("order_count"),
                "orderAmount": decimal(&sales, "order_amount"),
                "committedOrderCount": committed,
                "shippedOrderCount": shipped,
                "fulfillmentRate": ratio_i64(shipped, committed),
                "manualHoldCount": sales.get::<i64,_>("hold_count"),
                "shipmentCount": shipments.get::<i64,_>("shipment_count"),
                "shippedRevenue": decimal(&shipments, "shipped_revenue")
            },
            "purchasing": {
                "purchaseOrderCount": purchasing.get::<i64,_>("purchase_order_count"),
                "purchaseOrderAmount": decimal(&purchasing, "purchase_order_amount"),
                "receivedOrderCount": purchasing.get::<i64,_>("received_order_count"),
                "lineCount": line_count,
                "receivedLineCount": received_line_count,
                "receiptRate": ratio_i64(received_line_count, line_count)
            },
            "inventory": {
                "skuLocationCount": inventory.get::<i64,_>("sku_location_count"),
                "stockedLocationCount": inventory.get::<i64,_>("stocked_location_count"),
                "reservedLocationCount": inventory.get::<i64,_>("reserved_location_count"),
                "inventoryValue": decimal(&inventory, "inventory_value"),
                "stockoutCount": inventory.get::<i64,_>("stockout_count")
            },
            "profit": {
                "netRevenue": revenue.to_string(),
                "productCost": product_cost.to_string(),
                "grossProfit": (revenue-product_cost).to_string(),
                "managementOperatingProfit": operating_profit.to_string(),
                "managementOperatingMarginRate": if revenue == Decimal::ZERO { Value::Null } else { json!((operating_profit/revenue).round_dp(8).to_string()) },
                "sourceWatermark": profit.get::<i64,_>("source_watermark")
            },
            "reportHealth": {
                "status": report_status,
                "workerEnabled": self.projection_worker_enabled,
                "projectionFresh": projection_fresh,
                "freshnessAgeSeconds": freshness_age_seconds,
                "staleAfterSeconds": stale_after_seconds,
                "pendingEvents": pending_events,
                "pendingFailures": pending_failures,
                "updatedAt": updated_at,
                "alerts": alerts
            },
            "diagnostics": read_diagnostics(stages, duration_ms, DASHBOARD_TARGET_MS),
            "drilldowns": {
                "salesOrders": "/sales-orders",
                "shipments": "/shipments",
                "purchaseOrders": "/purchase-orders",
                "goodsReceipts": "/goods-receipts",
                "inventory": "/inventory",
                "orderProfit": "/order-profits",
                "profitability": "/profitability"
            },
            "scopeVersion": auth.scope_version,
            "effectiveScopeHash": auth.effective_scope_hash,
            "dataAsOf": Utc::now(),
            "warnings": ["经营管理口径，不是法定财务报表；不同币种不合并"],
            "boundary": "business_operations_only_not_financial_accounting"
        }))
    }
}

fn check(domain: &str, count: i64, evidence_path: &str) -> Value {
    json!({
        "domain": domain,
        "status": if count == 0 { "consistent" } else { "difference" },
        "differenceCount": count,
        "evidencePath": evidence_path
    })
}

fn record_stage(stages: &mut Vec<QueryStage>, name: &'static str, started: Instant) {
    stages.push(QueryStage {
        name,
        duration_ms: elapsed_ms(started),
    });
}

fn elapsed_ms(started: Instant) -> f64 {
    (started.elapsed().as_secs_f64() * 1_000_000.0).round() / 1_000.0
}

fn read_diagnostics(stages: Vec<QueryStage>, duration_ms: f64, target_ms: f64) -> Value {
    let slowest_stage = stages
        .iter()
        .max_by(|left, right| left.duration_ms.total_cmp(&right.duration_ms))
        .map(|stage| stage.name);
    json!({
        "status": if duration_ms <= target_ms { "healthy" } else { "slow" },
        "durationMs": duration_ms,
        "targetMs": target_ms,
        "slowestStage": slowest_stage,
        "stages": stages
    })
}

fn alert(code: &str, severity: &str, message: &str, evidence_path: &str) -> Value {
    json!({
        "code": code,
        "severity": severity,
        "message": message,
        "evidencePath": evidence_path
    })
}

fn append_projection_alerts(
    alerts: &mut Vec<Value>,
    worker_enabled: bool,
    fresh: bool,
    pending_events: i64,
    pending_failures: i64,
) {
    if pending_failures > 0 {
        alerts.push(alert(
            "PROJECTION_FAILURE",
            "critical",
            &format!("利润投影有 {pending_failures} 条失败待处理"),
            "/api/v1/operations/data-quality",
        ));
    }
    if pending_events > 0 {
        alerts.push(alert(
            "PROJECTION_BACKLOG",
            "warning",
            &format!("利润投影有 {pending_events} 条事件待消费"),
            "/api/v1/operations/data-quality",
        ));
    }
    if !worker_enabled {
        alerts.push(alert(
            "PROJECTION_WORKER_DISABLED",
            "warning",
            "利润投影 worker 未启用",
            "/api/v1/operations/data-quality",
        ));
    } else if !fresh {
        alerts.push(alert(
            "PROJECTION_STALE",
            "warning",
            "利润投影水位超过新鲜度阈值",
            "/api/v1/operations/data-quality",
        ));
    }
}

fn validate_period(value: &str) -> Result<(), DomainError> {
    let month = value.get(5..).and_then(|part| part.parse::<u8>().ok());
    if value.len() == 7
        && value.as_bytes().get(4) == Some(&b'-')
        && value[..4].bytes().all(|byte| byte.is_ascii_digit())
        && month.is_some_and(|value| (1..=12).contains(&value))
    {
        Ok(())
    } else {
        Err(DomainError::Invalid(
            "managementPeriod must use YYYY-MM".into(),
        ))
    }
}

fn period_bounds(value: &str) -> Result<(NaiveDate, NaiveDate), DomainError> {
    validate_period(value)?;
    let start = NaiveDate::parse_from_str(&format!("{value}-01"), "%Y-%m-%d")
        .map_err(|_| DomainError::Invalid("managementPeriod must use YYYY-MM".into()))?;
    let end = start
        .checked_add_months(Months::new(1))
        .ok_or_else(|| DomainError::Invalid("managementPeriod is out of range".into()))?;
    Ok((start, end))
}

fn decimal(row: &sqlx::postgres::PgRow, name: &str) -> String {
    row.get::<Decimal, _>(name).to_string()
}

fn ratio_i64(value: i64, total: i64) -> Value {
    if total == 0 {
        Value::Null
    } else {
        json!(format!(
            "{:.8}",
            (Decimal::from(value) / Decimal::from(total)).round_dp(8)
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_period_and_ratios_without_hiding_empty_denominators() {
        assert!(validate_period("2026-08").is_ok());
        assert!(validate_period("2026-13").is_err());
        assert_eq!(ratio_i64(1, 2), json!("0.50000000"));
        assert_eq!(ratio_i64(0, 0), Value::Null);
        assert_eq!(
            period_bounds("2026-12").unwrap(),
            (
                NaiveDate::from_ymd_opt(2026, 12, 1).unwrap(),
                NaiveDate::from_ymd_opt(2027, 1, 1).unwrap()
            )
        );
    }

    #[test]
    fn diagnostics_identify_the_slowest_stage_and_threshold_state() {
        let diagnostics = read_diagnostics(
            vec![
                QueryStage {
                    name: "salesOrders",
                    duration_ms: 8.0,
                },
                QueryStage {
                    name: "profitFacts",
                    duration_ms: 21.0,
                },
            ],
            31.0,
            25.0,
        );
        assert_eq!(diagnostics["status"], "slow");
        assert_eq!(diagnostics["slowestStage"], "profitFacts");
    }

    #[test]
    fn projection_alerts_are_structured_and_actionable() {
        let mut alerts = Vec::new();
        append_projection_alerts(&mut alerts, true, false, 3, 1);
        assert_eq!(alerts.len(), 3);
        assert_eq!(alerts[0]["code"], "PROJECTION_FAILURE");
        assert_eq!(alerts[0]["severity"], "critical");
        assert!(alerts.iter().all(|item| item["evidencePath"].is_string()));
    }
}
