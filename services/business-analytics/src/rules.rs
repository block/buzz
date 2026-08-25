use super::{AnalyticsError, AuthorizationScope, BusinessDataset};
use business_anomaly_contracts::{
    BusinessAnomaly, Confidence, FindingEvidence, FindingRule, Severity, RULE_SET_VERSION,
};
use business_query_contracts::{Money, ResourceRef};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::{collections::BTreeMap, str::FromStr};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalysisDomain {
    Profit,
    Receivable,
    Inventory,
    Purchase,
    CrossDomain,
    ProfitChange,
    All,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuleConfig {
    pub version: String,
    pub low_margin_threshold: String,
    pub receivable_overdue_days: u32,
    pub receivable_outstanding_threshold: String,
    pub uninvoiced_shipment_days: u32,
    pub invoice_unpaid_days: u32,
    pub purchase_price_window_days: u32,
    pub purchase_price_increase_rate: String,
    pub purchase_price_min_samples: usize,
    pub inventory_aging_days: u32,
    pub inventory_no_sales_days: u32,
    pub receipt_uninvoiced_days: u32,
    pub payment_receipt_gap: String,
    pub stale_after_minutes: i64,
}

impl Default for RuleConfig {
    fn default() -> Self {
        Self {
            version: RULE_SET_VERSION.into(),
            low_margin_threshold: "0.03".into(),
            receivable_overdue_days: 60,
            receivable_outstanding_threshold: "10000.00".into(),
            uninvoiced_shipment_days: 7,
            invoice_unpaid_days: 30,
            purchase_price_window_days: 30,
            purchase_price_increase_rate: "0.10".into(),
            purchase_price_min_samples: 3,
            inventory_aging_days: 180,
            inventory_no_sales_days: 90,
            receipt_uninvoiced_days: 30,
            payment_receipt_gap: "0.20".into(),
            stale_after_minutes: 60 * 24 * 365,
        }
    }
}

impl RuleConfig {
    /// Load the reviewed rule set shipped with this build.
    pub fn bundled() -> Result<Self, AnalyticsError> {
        serde_json::from_str(include_str!("../rules/trade-risk-v1.0.json"))
            .map_err(AnalyticsError::RuleConfigFile)
    }

    pub fn validate(&self) -> Result<(), AnalyticsError> {
        let rate = |value: &str| {
            Decimal::from_str(value)
                .ok()
                .filter(|v| *v >= Decimal::ZERO && *v <= Decimal::ONE)
        };
        if self.version.is_empty()
            || rate(&self.low_margin_threshold).is_none()
            || rate(&self.purchase_price_increase_rate).is_none()
            || rate(&self.payment_receipt_gap).is_none()
            || Decimal::from_str(&self.receivable_outstanding_threshold)
                .ok()
                .is_none_or(|value| value < Decimal::ZERO)
            || !(1..=100).contains(&self.purchase_price_min_samples)
            || !(1..=366).contains(&self.purchase_price_window_days)
            || !(1..=3_650).contains(&self.receivable_overdue_days)
            || !(1..=3_650).contains(&self.uninvoiced_shipment_days)
            || !(1..=3_650).contains(&self.invoice_unpaid_days)
            || !(1..=3_650).contains(&self.inventory_aging_days)
            || !(1..=366).contains(&self.inventory_no_sales_days)
            || !(1..=3_650).contains(&self.receipt_uninvoiced_days)
            || self.stale_after_minutes <= 0
        {
            return Err(AnalyticsError::RuleConfig(
                "threshold or version out of bounds",
            ));
        }
        Ok(())
    }
}

pub fn evaluate(
    dataset: &BusinessDataset,
    config: &RuleConfig,
    domain: AnalysisDomain,
    scope: &AuthorizationScope,
    trace_id: Uuid,
) -> Vec<BusinessAnomaly> {
    let mut out = Vec::new();
    if matches!(
        domain,
        AnalysisDomain::Profit | AnalysisDomain::CrossDomain | AnalysisDomain::All
    ) {
        profit_rules(dataset, config, scope, trace_id, &mut out);
    }
    if matches!(
        domain,
        AnalysisDomain::Receivable | AnalysisDomain::CrossDomain | AnalysisDomain::All
    ) {
        receivable_rules(dataset, config, scope, trace_id, &mut out);
    }
    if matches!(
        domain,
        AnalysisDomain::Inventory | AnalysisDomain::CrossDomain | AnalysisDomain::All
    ) {
        inventory_rules(dataset, config, scope, trace_id, &mut out);
    }
    if matches!(
        domain,
        AnalysisDomain::Purchase | AnalysisDomain::CrossDomain | AnalysisDomain::All
    ) {
        purchase_rules(dataset, config, scope, trace_id, &mut out);
    }
    if matches!(domain, AnalysisDomain::ProfitChange) {
        profit_change_rule(dataset, config, scope, trace_id, &mut out);
    }
    out
}

fn dec(value: &str) -> Option<Decimal> {
    Decimal::from_str(value).ok()
}
fn allowed_base(scope: &AuthorizationScope, legal: &str) -> bool {
    scope.allows_legal_entity(legal)
}
fn resource(kind: &str, id: &str) -> ResourceRef {
    let slug = match kind {
        "sales_order" => "sales-order",
        "purchase_order" => "purchase-order",
        "inventory" => "inventory",
        "customer" => "customer",
        "supplier" => "supplier",
        _ => "sales-order",
    };
    ResourceRef {
        r#type: kind.into(),
        id: Some(id.into()),
        title: format!("打开 {id}"),
        biz_uri: format!("biz://{slug}/{id}"),
    }
}
fn stable_id(rule: &str, object: &str) -> Uuid {
    let mut bytes = [0u8; 16];
    let digest = sha2::Sha256::digest(format!("{rule}:{object}"));
    bytes.copy_from_slice(&digest[..16]);
    Uuid::from_bytes(bytes)
}
#[allow(clippy::too_many_arguments)]
fn evidence(
    source: &str,
    object_type: &str,
    object_id: &str,
    version: &str,
    updated_at: chrono::DateTime<chrono::Utc>,
    field: &str,
    observed: impl ToString,
    threshold: Option<String>,
) -> FindingEvidence {
    FindingEvidence {
        source_system: source.into(),
        object_type: object_type.into(),
        object_id: object_id.into(),
        object_version: version.into(),
        updated_at,
        field: field.into(),
        observed_value: observed.to_string(),
        threshold,
    }
}
struct FindingArgs<'a> {
    rule: &'a str,
    kind: &'a str,
    object: &'a str,
    anomaly_type: &'a str,
    severity: Severity,
    confidence: Confidence,
    title: &'a str,
    summary: &'a str,
    observed: Option<String>,
    threshold: Option<String>,
    unit: Option<String>,
    impact: Option<Money>,
    evidence: Vec<FindingEvidence>,
    related: Vec<ResourceRef>,
    data_as_of: chrono::DateTime<chrono::Utc>,
    warnings: Vec<String>,
}
fn finding(config: &RuleConfig, args: FindingArgs<'_>) -> BusinessAnomaly {
    BusinessAnomaly {
        id: stable_id(args.rule, args.object),
        r#type: args.anomaly_type.into(),
        severity: args.severity,
        confidence: args.confidence,
        title: args.title.into(),
        summary_code: args.summary.into(),
        primary_resource: resource(args.kind, args.object),
        related_resources: args.related,
        impact: args.impact,
        rule: FindingRule {
            id: args.rule.into(),
            version: config.version.clone(),
            observed_value: args.observed,
            threshold: args.threshold,
            unit: args.unit,
        },
        evidence: args.evidence,
        data_as_of: args.data_as_of,
        warnings: args.warnings,
    }
}

fn profit_rules(
    dataset: &BusinessDataset,
    config: &RuleConfig,
    scope: &AuthorizationScope,
    _trace: Uuid,
    out: &mut Vec<BusinessAnomaly>,
) {
    let low_threshold = dec(&config.low_margin_threshold).unwrap_or(Decimal::new(3, 2));
    for fact in &dataset.order_profits {
        if !allowed_base(scope, &fact.identity.legal_entity_id)
            || !scope.allows_customer(&fact.customer_id)
            || !scope.allows_brand(&fact.brand_id)
        {
            continue;
        }
        let missing = [
            fact.product_cost.is_none(),
            fact.freight.is_none(),
            fact.commission.is_none(),
            fact.customer_rebate.is_none(),
        ]
        .into_iter()
        .filter(|v| *v)
        .count();
        if missing > 0 || fact.contribution_profit.is_none() {
            out.push(finding(
                config,
                FindingArgs {
                    rule: "DATA-QUALITY-001",
                    kind: "sales_order",
                    object: &fact.sales_order_id,
                    anomaly_type: "profit_data_incomplete",
                    severity: Severity::Medium,
                    confidence: Confidence::Low,
                    title: "订单利润数据不完整",
                    summary: "PROFIT_DATA_INCOMPLETE",
                    observed: Some(missing.to_string()),
                    threshold: Some("0".into()),
                    unit: Some("missing_fields".into()),
                    impact: None,
                    evidence: vec![evidence(
                        &fact.identity.source_system,
                        "order_profit",
                        &fact.sales_order_id,
                        &fact.identity.object_version,
                        fact.identity.updated_at,
                        "missing_required_fields",
                        missing,
                        Some("0".into()),
                    )],
                    related: vec![],
                    data_as_of: fact.identity.data_as_of,
                    warnings: vec!["MISSING_COST_OR_EXPENSE".into()],
                },
            ));
            continue;
        }
        let profit = fact
            .contribution_profit
            .as_ref()
            .and_then(|m| dec(&m.amount))
            .unwrap_or_default();
        let margin = fact
            .contribution_margin_rate
            .as_deref()
            .and_then(dec)
            .unwrap_or_default();
        if profit < Decimal::ZERO {
            out.push(finding(
                config,
                FindingArgs {
                    rule: "PROFIT-LOSS-001",
                    kind: "sales_order",
                    object: &fact.sales_order_id,
                    anomaly_type: "loss_order",
                    severity: Severity::High,
                    confidence: Confidence::High,
                    title: "销售订单实际亏损",
                    summary: "CONTRIBUTION_PROFIT_NEGATIVE",
                    observed: Some(profit.to_string()),
                    threshold: Some("0".into()),
                    unit: Some(fact.revenue.currency.clone()),
                    impact: fact.contribution_profit.clone(),
                    evidence: vec![evidence(
                        &fact.identity.source_system,
                        "order_profit",
                        &fact.sales_order_id,
                        &fact.identity.object_version,
                        fact.identity.updated_at,
                        "contribution_profit",
                        profit,
                        Some("0".into()),
                    )],
                    related: vec![resource("customer", &fact.customer_id)],
                    data_as_of: fact.identity.data_as_of,
                    warnings: vec![],
                },
            ));
        } else if margin < low_threshold {
            out.push(finding(
                config,
                FindingArgs {
                    rule: "PROFIT-MARGIN-002",
                    kind: "sales_order",
                    object: &fact.sales_order_id,
                    anomaly_type: "low_margin_order",
                    severity: Severity::Medium,
                    confidence: Confidence::High,
                    title: "销售订单贡献毛利率偏低",
                    summary: "CONTRIBUTION_MARGIN_BELOW_THRESHOLD",
                    observed: Some(margin.to_string()),
                    threshold: Some(low_threshold.to_string()),
                    unit: Some("ratio".into()),
                    impact: fact.contribution_profit.clone(),
                    evidence: vec![evidence(
                        &fact.identity.source_system,
                        "order_profit",
                        &fact.sales_order_id,
                        &fact.identity.object_version,
                        fact.identity.updated_at,
                        "contribution_margin_rate",
                        margin,
                        Some(low_threshold.to_string()),
                    )],
                    related: vec![],
                    data_as_of: fact.identity.data_as_of,
                    warnings: vec![],
                },
            ));
        }
        if profit < Decimal::ZERO
            && dataset.receivables.iter().any(|r| {
                r.customer_id == fact.customer_id
                    && allowed_base(scope, &r.identity.legal_entity_id)
                    && scope.allows_customer(&r.customer_id)
                    && r.overdue_days >= config.receivable_overdue_days
            })
        {
            out.push(finding(
                config,
                FindingArgs {
                    rule: "CROSS-LOSS-TERM-003",
                    kind: "sales_order",
                    object: &fact.sales_order_id,
                    anomaly_type: "loss_order_long_payment_term",
                    severity: Severity::Critical,
                    confidence: Confidence::High,
                    title: "亏损订单关联长账期客户",
                    summary: "LOSS_AND_OVERDUE_RECEIVABLE",
                    observed: Some(profit.to_string()),
                    threshold: Some("0".into()),
                    unit: Some(fact.revenue.currency.clone()),
                    impact: fact.contribution_profit.clone(),
                    evidence: vec![],
                    related: vec![resource("customer", &fact.customer_id)],
                    data_as_of: fact.identity.data_as_of,
                    warnings: vec![],
                },
            ));
        }
    }
}

fn receivable_rules(
    dataset: &BusinessDataset,
    config: &RuleConfig,
    scope: &AuthorizationScope,
    _trace: Uuid,
    out: &mut Vec<BusinessAnomaly>,
) {
    let threshold = dec(&config.receivable_outstanding_threshold).unwrap_or_default();
    for ar in &dataset.receivables {
        if !allowed_base(scope, &ar.identity.legal_entity_id)
            || !scope.allows_customer(&ar.customer_id)
        {
            continue;
        }
        let amount = dec(&ar.outstanding_amount.amount).unwrap_or_default();
        if ar.overdue_days >= config.receivable_overdue_days && amount > threshold {
            let orders = dataset
                .sales_orders
                .iter()
                .filter(|o| {
                    o.customer_id == ar.customer_id
                        && o.recent_shipment_or_open_order
                        && allowed_base(scope, &o.identity.legal_entity_id)
                        && scope.allows_customer(&o.customer_id)
                        && scope.allows_warehouse(&o.warehouse_id)
                        && scope.allows_brand(&o.brand_id)
                })
                .collect::<Vec<_>>();
            if !orders.is_empty() {
                out.push(finding(
                    config,
                    FindingArgs {
                        rule: "AR-SHIP-003",
                        kind: "customer",
                        object: &ar.customer_id,
                        anomaly_type: "overdue_customer_continued_shipping",
                        severity: Severity::High,
                        confidence: Confidence::High,
                        title: "逾期客户仍在继续发货",
                        summary: "OVERDUE_AND_ACTIVE_SHIPMENT",
                        observed: Some(ar.overdue_days.to_string()),
                        threshold: Some(config.receivable_overdue_days.to_string()),
                        unit: Some("days".into()),
                        impact: Some(ar.overdue_amount.clone()),
                        evidence: vec![evidence(
                            &ar.identity.source_system,
                            "receivable",
                            &ar.identity.object_id,
                            &ar.identity.object_version,
                            ar.identity.updated_at,
                            "overdue_days",
                            ar.overdue_days,
                            Some(config.receivable_overdue_days.to_string()),
                        )],
                        related: orders
                            .iter()
                            .map(|o| resource("sales_order", &o.identity.object_id))
                            .collect(),
                        data_as_of: ar.identity.data_as_of,
                        warnings: vec![],
                    },
                ));
            }
        }
    }
    for order in &dataset.sales_orders {
        if !allowed_base(scope, &order.identity.legal_entity_id)
            || !scope.allows_customer(&order.customer_id)
        {
            continue;
        }
        let shipped = dec(&order.shipped_amount.amount).unwrap_or_default();
        let invoiced = dec(&order.invoiced_amount.amount).unwrap_or_default();
        let received = dec(&order.received_amount.amount).unwrap_or_default();
        if shipped > invoiced && order.days_since_outbound > config.uninvoiced_shipment_days {
            out.push(finding(
                config,
                FindingArgs {
                    rule: "AR-UNINVOICED-004",
                    kind: "sales_order",
                    object: &order.identity.object_id,
                    anomaly_type: "shipped_not_invoiced",
                    severity: Severity::Medium,
                    confidence: Confidence::High,
                    title: "已出库未开票",
                    summary: "SHIPPED_AMOUNT_EXCEEDS_INVOICED",
                    observed: Some((shipped - invoiced).to_string()),
                    threshold: Some(config.uninvoiced_shipment_days.to_string()),
                    unit: Some("days".into()),
                    impact: Some(Money {
                        amount: (shipped - invoiced).to_string(),
                        currency: order.shipped_amount.currency.clone(),
                    }),
                    evidence: vec![],
                    related: vec![],
                    data_as_of: order.identity.data_as_of,
                    warnings: vec![],
                },
            ));
        }
        if invoiced > received && order.days_since_due > config.invoice_unpaid_days {
            out.push(finding(
                config,
                FindingArgs {
                    rule: "AR-UNPAID-005",
                    kind: "sales_order",
                    object: &order.identity.object_id,
                    anomaly_type: "invoiced_long_unpaid",
                    severity: Severity::High,
                    confidence: Confidence::High,
                    title: "已开票长期未回款",
                    summary: "INVOICED_AMOUNT_EXCEEDS_RECEIVED",
                    observed: Some((invoiced - received).to_string()),
                    threshold: Some(config.invoice_unpaid_days.to_string()),
                    unit: Some("days".into()),
                    impact: Some(Money {
                        amount: (invoiced - received).to_string(),
                        currency: order.invoiced_amount.currency.clone(),
                    }),
                    evidence: vec![],
                    related: vec![resource("customer", &order.customer_id)],
                    data_as_of: order.identity.data_as_of,
                    warnings: vec![],
                },
            ));
        }
    }
}

fn purchase_rules(
    dataset: &BusinessDataset,
    config: &RuleConfig,
    scope: &AuthorizationScope,
    _trace: Uuid,
    out: &mut Vec<BusinessAnomaly>,
) {
    let increase = dec(&config.purchase_price_increase_rate).unwrap_or(Decimal::new(1, 1));
    let gap = dec(&config.payment_receipt_gap).unwrap_or(Decimal::new(2, 1));
    let mut groups: BTreeMap<
        (&str, &str, &str, &str),
        Vec<&business_anomaly_contracts::PurchaseOrderFact>,
    > = BTreeMap::new();
    for po in &dataset.purchase_orders {
        if allowed_base(scope, &po.identity.legal_entity_id)
            && scope.allows_supplier(&po.supplier_id)
            && scope.allows_warehouse(&po.warehouse_id)
            && scope.allows_brand(&po.brand_id)
        {
            groups
                .entry((
                    &po.sku_id,
                    &po.supplier_id,
                    &po.unit_price.currency,
                    &po.unit,
                ))
                .or_default()
                .push(po);
        }
    }
    for (_, mut pos) in groups {
        pos.sort_by_key(|p| p.ordered_at);
        if let Some(current) = pos.last().copied() {
            let mut history = pos[..pos.len() - 1]
                .iter()
                .filter(|p| {
                    current
                        .ordered_at
                        .signed_duration_since(p.ordered_at)
                        .num_days()
                        <= i64::from(config.purchase_price_window_days)
                })
                .filter_map(|p| dec(&p.unit_price.amount))
                .collect::<Vec<_>>();
            if history.len() < config.purchase_price_min_samples {
                continue;
            }
            history.sort();
            let median = history[history.len() / 2];
            let price = dec(&current.unit_price.amount).unwrap_or_default();
            if median > Decimal::ZERO && (price - median) / median >= increase {
                out.push(finding(
                    config,
                    FindingArgs {
                        rule: "PO-PRICE-006",
                        kind: "purchase_order",
                        object: &current.identity.object_id,
                        anomaly_type: "purchase_price_increase",
                        severity: Severity::High,
                        confidence: Confidence::High,
                        title: "采购价格异常上涨",
                        summary: "UNIT_PRICE_ABOVE_MEDIAN",
                        observed: Some(((price - median) / median).to_string()),
                        threshold: Some(increase.to_string()),
                        unit: Some("ratio".into()),
                        impact: None,
                        evidence: vec![evidence(
                            &current.identity.source_system,
                            "purchase_order",
                            &current.identity.object_id,
                            &current.identity.object_version,
                            current.identity.updated_at,
                            "unit_price",
                            price,
                            Some(median.to_string()),
                        )],
                        related: vec![],
                        data_as_of: current.identity.data_as_of,
                        warnings: vec![],
                    },
                ));
            }
        }
    }
    for po in &dataset.purchase_orders {
        if !allowed_base(scope, &po.identity.legal_entity_id)
            || !scope.allows_supplier(&po.supplier_id)
            || !scope.allows_warehouse(&po.warehouse_id)
            || !scope.allows_brand(&po.brand_id)
        {
            continue;
        }
        let received = dec(&po.received_amount.amount).unwrap_or_default();
        let invoiced = dec(&po.invoiced_amount.amount).unwrap_or_default();
        let pay = dec(&po.payment_rate).unwrap_or_default();
        let receipt = dec(&po.receipt_rate).unwrap_or_default();
        if received > invoiced && po.days_since_receipt > config.receipt_uninvoiced_days {
            out.push(finding(
                config,
                FindingArgs {
                    rule: "PO-UNINVOICED-007",
                    kind: "purchase_order",
                    object: &po.identity.object_id,
                    anomaly_type: "received_not_invoiced",
                    severity: Severity::Medium,
                    confidence: Confidence::High,
                    title: "到货未收票",
                    summary: "RECEIVED_AMOUNT_EXCEEDS_INVOICED",
                    observed: Some((received - invoiced).to_string()),
                    threshold: Some(config.receipt_uninvoiced_days.to_string()),
                    unit: Some("days".into()),
                    impact: Some(Money {
                        amount: (received - invoiced).to_string(),
                        currency: po.received_amount.currency.clone(),
                    }),
                    evidence: vec![],
                    related: vec![resource("supplier", &po.supplier_id)],
                    data_as_of: po.identity.data_as_of,
                    warnings: vec![],
                },
            ));
        }
        if pay - receipt > gap {
            out.push(finding(
                config,
                FindingArgs {
                    rule: "PO-PAYMENT-008",
                    kind: "purchase_order",
                    object: &po.identity.object_id,
                    anomaly_type: "payment_ahead_of_receipt",
                    severity: Severity::High,
                    confidence: Confidence::High,
                    title: "付款进度显著早于到货",
                    summary: "PAYMENT_RECEIPT_GAP",
                    observed: Some((pay - receipt).to_string()),
                    threshold: Some(gap.to_string()),
                    unit: Some("ratio".into()),
                    impact: Some(po.paid_amount.clone()),
                    evidence: vec![],
                    related: vec![],
                    data_as_of: po.identity.data_as_of,
                    warnings: vec![],
                },
            ));
        }
    }
}

fn inventory_rules(
    dataset: &BusinessDataset,
    config: &RuleConfig,
    scope: &AuthorizationScope,
    _trace: Uuid,
    out: &mut Vec<BusinessAnomaly>,
) {
    for inv in &dataset.inventory {
        if !allowed_base(scope, &inv.identity.legal_entity_id)
            || !scope.allows_warehouse(&inv.warehouse_id)
            || !scope.allows_brand(&inv.brand_id)
        {
            continue;
        }
        let on = dec(&inv.on_hand_qty).unwrap_or_default();
        let available = dec(&inv.available_qty).unwrap_or_default();
        let sales = dec(&inv.sales_qty_last_90_days).unwrap_or_default();
        let transit = dec(&inv.in_transit_purchase_qty).unwrap_or_default();
        if inv.inventory_age_days >= config.inventory_aging_days && sales == Decimal::ZERO {
            out.push(finding(
                config,
                FindingArgs {
                    rule: "INV-AGING-009",
                    kind: "inventory",
                    object: &inv.sku_id,
                    anomaly_type: if transit > Decimal::ZERO {
                        "aged_inventory_still_purchasing"
                    } else {
                        "aged_inventory"
                    },
                    severity: if transit > Decimal::ZERO {
                        Severity::High
                    } else {
                        Severity::Medium
                    },
                    confidence: Confidence::High,
                    title: "库存积压",
                    summary: "AGED_WITHOUT_RECENT_SALES",
                    observed: Some(inv.inventory_age_days.to_string()),
                    threshold: Some(config.inventory_aging_days.to_string()),
                    unit: Some("days".into()),
                    impact: None,
                    evidence: vec![],
                    related: vec![],
                    data_as_of: inv.identity.data_as_of,
                    warnings: vec![],
                },
            ));
        }
        let demand = dataset
            .sales_orders
            .iter()
            .filter(|o| {
                o.sku_id == inv.sku_id
                    && o.warehouse_id == inv.warehouse_id
                    && allowed_base(scope, &o.identity.legal_entity_id)
                    && scope.allows_customer(&o.customer_id)
                    && scope.allows_warehouse(&o.warehouse_id)
                    && scope.allows_brand(&o.brand_id)
            })
            .filter_map(|o| dec(&o.open_order_demand_qty))
            .sum::<Decimal>();
        if available < demand {
            out.push(finding(
                config,
                FindingArgs {
                    rule: "INV-STOCKOUT-010",
                    kind: "inventory",
                    object: &inv.sku_id,
                    anomaly_type: "stockout_risk",
                    severity: Severity::High,
                    confidence: Confidence::High,
                    title: "缺货风险",
                    summary: "AVAILABLE_BELOW_OPEN_ORDER_DEMAND",
                    observed: Some(available.to_string()),
                    threshold: Some(demand.to_string()),
                    unit: Some("quantity".into()),
                    impact: None,
                    evidence: vec![],
                    related: vec![],
                    data_as_of: inv.identity.data_as_of,
                    warnings: vec!["Demand basis: confirmed open sales orders".into()],
                },
            ));
        }
        if on < Decimal::ZERO || available < Decimal::ZERO {
            out.push(finding(
                config,
                FindingArgs {
                    rule: "INV-NEGATIVE-011",
                    kind: "inventory",
                    object: &inv.sku_id,
                    anomaly_type: "negative_inventory",
                    severity: Severity::Critical,
                    confidence: Confidence::High,
                    title: "负库存",
                    summary: "NEGATIVE_ON_HAND_OR_AVAILABLE",
                    observed: Some(on.min(available).to_string()),
                    threshold: Some("0".into()),
                    unit: Some("quantity".into()),
                    impact: None,
                    evidence: vec![],
                    related: vec![],
                    data_as_of: inv.identity.data_as_of,
                    warnings: vec![],
                },
            ));
        }
    }
}

fn profit_change_rule(
    dataset: &BusinessDataset,
    config: &RuleConfig,
    scope: &AuthorizationScope,
    _trace: Uuid,
    out: &mut Vec<BusinessAnomaly>,
) {
    let facts = dataset
        .order_profits
        .iter()
        .filter(|f| allowed_base(scope, &f.identity.legal_entity_id))
        .collect::<Vec<_>>();
    if facts.len() < 2 {
        return;
    }
    let profit = facts
        .iter()
        .filter_map(|f| f.contribution_profit.as_ref().and_then(|m| dec(&m.amount)))
        .sum::<Decimal>();
    out.push(finding(
        config,
        FindingArgs {
            rule: "PROFIT-BRIDGE-001",
            kind: "sales_order",
            object: &facts[0].sales_order_id,
            anomaly_type: "profit_change_bridge",
            severity: Severity::Info,
            confidence: Confidence::Low,
            title: "利润变化确定性分解",
            summary: "PROFIT_CHANGE_BRIDGE_WITH_UNEXPLAINED_DIFFERENCE",
            observed: Some(profit.to_string()),
            threshold: None,
            unit: Some(facts[0].revenue.currency.clone()),
            impact: Some(Money {
                amount: profit.to_string(),
                currency: facts[0].revenue.currency.clone(),
            }),
            evidence: [
                "revenue_effect",
                "volume_effect",
                "price_effect",
                "product_mix_effect",
                "purchase_cost_effect",
                "freight_effect",
                "commission_effect",
                "discount_effect",
                "rebate_effect",
                "other_known_effect",
                "unexplained_difference",
            ]
            .into_iter()
            .map(|field| {
                evidence(
                    "desensitized-business-api",
                    "profit_bridge",
                    "all",
                    "v1",
                    dataset.data_as_of,
                    field,
                    if field == "unexplained_difference" {
                        profit.to_string()
                    } else {
                        "0.00".into()
                    },
                    None,
                )
            })
            .collect(),
            related: vec![],
            data_as_of: dataset.data_as_of,
            warnings: vec!["Structure effects require authoritative period snapshots".into()],
        },
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        acceptance_scope, BusinessAnalyticsService, BusinessDataset, ACCEPTANCE_FINANCE_USER,
    };
    #[test]
    fn all_twelve_rule_families_and_three_cross_domain_findings_execute() {
        let service = BusinessAnalyticsService::new(
            BusinessDataset::desensitized_acceptance().expect("data"),
            RuleConfig::default(),
        )
        .expect("service");
        let result = service.analyze(
            AnalysisDomain::All,
            &acceptance_scope(ACCEPTANCE_FINANCE_USER).expect("scope"),
            Uuid::new_v4(),
            100,
        );
        let types = result
            .findings
            .iter()
            .map(|f| f.r#type.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        for required in [
            "loss_order",
            "low_margin_order",
            "overdue_customer_continued_shipping",
            "shipped_not_invoiced",
            "invoiced_long_unpaid",
            "purchase_price_increase",
            "received_not_invoiced",
            "payment_ahead_of_receipt",
            "aged_inventory_still_purchasing",
            "stockout_risk",
            "negative_inventory",
            "profit_data_incomplete",
            "loss_order_long_payment_term",
        ] {
            assert!(types.contains(required), "missing {required}");
        }
    }
    #[test]
    fn equality_does_not_trigger_strict_thresholds() {
        let config = RuleConfig {
            low_margin_threshold: "0.03".into(),
            ..RuleConfig::default()
        };
        let service = BusinessAnalyticsService::new(
            BusinessDataset::desensitized_acceptance().expect("data"),
            config,
        )
        .expect("service");
        let result = service.analyze(
            AnalysisDomain::Profit,
            &acceptance_scope(ACCEPTANCE_FINANCE_USER).expect("scope"),
            Uuid::new_v4(),
            100,
        );
        assert!(!result
            .findings
            .iter()
            .any(|f| f.primary_resource.id.as_deref() == Some("SO-MARGIN-EQ")));
    }
    #[test]
    fn missing_profit_fields_block_profit_conclusion() {
        let service = BusinessAnalyticsService::new(
            BusinessDataset::desensitized_acceptance().expect("data"),
            RuleConfig::default(),
        )
        .expect("service");
        let result = service.analyze(
            AnalysisDomain::Profit,
            &acceptance_scope(ACCEPTANCE_FINANCE_USER).expect("scope"),
            Uuid::new_v4(),
            100,
        );
        let finding = result
            .findings
            .iter()
            .find(|f| f.primary_resource.id.as_deref() == Some("SO-PARTIAL-001"))
            .expect("quality finding");
        assert_eq!(finding.confidence, Confidence::Low);
        assert_eq!(finding.r#type, "profit_data_incomplete");
    }
}
