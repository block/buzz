use super::OperationsService;
use crate::{
    b2::{
        common::{authorize, begin_idempotent, finish_idempotent, request_hash},
        DomainError,
    },
    store::audit,
};
use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc, Weekday};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;

const SUBSCRIPTION_PERMISSION: &str = "management_report:manage_subscriptions";

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GenerateOperatingSnapshot {
    pub cadence: String,
    pub currency: String,
    pub period_start: NaiveDate,
    pub utc_offset_minutes: i16,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateSubscription {
    pub cadence: String,
    pub currency: String,
    pub utc_offset_minutes: i16,
    pub delivery_hour: i16,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubscriptionCommand {
    pub action: String,
    pub expected_version: i64,
}

impl OperationsService {
    pub async fn operating_trends(
        &self,
        actor: Uuid,
        cadence: &str,
        currency: &str,
        limit: i64,
    ) -> Result<Value, DomainError> {
        validate_cadence(cadence)?;
        crate::b2::common::validate_currency(currency)?;
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
        let rows = sqlx::query("SELECT id,cadence,period_start,period_end,currency::text,payload,data_quality_status,source_hash,generated_at,trace_id FROM operating_report_snapshots WHERE scope_hash=$1 AND cadence=$2 AND currency=$3 ORDER BY period_start DESC LIMIT $4")
            .bind(&auth.effective_scope_hash).bind(cadence).bind(currency).bind(limit.clamp(2, 60)).fetch_all(self.store.pool()).await?;
        let payloads = rows
            .iter()
            .map(|row| row.get::<Value, _>("payload"))
            .collect::<Vec<_>>();
        let items = rows
            .iter()
            .enumerate()
            .map(|(index, row)| {
                let payload = payloads[index].clone();
                let comparison = payloads.get(index + 1);
                json!({
                    "id": row.get::<Uuid,_>("id"),
                    "cadence": row.get::<String,_>("cadence"),
                    "periodStart": row.get::<NaiveDate,_>("period_start"),
                    "periodEnd": row.get::<NaiveDate,_>("period_end"),
                    "currency": row.get::<String,_>("currency"),
                    "metrics": payload,
                    "change": comparison.map(|previous| trend_change(&payload, previous)),
                    "dataQualityStatus": row.get::<String,_>("data_quality_status"),
                    "sourceHash": row.get::<String,_>("source_hash"),
                    "generatedAt": row.get::<DateTime<Utc>,_>("generated_at"),
                    "traceId": row.get::<Uuid,_>("trace_id")
                })
            })
            .collect::<Vec<_>>();
        Ok(json!({
            "items": items,
            "cadence": cadence,
            "currency": currency,
            "scopeVersion": auth.scope_version,
            "effectiveScopeHash": auth.effective_scope_hash,
            "dataAsOf": Utc::now(),
            "boundary": "business_operations_only_not_financial_accounting"
        }))
    }

    pub async fn generate_operating_snapshot(
        &self,
        actor: Uuid,
        trace_id: Uuid,
        idempotency_key: &str,
        input: &GenerateOperatingSnapshot,
    ) -> Result<Value, DomainError> {
        let hash = request_hash(input)?;
        let mut tx = self.store.pool().begin().await?;
        if let Some(value) = begin_idempotent::<Value>(
            &mut tx,
            actor,
            "operating_snapshot_generate",
            idempotency_key,
            &hash,
        )
        .await?
        {
            tx.commit().await?;
            return Ok(value);
        }
        let result = self
            .generate_operating_snapshot_once(actor, trace_id, input)
            .await?;
        finish_idempotent(
            &mut tx,
            actor,
            "operating_snapshot_generate",
            idempotency_key,
            &result,
        )
        .await?;
        tx.commit().await?;
        Ok(result)
    }

    async fn generate_operating_snapshot_once(
        &self,
        actor: Uuid,
        trace_id: Uuid,
        input: &GenerateOperatingSnapshot,
    ) -> Result<Value, DomainError> {
        validate_snapshot_input(input)?;
        let auth = authorize(
            &self.store,
            actor,
            "management_report:generate_snapshot",
            None,
            None,
            None,
            None,
            None,
        )
        .await?;
        let period_end =
            input.period_start + Duration::days(if input.cadence == "daily" { 1 } else { 7 });
        let local_today =
            (Utc::now() + Duration::minutes(i64::from(input.utc_offset_minutes))).date_naive();
        if period_end > local_today {
            return Err(DomainError::Invalid(
                "only completed operating periods can be frozen".into(),
            ));
        }
        if let Some(row) = sqlx::query("SELECT id,generated_at,source_hash,data_quality_status FROM operating_report_snapshots WHERE cadence=$1 AND period_start=$2 AND currency=$3 AND scope_hash=$4")
            .bind(&input.cadence).bind(input.period_start).bind(&input.currency).bind(&auth.effective_scope_hash).fetch_optional(self.store.pool()).await?
        {
            return Ok(snapshot_result(&row, false, trace_id));
        }
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
        let sales = sqlx::query("SELECT count(*) order_count,COALESCE(sum(gross_amount),0)::numeric(24,6) order_amount FROM sales_orders WHERE order_date>=$1 AND order_date<$2 AND currency=$3 AND legal_entity_id=ANY($4) AND customer_id=ANY($5) AND (brand_id IS NULL OR brand_id=ANY($6)) AND business_unit_id=ANY($7)")
            .bind(input.period_start).bind(period_end).bind(&input.currency).bind(&le).bind(&customer).bind(&brand).bind(&bu).fetch_one(self.store.pool()).await?;
        let shipments = sqlx::query("SELECT count(*) FILTER(WHERE s.status='confirmed') shipment_count,COALESCE(sum(s.sales_amount) FILTER(WHERE s.status='confirmed'),0)::numeric(24,6) shipped_revenue FROM shipments s JOIN sales_orders o ON o.id=s.sales_order_id WHERE s.shipment_date>=$1 AND s.shipment_date<$2 AND s.currency=$3 AND s.legal_entity_id=ANY($4) AND s.customer_id=ANY($5) AND s.warehouse_id=ANY($6) AND (o.brand_id IS NULL OR o.brand_id=ANY($7)) AND o.business_unit_id=ANY($8)")
            .bind(input.period_start).bind(period_end).bind(&input.currency).bind(&le).bind(&customer).bind(&wh).bind(&brand).bind(&bu).fetch_one(self.store.pool()).await?;
        let purchasing = sqlx::query("SELECT count(*) purchase_order_count,COALESCE(sum(gross_amount),0)::numeric(24,6) purchase_order_amount FROM purchase_orders WHERE order_date>=$1 AND order_date<$2 AND currency=$3 AND legal_entity_id=ANY($4) AND supplier_id=ANY($5) AND (brand_id IS NULL OR brand_id=ANY($6)) AND business_unit_id=ANY($7)")
            .bind(input.period_start).bind(period_end).bind(&input.currency).bind(&le).bind(&supplier).bind(&brand).bind(&bu).fetch_one(self.store.pool()).await?;
        let inventory = sqlx::query("SELECT COALESCE(sum(b.inventory_value),0)::numeric(24,6) inventory_value,count(*) FILTER(WHERE b.on_hand_quantity-b.reserved_quantity=0 AND b.reserved_quantity>0) stockout_count FROM inventory_balances b JOIN business_legal_entities e ON e.id=b.legal_entity_id WHERE e.functional_currency=$1 AND b.legal_entity_id=ANY($2) AND b.warehouse_id=ANY($3)")
            .bind(&input.currency).bind(&le).bind(&wh).fetch_one(self.store.pool()).await?;
        let profit = sqlx::query("SELECT COALESCE(sum(CASE direction WHEN 'normal' THEN amount ELSE -amount END) FILTER(WHERE metric_type='net_revenue'),0)::numeric(24,6) revenue,COALESCE(sum(CASE direction WHEN 'normal' THEN amount ELSE -amount END) FILTER(WHERE metric_type='product_cost'),0)::numeric(24,6) product_cost,COALESCE(sum(CASE direction WHEN 'normal' THEN amount ELSE -amount END) FILTER(WHERE metric_type IN ('outbound_freight','sales_commission','platform_fee','customer_rebate','other_direct_cost','allocated_operating_expense')),0)::numeric(24,6) operating_cost,COALESCE(sum(CASE direction WHEN 'normal' THEN amount ELSE -amount END) FILTER(WHERE metric_type='supplier_rebate'),0)::numeric(24,6) supplier_rebate FROM profit_facts WHERE business_date>=$1 AND business_date<$2 AND currency=$3 AND legal_entity_id=ANY($4) AND customer_id=ANY($5) AND warehouse_id=ANY($6) AND (brand_id IS NULL OR brand_id=ANY($7)) AND business_unit_id=ANY($8)")
            .bind(input.period_start).bind(period_end).bind(&input.currency).bind(&le).bind(&customer).bind(&wh).bind(&brand).bind(&bu).fetch_one(self.store.pool()).await?;
        let incidents = sqlx::query("SELECT count(*) FILTER(WHERE first_seen_at >= $2::date AND first_seen_at < $3::date) opened_count,count(*) FILTER(WHERE resolved_at >= $2::date AND resolved_at < $3::date) resolved_count,count(*) FILTER(WHERE due_at < COALESCE(resolved_at,$3::date) AND first_seen_at < $3::date) breached_count,COALESCE(avg(EXTRACT(EPOCH FROM (resolved_at-first_seen_at))/3600) FILTER(WHERE resolved_at >= $2::date AND resolved_at < $3::date),0)::numeric(18,3) average_resolution_hours FROM operating_report_incidents WHERE scope_hash=$1")
            .bind(&auth.effective_scope_hash).bind(input.period_start).bind(period_end).fetch_one(self.store.pool()).await?;
        let revenue: Decimal = profit.get("revenue");
        let operating_profit = revenue
            - profit.get::<Decimal, _>("product_cost")
            - profit.get::<Decimal, _>("operating_cost")
            + profit.get::<Decimal, _>("supplier_rebate");
        let quality = self.data_quality(actor).await?;
        let quality_status = quality["status"].as_str().unwrap_or("blocked");
        let payload = json!({
            "salesOrderCount": sales.get::<i64,_>("order_count"),
            "salesOrderAmount": sales.get::<Decimal,_>("order_amount").to_string(),
            "shipmentCount": shipments.get::<i64,_>("shipment_count"),
            "shippedRevenue": shipments.get::<Decimal,_>("shipped_revenue").to_string(),
            "purchaseOrderCount": purchasing.get::<i64,_>("purchase_order_count"),
            "purchaseOrderAmount": purchasing.get::<Decimal,_>("purchase_order_amount").to_string(),
            "inventoryValueAsOfGeneration": inventory.get::<Decimal,_>("inventory_value").to_string(),
            "stockoutCountAsOfGeneration": inventory.get::<i64,_>("stockout_count"),
            "managementOperatingProfit": operating_profit.to_string(),
            "incidentsOpened": incidents.get::<i64,_>("opened_count"),
            "incidentsResolved": incidents.get::<i64,_>("resolved_count"),
            "slaBreached": incidents.get::<i64,_>("breached_count"),
            "averageResolutionHours": incidents.get::<Decimal,_>("average_resolution_hours").to_string()
        });
        let source_hash = hex::encode(Sha256::digest(serde_json::to_vec(&json!({
            "cadence": input.cadence,
            "periodStart": input.period_start,
            "periodEnd": period_end,
            "currency": input.currency,
            "scopeHash": auth.effective_scope_hash,
            "metrics": payload
        }))?));
        let id = Uuid::new_v4();
        let mut tx = self.store.pool().begin().await?;
        let inserted = sqlx::query("INSERT INTO operating_report_snapshots(id,cadence,period_start,period_end,currency,scope_hash,payload,data_quality_status,source_hash,generated_by_user_id,trace_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11) ON CONFLICT(cadence,period_start,currency,scope_hash) DO NOTHING RETURNING id,generated_at,source_hash,data_quality_status")
            .bind(id).bind(&input.cadence).bind(input.period_start).bind(period_end).bind(&input.currency).bind(&auth.effective_scope_hash).bind(&payload).bind(quality_status).bind(&source_hash).bind(actor).bind(trace_id).fetch_optional(&mut *tx).await?;
        let (row, created) = if let Some(row) = inserted {
            audit(&mut tx, trace_id, actor, "operating_snapshot.generate", "operating_report_snapshot", &id.to_string(), json!({"cadence":input.cadence,"periodStart":input.period_start,"periodEnd":period_end,"currency":input.currency,"sourceHash":source_hash})).await?;
            (row, true)
        } else {
            (sqlx::query("SELECT id,generated_at,source_hash,data_quality_status FROM operating_report_snapshots WHERE cadence=$1 AND period_start=$2 AND currency=$3 AND scope_hash=$4").bind(&input.cadence).bind(input.period_start).bind(&input.currency).bind(&auth.effective_scope_hash).fetch_one(&mut *tx).await?, false)
        };
        tx.commit().await?;
        Ok(snapshot_result(&row, created, trace_id))
    }

    pub async fn list_operating_subscriptions(&self, actor: Uuid) -> Result<Value, DomainError> {
        authorize(
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
        let rows = sqlx::query("SELECT id,cadence,currency::text,utc_offset_minutes,delivery_hour,status,next_run_at,last_run_at,last_snapshot_id,version FROM operating_report_subscriptions WHERE owner_user_id=$1 ORDER BY cadence,currency")
            .bind(actor).fetch_all(self.store.pool()).await?;
        Ok(
            json!({"items":rows.iter().map(subscription_json).collect::<Vec<_>>(),"dataAsOf":Utc::now(),"boundary":"in_dock_operating_snapshots_only"}),
        )
    }

    pub async fn create_operating_subscription(
        &self,
        actor: Uuid,
        trace_id: Uuid,
        key: &str,
        input: &CreateSubscription,
    ) -> Result<Value, DomainError> {
        validate_subscription(input)?;
        authorize(
            &self.store,
            actor,
            SUBSCRIPTION_PERMISSION,
            None,
            None,
            None,
            None,
            None,
        )
        .await?;
        let hash = request_hash(input)?;
        let mut tx = self.store.pool().begin().await?;
        if let Some(value) =
            begin_idempotent::<Value>(&mut tx, actor, "operating_subscription_create", key, &hash)
                .await?
        {
            tx.commit().await?;
            return Ok(value);
        }
        let id = Uuid::new_v4();
        let next_run = next_run_at(
            &input.cadence,
            input.utc_offset_minutes,
            input.delivery_hour,
            Utc::now(),
        );
        let row = sqlx::query("INSERT INTO operating_report_subscriptions(id,owner_user_id,cadence,currency,utc_offset_minutes,delivery_hour,next_run_at) VALUES($1,$2,$3,$4,$5,$6,$7) ON CONFLICT(owner_user_id,cadence,currency) DO UPDATE SET utc_offset_minutes=EXCLUDED.utc_offset_minutes,delivery_hour=EXCLUDED.delivery_hour,next_run_at=EXCLUDED.next_run_at,status='active',version=operating_report_subscriptions.version+1 RETURNING id,cadence,currency::text,utc_offset_minutes,delivery_hour,status,next_run_at,last_run_at,last_snapshot_id,version")
            .bind(id).bind(actor).bind(&input.cadence).bind(&input.currency).bind(input.utc_offset_minutes).bind(input.delivery_hour).bind(next_run).fetch_one(&mut *tx).await?;
        let subscription_id: Uuid = row.get("id");
        subscription_event(
            &mut tx,
            subscription_id,
            "created",
            actor,
            trace_id,
            json!({"nextRunAt":next_run}),
        )
        .await?;
        audit(
            &mut tx,
            trace_id,
            actor,
            "operating_subscription.save",
            "operating_report_subscription",
            &subscription_id.to_string(),
            json!({"cadence":input.cadence,"currency":input.currency,"nextRunAt":next_run}),
        )
        .await?;
        let result = subscription_json(&row);
        finish_idempotent(
            &mut tx,
            actor,
            "operating_subscription_create",
            key,
            &result,
        )
        .await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn command_operating_subscription(
        &self,
        actor: Uuid,
        trace_id: Uuid,
        key: &str,
        id: Uuid,
        input: &SubscriptionCommand,
    ) -> Result<Value, DomainError> {
        authorize(
            &self.store,
            actor,
            SUBSCRIPTION_PERMISSION,
            None,
            None,
            None,
            None,
            None,
        )
        .await?;
        let hash = request_hash(&json!({"id":id,"input":input}))?;
        let mut tx = self.store.pool().begin().await?;
        if let Some(value) =
            begin_idempotent::<Value>(&mut tx, actor, "operating_subscription_command", key, &hash)
                .await?
        {
            tx.commit().await?;
            return Ok(value);
        }
        let current = sqlx::query("SELECT cadence,currency::text,utc_offset_minutes,delivery_hour,status,version FROM operating_report_subscriptions WHERE id=$1 AND owner_user_id=$2 FOR UPDATE")
            .bind(id).bind(actor).fetch_optional(&mut *tx).await?.ok_or(DomainError::NotFoundOrForbidden)?;
        if current.get::<i64, _>("version") != input.expected_version {
            return Err(DomainError::VersionConflict);
        }
        if !matches!(input.action.as_str(), "pause" | "resume") {
            return Err(DomainError::Invalid(
                "subscription action must be pause or resume".into(),
            ));
        }
        let status = if input.action == "pause" {
            "paused"
        } else {
            "active"
        };
        let next_run = next_run_at(
            &current.get::<String, _>("cadence"),
            current.get("utc_offset_minutes"),
            current.get("delivery_hour"),
            Utc::now(),
        );
        let row = sqlx::query("UPDATE operating_report_subscriptions SET status=$2,next_run_at=$3,version=version+1 WHERE id=$1 RETURNING id,cadence,currency::text,utc_offset_minutes,delivery_hour,status,next_run_at,last_run_at,last_snapshot_id,version")
            .bind(id).bind(status).bind(next_run).fetch_one(&mut *tx).await?;
        subscription_event(
            &mut tx,
            id,
            if status == "paused" {
                "paused"
            } else {
                "resumed"
            },
            actor,
            trace_id,
            json!({"nextRunAt":next_run}),
        )
        .await?;
        audit(
            &mut tx,
            trace_id,
            actor,
            "operating_subscription.command",
            "operating_report_subscription",
            &id.to_string(),
            json!({"action":input.action,"status":status,"nextRunAt":next_run}),
        )
        .await?;
        let result = subscription_json(&row);
        finish_idempotent(
            &mut tx,
            actor,
            "operating_subscription_command",
            key,
            &result,
        )
        .await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn run_due_operating_subscriptions(&self, limit: i64) -> Result<i64, DomainError> {
        let due = sqlx::query("WITH claimed AS (SELECT id FROM operating_report_subscriptions WHERE status='active' AND next_run_at<=now() ORDER BY next_run_at FOR UPDATE SKIP LOCKED LIMIT $1) UPDATE operating_report_subscriptions s SET next_run_at=now()+interval '15 minutes',last_run_at=now(),version=version+1 FROM claimed WHERE s.id=claimed.id RETURNING s.id,s.owner_user_id,s.cadence,s.currency::text,s.utc_offset_minutes,s.delivery_hour")
            .bind(limit.clamp(1, 50)).fetch_all(self.store.pool()).await?;
        let mut completed = 0_i64;
        for row in due {
            let id: Uuid = row.get("id");
            let actor: Uuid = row.get("owner_user_id");
            let cadence: String = row.get("cadence");
            let offset: i16 = row.get("utc_offset_minutes");
            let hour: i16 = row.get("delivery_hour");
            let trace_id = Uuid::new_v4();
            let local_today = (Utc::now() + Duration::minutes(i64::from(offset))).date_naive();
            let period_start = if cadence == "daily" {
                local_today - Duration::days(1)
            } else {
                local_today
                    - Duration::days(7 + i64::from(local_today.weekday().num_days_from_monday()))
            };
            let input = GenerateOperatingSnapshot {
                cadence: cadence.clone(),
                currency: row.get("currency"),
                period_start,
                utc_offset_minutes: offset,
            };
            let result = self
                .generate_operating_snapshot_once(actor, trace_id, &input)
                .await;
            let mut tx = self.store.pool().begin().await?;
            let next_run = next_run_at(&cadence, offset, hour, Utc::now() + Duration::minutes(1));
            match result {
                Ok(snapshot) => {
                    let snapshot_id = Uuid::parse_str(
                        snapshot["id"]
                            .as_str()
                            .ok_or_else(|| DomainError::Invalid("snapshot id missing".into()))?,
                    )
                    .map_err(|_| DomainError::Invalid("snapshot id invalid".into()))?;
                    sqlx::query("UPDATE operating_report_subscriptions SET next_run_at=$2,last_run_at=now(),last_snapshot_id=$3,version=version+1 WHERE id=$1").bind(id).bind(next_run).bind(snapshot_id).execute(&mut *tx).await?;
                    subscription_event(
                        &mut tx,
                        id,
                        "generated",
                        actor,
                        trace_id,
                        json!({"snapshotId":snapshot_id,"periodStart":period_start}),
                    )
                    .await?;
                    completed += 1;
                }
                Err(error) => {
                    sqlx::query("UPDATE operating_report_subscriptions SET next_run_at=$2,last_run_at=now(),version=version+1 WHERE id=$1").bind(id).bind(next_run).execute(&mut *tx).await?;
                    subscription_event(
                        &mut tx,
                        id,
                        "failed",
                        actor,
                        trace_id,
                        json!({"reason":error.to_string()}),
                    )
                    .await?;
                }
            }
            tx.commit().await?;
        }
        Ok(completed)
    }
}

fn validate_cadence(value: &str) -> Result<(), DomainError> {
    if matches!(value, "daily" | "weekly") {
        Ok(())
    } else {
        Err(DomainError::Invalid(
            "cadence must be daily or weekly".into(),
        ))
    }
}

fn validate_snapshot_input(input: &GenerateOperatingSnapshot) -> Result<(), DomainError> {
    validate_cadence(&input.cadence)?;
    crate::b2::common::validate_currency(&input.currency)?;
    if !(-720..=840).contains(&input.utc_offset_minutes) {
        return Err(DomainError::Invalid(
            "UTC offset is outside the supported range".into(),
        ));
    }
    if input.cadence == "weekly" && input.period_start.weekday() != Weekday::Mon {
        return Err(DomainError::Invalid(
            "weekly period must start on Monday".into(),
        ));
    }
    Ok(())
}

fn validate_subscription(input: &CreateSubscription) -> Result<(), DomainError> {
    validate_cadence(&input.cadence)?;
    crate::b2::common::validate_currency(&input.currency)?;
    if !(-720..=840).contains(&input.utc_offset_minutes) || !(0..=23).contains(&input.delivery_hour)
    {
        return Err(DomainError::Invalid(
            "invalid schedule offset or delivery hour".into(),
        ));
    }
    Ok(())
}

fn next_run_at(cadence: &str, offset: i16, hour: i16, now: DateTime<Utc>) -> DateTime<Utc> {
    let local = now + Duration::minutes(i64::from(offset));
    let mut candidate = local
        .date_naive()
        .and_hms_opt(hour as u32, 0, 0)
        .unwrap_or(local.naive_utc());
    if cadence == "weekly" {
        candidate += Duration::days(i64::from((7 - local.weekday().num_days_from_monday()) % 7));
    }
    if candidate <= local.naive_utc() {
        candidate += Duration::days(if cadence == "daily" { 1 } else { 7 });
    }
    DateTime::<Utc>::from_naive_utc_and_offset(
        candidate - Duration::minutes(i64::from(offset)),
        Utc,
    )
}

fn snapshot_result(row: &sqlx::postgres::PgRow, created: bool, trace_id: Uuid) -> Value {
    json!({"id":row.get::<Uuid,_>("id"),"created":created,"generatedAt":row.get::<DateTime<Utc>,_>("generated_at"),"sourceHash":row.get::<String,_>("source_hash"),"dataQualityStatus":row.get::<String,_>("data_quality_status"),"traceId":trace_id})
}

fn trend_change(current: &Value, previous: &Value) -> Value {
    let fields = [
        "salesOrderAmount",
        "shippedRevenue",
        "purchaseOrderAmount",
        "managementOperatingProfit",
        "slaBreached",
    ];
    Value::Object(
        fields
            .into_iter()
            .map(|field| {
                (
                    field.to_string(),
                    percentage_change(&current[field], &previous[field]),
                )
            })
            .collect(),
    )
}

fn percentage_change(current: &Value, previous: &Value) -> Value {
    let parse = |value: &Value| {
        value
            .as_str()
            .and_then(|text| text.parse::<Decimal>().ok())
            .or_else(|| value.as_i64().map(Decimal::from))
    };
    match (parse(current), parse(previous)) {
        (Some(current), Some(previous)) if previous != Decimal::ZERO => {
            json!(((current - previous) / previous * Decimal::from(100))
                .round_dp(2)
                .to_string())
        }
        _ => Value::Null,
    }
}

fn subscription_json(row: &sqlx::postgres::PgRow) -> Value {
    json!({"id":row.get::<Uuid,_>("id"),"cadence":row.get::<String,_>("cadence"),"currency":row.get::<String,_>("currency"),"utcOffsetMinutes":row.get::<i16,_>("utc_offset_minutes"),"deliveryHour":row.get::<i16,_>("delivery_hour"),"status":row.get::<String,_>("status"),"nextRunAt":row.get::<DateTime<Utc>,_>("next_run_at"),"lastRunAt":row.get::<Option<DateTime<Utc>>,_>("last_run_at"),"lastSnapshotId":row.get::<Option<Uuid>,_>("last_snapshot_id"),"version":row.get::<i64,_>("version")})
}

async fn subscription_event(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    event_type: &str,
    actor: Uuid,
    trace_id: Uuid,
    payload: Value,
) -> Result<(), DomainError> {
    sqlx::query("INSERT INTO operating_report_subscription_events(id,subscription_id,event_type,actor_user_id,trace_id,payload) VALUES($1,$2,$3,$4,$5,$6)").bind(Uuid::new_v4()).bind(id).bind(event_type).bind(actor).bind(trace_id).bind(payload).execute(&mut **tx).await?;
    Ok(())
}
