#![forbid(unsafe_code)]

mod config;

pub use config::Config;

use axum::{
    body::Body,
    extract::{Path, Request, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
#[cfg(test)]
use business_analytics::acceptance_scope;
use business_analytics::{
    AnalysisDomain, AuthorizationScope, BusinessAnalyticsService, BusinessDataset, RuleConfig,
};
use business_anomaly_contracts::{
    AnomalyFilterInput, BusinessAnomaly, CrossDomainRiskInput, GetAnomalyInput, InventoryRiskInput,
    ProfitChangeInput, ProfitRiskInput, PurchaseRiskInput, ReceivableRiskInput,
    ValidateAnomalyInput,
};
use business_iam::{DataScope, EffectiveGrant};
use business_query_contracts::{
    BusinessToolResult, BusinessToolStatus, Evidence, Pagination, ResourceRef, ScopeSummary,
};
use chrono::DateTime;
use rust_decimal::Decimal;
use serde::Serialize;
use serde_json::{json, Value};
use std::{collections::BTreeMap, str::FromStr, time::Duration};
use subtle::ConstantTimeEq;
use url::Url;
use uuid::Uuid;

const READ_TOOLS: [&str; 14] = [
    "get_sales_order",
    "search_sales_orders",
    "get_purchase_order",
    "search_purchase_orders",
    "query_inventory_balance",
    "query_receivables",
    "query_payables",
    "query_order_profit",
    "query_profitability_by_dimension",
    "get_management_profit_report",
    "get_management_report_snapshot",
    "get_profit_evidence",
    "get_operating_dashboard",
    "get_business_data_quality",
];
const ANOMALY_TOOLS: [&str; 8] = [
    "search_business_anomalies",
    "get_business_anomaly",
    "analyze_order_profit_risks",
    "analyze_receivable_risks",
    "analyze_inventory_risks",
    "analyze_purchase_cost_risks",
    "analyze_cross_domain_risks",
    "explain_profit_change",
];
const WRITE_TOOLS: [&str; 6] = [
    "create_sales_order_draft",
    "create_shipment_draft",
    "create_purchase_order_draft",
    "create_goods_receipt_draft",
    "create_customer_receipt_draft",
    "create_supplier_payment_draft",
];

#[derive(Clone)]
struct ApiState {
    credential_hash: [u8; 32],
    service_audience: String,
    analytics: BusinessAnalyticsService,
    verifier: DelegationVerifier,
    max_findings: usize,
    max_payload_bytes: usize,
    core: Option<CoreClient>,
}

#[derive(Clone)]
struct CoreClient {
    client: reqwest::Client,
    base_url: Url,
    credential: String,
}

#[derive(Clone)]
enum DelegationVerifier {
    Gateway {
        client: reqwest::Client,
        url: Url,
        credential: String,
    },
    #[cfg(test)]
    AcceptanceTest,
}

#[derive(Debug, Clone)]
struct RequestContext {
    enterprise_user_id: Uuid,
    identity_binding_id: Uuid,
    delegation_id: Uuid,
    agent_id: String,
    agent_turn_id: String,
    trace_id: Uuid,
    used_calls: i32,
    required_scope: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct VerifyRequest<'a> {
    delegation_id: Uuid,
    enterprise_user_id: Uuid,
    identity_binding_id: Uuid,
    agent_id: &'a str,
    agent_turn_id: &'a str,
    trace_id: Uuid,
    used_calls: i32,
    required_scope: &'a str,
}

enum VerifiedAuthority {
    Iam(EffectiveGrant),
    #[cfg(test)]
    AcceptanceTest,
}

impl DelegationVerifier {
    async fn verify(&self, context: &RequestContext) -> Option<VerifiedAuthority> {
        match self {
            #[cfg(test)]
            Self::AcceptanceTest => Some(VerifiedAuthority::AcceptanceTest),
            Self::Gateway {
                client,
                url,
                credential,
            } => {
                let Ok(endpoint) = url.join("internal/agent-delegations/verify") else {
                    return None;
                };
                client
                    .post(endpoint)
                    .header("x-business-service-credential", credential)
                    .header("x-trace-id", context.trace_id.to_string())
                    .json(&VerifyRequest {
                        delegation_id: context.delegation_id,
                        enterprise_user_id: context.enterprise_user_id,
                        identity_binding_id: context.identity_binding_id,
                        agent_id: &context.agent_id,
                        agent_turn_id: &context.agent_turn_id,
                        trace_id: context.trace_id,
                        used_calls: context.used_calls,
                        required_scope: &context.required_scope,
                    })
                    .send()
                    .await
                    .ok()
                    .filter(|response| response.status().is_success())?
                    .json::<EffectiveGrant>()
                    .await
                    .ok()
                    .map(VerifiedAuthority::Iam)
            }
        }
    }
}

fn sha256(value: &str) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    Sha256::digest(value.as_bytes()).into()
}

#[cfg(test)]
fn router_with_verifier(
    credential: String,
    verifier: DelegationVerifier,
) -> Result<Router, String> {
    router_with_runtime(
        credential,
        verifier,
        RuleConfig::bundled().map_err(|e| e.to_string())?,
        100,
        128 * 1024,
        None,
    )
}

fn router_with_runtime(
    credential: String,
    verifier: DelegationVerifier,
    rule_config: RuleConfig,
    max_findings: usize,
    max_payload_bytes: usize,
    core: Option<CoreClient>,
) -> Result<Router, String> {
    let analytics = BusinessAnalyticsService::new(
        BusinessDataset::desensitized_acceptance().map_err(|e| e.to_string())?,
        rule_config,
    )
    .map_err(|e| e.to_string())?;
    let state = ApiState {
        credential_hash: sha256(&credential),
        service_audience: "business-read-api".into(),
        analytics,
        verifier,
        max_findings,
        max_payload_bytes,
        core,
    };
    Ok(Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/v1/read/{tool}", post(read_tool))
        .route("/v1/write/{tool}", post(write_tool))
        .with_state(state))
}

async fn write_tool(
    State(state): State<ApiState>,
    Path(tool): Path<String>,
    request: Request<Body>,
) -> Response {
    if !WRITE_TOOLS.contains(&tool.as_str()) {
        return StatusCode::NOT_FOUND.into_response();
    }
    if !authorized_service(
        request.headers(),
        &state.credential_hash,
        &state.service_audience,
    ) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Some(context) = parse_context(request.headers()) else {
        return StatusCode::FORBIDDEN.into_response();
    };
    let Some(VerifiedAuthority::Iam(grant)) = state.verifier.verify(&context).await else {
        return StatusCode::FORBIDDEN.into_response();
    };
    let Some(required) = required_capability(&tool) else {
        return StatusCode::FORBIDDEN.into_response();
    };
    if context.required_scope != required || grant.capability.as_str() != required {
        return StatusCode::FORBIDDEN.into_response();
    }
    let bytes = match axum::body::to_bytes(request.into_body(), state.max_payload_bytes).await {
        Ok(value) => value,
        Err(_) => return StatusCode::PAYLOAD_TOO_LARGE.into_response(),
    };
    let input: Value = match serde_json::from_slice(&bytes) {
        Ok(value) if valid_write_input(&tool, &value) => value,
        _ => return (StatusCode::BAD_REQUEST, "invalid_write_input").into_response(),
    };
    let Some(core) = state.core.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    forward_draft_write(core, &tool, input, &context).await
}

fn valid_write_input(tool: &str, input: &Value) -> bool {
    match tool {
        "create_sales_order_draft" => {
            serde_json::from_value::<business_core::b2::model::CreateSalesOrder>(input.clone())
                .is_ok()
        }
        "create_shipment_draft" => {
            serde_json::from_value::<business_core::b2::model::CreateShipment>(input.clone())
                .is_ok()
        }
        "create_customer_receipt_draft" => {
            serde_json::from_value::<business_core::b2::model::CreateCustomerReceipt>(input.clone())
                .is_ok()
        }
        "create_purchase_order_draft" => {
            serde_json::from_value::<business_core::b3::model::CreatePurchaseOrder>(input.clone())
                .is_ok()
        }
        "create_goods_receipt_draft" => {
            serde_json::from_value::<business_core::b3::model::CreateGoodsReceipt>(input.clone())
                .is_ok()
        }
        "create_supplier_payment_draft" => {
            serde_json::from_value::<business_core::b3::model::CreateSupplierPayment>(input.clone())
                .is_ok()
        }
        _ => false,
    }
}

async fn forward_draft_write(
    core: &CoreClient,
    tool: &str,
    input: Value,
    context: &RequestContext,
) -> Response {
    let (endpoint, resource_type, uri_type) = match tool {
        "create_sales_order_draft" => {
            ("v1/agent-drafts/sales-orders", "sales_order", "sales-order")
        }
        "create_shipment_draft" => ("v1/agent-drafts/shipments", "shipment", "shipment"),
        "create_purchase_order_draft" => (
            "v1/agent-drafts/purchase-orders",
            "purchase_order",
            "purchase-order",
        ),
        "create_goods_receipt_draft" => (
            "v1/agent-drafts/goods-receipts",
            "goods_receipt",
            "goods-receipt",
        ),
        "create_customer_receipt_draft" => (
            "v1/agent-drafts/customer-receipts",
            "customer_receipt",
            "customer-receipt",
        ),
        "create_supplier_payment_draft" => (
            "v1/agent-drafts/supplier-payments",
            "supplier_payment",
            "supplier-payment",
        ),
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    let Ok(url) = core.base_url.join(endpoint) else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let response = core
        .client
        .post(url)
        .header("x-business-service-credential", &core.credential)
        .header("x-service-audience", "business-core")
        .header(
            "x-enterprise-user-id",
            context.enterprise_user_id.to_string(),
        )
        .header("x-trace-id", context.trace_id.to_string())
        .header(
            "idempotency-key",
            format!("agent:{}:{tool}", context.delegation_id),
        )
        .json(&input)
        .send()
        .await;
    let Ok(response) = response else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let status = response.status();
    let Ok(value) = response.json::<Value>().await else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    if !status.is_success() {
        return (status, Json(value)).into_response();
    }
    let Some(id) = value.get("id").and_then(Value::as_str) else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let expected_trace_id = context.trace_id.to_string();
    if value.get("status").and_then(Value::as_str) != Some("draft")
        || value.get("traceId").and_then(Value::as_str) != Some(expected_trace_id.as_str())
    {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    Json(json!({
        "schemaVersion": 1,
        "status": "ok",
        "item": value,
        "resourceRefs": [{
            "type": resource_type,
            "id": id,
            "title": "打开已创建的业务草稿",
            "bizUri": format!("biz://{uri_type}/{id}")
        }],
        "traceId": context.trace_id
    }))
    .into_response()
}

async fn read_tool(
    State(state): State<ApiState>,
    Path(tool): Path<String>,
    request: Request<Body>,
) -> Response {
    if !READ_TOOLS.contains(&tool.as_str()) && !ANOMALY_TOOLS.contains(&tool.as_str()) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let headers = request.headers();
    if !authorized_service(headers, &state.credential_hash, &state.service_audience) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Some(context) = parse_context(headers) else {
        return StatusCode::FORBIDDEN.into_response();
    };
    let Some(verified_authority) = state.verifier.verify(&context).await else {
        return StatusCode::FORBIDDEN.into_response();
    };
    let bytes = match axum::body::to_bytes(request.into_body(), state.max_payload_bytes).await {
        Ok(value) => value,
        Err(_) => return StatusCode::PAYLOAD_TOO_LARGE.into_response(),
    };
    let input: Value = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(_) => return (StatusCode::BAD_REQUEST, "invalid_filter").into_response(),
    };
    if ANOMALY_TOOLS.contains(&tool.as_str()) && !valid_anomaly_input(&tool, &input) {
        return (StatusCode::BAD_REQUEST, "invalid_filter").into_response();
    }
    let requested = requested_scope(&input);
    let authorized_scope = match verified_authority {
        VerifiedAuthority::Iam(grant) => {
            let Some(required) = required_capability(&tool) else {
                return StatusCode::FORBIDDEN.into_response();
            };
            if context.required_scope != required {
                return StatusCode::FORBIDDEN.into_response();
            }
            let Some(scope) = iam_authorization_scope(&grant, required) else {
                return StatusCode::FORBIDDEN.into_response();
            };
            scope
        }
        #[cfg(test)]
        VerifiedAuthority::AcceptanceTest => {
            let Some(scope) = acceptance_scope(context.enterprise_user_id) else {
                return StatusCode::FORBIDDEN.into_response();
            };
            scope
        }
    };
    let effective_scope = authorized_scope.intersect(&requested);
    let response = if READ_TOOLS.contains(&tool.as_str()) {
        if let Some(core) = &state.core {
            core_read_result(core, &tool, &input, &effective_scope, &context).await
        } else {
            legacy_read_result(
                &state.analytics,
                &tool,
                &input,
                &effective_scope,
                context.trace_id,
            )
        }
    } else if matches!(
        tool.as_str(),
        "analyze_order_profit_risks" | "analyze_cross_domain_risks"
    ) && state.core.is_some()
    {
        let Some(core) = state.core.as_ref() else {
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        };
        core_profit_risk_result(core, &tool, &input, &effective_scope, &context).await
    } else if tool == "explain_profit_change" {
        if let Some(core) = state.core.as_ref() {
            core_profit_change_result(core, &input, &effective_scope, &context).await
        } else {
            anomaly_result(
                &state.analytics,
                &tool,
                &input,
                &effective_scope,
                context.trace_id,
                state.max_findings,
            )
        }
    } else {
        anomaly_result(
            &state.analytics,
            &tool,
            &input,
            &effective_scope,
            context.trace_id,
            state.max_findings,
        )
    };
    if state.verifier.verify(&context).await.is_none() {
        return StatusCode::FORBIDDEN.into_response();
    }
    response
}

async fn core_profit_risk_result(
    core: &CoreClient,
    tool: &str,
    input: &Value,
    scope: &AuthorizationScope,
    context: &RequestContext,
) -> Response {
    let call = |endpoint: &'static str| async move {
        let url = core.base_url.join(endpoint).map_err(|_| ())?;
        let response = core
            .client
            .get(url)
            .header("x-business-service-credential", &core.credential)
            .header("x-service-audience", "business-core")
            .header(
                "x-enterprise-user-id",
                context.enterprise_user_id.to_string(),
            )
            .header("x-trace-id", context.trace_id.to_string())
            .send()
            .await
            .map_err(|_| ())?;
        if !response.status().is_success() {
            return Err(());
        }
        response.json::<Value>().await.map_err(|_| ())
    };
    let Ok(profits) = call("v1/order-profits?limit=100").await else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let receivables = if tool == "analyze_cross_domain_risks" {
        match call("v1/trade-receivables?limit=200").await {
            Ok(value) => value,
            Err(()) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
        }
    } else {
        json!({"items":[]})
    };
    let allowed = |values: &std::collections::BTreeSet<String>, item: &Value, key: &str| {
        values.is_empty()
            || item
                .get(key)
                .and_then(Value::as_str)
                .is_some_and(|value| values.contains(value))
    };
    let mut findings = Vec::new();
    let mut partial = false;
    let mut source_watermark = 0_i64;
    for item in profits
        .get("items")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if !allowed(&scope.legal_entity_ids, item, "legalEntityId")
            || !allowed(&scope.customer_ids, item, "customerId")
            || !allowed(&scope.brand_ids, item, "brandId")
            || !allowed(&scope.business_unit_ids, item, "businessUnitId")
        {
            continue;
        }
        if input
            .get("salespersonIds")
            .and_then(Value::as_array)
            .is_some_and(|values| {
                !values.iter().filter_map(Value::as_str).any(|value| {
                    item.get("salespersonUserId").and_then(Value::as_str) == Some(value)
                })
            })
        {
            continue;
        }
        let Some(order_id) = item.get("salesOrderId").and_then(Value::as_str) else {
            continue;
        };
        let currency = item
            .get("currency")
            .and_then(Value::as_str)
            .unwrap_or("CNY");
        let quality = item
            .get("dataQualityStatus")
            .and_then(Value::as_str)
            .unwrap_or("partial");
        let data_as_of = item
            .get("dataAsOf")
            .cloned()
            .unwrap_or_else(|| json!(chrono::Utc::now()));
        source_watermark = source_watermark.max(
            item.get("lastFactSequence")
                .and_then(Value::as_i64)
                .unwrap_or_default(),
        );
        if quality != "complete" {
            partial = true;
            findings.push(profit_finding(
                order_id,
                currency,
                "PROFIT-DATA-QUALITY",
                "profit_data_quality",
                "medium",
                "medium",
                "利润数据质量不足，未生成确定性亏损结论",
                "0",
                item,
                data_as_of,
            ));
            continue;
        }
        let profit = item
            .get("managementOperatingProfit")
            .and_then(Value::as_str)
            .and_then(|value| value.parse::<Decimal>().ok())
            .unwrap_or_default();
        let margin = item
            .get("managementOperatingMarginRate")
            .and_then(Value::as_str)
            .and_then(|value| value.parse::<Decimal>().ok());
        if tool == "analyze_order_profit_risks" {
            if profit < Decimal::ZERO {
                findings.push(profit_finding(
                    order_id,
                    currency,
                    "PROFIT-LOSS-001",
                    "order_management_loss",
                    "high",
                    "high",
                    "订单管理经营利润为负",
                    &profit.to_string(),
                    item,
                    data_as_of.clone(),
                ));
            }
            if margin.is_some_and(|value| value < Decimal::new(5, 2)) {
                findings.push(profit_finding(
                    order_id,
                    currency,
                    "PROFIT-MARGIN-002",
                    "order_low_management_margin",
                    "medium",
                    "high",
                    "订单管理经营利润率低于 5%",
                    &profit.to_string(),
                    item,
                    data_as_of,
                ));
            }
        } else if profit < Decimal::ZERO {
            let risky = receivables
                .get("items")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .any(|row| {
                    row.get("salesOrderId").and_then(Value::as_str) == Some(order_id)
                        && (row.get("isOverdue").and_then(Value::as_bool) == Some(true)
                            || row
                                .get("openAmount")
                                .and_then(Value::as_str)
                                .and_then(|value| value.parse::<Decimal>().ok())
                                .is_some_and(|value| value > Decimal::ZERO))
                });
            if risky {
                findings.push(profit_finding(
                    order_id,
                    currency,
                    "CROSS-LOSS-TERM-003",
                    "loss_with_open_receivable",
                    "high",
                    "high",
                    "亏损订单同时存在未结经营应收",
                    &profit.to_string(),
                    item,
                    data_as_of,
                ));
            }
        }
    }
    findings.truncate(100);
    Json(json!({
        "schemaVersion":1,"status":if partial{"partial"}else{"ok"},"runId":Uuid::new_v4(),
        "ruleSetVersion":business_anomaly_contracts::RULE_SET_VERSION,"dataAsOf":chrono::Utc::now(),
        "scopeSummary":{"legalEntityIds":scope.legal_entity_ids,"customerIds":scope.customer_ids,"brandIds":scope.brand_ids,"businessUnitIds":scope.business_unit_ids,"sourceWatermark":source_watermark},
        "totals":{"findingCount":findings.len(),"impactByCurrency":[]},"findings":findings,
        "pagination":{"hasMore":false,"nextCursor":null},"warnings":["经营管理口径，不是法定利润"],"traceId":context.trace_id
    })).into_response()
}

#[allow(clippy::too_many_arguments)]
fn profit_finding(
    order_id: &str,
    currency: &str,
    rule_id: &str,
    kind: &str,
    severity: &str,
    confidence: &str,
    title: &str,
    impact: &str,
    item: &Value,
    data_as_of: Value,
) -> Value {
    json!({"id":Uuid::new_v4(),"type":kind,"severity":severity,"confidence":confidence,"title":title,"summaryCode":rule_id.replace('-',"_"),
        "primaryResource":{"type":"order_profit","id":order_id,"title":"打开订单利润详情","bizUri":format!("biz://order-profit/{order_id}")},"relatedResources":[],"impact":{"amount":impact,"currency":currency},
        "rule":{"id":rule_id,"version":business_anomaly_contracts::RULE_SET_VERSION,"observedValue":impact,"threshold":if rule_id=="PROFIT-MARGIN-002"{"0.05"}else{"0"},"unit":currency},
        "evidence":[{"sourceSystem":"business-core-b4","objectType":"order_profit","objectId":order_id,"objectVersion":"management-profit-v1","updatedAt":data_as_of.clone(),"field":"orderProfit","observedValue":item.to_string(),"threshold":rule_id}],
        "dataAsOf":data_as_of,"warnings":["经营管理口径，不是法定利润"]})
}

async fn core_profit_change_result(
    core: &CoreClient,
    input: &Value,
    scope: &AuthorizationScope,
    context: &RequestContext,
) -> Response {
    let date = |range: &str, edge: &str| input.get(range)?.get(edge)?.as_str().map(str::to_owned);
    let (Some(base_from), Some(base_to), Some(comparison_from), Some(comparison_to)) = (
        date("basePeriod", "from"),
        date("basePeriod", "to"),
        date("comparisonPeriod", "from"),
        date("comparisonPeriod", "to"),
    ) else {
        return (StatusCode::BAD_REQUEST, "invalid_filter").into_response();
    };
    let Ok(mut url) = core.base_url.join("v1/profit-change") else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let currency = input
        .get("currency")
        .and_then(Value::as_str)
        .unwrap_or("CNY");
    url.query_pairs_mut()
        .append_pair("baseFrom", &base_from)
        .append_pair("baseTo", &base_to)
        .append_pair("comparisonFrom", &comparison_from)
        .append_pair("comparisonTo", &comparison_to)
        .append_pair("currency", currency);
    let response = core
        .client
        .get(url)
        .header("x-business-service-credential", &core.credential)
        .header("x-service-audience", "business-core")
        .header(
            "x-enterprise-user-id",
            context.enterprise_user_id.to_string(),
        )
        .header("x-trace-id", context.trace_id.to_string())
        .send()
        .await;
    let Ok(response) = response else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    if !response.status().is_success() {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    let Ok(bridge) = response.json::<Value>().await else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let as_of = bridge
        .get("dataAsOf")
        .cloned()
        .unwrap_or_else(|| json!(chrono::Utc::now()));
    let comparison_label = format!("{comparison_from}_{comparison_to}");
    let impact = bridge.get("change").and_then(Value::as_str).unwrap_or("0");
    let quality = bridge
        .get("dataQualityStatus")
        .and_then(Value::as_str)
        .unwrap_or("partial");
    Json(json!({
        "schemaVersion":1,
        "status":if quality=="complete"{"ok"}else{"partial"},
        "runId":Uuid::new_v4(),
        "ruleSetVersion":business_anomaly_contracts::RULE_SET_VERSION,
        "dataAsOf":as_of,
        "scopeSummary":{"legalEntityIds":scope.legal_entity_ids,"basePeriod":{"from":base_from,"to":base_to},"comparisonPeriod":{"from":comparison_from,"to":comparison_to},"currency":currency,"sourceWatermark":bridge.get("sourceWatermark")},
        "totals":{"findingCount":1,"impactByCurrency":[{"amount":impact,"currency":currency}]},
        "findings":[{
            "id":Uuid::new_v4(),"type":"profit_change_bridge","severity":"info","confidence":"high",
            "title":"管理经营利润期间变动桥接","summaryCode":"PROFIT_BRIDGE_001",
            "primaryResource":{"type":"management_report","id":comparison_label,"title":"打开管理利润报表","bizUri":format!("biz://management-report/{comparison_label}")},
            "relatedResources":[],"impact":{"amount":impact,"currency":currency},
            "rule":{"id":"PROFIT-BRIDGE-001","version":business_anomaly_contracts::RULE_SET_VERSION,"observedValue":impact,"threshold":"0","unit":currency},
            "evidence":[{"sourceSystem":"business-core-b4","objectType":"profit_change_bridge","objectId":comparison_label,"objectVersion":"management-profit-v1","updatedAt":as_of,"field":"components","observedValue":bridge.get("components").cloned().unwrap_or(Value::Null).to_string(),"threshold":"unexplainedDifference=0"}],
            "dataAsOf":as_of,"warnings":["经营管理口径，不是法定利润"]
        }],
        "pagination":{"hasMore":false,"nextCursor":null},
        "warnings":if quality=="complete"{vec!["经营管理口径，不是法定利润"]}else{vec!["利润投影数据质量为 partial","经营管理口径，不是法定利润"]},
        "traceId":context.trace_id
    })).into_response()
}

fn valid_anomaly_input(tool: &str, input: &Value) -> bool {
    let today = chrono::Utc::now().date_naive();
    macro_rules! valid {
        ($kind:ty) => {{
            serde_json::from_value::<$kind>(input.clone())
                .and_then(|mut value| {
                    value
                        .validate(today)
                        .map(|_| value)
                        .map_err(serde::de::Error::custom)
                })
                .is_ok()
        }};
    }
    match tool {
        "search_business_anomalies" => valid!(AnomalyFilterInput),
        "get_business_anomaly" => valid!(GetAnomalyInput),
        "analyze_order_profit_risks" => valid!(ProfitRiskInput),
        "analyze_receivable_risks" => valid!(ReceivableRiskInput),
        "analyze_inventory_risks" => valid!(InventoryRiskInput),
        "analyze_purchase_cost_risks" => valid!(PurchaseRiskInput),
        "analyze_cross_domain_risks" => valid!(CrossDomainRiskInput),
        "explain_profit_change" => valid!(ProfitChangeInput),
        _ => false,
    }
}

fn authorized_service(headers: &HeaderMap, expected: &[u8; 32], audience: &str) -> bool {
    let supplied = headers
        .get("x-business-service-credential")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    let supplied_audience = headers
        .get("x-business-service-audience")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    bool::from(sha256(supplied).ct_eq(expected)) && supplied_audience == audience
}

fn parse_context(headers: &HeaderMap) -> Option<RequestContext> {
    let text = |name: &'static str| headers.get(name)?.to_str().ok().map(str::to_owned);
    Some(RequestContext {
        enterprise_user_id: text("x-enterprise-user-id")?.parse().ok()?,
        identity_binding_id: text("x-identity-binding-id")?.parse().ok()?,
        delegation_id: text("x-agent-delegation-id")?.parse().ok()?,
        agent_id: text("x-agent-id")?,
        agent_turn_id: text("x-agent-turn-id")?,
        trace_id: text("x-trace-id")?.parse().ok()?,
        used_calls: text("x-agent-used-calls")?.parse().ok()?,
        required_scope: text("x-agent-required-scope")?,
    })
}

fn required_capability(tool: &str) -> Option<&'static str> {
    match tool {
        "create_sales_order_draft" => Some("sales_order:create"),
        "create_shipment_draft" => Some("shipment:create"),
        "create_purchase_order_draft" => Some("purchase_order:create"),
        "create_goods_receipt_draft" => Some("goods_receipt:create"),
        "create_customer_receipt_draft" => Some("customer_receipt:create"),
        "create_supplier_payment_draft" => Some("supplier_payment:create"),
        "get_sales_order" | "search_sales_orders" => Some("sales_order:read"),
        "get_purchase_order" | "search_purchase_orders" => Some("purchase_order:read"),
        "query_inventory_balance" => Some("inventory:read"),
        "query_receivables" => Some("receivable:read"),
        "query_payables" => Some("payable:read"),
        "query_order_profit"
        | "query_profitability_by_dimension"
        | "get_management_profit_report"
        | "get_management_report_snapshot"
        | "get_profit_evidence"
        | "get_operating_dashboard"
        | "get_business_data_quality" => Some("order_profit:read"),
        "search_business_anomalies"
        | "get_business_anomaly"
        | "analyze_order_profit_risks"
        | "analyze_receivable_risks"
        | "analyze_inventory_risks"
        | "analyze_purchase_cost_risks"
        | "analyze_cross_domain_risks"
        | "explain_profit_change" => Some("business_anomaly:read"),
        _ => None,
    }
}

fn iam_authorization_scope(
    grant: &EffectiveGrant,
    required_capability: &str,
) -> Option<AuthorizationScope> {
    if grant.capability.as_str() != required_capability {
        return None;
    }
    let DataScope::Restricted(dimensions) = &grant.data_scope else {
        return Some(AuthorizationScope::default());
    };
    let mut scope = AuthorizationScope::default();
    for (dimension, values) in dimensions {
        if values.is_empty() {
            return None;
        }
        let target = match dimension.as_str() {
            "legal_entity" | "legal_entity_id" | "legalEntityIds" => &mut scope.legal_entity_ids,
            "warehouse" | "warehouse_id" | "warehouseIds" => &mut scope.warehouse_ids,
            "customer" | "customer_id" | "customerIds" => &mut scope.customer_ids,
            "supplier" | "supplier_id" | "supplierIds" => &mut scope.supplier_ids,
            "brand" | "brand_id" | "brandIds" => &mut scope.brand_ids,
            "business_unit" | "business_unit_id" | "businessUnitIds" => {
                &mut scope.business_unit_ids
            }
            _ => return None,
        };
        target.extend(values.iter().cloned());
    }
    Some(scope)
}

fn requested_scope(input: &Value) -> AuthorizationScope {
    let values = |key: &str| {
        input
            .get(key)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect()
    };
    AuthorizationScope {
        legal_entity_ids: values("legalEntityIds"),
        warehouse_ids: values("warehouseIds"),
        customer_ids: values("customerIds"),
        supplier_ids: values("supplierIds"),
        brand_ids: values("brandIds"),
        business_unit_ids: values("businessUnitIds"),
    }
}

fn anomaly_result(
    analytics: &BusinessAnalyticsService,
    tool: &str,
    input: &Value,
    scope: &AuthorizationScope,
    trace_id: Uuid,
    max_findings: usize,
) -> Response {
    let limit = input
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(20)
        .min(max_findings as u64) as usize;
    if tool == "get_business_anomaly" {
        let Some(id) = input
            .get("findingId")
            .and_then(Value::as_str)
            .and_then(|v| v.parse().ok())
        else {
            return (StatusCode::BAD_REQUEST, "invalid_filter").into_response();
        };
        return analytics.finding(id, scope, trace_id).map_or_else(
            || StatusCode::NOT_FOUND.into_response(),
            |value| Json(value).into_response(),
        );
    }
    let domain = match tool {
        "analyze_order_profit_risks" => AnalysisDomain::Profit,
        "analyze_receivable_risks" => AnalysisDomain::Receivable,
        "analyze_inventory_risks" => AnalysisDomain::Inventory,
        "analyze_purchase_cost_risks" => AnalysisDomain::Purchase,
        "analyze_cross_domain_risks" => AnalysisDomain::CrossDomain,
        "explain_profit_change" => AnalysisDomain::ProfitChange,
        _ => AnalysisDomain::All,
    };
    let mut result = analytics.analyze(domain, scope, trace_id, 100);
    result
        .findings
        .retain(|finding| semantic_filter_matches(finding, input, analytics.dataset()));
    let offset = match input.get("cursor").and_then(Value::as_str) {
        Some(value) => match value
            .strip_prefix("offset:")
            .and_then(|value| value.parse::<usize>().ok())
        {
            Some(value) if value <= result.findings.len() => value,
            _ => return (StatusCode::BAD_REQUEST, "invalid_filter").into_response(),
        },
        None => 0,
    };
    let total_count = result.findings.len();
    let end = offset.saturating_add(limit).min(total_count);
    result.findings = result.findings[offset..end].to_vec();
    result.totals.finding_count = total_count;
    result.totals.impact_by_currency = impact_totals(&result.findings);
    result.pagination = Some(Pagination {
        has_more: end < total_count,
        next_cursor: (end < total_count).then(|| format!("offset:{end}")),
    });
    Json(result).into_response()
}

fn semantic_filter_matches(
    finding: &BusinessAnomaly,
    input: &Value,
    dataset: &BusinessDataset,
) -> bool {
    let requested = |key: &str| {
        input
            .get(key)
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>())
            .filter(|items| !items.is_empty())
    };
    if input.get("active").and_then(Value::as_bool) == Some(false) {
        return false;
    }
    if requested("anomalyTypes").is_some_and(|items| !items.contains(&finding.r#type.as_str())) {
        return false;
    }
    let severity = match finding.severity {
        business_anomaly_contracts::Severity::Info => "info",
        business_anomaly_contracts::Severity::Low => "low",
        business_anomaly_contracts::Severity::Medium => "medium",
        business_anomaly_contracts::Severity::High => "high",
        business_anomaly_contracts::Severity::Critical => "critical",
    };
    if requested("severities").is_some_and(|items| !items.contains(&severity)) {
        return false;
    }
    if requested("skuIds").is_some_and(|items| {
        !finding_skus(finding, dataset)
            .iter()
            .any(|value| items.contains(value))
    }) {
        return false;
    }
    if requested("salespersonIds").is_some_and(|items| {
        finding_salesperson(finding, dataset).is_none_or(|value| !items.contains(&value))
    }) {
        return false;
    }
    if let Some(minimum) = input.get("overdueDaysMin").and_then(Value::as_u64) {
        if finding_overdue_days(finding, dataset).is_none_or(|days| u64::from(days) < minimum) {
            return false;
        }
    }
    if let Some(range) = input.get("dateRange") {
        let date = finding_business_date(finding, dataset);
        let from = range
            .get("from")
            .and_then(Value::as_str)
            .and_then(|value| chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d").ok());
        let to = range
            .get("to")
            .and_then(Value::as_str)
            .and_then(|value| chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d").ok());
        if from.is_some_and(|from| date < from) || to.is_some_and(|to| date > to) {
            return false;
        }
    }
    true
}

fn finding_skus<'a>(finding: &'a BusinessAnomaly, dataset: &'a BusinessDataset) -> Vec<&'a str> {
    std::iter::once(&finding.primary_resource)
        .chain(finding.related_resources.iter())
        .filter_map(|resource| {
            let id = resource.id.as_deref()?;
            match resource.r#type.as_str() {
                "inventory" => Some(id),
                "sales_order" => dataset
                    .sales_orders
                    .iter()
                    .find(|order| order.identity.object_id == id)
                    .map(|order| order.sku_id.as_str()),
                "purchase_order" => dataset
                    .purchase_orders
                    .iter()
                    .find(|order| order.identity.object_id == id)
                    .map(|order| order.sku_id.as_str()),
                _ => None,
            }
        })
        .collect()
}

fn finding_salesperson<'a>(
    finding: &BusinessAnomaly,
    dataset: &'a BusinessDataset,
) -> Option<&'a str> {
    let id = finding.primary_resource.id.as_deref()?;
    dataset
        .order_profits
        .iter()
        .find(|profit| profit.sales_order_id == id)
        .map(|profit| profit.salesperson_id.as_str())
        .or_else(|| {
            dataset
                .sales_orders
                .iter()
                .find(|order| order.identity.object_id == id)
                .map(|order| order.salesperson_id.as_str())
        })
}

fn finding_overdue_days(finding: &BusinessAnomaly, dataset: &BusinessDataset) -> Option<u32> {
    let id = finding.primary_resource.id.as_deref()?;
    if finding.primary_resource.r#type == "customer" {
        return dataset
            .receivables
            .iter()
            .filter(|receivable| receivable.customer_id == id)
            .map(|receivable| receivable.overdue_days)
            .max();
    }
    dataset
        .sales_orders
        .iter()
        .find(|order| order.identity.object_id == id)
        .map(|order| order.days_since_due)
}

fn finding_business_date(
    finding: &BusinessAnomaly,
    dataset: &BusinessDataset,
) -> chrono::NaiveDate {
    let id = finding.primary_resource.id.as_deref().unwrap_or_default();
    dataset
        .sales_orders
        .iter()
        .find(|order| order.identity.object_id == id)
        .map(|order| order.ordered_at)
        .or_else(|| {
            dataset
                .purchase_orders
                .iter()
                .find(|order| order.identity.object_id == id)
                .map(|order| order.ordered_at)
        })
        .unwrap_or_else(|| finding.data_as_of.date_naive())
}

fn impact_totals(findings: &[BusinessAnomaly]) -> Vec<business_query_contracts::Money> {
    let mut totals = BTreeMap::<String, rust_decimal::Decimal>::new();
    for impact in findings
        .iter()
        .filter_map(|finding| finding.impact.as_ref())
    {
        if let Ok(amount) = rust_decimal::Decimal::from_str(&impact.amount) {
            *totals.entry(impact.currency.clone()).or_default() += amount;
        }
    }
    totals
        .into_iter()
        .map(|(currency, amount)| business_query_contracts::Money {
            amount: amount.to_string(),
            currency,
        })
        .collect()
}

fn legacy_read_result(
    analytics: &BusinessAnalyticsService,
    tool: &str,
    input: &Value,
    scope: &AuthorizationScope,
    trace_id: Uuid,
) -> Response {
    let data = analytics.dataset();
    let mut items = match tool {
        "get_sales_order" | "search_sales_orders" => data
            .sales_orders
            .iter()
            .filter(|v| {
                scope.allows_legal_entity(&v.identity.legal_entity_id)
                    && scope.allows_customer(&v.customer_id)
                    && scope.allows_warehouse(&v.warehouse_id)
                    && scope.allows_brand(&v.brand_id)
            })
            .filter(|v| {
                input
                    .get("orderId")
                    .and_then(Value::as_str)
                    .is_none_or(|id| id == v.identity.object_id)
            })
            .filter_map(|v| serde_json::to_value(v).ok())
            .collect::<Vec<_>>(),
        "get_purchase_order" | "search_purchase_orders" => data
            .purchase_orders
            .iter()
            .filter(|v| {
                scope.allows_legal_entity(&v.identity.legal_entity_id)
                    && scope.allows_supplier(&v.supplier_id)
                    && scope.allows_warehouse(&v.warehouse_id)
                    && scope.allows_brand(&v.brand_id)
            })
            .filter(|v| {
                input
                    .get("orderId")
                    .and_then(Value::as_str)
                    .is_none_or(|id| id == v.identity.object_id)
            })
            .filter_map(|v| serde_json::to_value(v).ok())
            .collect(),
        "query_inventory_balance" => data
            .inventory
            .iter()
            .filter(|v| {
                scope.allows_legal_entity(&v.identity.legal_entity_id)
                    && scope.allows_warehouse(&v.warehouse_id)
                    && scope.allows_brand(&v.brand_id)
            })
            .filter_map(|v| serde_json::to_value(v).ok())
            .collect(),
        "query_receivables" => data
            .receivables
            .iter()
            .filter(|v| {
                scope.allows_legal_entity(&v.identity.legal_entity_id)
                    && scope.allows_customer(&v.customer_id)
            })
            .filter_map(|v| serde_json::to_value(v).ok())
            .collect(),
        "query_payables" => data
            .payables
            .iter()
            .filter(|v| {
                scope.allows_legal_entity(&v.identity.legal_entity_id)
                    && scope.allows_supplier(&v.supplier_id)
            })
            .filter_map(|v| serde_json::to_value(v).ok())
            .collect(),
        "query_order_profit" => data
            .order_profits
            .iter()
            .filter(|v| {
                scope.allows_legal_entity(&v.identity.legal_entity_id)
                    && scope.allows_customer(&v.customer_id)
                    && scope.allows_brand(&v.brand_id)
            })
            .filter_map(|v| serde_json::to_value(v).ok())
            .collect(),
        _ => Vec::new(),
    };
    let exact = tool.starts_with("get_");
    if exact && items.is_empty() {
        return StatusCode::NOT_FOUND.into_response();
    }
    let limit = input
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(20)
        .min(100) as usize;
    let has_more = items.len() > limit;
    items.truncate(limit);
    let refs = items
        .iter()
        .filter_map(|item| item.get("objectId").and_then(Value::as_str))
        .map(|id| {
            let slug = if tool.contains("purchase") {
                "purchase-order"
            } else if tool.contains("inventory") {
                "inventory"
            } else {
                "sales-order"
            };
            ResourceRef {
                r#type: slug.replace('-', "_"),
                id: Some(id.into()),
                title: format!("打开 {id}"),
                biz_uri: format!("biz://{slug}/{id}"),
            }
        })
        .collect();
    let result = BusinessToolResult {
        schema_version: 1,
        status: BusinessToolStatus::Ok,
        as_of: data.data_as_of,
        scope_summary: ScopeSummary {
            legal_entity_ids: scope.legal_entity_ids.iter().cloned().collect(),
            period: Some("desensitized acceptance snapshot".into()),
            currency: Some("CNY".into()),
        },
        summary: BTreeMap::from([
            ("effectiveScopeHash".into(), json!(scope.hash())),
            ("dataClassification".into(), json!(data.classification)),
        ]),
        items,
        pagination: Some(Pagination {
            next_cursor: has_more.then(|| "page:2".into()),
            has_more,
        }),
        resource_refs: refs,
        evidence: vec![Evidence {
            source_system: "acceptance-business-system".into(),
            object_type: tool.into(),
            object_id: data.dataset_id.clone(),
            version: Some("desensitized-v1".into()),
            updated_at: data.data_as_of,
        }],
        warnings: vec![],
        trace_id,
    };
    Json(result).into_response()
}

async fn core_read_result(
    core: &CoreClient,
    tool: &str,
    input: &Value,
    scope: &AuthorizationScope,
    context: &RequestContext,
) -> Response {
    let endpoint = match tool {
        "get_sales_order" | "search_sales_orders" => "v1/sales-orders",
        "query_inventory_balance" => "v1/inventory-balances",
        "query_receivables" => "v1/trade-receivables",
        "get_purchase_order" | "search_purchase_orders" => "v1/purchase-orders",
        "query_payables" => "v1/trade-payables",
        "query_order_profit" => "v1/order-profits",
        "query_profitability_by_dimension" => "v1/profitability",
        "get_management_profit_report" => "v1/management-profit-report",
        "get_management_report_snapshot" => "v1/management-report-snapshots",
        "get_profit_evidence" => "v1/profit-evidence",
        "get_operating_dashboard" => "v1/operations/dashboard",
        "get_business_data_quality" => "v1/operations/data-quality",
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    let Ok(mut url) = core.base_url.join(endpoint) else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    if tool == "get_management_report_snapshot" {
        let Some(id) = input
            .get("snapshotId")
            .and_then(Value::as_str)
            .and_then(|value| Uuid::parse_str(value).ok())
        else {
            return (StatusCode::BAD_REQUEST, "invalid_filter").into_response();
        };
        let Ok(joined) = core
            .base_url
            .join(&format!("v1/management-report-snapshots/{id}"))
        else {
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        };
        url = joined;
    } else if tool == "get_profit_evidence" {
        let Some(id) = input
            .get("orderId")
            .and_then(Value::as_str)
            .and_then(|value| Uuid::parse_str(value).ok())
        else {
            return (StatusCode::BAD_REQUEST, "invalid_filter").into_response();
        };
        let Ok(joined) = core.base_url.join(&format!("v1/profit-evidence/{id}")) else {
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        };
        url = joined;
    }
    if matches!(
        tool,
        "query_order_profit"
            | "query_profitability_by_dimension"
            | "get_management_profit_report"
            | "get_operating_dashboard"
    ) {
        let mut query = url.query_pairs_mut();
        for (input_key, query_key) in [
            ("orderId", "orderId"),
            ("managementPeriod", "managementPeriod"),
            ("currency", "currency"),
            ("dimensionOne", "dimensionOne"),
            ("dimensionTwo", "dimensionTwo"),
            ("limit", "limit"),
        ] {
            if let Some(value) = input.get(input_key) {
                if let Some(value) = value.as_str() {
                    if input_key == "orderId" && Uuid::parse_str(value).is_err() {
                        query.append_pair("orderNumber", value);
                        continue;
                    }
                    query.append_pair(query_key, value);
                } else if let Some(value) = value.as_u64() {
                    query.append_pair(query_key, &value.to_string());
                }
            }
        }
    }
    let response = core
        .client
        .get(url)
        .header("x-business-service-credential", &core.credential)
        .header("x-service-audience", "business-core")
        .header(
            "x-enterprise-user-id",
            context.enterprise_user_id.to_string(),
        )
        .header("x-trace-id", context.trace_id.to_string())
        .send()
        .await;
    let Ok(response) = response else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return StatusCode::NOT_FOUND.into_response();
    }
    if !response.status().is_success() {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    let Ok(mut envelope) = response.json::<Value>().await else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let mut items = envelope
        .get_mut("items")
        .and_then(Value::as_array_mut)
        .map(std::mem::take)
        .unwrap_or_default();
    if items.is_empty()
        && matches!(
            tool,
            "get_management_profit_report"
                | "get_operating_dashboard"
                | "get_business_data_quality"
        )
    {
        items.push(envelope.clone());
    } else if items.is_empty() && tool == "get_profit_evidence" {
        items = envelope
            .get_mut("facts")
            .and_then(Value::as_array_mut)
            .map(std::mem::take)
            .unwrap_or_default();
    }
    if let Some(exact) = input.get("orderId").and_then(Value::as_str) {
        items.retain(|item| {
            item.get("id").and_then(Value::as_str) == Some(exact)
                || item.get("salesOrderId").and_then(Value::as_str) == Some(exact)
                || item.get("orderNumber").and_then(Value::as_str) == Some(exact)
                || item.get("purchaseOrderNumber").and_then(Value::as_str) == Some(exact)
        });
    }
    if tool == "query_order_profit" {
        let permits = |values: &std::collections::BTreeSet<String>, item: &Value, key: &str| {
            values.is_empty()
                || item
                    .get(key)
                    .and_then(Value::as_str)
                    .is_some_and(|value| values.contains(value))
        };
        items.retain(|item| {
            permits(&scope.legal_entity_ids, item, "legalEntityId")
                && permits(&scope.customer_ids, item, "customerId")
                && permits(&scope.brand_ids, item, "brandId")
                && permits(&scope.business_unit_ids, item, "businessUnitId")
        });
    }
    let exact = tool.starts_with("get_");
    if exact && items.is_empty() {
        return StatusCode::NOT_FOUND.into_response();
    }
    let limit = input
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(20)
        .min(100) as usize;
    let has_more = items.len() > limit;
    items.truncate(limit);
    let refs = items
        .iter()
        .filter_map(|item| resource_ref(tool, item))
        .collect();
    let as_of = envelope
        .get("dataAsOf")
        .and_then(Value::as_str)
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&chrono::Utc))
        .unwrap_or_else(chrono::Utc::now);
    Json(BusinessToolResult {
        schema_version: 1,
        status: BusinessToolStatus::Ok,
        as_of,
        scope_summary: ScopeSummary {
            legal_entity_ids: scope.legal_entity_ids.iter().cloned().collect(),
            period: input
                .get("managementPeriod")
                .and_then(Value::as_str)
                .map(str::to_owned),
            currency: input
                .get("currency")
                .and_then(Value::as_str)
                .map(str::to_owned),
        },
        summary: BTreeMap::from([
            ("effectiveScopeHash".into(), json!(scope.hash())),
            (
                "source".into(),
                json!(if matches!(
                    tool,
                    "get_operating_dashboard" | "get_business_data_quality"
                ) {
                    "business-core-s1"
                } else if matches!(
                    tool,
                    "query_order_profit"
                        | "query_profitability_by_dimension"
                        | "get_management_profit_report"
                        | "get_management_report_snapshot"
                        | "get_profit_evidence"
                ) {
                    "business-core-b4"
                } else if tool.contains("purchase") || tool.contains("payable") {
                    "business-core-b3"
                } else {
                    "business-core-b2"
                }),
            ),
            (
                "ruleVersion".into(),
                envelope.get("ruleVersion").cloned().unwrap_or(Value::Null),
            ),
            (
                "sourceWatermark".into(),
                envelope
                    .get("sourceWatermark")
                    .cloned()
                    .unwrap_or(Value::Null),
            ),
        ]),
        items,
        pagination: Some(Pagination {
            next_cursor: has_more.then(|| "page:2".into()),
            has_more,
        }),
        resource_refs: refs,
        evidence: vec![Evidence {
            source_system: if matches!(
                tool,
                "get_operating_dashboard" | "get_business_data_quality"
            ) {
                "business-core-s1".into()
            } else if matches!(
                tool,
                "query_order_profit"
                    | "query_profitability_by_dimension"
                    | "get_management_profit_report"
                    | "get_management_report_snapshot"
                    | "get_profit_evidence"
            ) {
                "business-core-b4".into()
            } else if tool.contains("purchase") || tool.contains("payable") {
                "business-core-b3".into()
            } else {
                "business-core-b2".into()
            },
            object_type: tool.into(),
            object_id: "authoritative-postgresql".into(),
            version: Some(
                if matches!(
                    tool,
                    "get_operating_dashboard" | "get_business_data_quality"
                ) {
                    "S1".into()
                } else if matches!(
                    tool,
                    "query_order_profit"
                        | "query_profitability_by_dimension"
                        | "get_management_profit_report"
                        | "get_management_report_snapshot"
                        | "get_profit_evidence"
                ) {
                    "B4".into()
                } else if tool.contains("purchase") || tool.contains("payable") {
                    "B3".into()
                } else {
                    "B2".into()
                },
            ),
            updated_at: as_of,
        }],
        warnings: envelope
            .get("warnings")
            .and_then(Value::as_array)
            .map(|warnings| {
                warnings
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
        trace_id: context.trace_id,
    })
    .into_response()
}

fn resource_ref(tool: &str, item: &Value) -> Option<ResourceRef> {
    if tool == "get_operating_dashboard" {
        return Some(ResourceRef {
            r#type: "operations_dashboard".into(),
            id: None,
            title: "经营驾驶舱".into(),
            biz_uri: "biz://operations-dashboard".into(),
        });
    }
    if tool == "get_business_data_quality" {
        return Some(ResourceRef {
            r#type: "data_quality".into(),
            id: None,
            title: "业务数据质量".into(),
            biz_uri: "biz://data-quality".into(),
        });
    }
    if matches!(
        tool,
        "query_profitability_by_dimension" | "get_management_profit_report" | "get_profit_evidence"
    ) {
        return None;
    }
    let (kind, id, title) = if tool == "query_order_profit" {
        (
            "order-profit",
            item.get("salesOrderId")?.as_str()?,
            "订单真实利润",
        )
    } else if tool == "get_management_report_snapshot" {
        (
            "management-report",
            item.get("id")?.as_str()?,
            "管理利润报表快照",
        )
    } else if tool == "get_profit_evidence" {
        ("profit-evidence", item.get("id")?.as_str()?, "利润事实凭据")
    } else if tool == "query_profitability_by_dimension" {
        (
            "profitability",
            item.get("dimensionOneId")?.as_str()?,
            "盈利分析",
        )
    } else if tool == "get_management_profit_report" {
        (
            "management-report-current",
            item.get("managementPeriod")?.as_str()?,
            "当前管理利润报表",
        )
    } else if tool.contains("sales_order") {
        (
            "sales-order",
            item.get("id")?.as_str()?,
            item.get("orderNumber")?.as_str()?,
        )
    } else if tool.contains("purchase_order") {
        (
            "purchase-order",
            item.get("id")?.as_str()?,
            item.get("purchaseOrderNumber")?.as_str()?,
        )
    } else if tool.contains("inventory") {
        ("inventory", item.get("skuId")?.as_str()?, "库存台账")
    } else if tool.contains("payable") {
        ("supplier", item.get("supplierId")?.as_str()?, "供应商应付")
    } else {
        ("customer", item.get("customerId")?.as_str()?, "客户应收")
    };
    let biz_uri = if kind == "customer" {
        format!("biz://customer/{id}/receivables")
    } else if kind == "supplier" {
        format!("biz://supplier/{id}/payables")
    } else {
        format!("biz://{kind}/{id}")
    };
    Some(ResourceRef {
        r#type: kind.replace('-', "_"),
        id: Some(id.into()),
        title: title.into(),
        biz_uri,
    })
}

/// Run the production API. B2 reads use Business Core; the bundled dataset is
/// retained for the pre-existing anomaly acceptance path and tests.
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().with_ansi(false).init();
    let config = Config::from_env().map_err(|e| format!("configuration error: {e}"))?;
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(5))
        .build()?;
    let router = router_with_runtime(
        config.service_credential.clone(),
        DelegationVerifier::Gateway {
            client,
            url: config.gateway_base_url,
            credential: config.service_credential,
        },
        config.rule_config,
        config.max_findings,
        config.max_payload_bytes,
        Some(CoreClient {
            client: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(2))
                .timeout(Duration::from_secs(5))
                .build()?,
            base_url: config.core_base_url,
            credential: config.core_credential,
        }),
    )?;
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(async move {
            let listener = tokio::net::TcpListener::bind(config.bind).await?;
            axum::serve(listener, router).await?;
            Ok::<_, Box<dyn std::error::Error>>(())
        })
}

#[cfg(test)]
mod tests;
