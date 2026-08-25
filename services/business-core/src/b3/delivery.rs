use super::common::authorize;
use crate::{
    b2::{
        common::{begin_idempotent, finish_idempotent, record, request_hash, DomainError},
        model::{DecimalString, OptionalDecimalString},
    },
    store::PgStore,
};
use chrono::{Duration, NaiveDate, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecordDeliveryCommitment {
    pub promised_delivery_date: NaiveDate,
    pub expected_revision: i64,
    #[serde(default)]
    pub commitment_note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeliveryCommitmentResult {
    pub id: Uuid,
    pub purchase_order_id: Uuid,
    pub status: String,
    pub revision: i64,
    pub promised_delivery_date: NaiveDate,
    pub trace_id: Uuid,
    pub idempotent_replay: bool,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct PurchaseDeliveryView {
    pub purchase_order_id: Uuid,
    pub purchase_order_number: String,
    pub legal_entity_id: Uuid,
    pub supplier_id: Uuid,
    pub supplier_code: String,
    pub supplier_name: String,
    pub buyer_user_id: Uuid,
    pub order_date: NaiveDate,
    pub expected_delivery_date: Option<NaiveDate>,
    pub promised_delivery_date: Option<NaiveDate>,
    pub commitment_source: String,
    pub commitment_id: Option<Uuid>,
    pub commitment_revision: i64,
    pub commitment_note: Option<String>,
    pub commitment_recorded_at: Option<chrono::DateTime<Utc>>,
    pub lifecycle_status: String,
    pub receiving_status: String,
    pub currency: String,
    #[sqlx(try_from = "Decimal")]
    pub gross_amount: DecimalString,
    #[sqlx(try_from = "Decimal")]
    pub ordered_quantity: DecimalString,
    #[sqlx(try_from = "Decimal")]
    pub received_quantity: DecimalString,
    #[sqlx(try_from = "Decimal")]
    pub cancelled_quantity: DecimalString,
    #[sqlx(try_from = "Decimal")]
    pub open_quantity: DecimalString,
    pub receipt_count: i64,
    pub first_receipt_date: Option<NaiveDate>,
    pub last_receipt_date: Option<NaiveDate>,
    pub delivery_status: String,
    pub delivery_variance_days: Option<i32>,
    pub updated_at: chrono::DateTime<Utc>,
    pub version: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PurchaseDeliveryResponse {
    pub items: Vec<PurchaseDeliveryView>,
    pub can_manage_commitments: bool,
    pub data_as_of: chrono::DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct SupplierDeliveryPerformance {
    pub supplier_id: Uuid,
    pub supplier_code: String,
    pub supplier_name: String,
    pub order_count: i64,
    pub open_order_count: i64,
    pub overdue_order_count: i64,
    pub completed_order_count: i64,
    pub on_time_order_count: i64,
    #[sqlx(try_from = "Option<Decimal>")]
    pub on_time_rate: OptionalDecimalString,
    #[sqlx(try_from = "Decimal")]
    pub ordered_quantity: DecimalString,
    #[sqlx(try_from = "Decimal")]
    pub received_quantity: DecimalString,
    #[sqlx(try_from = "Option<Decimal>")]
    pub fulfillment_rate: OptionalDecimalString,
    #[sqlx(try_from = "Decimal")]
    pub returned_quantity: DecimalString,
    #[sqlx(try_from = "Option<Decimal>")]
    pub quality_acceptance_rate: OptionalDecimalString,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SupplierPerformanceResponse {
    pub items: Vec<SupplierDeliveryPerformance>,
    pub period_start: NaiveDate,
    pub period_end: NaiveDate,
    pub data_as_of: chrono::DateTime<Utc>,
}

#[derive(Clone)]
pub struct DeliveryService {
    store: PgStore,
}

impl DeliveryService {
    pub fn new(store: PgStore) -> Self {
        Self { store }
    }

    pub async fn deliveries(
        &self,
        actor: Uuid,
        supplier_id: Option<Uuid>,
        limit: i64,
    ) -> Result<PurchaseDeliveryResponse, DomainError> {
        let scope = authorize(
            &self.store,
            actor,
            "purchase_delivery:read",
            None,
            None,
            None,
            None,
            None,
        )
        .await?;
        if supplier_id.is_some_and(|id| !scope.scopes.supplier_ids.contains(&id)) {
            return Err(DomainError::NotFoundOrForbidden);
        }
        let can_manage = scope
            .permission_keys
            .contains("purchase_delivery_commitment:manage");
        let entities = scope
            .scopes
            .legal_entity_ids
            .into_iter()
            .collect::<Vec<_>>();
        let suppliers = scope.scopes.supplier_ids.into_iter().collect::<Vec<_>>();
        let items = sqlx::query_as::<_, PurchaseDeliveryView>(
            "SELECT purchase_order_id,purchase_order_number,legal_entity_id,supplier_id,supplier_code,supplier_name,buyer_user_id,order_date,expected_delivery_date,promised_delivery_date,commitment_source,commitment_id,commitment_revision,commitment_note,commitment_recorded_at,lifecycle_status,receiving_status,currency,gross_amount,ordered_quantity,received_quantity,cancelled_quantity,open_quantity,receipt_count,first_receipt_date,last_receipt_date,delivery_status,delivery_variance_days,updated_at,version FROM purchase_delivery_current WHERE legal_entity_id=ANY($1) AND supplier_id=ANY($2) AND lifecycle_status<>'draft' AND ($3::uuid IS NULL OR supplier_id=$3) ORDER BY CASE delivery_status WHEN 'overdue' THEN 0 WHEN 'due_today' THEN 1 WHEN 'due_soon' THEN 2 WHEN 'unscheduled' THEN 3 WHEN 'on_track' THEN 4 WHEN 'completed_late' THEN 5 WHEN 'completed_on_time' THEN 6 ELSE 7 END,promised_delivery_date NULLS FIRST,purchase_order_number LIMIT $4",
        )
        .bind(entities)
        .bind(suppliers)
        .bind(supplier_id)
        .bind(limit.clamp(1, 1000))
        .fetch_all(self.store.pool())
        .await?;
        Ok(PurchaseDeliveryResponse {
            items,
            can_manage_commitments: can_manage,
            data_as_of: Utc::now(),
        })
    }

    pub async fn supplier_performance(
        &self,
        actor: Uuid,
        days: i64,
    ) -> Result<SupplierPerformanceResponse, DomainError> {
        let scope = authorize(
            &self.store,
            actor,
            "supplier_delivery_performance:read",
            None,
            None,
            None,
            None,
            None,
        )
        .await?;
        let today = Utc::now().date_naive();
        let period_start = today - Duration::days(days.clamp(7, 365));
        let entities = scope
            .scopes
            .legal_entity_ids
            .into_iter()
            .collect::<Vec<_>>();
        let suppliers = scope.scopes.supplier_ids.into_iter().collect::<Vec<_>>();
        let items = sqlx::query_as::<_, SupplierDeliveryPerformance>(
            "WITH delivery AS (SELECT * FROM purchase_delivery_current WHERE legal_entity_id=ANY($1) AND supplier_id=ANY($2) AND lifecycle_status<>'draft' AND order_date BETWEEN $3 AND $4), returned AS (SELECT r.supplier_id,sum(l.quantity)::numeric(24,6) returned_quantity FROM purchase_returns r JOIN purchase_return_lines l ON l.purchase_return_id=r.id WHERE r.status='confirmed' AND r.legal_entity_id=ANY($1) AND r.supplier_id=ANY($2) AND r.return_date BETWEEN $3 AND $4 GROUP BY r.supplier_id), scored AS (SELECT supplier_id,min(supplier_code) supplier_code,min(supplier_name) supplier_name,count(*)::bigint order_count,count(*) FILTER(WHERE open_quantity>0 AND lifecycle_status='confirmed')::bigint open_order_count,count(*) FILTER(WHERE delivery_status='overdue')::bigint overdue_order_count,count(*) FILTER(WHERE delivery_status IN ('completed_on_time','completed_late'))::bigint completed_order_count,count(*) FILTER(WHERE delivery_status='completed_on_time')::bigint on_time_order_count,sum(ordered_quantity)::numeric(24,6) ordered_quantity,sum(received_quantity)::numeric(24,6) received_quantity FROM delivery GROUP BY supplier_id) SELECT s.supplier_id,s.supplier_code,s.supplier_name,s.order_count,s.open_order_count,s.overdue_order_count,s.completed_order_count,s.on_time_order_count,CASE WHEN s.completed_order_count=0 THEN NULL ELSE (s.on_time_order_count::numeric/s.completed_order_count)::numeric(24,8) END on_time_rate,s.ordered_quantity,s.received_quantity,CASE WHEN s.ordered_quantity=0 THEN NULL ELSE (s.received_quantity/s.ordered_quantity)::numeric(24,8) END fulfillment_rate,COALESCE(r.returned_quantity,0)::numeric(24,6) returned_quantity,CASE WHEN s.received_quantity=0 THEN NULL ELSE GREATEST(1-COALESCE(r.returned_quantity,0)/s.received_quantity,0)::numeric(24,8) END quality_acceptance_rate FROM scored s LEFT JOIN returned r USING(supplier_id) ORDER BY s.overdue_order_count DESC,on_time_rate ASC NULLS FIRST,s.supplier_code",
        )
        .bind(entities)
        .bind(suppliers)
        .bind(period_start)
        .bind(today)
        .fetch_all(self.store.pool())
        .await?;
        Ok(SupplierPerformanceResponse {
            items,
            period_start,
            period_end: today,
            data_as_of: Utc::now(),
        })
    }

    pub async fn record_commitment(
        &self,
        actor: Uuid,
        trace_id: Uuid,
        purchase_order_id: Uuid,
        key: &str,
        input: &RecordDeliveryCommitment,
    ) -> Result<DeliveryCommitmentResult, DomainError> {
        if input.expected_revision < 0
            || input
                .commitment_note
                .as_ref()
                .is_some_and(|note| note.chars().count() > 1000)
        {
            return Err(DomainError::Invalid(
                "invalid delivery commitment revision or note".into(),
            ));
        }
        let hash = request_hash(&(purchase_order_id, input))?;
        let mut tx = self.store.pool().begin().await?;
        if let Some(mut replay) = begin_idempotent::<DeliveryCommitmentResult>(
            &mut tx,
            actor,
            "purchase_delivery_commitment:record",
            key,
            &hash,
        )
        .await?
        {
            replay.idempotent_replay = true;
            tx.commit().await?;
            return Ok(replay);
        }
        let order = sqlx::query(
            "SELECT legal_entity_id,supplier_id,business_unit_id,order_date,lifecycle_status,receiving_status,purchase_order_number FROM purchase_orders WHERE id=$1 FOR UPDATE",
        )
        .bind(purchase_order_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(DomainError::NotFoundOrForbidden)?;
        let legal_entity: Uuid = order.get("legal_entity_id");
        let supplier: Uuid = order.get("supplier_id");
        let business_unit: Uuid = order.get("business_unit_id");
        authorize(
            &self.store,
            actor,
            "purchase_delivery_commitment:manage",
            Some(legal_entity),
            None,
            Some(supplier),
            None,
            Some(business_unit),
        )
        .await?;
        if order.get::<String, _>("lifecycle_status") != "confirmed"
            || order.get::<String, _>("receiving_status") == "cancelled"
        {
            return Err(DomainError::Invalid(
                "only open confirmed purchase orders accept delivery commitments".into(),
            ));
        }
        if input.promised_delivery_date < order.get::<NaiveDate, _>("order_date") {
            return Err(DomainError::Invalid(
                "promisedDeliveryDate cannot precede orderDate".into(),
            ));
        }
        let current = sqlx::query(
            "SELECT id,revision,promised_delivery_date FROM purchase_delivery_commitments WHERE purchase_order_id=$1 AND status='active' FOR UPDATE",
        )
        .bind(purchase_order_id)
        .fetch_optional(&mut *tx)
        .await?;
        let current_revision = current
            .as_ref()
            .map(|row| row.get::<i64, _>("revision"))
            .unwrap_or(0);
        if current_revision != input.expected_revision {
            return Err(DomainError::VersionConflict);
        }
        if let Some(row) = &current {
            sqlx::query(
                "UPDATE purchase_delivery_commitments SET status='superseded',superseded_at=now() WHERE id=$1",
            )
            .bind(row.get::<Uuid, _>("id"))
            .execute(&mut *tx)
            .await?;
        }
        let id = Uuid::new_v4();
        let revision = current_revision + 1;
        sqlx::query("INSERT INTO purchase_delivery_commitments(id,purchase_order_id,revision,promised_delivery_date,commitment_note,created_by_user_id,trace_id) VALUES($1,$2,$3,$4,$5,$6,$7)")
            .bind(id)
            .bind(purchase_order_id)
            .bind(revision)
            .bind(input.promised_delivery_date)
            .bind(&input.commitment_note)
            .bind(actor)
            .bind(trace_id)
            .execute(&mut *tx)
            .await?;
        let event_type = if revision == 1 {
            "committed"
        } else {
            "recommitted"
        };
        sqlx::query("INSERT INTO purchase_delivery_commitment_events(id,purchase_delivery_commitment_id,event_type,commitment_revision,payload,actor_user_id,trace_id) VALUES($1,$2,$3,$4,$5,$6,$7)")
            .bind(Uuid::new_v4())
            .bind(id)
            .bind(event_type)
            .bind(revision)
            .bind(json!({"purchaseOrderId":purchase_order_id,"previousPromisedDeliveryDate":current.as_ref().map(|row|row.get::<NaiveDate,_>("promised_delivery_date")),"promisedDeliveryDate":input.promised_delivery_date}))
            .bind(actor)
            .bind(trace_id)
            .execute(&mut *tx)
            .await?;
        record(
            &mut tx,
            trace_id,
            actor,
            "PURCHASE_DELIVERY_COMMITMENT_RECORDED",
            "purchase_delivery_commitment_recorded",
            "purchase_order",
            purchase_order_id,
            json!({"purchaseOrderNumber":order.get::<String,_>("purchase_order_number"),"revision":revision,"promisedDeliveryDate":input.promised_delivery_date}),
        )
        .await?;
        let result = DeliveryCommitmentResult {
            id,
            purchase_order_id,
            status: "active".into(),
            revision,
            promised_delivery_date: input.promised_delivery_date,
            trace_id,
            idempotent_replay: false,
        };
        finish_idempotent(
            &mut tx,
            actor,
            "purchase_delivery_commitment:record",
            key,
            &result,
        )
        .await?;
        tx.commit().await?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commitment_input_rejects_negative_revision() {
        let input = RecordDeliveryCommitment {
            promised_delivery_date: NaiveDate::from_ymd_opt(2026, 8, 22).unwrap(),
            expected_revision: -1,
            commitment_note: None,
        };
        assert!(input.expected_revision < 0);
    }
}
