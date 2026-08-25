use crate::{bundled_catalog, ActionEngine, ActionError, Actor, FindingObservation};
use business_action_contracts::{
    FindingScope, ACTION_PROPOSAL_READ, APPROVAL_DRAFT_CREATE, BUSINESS_ACTION_READ,
    FINDING_ACKNOWLEDGE, FINDING_READ, WORK_ITEM_ASSIGN, WORK_ITEM_COMPLETE, WORK_ITEM_CREATE,
    WORK_ITEM_UPDATE,
};
use business_analytics::{
    acceptance_scope, AnalysisDomain, BusinessAnalyticsService, BusinessDataset, RuleConfig,
    ACCEPTANCE_FINANCE_USER, ACCEPTANCE_SALES_USER,
};
use business_anomaly_contracts::BusinessAnomaly;
use std::collections::BTreeSet;
use uuid::Uuid;

pub const ACCEPTANCE_CLASSIFICATION: &str = "desensitized-acceptance-production-disabled";

pub fn acceptance_engine(trace_id: Uuid) -> Result<ActionEngine, ActionError> {
    let dataset =
        BusinessDataset::desensitized_acceptance().map_err(|_| ActionError::InvalidRequest)?;
    let analytics = BusinessAnalyticsService::new(
        dataset.clone(),
        RuleConfig::bundled().map_err(|_| ActionError::InvalidRequest)?,
    )
    .map_err(|_| ActionError::InvalidRequest)?;
    let scope = acceptance_scope(ACCEPTANCE_FINANCE_USER).ok_or(ActionError::InvalidRequest)?;
    let scope_hash = scope.hash();
    let result = analytics.analyze(AnalysisDomain::All, &scope, trace_id, 100);
    let observations = result
        .findings
        .into_iter()
        .map(|finding| FindingObservation {
            scope: resolve_scope(&dataset, &finding),
            finding,
        })
        .collect();
    let mut engine =
        ActionEngine::new(bundled_catalog().map_err(|_| ActionError::InvalidRequest)?)?;
    engine.ingest_run(
        observations,
        business_action_contracts::RunStatus::Completed,
        &scope_hash,
        &result.rule_set_version,
        result.data_as_of,
        trace_id,
    )?;
    Ok(engine)
}

pub fn acceptance_actor(user_id: Uuid, trace_id: Uuid) -> Option<Actor> {
    let mut permissions = BTreeSet::from([
        BUSINESS_ACTION_READ.into(),
        FINDING_READ.into(),
        ACTION_PROPOSAL_READ.into(),
    ]);
    match user_id {
        ACCEPTANCE_FINANCE_USER => {
            permissions.extend(
                [
                    FINDING_ACKNOWLEDGE,
                    WORK_ITEM_CREATE,
                    WORK_ITEM_UPDATE,
                    WORK_ITEM_ASSIGN,
                    WORK_ITEM_COMPLETE,
                    APPROVAL_DRAFT_CREATE,
                ]
                .into_iter()
                .map(str::to_owned),
            );
        }
        ACCEPTANCE_SALES_USER => {}
        _ => return None,
    }
    Some(Actor {
        user_id,
        permissions,
        authorized_scope: acceptance_scope(user_id)?,
        trace_id,
    })
}

fn resolve_scope(dataset: &BusinessDataset, finding: &BusinessAnomaly) -> FindingScope {
    let id = finding.primary_resource.id.as_deref().unwrap_or_default();
    if let Some(value) = dataset.sales_orders.iter().find(|value| {
        value.identity.object_id == id || value.customer_id == id || value.sku_id == id
    }) {
        return FindingScope {
            legal_entity_id: value.identity.legal_entity_id.clone(),
            warehouse_id: Some(value.warehouse_id.clone()),
            customer_id: Some(value.customer_id.clone()),
            supplier_id: None,
            brand_id: Some(value.brand_id.clone()),
            business_unit_id: Some(value.business_unit_id.clone()),
        };
    }
    if let Some(value) = dataset.purchase_orders.iter().find(|value| {
        value.identity.object_id == id || value.supplier_id == id || value.sku_id == id
    }) {
        return FindingScope {
            legal_entity_id: value.identity.legal_entity_id.clone(),
            warehouse_id: Some(value.warehouse_id.clone()),
            customer_id: None,
            supplier_id: Some(value.supplier_id.clone()),
            brand_id: Some(value.brand_id.clone()),
            business_unit_id: None,
        };
    }
    if let Some(value) = dataset
        .inventory
        .iter()
        .find(|value| value.identity.object_id == id || value.sku_id == id)
    {
        return FindingScope {
            legal_entity_id: value.identity.legal_entity_id.clone(),
            warehouse_id: Some(value.warehouse_id.clone()),
            customer_id: None,
            supplier_id: None,
            brand_id: Some(value.brand_id.clone()),
            business_unit_id: None,
        };
    }
    if let Some(value) = dataset
        .receivables
        .iter()
        .find(|value| value.identity.object_id == id || value.customer_id == id)
    {
        return FindingScope {
            legal_entity_id: value.identity.legal_entity_id.clone(),
            warehouse_id: None,
            customer_id: Some(value.customer_id.clone()),
            supplier_id: None,
            brand_id: None,
            business_unit_id: None,
        };
    }
    if let Some(value) = dataset
        .order_profits
        .iter()
        .find(|value| value.sales_order_id == id || value.customer_id == id)
    {
        return FindingScope {
            legal_entity_id: value.identity.legal_entity_id.clone(),
            warehouse_id: None,
            customer_id: Some(value.customer_id.clone()),
            supplier_id: None,
            brand_id: Some(value.brand_id.clone()),
            business_unit_id: None,
        };
    }
    FindingScope {
        legal_entity_id: "LE-A".into(),
        ..FindingScope::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acceptance_is_explicit_and_sales_has_no_write_permissions() {
        assert!(ACCEPTANCE_CLASSIFICATION.contains("production-disabled"));
        let sales = acceptance_actor(ACCEPTANCE_SALES_USER, Uuid::new_v4()).expect("sales");
        assert!(sales.can(FINDING_READ));
        assert!(!sales.can(WORK_ITEM_CREATE));
        let finance = acceptance_actor(ACCEPTANCE_FINANCE_USER, Uuid::new_v4()).expect("finance");
        assert!(finance.can(WORK_ITEM_CREATE));
    }
}
