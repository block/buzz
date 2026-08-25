use super::common::authorize;
use crate::{
    b2::{
        common::{
            begin_idempotent, finish_idempotent, next_number, record, request_hash, DomainError,
        },
        model::{DecimalString, VersionCommand},
    },
    store::PgStore,
};
use chrono::{Duration, NaiveDate, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{Postgres, Row, Transaction};
use std::collections::BTreeSet;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpsertReplenishmentPolicy {
    pub legal_entity_id: Uuid,
    pub warehouse_id: Uuid,
    pub sku_id: Uuid,
    pub preferred_supplier_id: Uuid,
    pub unit_of_measure_id: Uuid,
    pub safety_stock: DecimalString,
    pub reorder_point: DecimalString,
    pub target_stock: DecimalString,
    pub minimum_order_quantity: DecimalString,
    pub order_multiple: DecimalString,
    pub lead_time_days: i32,
    pub status: String,
    #[serde(default)]
    pub expected_version: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreatePurchaseRequisition {
    pub policy_ids: Vec<Uuid>,
    pub request_date: NaiveDate,
    #[serde(default)]
    pub business_note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConvertPurchaseRequisition {
    pub expected_version: i64,
    pub purchase_order_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PolicyCommandResult {
    pub id: Uuid,
    pub status: String,
    pub version: i64,
    pub trace_id: Uuid,
    pub idempotent_replay: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequisitionCommandResult {
    pub id: Uuid,
    pub number: String,
    pub status: String,
    pub version: i64,
    pub trace_id: Uuid,
    pub idempotent_replay: bool,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ReplenishmentSuggestion {
    pub id: Uuid,
    pub legal_entity_id: Uuid,
    pub warehouse_id: Uuid,
    pub warehouse_code: String,
    pub warehouse_name: String,
    pub sku_id: Uuid,
    pub sku_code: String,
    pub sku_name: String,
    pub preferred_supplier_id: Uuid,
    pub supplier_code: String,
    pub supplier_name: String,
    pub unit_of_measure_id: Uuid,
    pub currency: String,
    #[sqlx(try_from = "Decimal")]
    pub safety_stock: DecimalString,
    #[sqlx(try_from = "Decimal")]
    pub reorder_point: DecimalString,
    #[sqlx(try_from = "Decimal")]
    pub target_stock: DecimalString,
    #[sqlx(try_from = "Decimal")]
    pub minimum_order_quantity: DecimalString,
    #[sqlx(try_from = "Decimal")]
    pub order_multiple: DecimalString,
    pub lead_time_days: i32,
    pub status: String,
    #[sqlx(try_from = "Decimal")]
    pub on_hand_quantity: DecimalString,
    #[sqlx(try_from = "Decimal")]
    pub reserved_quantity: DecimalString,
    #[sqlx(try_from = "Decimal")]
    pub quarantined_quantity: DecimalString,
    #[sqlx(try_from = "Decimal")]
    pub available_quantity: DecimalString,
    #[sqlx(try_from = "Decimal")]
    pub inbound_quantity: DecimalString,
    #[sqlx(try_from = "Decimal")]
    pub open_requisition_quantity: DecimalString,
    #[sqlx(try_from = "Decimal")]
    pub projected_quantity: DecimalString,
    #[sqlx(try_from = "Decimal")]
    pub inventory_value: DecimalString,
    pub risk_state: String,
    #[sqlx(try_from = "Decimal")]
    pub suggested_quantity: DecimalString,
    pub suggested_required_date: NaiveDate,
    pub version: i64,
    pub updated_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct PurchaseRequisitionSummary {
    pub id: Uuid,
    pub requisition_number: String,
    pub legal_entity_id: Uuid,
    pub warehouse_id: Uuid,
    pub supplier_id: Uuid,
    pub request_date: NaiveDate,
    pub required_date: NaiveDate,
    pub currency: String,
    pub status: String,
    pub line_count: i64,
    #[sqlx(try_from = "Decimal")]
    pub total_quantity: DecimalString,
    pub version: i64,
    pub updated_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ReplenishmentInventoryOption {
    pub legal_entity_id: Uuid,
    pub warehouse_id: Uuid,
    pub warehouse_code: String,
    pub warehouse_name: String,
    pub sku_id: Uuid,
    pub sku_code: String,
    pub sku_name: String,
    pub unit_of_measure_id: Uuid,
    #[sqlx(try_from = "Decimal")]
    pub available_quantity: DecimalString,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ReplenishmentSupplierOption {
    pub id: Uuid,
    pub code: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplenishmentOptions {
    pub inventory: Vec<ReplenishmentInventoryOption>,
    pub suppliers: Vec<ReplenishmentSupplierOption>,
    pub data_as_of: chrono::DateTime<Utc>,
}

#[derive(Clone)]
pub struct ReplenishmentService {
    store: PgStore,
    prefix: String,
}

impl ReplenishmentService {
    pub fn new(store: PgStore, prefix: String) -> Self {
        Self { store, prefix }
    }

    pub async fn options(&self, actor: Uuid) -> Result<ReplenishmentOptions, DomainError> {
        let scope = authorize(
            &self.store,
            actor,
            "replenishment:read",
            None,
            None,
            None,
            None,
            None,
        )
        .await?;
        let entities = scope
            .scopes
            .legal_entity_ids
            .into_iter()
            .collect::<Vec<_>>();
        let warehouses = scope.scopes.warehouse_ids.into_iter().collect::<Vec<_>>();
        let suppliers = scope.scopes.supplier_ids.into_iter().collect::<Vec<_>>();
        let inventory=sqlx::query_as::<_,ReplenishmentInventoryOption>("SELECT b.legal_entity_id,b.warehouse_id,w.code warehouse_code,w.name warehouse_name,b.sku_id,s.code sku_code,s.name sku_name,p.base_uom_id unit_of_measure_id,(b.on_hand_quantity-b.reserved_quantity-b.quarantined_quantity)::numeric(24,6) available_quantity FROM inventory_balances b JOIN business_warehouses w ON w.id=b.warehouse_id JOIN business_skus s ON s.id=b.sku_id JOIN business_products p ON p.id=s.product_id WHERE b.legal_entity_id=ANY($1) AND b.warehouse_id=ANY($2) AND s.status='active' ORDER BY w.code,s.code LIMIT 1000").bind(&entities).bind(&warehouses).fetch_all(self.store.pool()).await?;
        let supplier_options=sqlx::query_as::<_,ReplenishmentSupplierOption>("SELECT id,code,name FROM business_suppliers WHERE id=ANY($1) AND status='active' ORDER BY code LIMIT 500").bind(&suppliers).fetch_all(self.store.pool()).await?;
        Ok(ReplenishmentOptions {
            inventory,
            suppliers: supplier_options,
            data_as_of: Utc::now(),
        })
    }

    pub async fn suggestions(
        &self,
        actor: Uuid,
        limit: i64,
    ) -> Result<Vec<ReplenishmentSuggestion>, DomainError> {
        let scope = authorize(
            &self.store,
            actor,
            "replenishment:read",
            None,
            None,
            None,
            None,
            None,
        )
        .await?;
        sqlx::query_as::<_,ReplenishmentSuggestion>("SELECT id,legal_entity_id,warehouse_id,warehouse_code,warehouse_name,sku_id,sku_code,sku_name,preferred_supplier_id,supplier_code,supplier_name,unit_of_measure_id,currency,safety_stock,reorder_point,target_stock,minimum_order_quantity,order_multiple,lead_time_days,status,on_hand_quantity,reserved_quantity,quarantined_quantity,available_quantity,inbound_quantity,open_requisition_quantity,projected_quantity,inventory_value,risk_state,suggested_quantity,suggested_required_date,version,updated_at FROM inventory_replenishment_current WHERE legal_entity_id=ANY($1) AND warehouse_id=ANY($2) AND preferred_supplier_id=ANY($3) ORDER BY CASE risk_state WHEN 'critical' THEN 0 WHEN 'warning' THEN 1 WHEN 'requisition_open' THEN 2 WHEN 'inbound_covered' THEN 3 WHEN 'healthy' THEN 4 ELSE 5 END,sku_code LIMIT $4")
            .bind(scope.scopes.legal_entity_ids.into_iter().collect::<Vec<_>>())
            .bind(scope.scopes.warehouse_ids.into_iter().collect::<Vec<_>>())
            .bind(scope.scopes.supplier_ids.into_iter().collect::<Vec<_>>())
            .bind(limit.clamp(1, 1000)).fetch_all(self.store.pool()).await.map_err(Into::into)
    }

    pub async fn requisitions(
        &self,
        actor: Uuid,
        limit: i64,
    ) -> Result<Vec<PurchaseRequisitionSummary>, DomainError> {
        let scope = authorize(
            &self.store,
            actor,
            "replenishment:read",
            None,
            None,
            None,
            None,
            None,
        )
        .await?;
        sqlx::query_as::<_,PurchaseRequisitionSummary>("SELECT r.id,r.requisition_number,r.legal_entity_id,r.warehouse_id,r.supplier_id,r.request_date,r.required_date,r.currency::text currency,r.status,count(l.id) line_count,sum(l.requested_quantity)::numeric(24,6) total_quantity,r.version,r.updated_at FROM purchase_requisitions r JOIN purchase_requisition_lines l ON l.purchase_requisition_id=r.id WHERE r.legal_entity_id=ANY($1) AND r.warehouse_id=ANY($2) AND r.supplier_id=ANY($3) GROUP BY r.id ORDER BY r.request_date DESC,r.requisition_number DESC LIMIT $4")
            .bind(scope.scopes.legal_entity_ids.into_iter().collect::<Vec<_>>())
            .bind(scope.scopes.warehouse_ids.into_iter().collect::<Vec<_>>())
            .bind(scope.scopes.supplier_ids.into_iter().collect::<Vec<_>>())
            .bind(limit.clamp(1,500)).fetch_all(self.store.pool()).await.map_err(Into::into)
    }

    pub async fn upsert_policy(
        &self,
        actor: Uuid,
        trace_id: Uuid,
        key: &str,
        input: &UpsertReplenishmentPolicy,
    ) -> Result<PolicyCommandResult, DomainError> {
        validate_policy(input)?;
        authorize(
            &self.store,
            actor,
            "replenishment_policy:manage",
            Some(input.legal_entity_id),
            Some(input.warehouse_id),
            Some(input.preferred_supplier_id),
            None,
            None,
        )
        .await?;
        let hash = request_hash(input)?;
        let mut tx = self.store.pool().begin().await?;
        if let Some(mut replay) = begin_idempotent::<PolicyCommandResult>(
            &mut tx,
            actor,
            "replenishment_policy:upsert",
            key,
            &hash,
        )
        .await?
        {
            replay.idempotent_replay = true;
            tx.commit().await?;
            return Ok(replay);
        }
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!(
                "{}:{}:{}",
                input.legal_entity_id, input.warehouse_id, input.sku_id
            ))
            .execute(&mut *tx)
            .await?;
        let valid:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM business_warehouses w JOIN business_skus s ON s.id=$3 JOIN business_products p ON p.id=s.product_id JOIN business_suppliers supplier ON supplier.id=$4 WHERE w.id=$2 AND w.legal_entity_id=$1 AND p.base_uom_id=$5 AND w.status='active' AND s.status='active' AND supplier.legal_entity_id=$1 AND supplier.status='active')").bind(input.legal_entity_id).bind(input.warehouse_id).bind(input.sku_id).bind(input.preferred_supplier_id).bind(input.unit_of_measure_id).fetch_one(&mut *tx).await?;
        if !valid {
            return Err(DomainError::NotFoundOrForbidden);
        }
        let existing=sqlx::query("SELECT id,version FROM inventory_replenishment_policies WHERE legal_entity_id=$1 AND warehouse_id=$2 AND sku_id=$3 FOR UPDATE").bind(input.legal_entity_id).bind(input.warehouse_id).bind(input.sku_id).fetch_optional(&mut *tx).await?;
        let (id, version) = if let Some(row) = existing {
            let version: i64 = row.get("version");
            if input.expected_version != Some(version) {
                return Err(DomainError::VersionConflict);
            }
            let id: Uuid = row.get("id");
            sqlx::query("UPDATE inventory_replenishment_policies SET preferred_supplier_id=$2,unit_of_measure_id=$3,safety_stock=$4,reorder_point=$5,target_stock=$6,minimum_order_quantity=$7,order_multiple=$8,lead_time_days=$9,status=$10,updated_by_user_id=$11,trace_id=$12 WHERE id=$1").bind(id).bind(input.preferred_supplier_id).bind(input.unit_of_measure_id).bind(input.safety_stock.0).bind(input.reorder_point.0).bind(input.target_stock.0).bind(input.minimum_order_quantity.0).bind(input.order_multiple.0).bind(input.lead_time_days).bind(&input.status).bind(actor).bind(trace_id).execute(&mut *tx).await?;
            (id, version + 1)
        } else {
            if input.expected_version.is_some() {
                return Err(DomainError::VersionConflict);
            }
            let id = Uuid::new_v4();
            sqlx::query("INSERT INTO inventory_replenishment_policies(id,legal_entity_id,warehouse_id,sku_id,preferred_supplier_id,unit_of_measure_id,safety_stock,reorder_point,target_stock,minimum_order_quantity,order_multiple,lead_time_days,status,created_by_user_id,updated_by_user_id,trace_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$14,$15)").bind(id).bind(input.legal_entity_id).bind(input.warehouse_id).bind(input.sku_id).bind(input.preferred_supplier_id).bind(input.unit_of_measure_id).bind(input.safety_stock.0).bind(input.reorder_point.0).bind(input.target_stock.0).bind(input.minimum_order_quantity.0).bind(input.order_multiple.0).bind(input.lead_time_days).bind(&input.status).bind(actor).bind(trace_id).execute(&mut *tx).await?;
            (id, 1)
        };
        record(
            &mut tx,
            trace_id,
            actor,
            "REPLENISHMENT_POLICY_UPSERTED",
            "replenishment_policy_upserted",
            "replenishment_policy",
            id,
            json!({"warehouseId":input.warehouse_id,"skuId":input.sku_id,"version":version}),
        )
        .await?;
        let result = PolicyCommandResult {
            id,
            status: input.status.clone(),
            version,
            trace_id,
            idempotent_replay: false,
        };
        finish_idempotent(&mut tx, actor, "replenishment_policy:upsert", key, &result).await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn create_requisition(
        &self,
        actor: Uuid,
        trace_id: Uuid,
        key: &str,
        input: &CreatePurchaseRequisition,
    ) -> Result<RequisitionCommandResult, DomainError> {
        if input.policy_ids.is_empty()
            || input.policy_ids.len() > 200
            || input
                .business_note
                .as_ref()
                .is_some_and(|v| v.chars().count() > 1000)
        {
            return Err(DomainError::Invalid(
                "purchase requisition requires 1-200 policies and a bounded note".into(),
            ));
        }
        let ids = input.policy_ids.iter().copied().collect::<BTreeSet<_>>();
        if ids.len() != input.policy_ids.len() {
            return Err(DomainError::Invalid(
                "duplicate replenishment policy".into(),
            ));
        }
        let hash = request_hash(input)?;
        let mut tx = self.store.pool().begin().await?;
        if let Some(mut replay) = begin_idempotent::<RequisitionCommandResult>(
            &mut tx,
            actor,
            "purchase_requisition:create",
            key,
            &hash,
        )
        .await?
        {
            replay.idempotent_replay = true;
            tx.commit().await?;
            return Ok(replay);
        }
        let rows=sqlx::query("SELECT id,legal_entity_id,warehouse_id,preferred_supplier_id FROM inventory_replenishment_policies WHERE id=ANY($1) AND status='active' ORDER BY id FOR UPDATE").bind(ids.iter().copied().collect::<Vec<_>>()).fetch_all(&mut *tx).await?;
        if rows.len() != ids.len() {
            return Err(DomainError::NotFoundOrForbidden);
        }
        let entity: Uuid = rows[0].get("legal_entity_id");
        let warehouse: Uuid = rows[0].get("warehouse_id");
        let supplier: Uuid = rows[0].get("preferred_supplier_id");
        if rows.iter().any(|r| {
            r.get::<Uuid, _>("legal_entity_id") != entity
                || r.get::<Uuid, _>("warehouse_id") != warehouse
                || r.get::<Uuid, _>("preferred_supplier_id") != supplier
        }) {
            return Err(DomainError::Invalid(
                "one requisition must use one legal entity, warehouse and supplier".into(),
            ));
        }
        authorize(
            &self.store,
            actor,
            "purchase_requisition:create",
            Some(entity),
            Some(warehouse),
            Some(supplier),
            None,
            None,
        )
        .await?;
        let suggestions=sqlx::query("SELECT id,sku_id,unit_of_measure_id,currency,suggested_quantity,available_quantity,inbound_quantity,open_requisition_quantity,reorder_point,target_stock,lead_time_days FROM inventory_replenishment_current WHERE id=ANY($1) ORDER BY id").bind(ids.iter().copied().collect::<Vec<_>>()).fetch_all(&mut *tx).await?;
        if suggestions.len() != ids.len()
            || suggestions
                .iter()
                .any(|r| r.get::<Decimal, _>("suggested_quantity") <= Decimal::ZERO)
        {
            return Err(DomainError::Invalid(
                "selected replenishment suggestions are no longer actionable".into(),
            ));
        }
        let max_lead_time = suggestions
            .iter()
            .map(|row| row.get::<i32, _>("lead_time_days"))
            .max()
            .ok_or_else(|| DomainError::Invalid("required date unavailable".into()))?;
        let required_date = input.request_date + Duration::days(i64::from(max_lead_time));
        let currency: String = suggestions[0].get("currency");
        let business_unit_id: Uuid = sqlx::query_scalar(
            "SELECT business_unit_id FROM business_warehouses WHERE id=$1 AND legal_entity_id=$2 AND status='active'",
        )
        .bind(warehouse)
        .bind(entity)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(DomainError::NotFoundOrForbidden)?;
        let id = Uuid::new_v4();
        let number = next_number(
            &mut tx,
            "purchase_requisition",
            &self.prefix,
            id,
            crate::numbering::NumberingContext::new(entity, Some(business_unit_id)),
        )
        .await?;
        sqlx::query("INSERT INTO purchase_requisitions(id,requisition_number,legal_entity_id,warehouse_id,supplier_id,request_date,required_date,currency,business_note,created_by_user_id,trace_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)").bind(id).bind(&number).bind(entity).bind(warehouse).bind(supplier).bind(input.request_date).bind(required_date).bind(&currency).bind(&input.business_note).bind(actor).bind(trace_id).execute(&mut *tx).await?;
        for row in &suggestions {
            sqlx::query("INSERT INTO purchase_requisition_lines(id,purchase_requisition_id,replenishment_policy_id,sku_id,unit_of_measure_id,requested_quantity,snapshot_available_quantity,snapshot_inbound_quantity,snapshot_open_requisition_quantity,snapshot_reorder_point,snapshot_target_stock) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)").bind(Uuid::new_v4()).bind(id).bind(row.get::<Uuid,_>("id")).bind(row.get::<Uuid,_>("sku_id")).bind(row.get::<Uuid,_>("unit_of_measure_id")).bind(row.get::<Decimal,_>("suggested_quantity")).bind(row.get::<Decimal,_>("available_quantity")).bind(row.get::<Decimal,_>("inbound_quantity")).bind(row.get::<Decimal,_>("open_requisition_quantity")).bind(row.get::<Decimal,_>("reorder_point")).bind(row.get::<Decimal,_>("target_stock")).execute(&mut *tx).await?;
        }
        requisition_event(
            &mut tx,
            id,
            "created",
            1,
            actor,
            trace_id,
            json!({"lineCount":suggestions.len()}),
        )
        .await?;
        record(
            &mut tx,
            trace_id,
            actor,
            "PURCHASE_REQUISITION_CREATED",
            "purchase_requisition_created",
            "purchase_requisition",
            id,
            json!({"requisitionNumber":number,"lineCount":suggestions.len()}),
        )
        .await?;
        let result = RequisitionCommandResult {
            id,
            number,
            status: "draft".into(),
            version: 1,
            trace_id,
            idempotent_replay: false,
        };
        finish_idempotent(&mut tx, actor, "purchase_requisition:create", key, &result).await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn transition_requisition(
        &self,
        actor: Uuid,
        trace_id: Uuid,
        id: Uuid,
        key: &str,
        input: &VersionCommand,
        confirm: bool,
    ) -> Result<RequisitionCommandResult, DomainError> {
        let operation = if confirm {
            "purchase_requisition:confirm"
        } else {
            "purchase_requisition:cancel"
        };
        let hash = request_hash(input)?;
        let mut tx = self.store.pool().begin().await?;
        if let Some(mut replay) =
            begin_idempotent::<RequisitionCommandResult>(&mut tx, actor, operation, key, &hash)
                .await?
        {
            replay.idempotent_replay = true;
            tx.commit().await?;
            return Ok(replay);
        }
        let row=sqlx::query("SELECT requisition_number,legal_entity_id,warehouse_id,supplier_id,status,version FROM purchase_requisitions WHERE id=$1 FOR UPDATE").bind(id).fetch_optional(&mut *tx).await?.ok_or(DomainError::NotFoundOrForbidden)?;
        let permission = if confirm {
            "purchase_requisition:confirm"
        } else {
            "purchase_requisition:cancel"
        };
        authorize(
            &self.store,
            actor,
            permission,
            Some(row.get("legal_entity_id")),
            Some(row.get("warehouse_id")),
            Some(row.get("supplier_id")),
            None,
            None,
        )
        .await?;
        let current_status = row.get::<String, _>("status");
        let valid_status = if confirm {
            current_status == "draft"
        } else {
            matches!(current_status.as_str(), "draft" | "confirmed")
        };
        if !valid_status || row.get::<i64, _>("version") != input.expected_version {
            return Err(DomainError::VersionConflict);
        }
        let status = if confirm { "confirmed" } else { "cancelled" };
        let version = input.expected_version + 1;
        if confirm {
            sqlx::query("UPDATE purchase_requisitions SET status='confirmed',confirmed_by_user_id=$2,confirmed_at=now(),trace_id=$3 WHERE id=$1").bind(id).bind(actor).bind(trace_id).execute(&mut *tx).await?;
        } else {
            sqlx::query("UPDATE purchase_requisitions SET status='cancelled',cancelled_by_user_id=$2,cancelled_at=now(),trace_id=$3 WHERE id=$1").bind(id).bind(actor).bind(trace_id).execute(&mut *tx).await?;
        }
        requisition_event(&mut tx, id, status, version, actor, trace_id, json!({})).await?;
        record(
            &mut tx,
            trace_id,
            actor,
            if confirm {
                "PURCHASE_REQUISITION_CONFIRMED"
            } else {
                "PURCHASE_REQUISITION_CANCELLED"
            },
            if confirm {
                "purchase_requisition_confirmed"
            } else {
                "purchase_requisition_cancelled"
            },
            "purchase_requisition",
            id,
            json!({"version":version}),
        )
        .await?;
        let result = RequisitionCommandResult {
            id,
            number: row.get("requisition_number"),
            status: status.into(),
            version,
            trace_id,
            idempotent_replay: false,
        };
        finish_idempotent(&mut tx, actor, operation, key, &result).await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn convert_requisition(
        &self,
        actor: Uuid,
        trace_id: Uuid,
        id: Uuid,
        key: &str,
        input: &ConvertPurchaseRequisition,
    ) -> Result<RequisitionCommandResult, DomainError> {
        let hash = request_hash(input)?;
        let mut tx = self.store.pool().begin().await?;
        if let Some(mut replay) = begin_idempotent::<RequisitionCommandResult>(
            &mut tx,
            actor,
            "purchase_requisition:convert",
            key,
            &hash,
        )
        .await?
        {
            replay.idempotent_replay = true;
            tx.commit().await?;
            return Ok(replay);
        }
        let row = sqlx::query("SELECT requisition_number,legal_entity_id,warehouse_id,supplier_id,status,version FROM purchase_requisitions WHERE id=$1 FOR UPDATE")
            .bind(id).fetch_optional(&mut *tx).await?.ok_or(DomainError::NotFoundOrForbidden)?;
        let entity: Uuid = row.get("legal_entity_id");
        let warehouse: Uuid = row.get("warehouse_id");
        let supplier: Uuid = row.get("supplier_id");
        authorize(
            &self.store,
            actor,
            "purchase_requisition:convert",
            Some(entity),
            Some(warehouse),
            Some(supplier),
            None,
            None,
        )
        .await?;
        if row.get::<String, _>("status") != "confirmed"
            || row.get::<i64, _>("version") != input.expected_version
        {
            return Err(DomainError::VersionConflict);
        }
        let valid_order: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM purchase_orders o WHERE o.id=$1 AND o.legal_entity_id=$2 AND o.supplier_id=$3 AND o.lifecycle_status IN ('draft','confirmed') AND NOT EXISTS(SELECT 1 FROM purchase_requisition_lines r WHERE r.purchase_requisition_id=$4 AND COALESCE((SELECT sum(l.ordered_quantity-l.cancelled_quantity) FROM purchase_order_lines l WHERE l.purchase_order_id=o.id AND l.warehouse_id=$5 AND l.sku_id=r.sku_id),0)<r.requested_quantity))",
        )
        .bind(input.purchase_order_id)
        .bind(entity)
        .bind(supplier)
        .bind(id)
        .bind(warehouse)
        .fetch_one(&mut *tx)
        .await?;
        if !valid_order {
            return Err(DomainError::Invalid(
                "purchase order does not cover every requisition line".into(),
            ));
        }
        let version = input.expected_version + 1;
        sqlx::query("UPDATE purchase_requisitions SET status='converted',purchase_order_id=$2,converted_by_user_id=$3,converted_at=now(),trace_id=$4 WHERE id=$1")
            .bind(id).bind(input.purchase_order_id).bind(actor).bind(trace_id).execute(&mut *tx).await?;
        requisition_event(
            &mut tx,
            id,
            "converted",
            version,
            actor,
            trace_id,
            json!({"purchaseOrderId":input.purchase_order_id}),
        )
        .await?;
        record(
            &mut tx,
            trace_id,
            actor,
            "PURCHASE_REQUISITION_CONVERTED",
            "purchase_requisition_converted",
            "purchase_requisition",
            id,
            json!({"purchaseOrderId":input.purchase_order_id,"version":version}),
        )
        .await?;
        let result = RequisitionCommandResult {
            id,
            number: row.get("requisition_number"),
            status: "converted".into(),
            version,
            trace_id,
            idempotent_replay: false,
        };
        finish_idempotent(&mut tx, actor, "purchase_requisition:convert", key, &result).await?;
        tx.commit().await?;
        Ok(result)
    }
}

fn validate_policy(input: &UpsertReplenishmentPolicy) -> Result<(), DomainError> {
    if input.safety_stock.0 < Decimal::ZERO
        || input.reorder_point.0 < input.safety_stock.0
        || input.target_stock.0 <= input.reorder_point.0
        || input.minimum_order_quantity.0 <= Decimal::ZERO
        || input.order_multiple.0 <= Decimal::ZERO
        || !(0..=3650).contains(&input.lead_time_days)
        || !matches!(input.status.as_str(), "active" | "paused")
    {
        return Err(DomainError::Invalid(
            "invalid replenishment policy thresholds".into(),
        ));
    }
    Ok(())
}

async fn requisition_event(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    event: &str,
    version: i64,
    actor: Uuid,
    trace_id: Uuid,
    payload: serde_json::Value,
) -> Result<(), DomainError> {
    sqlx::query("INSERT INTO purchase_requisition_events(id,purchase_requisition_id,event_type,requisition_version,payload,actor_user_id,trace_id) VALUES($1,$2,$3,$4,$5,$6,$7)").bind(Uuid::new_v4()).bind(id).bind(event).bind(version).bind(payload).bind(actor).bind(trace_id).execute(&mut **tx).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn thresholds_must_be_ordered() {
        let id = Uuid::new_v4();
        let input = UpsertReplenishmentPolicy {
            legal_entity_id: id,
            warehouse_id: id,
            sku_id: id,
            preferred_supplier_id: id,
            unit_of_measure_id: id,
            safety_stock: DecimalString(Decimal::new(10, 0)),
            reorder_point: DecimalString(Decimal::new(5, 0)),
            target_stock: DecimalString(Decimal::new(20, 0)),
            minimum_order_quantity: DecimalString(Decimal::ONE),
            order_multiple: DecimalString(Decimal::ONE),
            lead_time_days: 7,
            status: "active".into(),
            expected_version: None,
        };
        assert!(validate_policy(&input).is_err());
    }
}
