use super::common::authorize;
use crate::{
    b2::{
        common::{money, record},
        DomainError,
    },
    store::PgStore,
};
use chrono::{Datelike, Utc};
use rust_decimal::Decimal;
use serde_json::json;
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;

const CONSUMER: &str = "profit_projection_v1";

#[derive(Clone)]
pub struct ProfitProjectionService {
    store: PgStore,
    retry_limit: u32,
}

impl ProfitProjectionService {
    pub fn new(store: PgStore) -> Self {
        Self {
            store,
            retry_limit: 5,
        }
    }

    pub fn with_retry_limit(store: PgStore, retry_limit: u32) -> Self {
        Self { store, retry_limit }
    }

    pub async fn project_pending(
        &self,
        actor: Uuid,
        trace_id: Uuid,
        limit: i64,
    ) -> Result<serde_json::Value, DomainError> {
        let mut tx = self.store.pool().begin().await?;
        let retry_events=sqlx::query("SELECT o.id,o.topic,o.aggregate_id,o.payload,o.created_at FROM profit_projection_failures f JOIN business_core_outbox o ON o.id=f.outbox_event_id WHERE f.status='pending' AND f.retry_count<$1 ORDER BY f.last_failed_at,f.outbox_event_id LIMIT $2")
            .bind(i64::from(self.retry_limit)).bind(limit.clamp(1,1000)).fetch_all(&mut *tx).await?;
        let mut retried = 0_i64;
        let mut projected = 0_i64;
        let mut failures = 0_i64;
        for event in &retry_events {
            retried += 1;
            match project_event(&mut tx, event, actor, trace_id).await {
                Ok(count) => projected += count,
                Err(error) => {
                    failures += 1;
                    sqlx::query("UPDATE profit_projection_failures SET retry_count=retry_count+1,last_failed_at=now(),error_summary=$2,trace_id=$3 WHERE outbox_event_id=$1")
                        .bind(event.get::<Uuid,_>("id")).bind(error.to_string()).bind(trace_id).execute(&mut *tx).await?;
                }
            }
        }
        sqlx::query("INSERT INTO profit_projection_offsets(consumer_name) VALUES($1) ON CONFLICT DO NOTHING")
            .bind(CONSUMER)
            .execute(&mut *tx)
            .await?;
        let offset=sqlx::query("SELECT last_outbox_created_at,last_outbox_event_id FROM profit_projection_offsets WHERE consumer_name=$1 FOR UPDATE")
            .bind(CONSUMER).fetch_one(&mut *tx).await?;
        let last_created = offset.get::<Option<chrono::DateTime<Utc>>, _>("last_outbox_created_at");
        let last_id = offset.get::<Option<Uuid>, _>("last_outbox_event_id");
        let events=sqlx::query("SELECT id,topic,aggregate_id,payload,created_at FROM business_core_outbox WHERE topic IN ('shipment_confirmed','shipment_reversed','sales_return_confirmed') AND ($2::timestamptz IS NULL OR (created_at,id)>($2,$3)) ORDER BY created_at,id LIMIT $1")
            .bind(limit.clamp(1,1000)).bind(last_created).bind(last_id)
            .fetch_all(&mut *tx)
            .await?;
        for event in &events {
            match project_event(&mut tx, event, actor, trace_id).await {
                Ok(count) => projected += count,
                Err(error) => {
                    failures += 1;
                    let aggregate_id =
                        Uuid::parse_str(event.get::<String, _>("aggregate_id").as_str())
                            .unwrap_or_else(|_| Uuid::nil());
                    sqlx::query("INSERT INTO profit_projection_failures(id,outbox_event_id,topic,aggregate_id,error_code,error_summary,trace_id) VALUES($1,$2,$3,$4,'PROJECTION_FAILED',$5,$6) ON CONFLICT(outbox_event_id) DO UPDATE SET retry_count=profit_projection_failures.retry_count+1,last_failed_at=now(),error_summary=EXCLUDED.error_summary,trace_id=EXCLUDED.trace_id")
                        .bind(Uuid::new_v4()).bind(event.get::<Uuid,_>("id")).bind(event.get::<String,_>("topic")).bind(aggregate_id).bind(error.to_string()).bind(trace_id).execute(&mut *tx).await?;
                }
            }
            sqlx::query("UPDATE profit_projection_offsets SET last_outbox_created_at=$2,last_outbox_event_id=$3,last_fact_sequence=(SELECT max(fact_sequence) FROM profit_facts) WHERE consumer_name=$1")
                .bind(CONSUMER).bind(event.get::<chrono::DateTime<Utc>,_>("created_at")).bind(event.get::<Uuid,_>("id")).execute(&mut *tx).await?;
        }
        sqlx::query("UPDATE profit_projection_offsets SET updated_at=now(),version=version+1 WHERE consumer_name=$1")
            .bind(CONSUMER)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(
            json!({"events":events.len(),"retried":retried,"factsProjected":projected,"failures":failures,"dataAsOf":Utc::now()}),
        )
    }

    pub async fn rebuild(
        &self,
        actor: Uuid,
        trace_id: Uuid,
    ) -> Result<serde_json::Value, DomainError> {
        sqlx::query("INSERT INTO profit_projection_offsets(consumer_name) VALUES($1) ON CONFLICT(consumer_name) DO UPDATE SET last_outbox_created_at=NULL,last_outbox_event_id=NULL,last_fact_sequence=NULL,updated_at=now(),version=profit_projection_offsets.version+1")
            .bind(CONSUMER).execute(self.store.pool()).await?;
        let mut events = 0_i64;
        let mut projected = 0_i64;
        loop {
            let result = self.project_pending(actor, trace_id, 500).await?;
            let count = result["events"].as_i64().unwrap_or_default();
            events += count;
            projected += result["factsProjected"].as_i64().unwrap_or_default();
            if count < 500 {
                break;
            }
        }
        let mut tx = self.store.pool().begin().await?;
        record(
            &mut tx,
            trace_id,
            actor,
            "PROFIT_FACT_REBUILD_COMPLETED",
            "profit_fact_rebuild_completed",
            "profit_projection",
            Uuid::nil(),
            json!({"events":events,"factsProjected":projected}),
        )
        .await?;
        tx.commit().await?;
        Ok(json!({"events":events,"factsProjected":projected,"idempotentFactsPreserved":true}))
    }

    pub async fn reconcile(&self, actor: Uuid) -> Result<serde_json::Value, DomainError> {
        let scope = authorize(
            &self.store,
            actor,
            "profit:read",
            None,
            None,
            None,
            None,
            None,
        )
        .await?;
        let rows=sqlx::query("SELECT r.shipment_line_id,r.shipment_id,r.sales_order_id,r.status,r.expected_revenue,r.actual_revenue,r.revenue_difference,r.expected_cost,r.actual_cost,r.cost_difference,r.fact_count,r.last_fact_sequence FROM profit_projection_reconciliation r JOIN sales_orders o ON o.id=r.sales_order_id WHERE (r.revenue_difference<>0 OR r.cost_difference<>0) AND o.legal_entity_id=ANY($1) AND o.customer_id=ANY($2) AND (o.brand_id IS NULL OR o.brand_id=ANY($3)) AND o.business_unit_id=ANY($4) AND NOT EXISTS(SELECT 1 FROM profit_facts f WHERE f.sales_order_id=o.id AND f.warehouse_id IS NOT NULL AND NOT(f.warehouse_id=ANY($5))) ORDER BY r.shipment_id,r.shipment_line_id LIMIT 500")
            .bind(scope.scopes.legal_entity_ids.into_iter().collect::<Vec<_>>()).bind(scope.scopes.customer_ids.into_iter().collect::<Vec<_>>()).bind(scope.scopes.brand_ids.into_iter().collect::<Vec<_>>()).bind(scope.scopes.business_unit_ids.into_iter().collect::<Vec<_>>()).bind(scope.scopes.warehouse_ids.into_iter().collect::<Vec<_>>()).fetch_all(self.store.pool()).await?;
        Ok(
            json!({"consistent":rows.is_empty(),"differences":rows.into_iter().map(|row|json!({
            "shipmentLineId":row.get::<Uuid,_>("shipment_line_id"),"shipmentId":row.get::<Uuid,_>("shipment_id"),"salesOrderId":row.get::<Uuid,_>("sales_order_id"),"status":row.get::<String,_>("status"),
            "expectedRevenue":row.get::<Decimal,_>("expected_revenue").to_string(),"actualRevenue":row.get::<Decimal,_>("actual_revenue").to_string(),"revenueDifference":row.get::<Decimal,_>("revenue_difference").to_string(),
            "expectedCost":row.get::<Option<Decimal>,_>("expected_cost").unwrap_or_default().to_string(),"actualCost":row.get::<Decimal,_>("actual_cost").to_string(),"costDifference":row.get::<Decimal,_>("cost_difference").to_string(),"factCount":row.get::<i64,_>("fact_count"),"lastFactSequence":row.get::<Option<i64>,_>("last_fact_sequence")
        })).collect::<Vec<_>>() }),
        )
    }
}

async fn project_event(
    tx: &mut Transaction<'_, Postgres>,
    event: &sqlx::postgres::PgRow,
    actor: Uuid,
    trace_id: Uuid,
) -> Result<i64, DomainError> {
    let outbox_id: Uuid = event.get("id");
    let topic: String = event.get("topic");
    if topic == "sales_return_confirmed" {
        return project_sales_return(tx, event, actor, trace_id).await;
    }
    let shipment_id = Uuid::parse_str(event.get::<String, _>("aggregate_id").as_str())
        .map_err(|_| DomainError::Invalid("shipment outbox aggregate is invalid".into()))?;
    let payload: serde_json::Value = event.get("payload");
    let version = payload["version"]
        .as_i64()
        .unwrap_or(if topic == "shipment_confirmed" { 2 } else { 3 });
    let direction = if topic == "shipment_confirmed" {
        "normal"
    } else {
        "reversal"
    };
    let source_event_type = if topic == "shipment_confirmed" {
        "confirmed"
    } else {
        "reversed"
    };
    let attribution = sqlx::query("SELECT actor_user_id,trace_id FROM shipment_events WHERE shipment_id=$1 AND event_type=$2 AND shipment_version=$3 ORDER BY created_at DESC LIMIT 1")
        .bind(shipment_id).bind(source_event_type).bind(version).fetch_optional(&mut **tx).await?;
    let (effective_actor, effective_trace) = attribution
        .map(|row| {
            (
                row.get::<Uuid, _>("actor_user_id"),
                row.get::<Uuid, _>("trace_id"),
            )
        })
        .unwrap_or((actor, trace_id));
    let lines=sqlx::query("SELECT s.sales_order_id,s.legal_entity_id,s.customer_id,s.warehouse_id,s.shipment_date,s.currency::text,sl.id shipment_line_id,sl.sales_order_line_id,sl.sku_id,sl.quantity,sl.sales_amount,sl.total_cost,sl.cost_snapshot_at,so.salesperson_user_id,sol.business_unit_id,sol.department_id,COALESCE(sol.brand_id,p.brand_id) brand_id,p.category_id product_category_id FROM shipments s JOIN shipment_lines sl ON sl.shipment_id=s.id JOIN sales_orders so ON so.id=s.sales_order_id JOIN sales_order_lines sol ON sol.id=sl.sales_order_line_id JOIN business_skus sku ON sku.id=sl.sku_id JOIN business_products p ON p.id=sku.product_id WHERE s.id=$1 ORDER BY sl.id")
        .bind(shipment_id).fetch_all(&mut **tx).await?;
    if lines.is_empty() {
        return Err(DomainError::NotFoundOrForbidden);
    }
    let mut inserted = 0_i64;
    for line in lines {
        let cost = line
            .get::<Option<Decimal>, _>("total_cost")
            .ok_or(DomainError::MissingInventoryCost)?;
        let period = format!(
            "{:04}-{:02}",
            line.get::<chrono::NaiveDate, _>("shipment_date").year(),
            line.get::<chrono::NaiveDate, _>("shipment_date").month()
        );
        for (metric, amount) in [
            ("net_revenue", line.get::<Decimal, _>("sales_amount")),
            ("product_cost", cost),
        ] {
            let result=sqlx::query("INSERT INTO profit_facts(id,metric_type,direction,amount,currency,quantity,legal_entity_id,sales_order_id,sales_order_line_id,shipment_id,shipment_line_id,customer_id,sku_id,product_category_id,brand_id,salesperson_user_id,business_unit_id,department_id,warehouse_id,business_date,management_period,source_system,source_type,source_id,source_line_id,source_event_id,source_event_version,data_as_of,trace_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21,'business_core_b2','shipment',$10,$11,$22,$23,$24,$25) ON CONFLICT(source_event_id,metric_type,source_line_id,direction) DO NOTHING")
                .bind(Uuid::new_v4()).bind(metric).bind(direction).bind(money(amount)).bind(line.get::<String,_>("currency")).bind(line.get::<Decimal,_>("quantity")).bind(line.get::<Uuid,_>("legal_entity_id")).bind(line.get::<Uuid,_>("sales_order_id")).bind(line.get::<Uuid,_>("sales_order_line_id")).bind(shipment_id).bind(line.get::<Uuid,_>("shipment_line_id")).bind(line.get::<Uuid,_>("customer_id")).bind(line.get::<Uuid,_>("sku_id")).bind(line.get::<Uuid,_>("product_category_id")).bind(line.get::<Option<Uuid>,_>("brand_id")).bind(line.get::<Uuid,_>("salesperson_user_id")).bind(line.get::<Uuid,_>("business_unit_id")).bind(line.get::<Option<Uuid>,_>("department_id")).bind(line.get::<Uuid,_>("warehouse_id")).bind(line.get::<chrono::NaiveDate,_>("shipment_date")).bind(period.clone()).bind(outbox_id).bind(version).bind(event.get::<chrono::DateTime<Utc>,_>("created_at")).bind(effective_trace).execute(&mut **tx).await?;
            inserted += i64::try_from(result.rows_affected()).unwrap_or_default();
        }
    }
    if inserted > 0 {
        record(
            tx,
            effective_trace,
            effective_actor,
            "PROFIT_FACT_PROJECTED",
            "profit_fact_projected",
            "shipment",
            shipment_id,
            json!({"sourceEventId":outbox_id,"direction":direction,"factCount":inserted}),
        )
        .await?;
    }
    sqlx::query("UPDATE profit_projection_failures SET status='resolved',resolved_at=now() WHERE outbox_event_id=$1 AND status='pending'")
        .bind(outbox_id).execute(&mut **tx).await?;
    Ok(inserted)
}

async fn project_sales_return(
    tx: &mut Transaction<'_, Postgres>,
    event: &sqlx::postgres::PgRow,
    actor: Uuid,
    trace_id: Uuid,
) -> Result<i64, DomainError> {
    let outbox_id: Uuid = event.get("id");
    let return_id = Uuid::parse_str(event.get::<String, _>("aggregate_id").as_str())
        .map_err(|_| DomainError::Invalid("sales return aggregate is invalid".into()))?;
    let payload: serde_json::Value = event.get("payload");
    let version = payload["version"].as_i64().unwrap_or(2);
    let attribution = sqlx::query("SELECT actor_user_id,trace_id FROM sales_return_events WHERE sales_return_id=$1 AND event_type='confirmed' AND return_version=$2 ORDER BY created_at DESC LIMIT 1")
        .bind(return_id).bind(version).fetch_optional(&mut **tx).await?;
    let (effective_actor, effective_trace) = attribution
        .map(|row| (row.get("actor_user_id"), row.get("trace_id")))
        .unwrap_or((actor, trace_id));
    let lines=sqlx::query("SELECT r.sales_order_id,r.legal_entity_id,r.customer_id,r.warehouse_id,r.return_date,r.currency::text,l.shipment_line_id,l.sales_amount,l.total_cost,l.quantity,sl.sales_order_line_id,sl.shipment_id,l.sku_id,o.salesperson_user_id,sol.business_unit_id,sol.department_id,COALESCE(sol.brand_id,p.brand_id) brand_id,p.category_id product_category_id FROM sales_returns r JOIN sales_return_lines l ON l.sales_return_id=r.id JOIN shipment_lines sl ON sl.id=l.shipment_line_id JOIN sales_orders o ON o.id=r.sales_order_id JOIN sales_order_lines sol ON sol.id=sl.sales_order_line_id JOIN business_skus sku ON sku.id=l.sku_id JOIN business_products p ON p.id=sku.product_id WHERE r.id=$1 AND r.status='confirmed' ORDER BY l.id")
        .bind(return_id).fetch_all(&mut **tx).await?;
    if lines.is_empty() {
        return Err(DomainError::NotFoundOrForbidden);
    }
    let mut inserted = 0_i64;
    for line in lines {
        let date: chrono::NaiveDate = line.get("return_date");
        let period = format!("{:04}-{:02}", date.year(), date.month());
        for (metric, amount) in [
            ("net_revenue", line.get::<Decimal, _>("sales_amount")),
            ("product_cost", line.get::<Decimal, _>("total_cost")),
        ] {
            let result=sqlx::query("INSERT INTO profit_facts(id,metric_type,direction,amount,currency,quantity,legal_entity_id,sales_order_id,sales_order_line_id,shipment_id,shipment_line_id,customer_id,sku_id,product_category_id,brand_id,salesperson_user_id,business_unit_id,department_id,warehouse_id,business_date,management_period,source_system,source_type,source_id,source_line_id,source_event_id,source_event_version,data_as_of,trace_id) VALUES($1,$2,'reversal',$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,'business_core_returns','sales_return',$21,$22,$23,$24,$25,$26) ON CONFLICT(source_event_id,metric_type,source_line_id,direction) DO NOTHING")
                .bind(Uuid::new_v4()).bind(metric).bind(money(amount)).bind(line.get::<String,_>("currency")).bind(line.get::<Decimal,_>("quantity")).bind(line.get::<Uuid,_>("legal_entity_id")).bind(line.get::<Uuid,_>("sales_order_id")).bind(line.get::<Uuid,_>("sales_order_line_id")).bind(line.get::<Uuid,_>("shipment_id")).bind(line.get::<Uuid,_>("shipment_line_id")).bind(line.get::<Uuid,_>("customer_id")).bind(line.get::<Uuid,_>("sku_id")).bind(line.get::<Uuid,_>("product_category_id")).bind(line.get::<Option<Uuid>,_>("brand_id")).bind(line.get::<Uuid,_>("salesperson_user_id")).bind(line.get::<Uuid,_>("business_unit_id")).bind(line.get::<Option<Uuid>,_>("department_id")).bind(line.get::<Uuid,_>("warehouse_id")).bind(date).bind(period.clone()).bind(return_id).bind(line.get::<Uuid,_>("shipment_line_id")).bind(outbox_id).bind(version).bind(event.get::<chrono::DateTime<Utc>,_>("created_at")).bind(effective_trace).execute(&mut **tx).await?;
            inserted += i64::try_from(result.rows_affected()).unwrap_or_default();
        }
    }
    if inserted > 0 {
        record(
            tx,
            effective_trace,
            effective_actor,
            "PROFIT_FACT_PROJECTED",
            "profit_fact_projected",
            "sales_return",
            return_id,
            json!({"sourceEventId":outbox_id,"direction":"reversal","factCount":inserted}),
        )
        .await?;
    }
    sqlx::query("UPDATE profit_projection_failures SET status='resolved',resolved_at=now() WHERE outbox_event_id=$1 AND status='pending'").bind(outbox_id).execute(&mut **tx).await?;
    Ok(inserted)
}
