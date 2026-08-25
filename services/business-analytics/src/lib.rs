#![forbid(unsafe_code)]

mod rules;

use business_anomaly_contracts::{
    AnomalyStatus, AnomalyTotals, BusinessAnomaly, BusinessAnomalyResult, InventoryPosition,
    OrderProfitFact, PayablePosition, PurchaseOrderFact, ReceivablePosition, SalesOrderFact,
};
use business_query_contracts::Money;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

const DENY_ALL_SCOPE_VALUE: &str = "__buzz_scope_deny_all__";

pub use rules::{AnalysisDomain, RuleConfig};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BusinessDataset {
    pub dataset_id: String,
    pub classification: String,
    pub data_as_of: DateTime<Utc>,
    pub sales_orders: Vec<SalesOrderFact>,
    pub purchase_orders: Vec<PurchaseOrderFact>,
    pub inventory: Vec<InventoryPosition>,
    pub receivables: Vec<ReceivablePosition>,
    pub payables: Vec<PayablePosition>,
    pub order_profits: Vec<OrderProfitFact>,
    /// Adversarial source strings used only to prove that ingestion does not
    /// promote business text into facts, findings, links, or instructions.
    #[serde(default)]
    pub untrusted_text_fixtures: Vec<String>,
}

impl BusinessDataset {
    /// Load the versioned, desensitized acceptance dataset bundled with this crate.
    pub fn desensitized_acceptance() -> Result<Self, AnalyticsError> {
        serde_json::from_str(include_str!("../fixtures/desensitized-v1.json"))
            .map_err(AnalyticsError::Dataset)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuthorizationScope {
    pub legal_entity_ids: BTreeSet<String>,
    pub warehouse_ids: BTreeSet<String>,
    pub customer_ids: BTreeSet<String>,
    pub supplier_ids: BTreeSet<String>,
    pub brand_ids: BTreeSet<String>,
    pub business_unit_ids: BTreeSet<String>,
}

impl AuthorizationScope {
    pub fn intersect(&self, requested: &Self) -> Self {
        Self {
            legal_entity_ids: intersect_dimension(
                &self.legal_entity_ids,
                &requested.legal_entity_ids,
            ),
            warehouse_ids: intersect_dimension(&self.warehouse_ids, &requested.warehouse_ids),
            customer_ids: intersect_dimension(&self.customer_ids, &requested.customer_ids),
            supplier_ids: intersect_dimension(&self.supplier_ids, &requested.supplier_ids),
            brand_ids: intersect_dimension(&self.brand_ids, &requested.brand_ids),
            business_unit_ids: intersect_dimension(
                &self.business_unit_ids,
                &requested.business_unit_ids,
            ),
        }
    }

    pub fn hash(&self) -> String {
        let encoded = serde_json::to_vec(self).unwrap_or_default();
        hex::encode(Sha256::digest(encoded))
    }

    pub fn allows_legal_entity(&self, value: &str) -> bool {
        self.legal_entity_ids.is_empty() || self.legal_entity_ids.contains(value)
    }
    pub fn allows_warehouse(&self, value: &str) -> bool {
        self.warehouse_ids.is_empty() || self.warehouse_ids.contains(value)
    }
    pub fn allows_customer(&self, value: &str) -> bool {
        self.customer_ids.is_empty() || self.customer_ids.contains(value)
    }
    pub fn allows_supplier(&self, value: &str) -> bool {
        self.supplier_ids.is_empty() || self.supplier_ids.contains(value)
    }
    pub fn allows_brand(&self, value: &str) -> bool {
        self.brand_ids.is_empty() || self.brand_ids.contains(value)
    }
}

fn intersect_dimension(
    authorized: &BTreeSet<String>,
    requested: &BTreeSet<String>,
) -> BTreeSet<String> {
    if requested.is_empty() {
        return authorized.clone();
    }
    if authorized.is_empty() {
        return requested.clone();
    }
    let intersection = authorized
        .intersection(requested)
        .cloned()
        .collect::<BTreeSet<_>>();
    if intersection.is_empty() {
        BTreeSet::from([DENY_ALL_SCOPE_VALUE.to_owned()])
    } else {
        intersection
    }
}

#[derive(Debug, Clone)]
pub struct BusinessAnalyticsService {
    dataset: BusinessDataset,
    config: RuleConfig,
}

impl BusinessAnalyticsService {
    pub fn new(dataset: BusinessDataset, config: RuleConfig) -> Result<Self, AnalyticsError> {
        config.validate()?;
        Ok(Self { dataset, config })
    }

    pub fn dataset(&self) -> &BusinessDataset {
        &self.dataset
    }

    pub fn analyze(
        &self,
        domain: AnalysisDomain,
        scope: &AuthorizationScope,
        trace_id: Uuid,
        limit: usize,
    ) -> BusinessAnomalyResult {
        let mut findings = rules::evaluate(&self.dataset, &self.config, domain, scope, trace_id);
        findings.sort_by(|a, b| b.severity.cmp(&a.severity).then_with(|| a.id.cmp(&b.id)));
        let total_count = findings.len();
        let has_more = total_count > limit;
        findings.truncate(limit.min(100));
        let warnings = findings
            .iter()
            .flat_map(|finding| finding.warnings.iter().cloned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let mut warnings = warnings;
        let age_minutes = Utc::now()
            .signed_duration_since(self.dataset.data_as_of)
            .num_minutes()
            .max(0);
        if age_minutes > self.config.stale_after_minutes {
            warnings.push("STALE_SOURCE_DATA".into());
        }
        let status = if warnings.is_empty() {
            AnomalyStatus::Ok
        } else {
            AnomalyStatus::Partial
        };
        BusinessAnomalyResult {
            schema_version: 1,
            status,
            run_id: Uuid::new_v4(),
            rule_set_version: self.config.version.clone(),
            data_as_of: self.dataset.data_as_of,
            scope_summary: BTreeMap::from([
                (
                    "effectiveScopeHash".into(),
                    serde_json::Value::String(scope.hash()),
                ),
                (
                    "datasetId".into(),
                    serde_json::Value::String(self.dataset.dataset_id.clone()),
                ),
                (
                    "classification".into(),
                    serde_json::Value::String(self.dataset.classification.clone()),
                ),
            ]),
            totals: AnomalyTotals {
                finding_count: total_count,
                impact_by_currency: sum_impact(&findings),
            },
            findings,
            pagination: Some(business_query_contracts::Pagination {
                has_more,
                next_cursor: has_more.then(|| format!("run:{total_count}:page:2")),
            }),
            warnings,
            trace_id,
        }
    }

    pub fn finding(
        &self,
        finding_id: Uuid,
        scope: &AuthorizationScope,
        trace_id: Uuid,
    ) -> Option<BusinessAnomalyResult> {
        let mut result = self.analyze(AnalysisDomain::All, scope, trace_id, 100);
        result.findings.retain(|finding| finding.id == finding_id);
        if result.findings.is_empty() {
            return None;
        }
        result.totals.finding_count = 1;
        result.totals.impact_by_currency = sum_impact(&result.findings);
        Some(result)
    }
}

fn sum_impact(findings: &[BusinessAnomaly]) -> Vec<Money> {
    use rust_decimal::Decimal;
    use std::str::FromStr;
    let mut totals = BTreeMap::<String, Decimal>::new();
    for impact in findings
        .iter()
        .filter_map(|finding| finding.impact.as_ref())
    {
        if let Ok(amount) = Decimal::from_str(&impact.amount) {
            *totals.entry(impact.currency.clone()).or_default() += amount;
        }
    }
    totals
        .into_iter()
        .map(|(currency, amount)| Money {
            amount: amount.to_string(),
            currency,
        })
        .collect()
}

#[derive(Debug, thiserror::Error)]
pub enum AnalyticsError {
    #[error("invalid dataset: {0}")]
    Dataset(serde_json::Error),
    #[error("invalid rule configuration file: {0}")]
    RuleConfigFile(serde_json::Error),
    #[error("invalid rule configuration: {0}")]
    RuleConfig(&'static str),
}

pub const ACCEPTANCE_FINANCE_USER: Uuid = Uuid::from_u128(0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa1);
pub const ACCEPTANCE_SALES_USER: Uuid = Uuid::from_u128(0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb1);

pub fn acceptance_scope(user_id: Uuid) -> Option<AuthorizationScope> {
    let set = |values: &[&str]| values.iter().map(|value| (*value).to_string()).collect();
    match user_id {
        ACCEPTANCE_FINANCE_USER => Some(AuthorizationScope {
            legal_entity_ids: set(&["LE-A", "LE-B"]),
            warehouse_ids: set(&["W01", "W02"]),
            customer_ids: set(&["C001", "C002", "C003"]),
            supplier_ids: set(&["S001", "S002"]),
            brand_ids: set(&["BR-A", "BR-B"]),
            business_unit_ids: set(&["BU-FIN", "BU-SALES"]),
        }),
        ACCEPTANCE_SALES_USER => Some(AuthorizationScope {
            legal_entity_ids: set(&["LE-A"]),
            warehouse_ids: set(&["W01"]),
            customer_ids: set(&["C001", "C002"]),
            supplier_ids: set(&["S001"]),
            brand_ids: set(&["BR-A"]),
            business_unit_ids: set(&["BU-SALES"]),
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desensitized_dataset_is_explicitly_classified() {
        let dataset = BusinessDataset::desensitized_acceptance().expect("fixture");
        assert_eq!(dataset.classification, "desensitized-test-data");
        assert!(!dataset.order_profits.is_empty());
        assert_eq!(dataset.untrusted_text_fixtures.len(), 3);
    }

    #[test]
    fn different_users_receive_different_scopes_and_results() {
        let service = BusinessAnalyticsService::new(
            BusinessDataset::desensitized_acceptance().expect("fixture"),
            RuleConfig::bundled().expect("bundled rules"),
        )
        .expect("service");
        let finance = acceptance_scope(ACCEPTANCE_FINANCE_USER).expect("finance");
        let sales = acceptance_scope(ACCEPTANCE_SALES_USER).expect("sales");
        assert_ne!(finance.hash(), sales.hash());
        let finance_result = service.analyze(AnalysisDomain::All, &finance, Uuid::new_v4(), 100);
        let sales_result = service.analyze(AnalysisDomain::All, &sales, Uuid::new_v4(), 100);
        assert!(finance_result.totals.finding_count > sales_result.totals.finding_count);
    }

    #[test]
    fn requested_scope_is_intersected_across_five_dimensions() {
        let authorized = acceptance_scope(ACCEPTANCE_SALES_USER).expect("sales");
        let requested = AuthorizationScope {
            legal_entity_ids: ["LE-A", "LE-B"].into_iter().map(String::from).collect(),
            warehouse_ids: ["W01", "W02"].into_iter().map(String::from).collect(),
            customer_ids: ["C001", "C003"].into_iter().map(String::from).collect(),
            supplier_ids: ["S001", "S002"].into_iter().map(String::from).collect(),
            brand_ids: ["BR-A", "BR-B"].into_iter().map(String::from).collect(),
            business_unit_ids: BTreeSet::new(),
        };
        let effective = authorized.intersect(&requested);
        assert_eq!(
            effective.legal_entity_ids,
            ["LE-A"].into_iter().map(String::from).collect()
        );
        assert_eq!(
            effective.warehouse_ids,
            ["W01"].into_iter().map(String::from).collect()
        );
        assert_eq!(
            effective.customer_ids,
            ["C001"].into_iter().map(String::from).collect()
        );
        assert_eq!(
            effective.supplier_ids,
            ["S001"].into_iter().map(String::from).collect()
        );
        assert_eq!(
            effective.brand_ids,
            ["BR-A"].into_iter().map(String::from).collect()
        );
    }

    #[test]
    fn disjoint_requested_scope_fails_closed_instead_of_becoming_wildcard() {
        let authorized = acceptance_scope(ACCEPTANCE_SALES_USER).expect("scope");
        let requested = AuthorizationScope {
            legal_entity_ids: BTreeSet::from(["LE-B".into()]),
            warehouse_ids: BTreeSet::from(["W02".into()]),
            ..AuthorizationScope::default()
        };
        let effective = authorized.intersect(&requested);
        assert!(!effective.allows_legal_entity("LE-A"));
        assert!(!effective.allows_legal_entity("LE-B"));
        assert!(!effective.allows_warehouse("W01"));
        assert!(!effective.allows_warehouse("W02"));
    }

    #[test]
    fn rule_set_version_is_stable() {
        let config = RuleConfig::bundled().expect("bundled rules");
        config.validate().expect("valid rules");
        assert_eq!(config.version, business_anomaly_contracts::RULE_SET_VERSION);
        assert_eq!(config.purchase_price_min_samples, 3);
    }

    #[test]
    fn source_prompt_injection_text_never_reaches_anomaly_output() {
        let dataset = BusinessDataset::desensitized_acceptance().expect("fixture");
        let injected = dataset.untrusted_text_fixtures.clone();
        let service =
            BusinessAnalyticsService::new(dataset, RuleConfig::bundled().expect("bundled rules"))
                .expect("service");
        let result = service.analyze(
            AnalysisDomain::All,
            &acceptance_scope(ACCEPTANCE_FINANCE_USER).expect("scope"),
            Uuid::new_v4(),
            100,
        );
        let encoded = serde_json::to_string(&result).expect("result json");
        for value in injected {
            assert!(!encoded.contains(&value));
        }
        assert!(!encoded.contains("javascript:"));
    }
}
