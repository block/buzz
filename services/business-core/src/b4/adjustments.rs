use super::{
    allocation::{largest_remainder, AllocationTarget},
    common::{authorize, period},
    model::{
        CommandResult, CreateAdjustmentBatch, PostAdjustment, PreviewResult,
        ReplaceAdjustmentDraft, VersionCommand,
    },
};
use crate::{
    b2::{
        common::{
            begin_idempotent, finish_idempotent, money, next_number, record, request_hash,
            validate_currency,
        },
        model::DecimalString,
        DomainError,
    },
    model::AuthorizationSnapshot,
    store::PgStore,
};
use chrono::Utc;
use rust_decimal::Decimal;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;

const METRICS: [&str; 7] = [
    "outbound_freight",
    "sales_commission",
    "platform_fee",
    "customer_rebate",
    "supplier_rebate",
    "other_direct_cost",
    "allocated_operating_expense",
];
const BASES: [&str; 5] = [
    "direct",
    "net_revenue",
    "product_cost",
    "shipped_quantity",
    "fixed_weight",
];

#[derive(Clone)]
pub struct AdjustmentService {
    store: PgStore,
    prefix: String,
    max_targets: usize,
}

impl AdjustmentService {
    pub fn new(store: PgStore, prefix: String, max_targets: usize) -> Self {
        Self {
            store,
            prefix,
            max_targets,
        }
    }

    pub async fn create(
        &self,
        actor: Uuid,
        trace_id: Uuid,
        key: &str,
        input: &CreateAdjustmentBatch,
    ) -> Result<CommandResult, DomainError> {
        validate(input)?;
        let authorization = authorize(
            &self.store,
            actor,
            "profit_adjustment:create",
            Some(input.legal_entity_id),
            None,
            None,
            None,
            None,
        )
        .await?;
        ensure_input_scope(&authorization, input)?;
        let hash = request_hash(input)?;
        let mut tx = self.store.pool().begin().await?;
        if let Some(mut replay) = begin_idempotent::<CommandResult>(
            &mut tx,
            actor,
            "profit_adjustment:create",
            key,
            &hash,
        )
        .await?
        {
            replay.idempotent_replay = true;
            tx.commit().await?;
            return Ok(replay);
        }
        let id = Uuid::new_v4();
        let number = next_number(
            &mut tx,
            "profit_adjustment",
            &self.prefix,
            id,
            crate::numbering::NumberingContext::new(input.legal_entity_id, None),
        )
        .await?;
        sqlx::query("INSERT INTO operational_adjustment_batches(id,adjustment_number,legal_entity_id,currency,management_period,created_by_user_id,updated_by_user_id,trace_id) VALUES($1,$2,$3,$4,$5,$6,$6,$7)")
            .bind(id).bind(&number).bind(input.legal_entity_id).bind(&input.currency).bind(&input.management_period).bind(actor).bind(trace_id).execute(&mut *tx).await?;
        insert_lines(&mut tx, id, input).await?;
        event(
            &mut tx,
            id,
            "created",
            1,
            actor,
            trace_id,
            json!({"lineCount":input.lines.len()}),
        )
        .await?;
        record(&mut tx,trace_id,actor,"OPERATIONAL_ADJUSTMENT_CREATED","operational_adjustment_created","operational_adjustment",id,json!({"adjustmentNumber":number,"lineCount":input.lines.len(),"managementPeriod":input.management_period})).await?;
        let result = CommandResult {
            id,
            number,
            status: "draft".into(),
            version: 1,
            trace_id,
            idempotent_replay: false,
        };
        finish_idempotent(&mut tx, actor, "profit_adjustment:create", key, &result).await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn replace_draft(
        &self,
        actor: Uuid,
        trace_id: Uuid,
        batch_id: Uuid,
        key: &str,
        input: &ReplaceAdjustmentDraft,
    ) -> Result<CommandResult, DomainError> {
        validate(&input.batch)?;
        let scope = self.scope(batch_id).await?;
        let authorization = authorize(
            &self.store,
            actor,
            "profit_adjustment:update_draft",
            Some(scope.0),
            None,
            None,
            None,
            None,
        )
        .await?;
        ensure_input_scope(&authorization, &input.batch)?;
        let hash = request_hash(input)?;
        let mut tx = self.store.pool().begin().await?;
        if let Some(mut replay) = begin_idempotent::<CommandResult>(
            &mut tx,
            actor,
            "profit_adjustment:update_draft",
            key,
            &hash,
        )
        .await?
        {
            replay.idempotent_replay = true;
            tx.commit().await?;
            return Ok(replay);
        }
        let row=sqlx::query("SELECT adjustment_number,status,version FROM operational_adjustment_batches WHERE id=$1 FOR UPDATE").bind(batch_id).fetch_one(&mut *tx).await?;
        if row.get::<i64, _>("version") != input.expected_version {
            return Err(DomainError::VersionConflict);
        }
        if !matches!(
            row.get::<String, _>("status").as_str(),
            "draft" | "previewed"
        ) {
            return Err(DomainError::Invalid(
                "only draft or previewed adjustments can be edited".into(),
            ));
        }
        sqlx::query("DELETE FROM operational_adjustment_lines WHERE batch_id=$1")
            .bind(batch_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE operational_adjustment_batches SET legal_entity_id=$2,currency=$3,management_period=$4,status='draft',previewed_at=NULL,updated_by_user_id=$5,trace_id=$6 WHERE id=$1").bind(batch_id).bind(input.batch.legal_entity_id).bind(&input.batch.currency).bind(&input.batch.management_period).bind(actor).bind(trace_id).execute(&mut *tx).await?;
        insert_lines(&mut tx, batch_id, &input.batch).await?;
        let version = input.expected_version + 1;
        event(
            &mut tx,
            batch_id,
            "updated",
            version,
            actor,
            trace_id,
            json!({"lineCount":input.batch.lines.len()}),
        )
        .await?;
        record(
            &mut tx,
            trace_id,
            actor,
            "OPERATIONAL_ADJUSTMENT_UPDATED",
            "operational_adjustment_updated",
            "operational_adjustment",
            batch_id,
            json!({"version":version,"lineCount":input.batch.lines.len()}),
        )
        .await?;
        let result = CommandResult {
            id: batch_id,
            number: row.get("adjustment_number"),
            status: "draft".into(),
            version,
            trace_id,
            idempotent_replay: false,
        };
        finish_idempotent(
            &mut tx,
            actor,
            "profit_adjustment:update_draft",
            key,
            &result,
        )
        .await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn preview(
        &self,
        actor: Uuid,
        trace_id: Uuid,
        batch_id: Uuid,
        key: &str,
        input: &VersionCommand,
    ) -> Result<PreviewResult, DomainError> {
        let scope = self.scope(batch_id).await?;
        let authorization = authorize(
            &self.store,
            actor,
            "profit_adjustment:preview",
            Some(scope.0),
            None,
            None,
            None,
            None,
        )
        .await?;
        let hash = request_hash(input)?;
        let mut tx = self.store.pool().begin().await?;
        if let Some(mut replay) = begin_idempotent::<PreviewResult>(
            &mut tx,
            actor,
            "profit_adjustment:preview",
            key,
            &hash,
        )
        .await?
        {
            replay.idempotent_replay = true;
            tx.commit().await?;
            return Ok(replay);
        }
        let batch=sqlx::query("SELECT adjustment_number,legal_entity_id,currency::text,management_period,status,version FROM operational_adjustment_batches WHERE id=$1 FOR UPDATE").bind(batch_id).fetch_one(&mut *tx).await?;
        if batch.get::<i64, _>("version") != input.expected_version {
            return Err(DomainError::VersionConflict);
        }
        if !matches!(
            batch.get::<String, _>("status").as_str(),
            "draft" | "previewed"
        ) {
            return Err(DomainError::Invalid("adjustment is not previewable".into()));
        }
        let lines=sqlx::query("SELECT id,metric_type,amount,business_date,allocation_basis,allocation_scope FROM operational_adjustment_lines WHERE batch_id=$1 ORDER BY line_number").bind(batch_id).fetch_all(&mut *tx).await?;
        let watermark: i64 =
            sqlx::query_scalar("SELECT COALESCE(max(fact_sequence),0) FROM profit_facts")
                .fetch_one(&mut *tx)
                .await?;
        let mut payload_lines = Vec::new();
        let mut total = Decimal::ZERO;
        let mut allocated = Decimal::ZERO;
        for line in lines {
            let amount: Decimal = line.get("amount");
            total += amount;
            let targets = targets(&mut tx, &batch, &line, self.max_targets).await?;
            for target in &targets {
                ensure_order_scope(&mut tx, target.0, &authorization).await?;
                ensure_line_target_scope(
                    &mut tx,
                    target.0,
                    &line.get::<Value, _>("allocation_scope"),
                )
                .await?;
            }
            let allocations = largest_remainder(
                amount,
                &targets
                    .iter()
                    .map(|row| AllocationTarget {
                        sales_order_id: row.0,
                        weight: row.1,
                    })
                    .collect::<Vec<_>>(),
            )?;
            allocated += allocations.iter().map(|row| row.amount).sum::<Decimal>();
            payload_lines.push(json!({"lineId":line.get::<Uuid,_>("id"),"metricType":line.get::<String,_>("metric_type"),"businessDate":line.get::<chrono::NaiveDate,_>("business_date"),"targets":allocations.into_iter().map(|row|json!({"salesOrderId":row.sales_order_id,"weight":row.weight.to_string(),"amount":row.amount.to_string(),"remainderRank":row.remainder_rank})).collect::<Vec<_>>() }));
        }
        let source_hash = hex::encode(Sha256::digest(serde_json::to_vec(
            &json!({"watermark":watermark,"lines":payload_lines}),
        )?));
        let next_version = input.expected_version + 1;
        let preview_hash = hex::encode(Sha256::digest(serde_json::to_vec(
            &json!({"batchId":batch_id,"batchVersion":next_version,"sourceHash":source_hash,"allocations":payload_lines}),
        )?));
        let preview_id = Uuid::new_v4();
        let payload =
            json!({"lines":payload_lines,"boundary":"management_only_not_general_ledger"});
        sqlx::query("INSERT INTO operational_adjustment_previews(id,batch_id,preview_hash,source_hash,source_watermark,batch_version,total_amount,allocated_amount,unallocated_amount,payload,created_by_user_id,trace_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)")
            .bind(preview_id).bind(batch_id).bind(&preview_hash).bind(&source_hash).bind(watermark).bind(next_version).bind(money(total)).bind(money(allocated)).bind(money(total-allocated)).bind(&payload).bind(actor).bind(trace_id).execute(&mut *tx).await?;
        sqlx::query("UPDATE operational_adjustment_batches SET status='previewed',previewed_at=now(),updated_by_user_id=$2,trace_id=$3 WHERE id=$1").bind(batch_id).bind(actor).bind(trace_id).execute(&mut *tx).await?;
        event(
            &mut tx,
            batch_id,
            "previewed",
            next_version,
            actor,
            trace_id,
            json!({"previewId":preview_id,"previewHash":preview_hash,"sourceWatermark":watermark}),
        )
        .await?;
        record(&mut tx,trace_id,actor,"PROFIT_ALLOCATION_PREVIEWED","profit_allocation_previewed","operational_adjustment",batch_id,json!({"previewId":preview_id,"targetCount":payload["lines"].as_array().map_or(0,Vec::len),"sourceWatermark":watermark})).await?;
        let result = PreviewResult {
            preview_id,
            batch_id,
            preview_hash,
            source_hash,
            source_watermark: watermark,
            batch_version: next_version,
            total_amount: DecimalString(money(total)),
            allocated_amount: DecimalString(money(allocated)),
            unallocated_amount: DecimalString(money(total - allocated)),
            allocations: payload,
            data_as_of: Utc::now(),
            trace_id,
            idempotent_replay: false,
        };
        finish_idempotent(&mut tx, actor, "profit_adjustment:preview", key, &result).await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn post(
        &self,
        actor: Uuid,
        trace_id: Uuid,
        batch_id: Uuid,
        key: &str,
        input: &PostAdjustment,
    ) -> Result<CommandResult, DomainError> {
        let scope = self.scope(batch_id).await?;
        let authorization = authorize(
            &self.store,
            actor,
            "profit_adjustment:post",
            Some(scope.0),
            None,
            None,
            None,
            None,
        )
        .await?;
        let hash = request_hash(input)?;
        let mut tx = self.store.pool().begin().await?;
        if let Some(mut replay) =
            begin_idempotent::<CommandResult>(&mut tx, actor, "profit_adjustment:post", key, &hash)
                .await?
        {
            replay.idempotent_replay = true;
            tx.commit().await?;
            return Ok(replay);
        }
        let batch=sqlx::query("SELECT adjustment_number,legal_entity_id,currency::text,management_period,status,version FROM operational_adjustment_batches WHERE id=$1 FOR UPDATE").bind(batch_id).fetch_one(&mut *tx).await?;
        if batch.get::<i64, _>("version") != input.expected_version {
            return Err(DomainError::VersionConflict);
        }
        if batch.get::<String, _>("status") != "previewed" {
            return Err(DomainError::Invalid(
                "only previewed adjustments can be posted".into(),
            ));
        }
        let preview=sqlx::query("SELECT preview_hash,source_watermark,batch_version,payload,unallocated_amount FROM operational_adjustment_previews WHERE id=$1 AND batch_id=$2").bind(input.preview_id).bind(batch_id).fetch_optional(&mut *tx).await?.ok_or(DomainError::StalePreview)?;
        let watermark: i64 =
            sqlx::query_scalar("SELECT COALESCE(max(fact_sequence),0) FROM profit_facts")
                .fetch_one(&mut *tx)
                .await?;
        if preview.get::<String, _>("preview_hash") != input.preview_hash
            || preview.get::<i64, _>("batch_version") != input.expected_version
            || preview.get::<i64, _>("source_watermark") != watermark
            || preview.get::<Decimal, _>("unallocated_amount") != Decimal::ZERO
        {
            return Err(DomainError::StalePreview);
        }
        let post_event = Uuid::new_v4();
        let payload: Value = preview.get("payload");
        let mut fact_count = 0_i64;
        for line in payload["lines"]
            .as_array()
            .ok_or_else(|| DomainError::Invalid("preview payload is invalid".into()))?
        {
            let line_id = Uuid::parse_str(line["lineId"].as_str().unwrap_or_default())
                .map_err(|_| DomainError::StalePreview)?;
            let metric = line["metricType"]
                .as_str()
                .ok_or(DomainError::StalePreview)?;
            let business_date = chrono::NaiveDate::parse_from_str(
                line["businessDate"].as_str().unwrap_or_default(),
                "%Y-%m-%d",
            )
            .map_err(|_| DomainError::StalePreview)?;
            for target in line["targets"]
                .as_array()
                .ok_or(DomainError::StalePreview)?
            {
                let order_id = Uuid::parse_str(target["salesOrderId"].as_str().unwrap_or_default())
                    .map_err(|_| DomainError::StalePreview)?;
                let amount = target["amount"]
                    .as_str()
                    .unwrap_or_default()
                    .parse::<Decimal>()
                    .map_err(|_| DomainError::StalePreview)?;
                let weight = target["weight"]
                    .as_str()
                    .unwrap_or_default()
                    .parse::<Decimal>()
                    .map_err(|_| DomainError::StalePreview)?;
                let rank = target["remainderRank"]
                    .as_i64()
                    .ok_or(DomainError::StalePreview)? as i32;
                let dims = order_dimensions(&mut tx, order_id).await?;
                ensure_order_scope(&mut tx, order_id, &authorization).await?;
                if dims.get::<Uuid, _>("legal_entity_id") != batch.get::<Uuid, _>("legal_entity_id")
                    || dims.get::<String, _>("currency") != batch.get::<String, _>("currency")
                {
                    return Err(DomainError::NotFoundOrForbidden);
                }
                let allocation_id = Uuid::new_v4();
                let fact_id = Uuid::new_v4();
                sqlx::query("INSERT INTO profit_facts(id,metric_type,direction,amount,currency,legal_entity_id,sales_order_id,customer_id,brand_id,salesperson_user_id,business_unit_id,department_id,business_date,management_period,source_system,source_type,source_id,source_line_id,source_event_id,source_event_version,data_as_of,trace_id) VALUES($1,$2,'normal',$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,'business_core_b4','operational_adjustment',$14,$15,$16,$17,now(),$18)")
                    .bind(fact_id).bind(metric).bind(amount).bind(batch.get::<String,_>("currency")).bind(batch.get::<Uuid,_>("legal_entity_id")).bind(order_id).bind(dims.get::<Uuid,_>("customer_id")).bind(dims.get::<Option<Uuid>,_>("brand_id")).bind(dims.get::<Uuid,_>("salesperson_user_id")).bind(dims.get::<Uuid,_>("business_unit_id")).bind(dims.get::<Option<Uuid>,_>("department_id")).bind(business_date).bind(batch.get::<String,_>("management_period")).bind(batch_id).bind(allocation_id).bind(post_event).bind(input.expected_version+1).bind(trace_id).execute(&mut *tx).await?;
                sqlx::query("INSERT INTO operational_adjustment_allocations(id,batch_id,adjustment_line_id,preview_id,sales_order_id,weight,allocated_amount,remainder_rank,profit_fact_id,trace_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)")
                    .bind(allocation_id).bind(batch_id).bind(line_id).bind(input.preview_id).bind(order_id).bind(weight).bind(amount).bind(rank).bind(fact_id).bind(trace_id).execute(&mut *tx).await?;
                fact_count += 1;
            }
        }
        sqlx::query("UPDATE operational_adjustment_batches SET status='posted',posted_at=now(),updated_by_user_id=$2,trace_id=$3 WHERE id=$1").bind(batch_id).bind(actor).bind(trace_id).execute(&mut *tx).await?;
        let version = input.expected_version + 1;
        event_with_id(
            &mut tx,
            post_event,
            batch_id,
            "posted",
            version,
            actor,
            trace_id,
            json!({"previewId":input.preview_id,"factCount":fact_count}),
        )
        .await?;
        record(
            &mut tx,
            trace_id,
            actor,
            "OPERATIONAL_ADJUSTMENT_POSTED",
            "operational_adjustment_posted",
            "operational_adjustment",
            batch_id,
            json!({"previewId":input.preview_id,"factCount":fact_count,"version":version}),
        )
        .await?;
        let result = CommandResult {
            id: batch_id,
            number: batch.get("adjustment_number"),
            status: "posted".into(),
            version,
            trace_id,
            idempotent_replay: false,
        };
        finish_idempotent(&mut tx, actor, "profit_adjustment:post", key, &result).await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn reverse(
        &self,
        actor: Uuid,
        trace_id: Uuid,
        batch_id: Uuid,
        key: &str,
        input: &VersionCommand,
    ) -> Result<CommandResult, DomainError> {
        let scope = self.scope(batch_id).await?;
        let authorization = authorize(
            &self.store,
            actor,
            "profit_adjustment:reverse",
            Some(scope.0),
            None,
            None,
            None,
            None,
        )
        .await?;
        let hash = request_hash(input)?;
        let mut tx = self.store.pool().begin().await?;
        if let Some(mut replay) = begin_idempotent::<CommandResult>(
            &mut tx,
            actor,
            "profit_adjustment:reverse",
            key,
            &hash,
        )
        .await?
        {
            replay.idempotent_replay = true;
            tx.commit().await?;
            return Ok(replay);
        }
        let batch=sqlx::query("SELECT adjustment_number,status,version FROM operational_adjustment_batches WHERE id=$1 FOR UPDATE").bind(batch_id).fetch_one(&mut *tx).await?;
        let target_orders = sqlx::query_scalar::<_, Uuid>("SELECT DISTINCT sales_order_id FROM operational_adjustment_allocations WHERE batch_id=$1")
            .bind(batch_id).fetch_all(&mut *tx).await?;
        for order_id in target_orders {
            ensure_order_scope(&mut tx, order_id, &authorization).await?;
        }
        if batch.get::<i64, _>("version") != input.expected_version {
            return Err(DomainError::VersionConflict);
        }
        if batch.get::<String, _>("status") != "posted" {
            return Err(DomainError::Invalid(
                "only posted adjustments can be reversed".into(),
            ));
        }
        let reverse_event = Uuid::new_v4();
        let version = input.expected_version + 1;
        let inserted=sqlx::query("INSERT INTO profit_facts(id,metric_type,direction,amount,currency,quantity,legal_entity_id,sales_order_id,sales_order_line_id,shipment_id,shipment_line_id,customer_id,sku_id,product_category_id,brand_id,salesperson_user_id,business_unit_id,department_id,warehouse_id,business_date,management_period,source_system,source_type,source_id,source_line_id,source_event_id,source_event_version,data_as_of,trace_id) SELECT gen_random_uuid(),f.metric_type,'reversal',f.amount,f.currency,f.quantity,f.legal_entity_id,f.sales_order_id,f.sales_order_line_id,f.shipment_id,f.shipment_line_id,f.customer_id,f.sku_id,f.product_category_id,f.brand_id,f.salesperson_user_id,f.business_unit_id,f.department_id,f.warehouse_id,f.business_date,f.management_period,'business_core_b4','operational_adjustment',$1,a.id,$2,$3,now(),$4 FROM operational_adjustment_allocations a JOIN profit_facts f ON f.id=a.profit_fact_id WHERE a.batch_id=$1").bind(batch_id).bind(reverse_event).bind(version).bind(trace_id).execute(&mut *tx).await?;
        sqlx::query("UPDATE operational_adjustment_batches SET status='reversed',reversed_at=now(),updated_by_user_id=$2,trace_id=$3 WHERE id=$1").bind(batch_id).bind(actor).bind(trace_id).execute(&mut *tx).await?;
        event_with_id(
            &mut tx,
            reverse_event,
            batch_id,
            "reversed",
            version,
            actor,
            trace_id,
            json!({"factCount":inserted.rows_affected()}),
        )
        .await?;
        record(
            &mut tx,
            trace_id,
            actor,
            "OPERATIONAL_ADJUSTMENT_REVERSED",
            "operational_adjustment_reversed",
            "operational_adjustment",
            batch_id,
            json!({"factCount":inserted.rows_affected(),"version":version}),
        )
        .await?;
        let result = CommandResult {
            id: batch_id,
            number: batch.get("adjustment_number"),
            status: "reversed".into(),
            version,
            trace_id,
            idempotent_replay: false,
        };
        finish_idempotent(&mut tx, actor, "profit_adjustment:reverse", key, &result).await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn list(&self, actor: Uuid, limit: i64) -> Result<Value, DomainError> {
        let snapshot = authorize(
            &self.store,
            actor,
            "profit_adjustment:read",
            None,
            None,
            None,
            None,
            None,
        )
        .await?;
        let rows=sqlx::query("SELECT id,adjustment_number,legal_entity_id,currency::text,management_period,status,rule_version,updated_at,version FROM operational_adjustment_batches WHERE legal_entity_id=ANY($1) ORDER BY updated_at DESC,id LIMIT $2").bind(snapshot.scopes.legal_entity_ids.into_iter().collect::<Vec<_>>()).bind(limit.clamp(1,200)).fetch_all(self.store.pool()).await?;
        Ok(
            json!({"items":rows.into_iter().map(|r|json!({"id":r.get::<Uuid,_>("id"),"adjustmentNumber":r.get::<String,_>("adjustment_number"),"legalEntityId":r.get::<Uuid,_>("legal_entity_id"),"currency":r.get::<String,_>("currency"),"managementPeriod":r.get::<String,_>("management_period"),"status":r.get::<String,_>("status"),"ruleVersion":r.get::<String,_>("rule_version"),"updatedAt":r.get::<chrono::DateTime<Utc>,_>("updated_at"),"version":r.get::<i64,_>("version")})).collect::<Vec<_>>(),"dataAsOf":Utc::now(),"boundary":"management_only_not_general_ledger"}),
        )
    }

    async fn scope(&self, id: Uuid) -> Result<(Uuid,), DomainError> {
        sqlx::query_as("SELECT legal_entity_id FROM operational_adjustment_batches WHERE id=$1")
            .bind(id)
            .fetch_optional(self.store.pool())
            .await?
            .ok_or(DomainError::NotFoundOrForbidden)
    }
}

fn validate(input: &CreateAdjustmentBatch) -> Result<(), DomainError> {
    validate_currency(&input.currency)?;
    period(&input.management_period)?;
    if input.lines.is_empty() || input.lines.len() > 200 {
        return Err(DomainError::Invalid(
            "adjustment requires 1..200 lines".into(),
        ));
    }
    for line in &input.lines {
        if !METRICS.contains(&line.metric_type.as_str())
            || !BASES.contains(&line.allocation_basis.as_str())
        {
            return Err(DomainError::Invalid(
                "unsupported adjustment metric or allocation basis".into(),
            ));
        }
        if line.amount.0 <= Decimal::ZERO || line.amount.0.round_dp(2) != line.amount.0 {
            return Err(DomainError::Invalid(
                "adjustment amount must use two-decimal currency precision".into(),
            ));
        }
        if line.allocation_basis == "direct" && line.direct_sales_order_id.is_none() {
            return Err(DomainError::Invalid(
                "direct allocation requires salesOrderId".into(),
            ));
        }
        if line.allocation_basis != "fixed_weight" && !line.fixed_weights.is_empty() {
            return Err(DomainError::Invalid(
                "fixedWeights require fixed_weight basis".into(),
            ));
        }
    }
    Ok(())
}

fn ensure_input_scope(
    authorization: &AuthorizationSnapshot,
    input: &CreateAdjustmentBatch,
) -> Result<(), DomainError> {
    let scopes = &authorization.scopes;
    let allowed = scopes.legal_entity_ids.contains(&input.legal_entity_id)
        && input.lines.iter().all(|line| {
            line.customer_id
                .is_none_or(|id| scopes.customer_ids.contains(&id))
                && line
                    .brand_id
                    .is_none_or(|id| scopes.brand_ids.contains(&id))
                && line
                    .business_unit_id
                    .is_none_or(|id| scopes.business_unit_ids.contains(&id))
                && line
                    .warehouse_id
                    .is_none_or(|id| scopes.warehouse_ids.contains(&id))
        });
    if allowed {
        Ok(())
    } else {
        Err(DomainError::NotFoundOrForbidden)
    }
}

async fn ensure_order_scope(
    tx: &mut Transaction<'_, Postgres>,
    order_id: Uuid,
    authorization: &AuthorizationSnapshot,
) -> Result<(), DomainError> {
    let dims = order_dimensions(tx, order_id).await?;
    let scopes = &authorization.scopes;
    let allowed = scopes
        .legal_entity_ids
        .contains(&dims.get::<Uuid, _>("legal_entity_id"))
        && scopes
            .customer_ids
            .contains(&dims.get::<Uuid, _>("customer_id"))
        && dims
            .get::<Option<Uuid>, _>("brand_id")
            .is_none_or(|id| scopes.brand_ids.contains(&id))
        && scopes
            .business_unit_ids
            .contains(&dims.get::<Uuid, _>("business_unit_id"));
    if !allowed {
        return Err(DomainError::NotFoundOrForbidden);
    }
    let outside_warehouse: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM profit_facts WHERE sales_order_id=$1 AND warehouse_id IS NOT NULL AND NOT (warehouse_id=ANY($2)))")
        .bind(order_id)
        .bind(scopes.warehouse_ids.iter().copied().collect::<Vec<_>>())
        .fetch_one(&mut **tx)
        .await?;
    if outside_warehouse {
        Err(DomainError::NotFoundOrForbidden)
    } else {
        Ok(())
    }
}

async fn insert_lines(
    tx: &mut Transaction<'_, Postgres>,
    batch_id: Uuid,
    input: &CreateAdjustmentBatch,
) -> Result<(), DomainError> {
    for (index, line) in input.lines.iter().enumerate() {
        let scope = json!({"salesOrderIds":line.sales_order_ids,"fixedWeights":line.fixed_weights,"customerId":line.customer_id,"skuId":line.sku_id,"brandId":line.brand_id,"salespersonUserId":line.salesperson_user_id,"businessUnitId":line.business_unit_id,"departmentId":line.department_id,"warehouseId":line.warehouse_id});
        sqlx::query("INSERT INTO operational_adjustment_lines(id,batch_id,line_number,metric_type,amount,currency,business_date,management_period,legal_entity_id,direct_sales_order_id,customer_id,sku_id,brand_id,salesperson_user_id,business_unit_id,department_id,warehouse_id,allocation_basis,allocation_scope,source_reference,reason_code,business_note) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21,$22)")
            .bind(Uuid::new_v4()).bind(batch_id).bind(i32::try_from(index+1).unwrap_or(i32::MAX)).bind(&line.metric_type).bind(line.amount.0).bind(&input.currency).bind(line.business_date).bind(&input.management_period).bind(input.legal_entity_id).bind(line.direct_sales_order_id).bind(line.customer_id).bind(line.sku_id).bind(line.brand_id).bind(line.salesperson_user_id).bind(line.business_unit_id).bind(line.department_id).bind(line.warehouse_id).bind(&line.allocation_basis).bind(scope).bind(&line.source_reference).bind(&line.reason_code).bind(&line.business_note).execute(&mut **tx).await?;
    }
    Ok(())
}

async fn targets(
    tx: &mut Transaction<'_, Postgres>,
    batch: &sqlx::postgres::PgRow,
    line: &sqlx::postgres::PgRow,
    max: usize,
) -> Result<Vec<(Uuid, Decimal)>, DomainError> {
    let basis: String = line.get("allocation_basis");
    let scope: Value = line.get("allocation_scope");
    if basis == "direct" {
        let id: Uuid = sqlx::query_scalar(
            "SELECT direct_sales_order_id FROM operational_adjustment_lines WHERE id=$1",
        )
        .bind(line.get::<Uuid, _>("id"))
        .fetch_one(&mut **tx)
        .await?;
        let weight:Decimal=sqlx::query_scalar("SELECT COALESCE(net_revenue,0) FROM order_profit_current WHERE sales_order_id=$1 AND legal_entity_id=$2 AND currency=$3").bind(id).bind(batch.get::<Uuid,_>("legal_entity_id")).bind(batch.get::<String,_>("currency")).fetch_optional(&mut **tx).await?.ok_or(DomainError::NotFoundOrForbidden)?;
        if weight < Decimal::ZERO {
            return Err(DomainError::NotFoundOrForbidden);
        }
        return Ok(vec![(id, Decimal::ONE)]);
    }
    let ids: Vec<Uuid> = scope["salesOrderIds"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str())
        .filter_map(|v| Uuid::parse_str(v).ok())
        .collect();
    let scoped_id = |key: &str| {
        scope[key]
            .as_str()
            .and_then(|value| Uuid::parse_str(value).ok())
    };
    if basis == "fixed_weight" {
        let weights = scope["fixedWeights"]
            .as_array()
            .ok_or_else(|| DomainError::Invalid("fixed weight targets are required".into()))?;
        if weights.len() > max {
            return Err(DomainError::Invalid(
                "allocation target limit exceeded".into(),
            ));
        }
        let mut result = Vec::new();
        for item in weights {
            let id = Uuid::parse_str(item["salesOrderId"].as_str().unwrap_or_default())
                .map_err(|_| DomainError::Invalid("invalid fixed weight order".into()))?;
            order_dimensions(tx, id).await?;
            let weight = item["weight"]
                .as_str()
                .unwrap_or_default()
                .parse::<Decimal>()
                .map_err(|_| DomainError::Invalid("invalid fixed weight".into()))?;
            result.push((id, weight));
        }
        return Ok(result);
    }
    let metric = if basis == "product_cost" {
        "product_cost"
    } else {
        "net_revenue"
    };
    let rows=sqlx::query("SELECT sales_order_id,CASE WHEN $1='shipped_quantity' THEN COALESCE(sum(quantity) FILTER(WHERE metric_type='net_revenue' AND direction='normal'),0)-COALESCE(sum(quantity) FILTER(WHERE metric_type='net_revenue' AND direction='reversal'),0) ELSE COALESCE(sum(CASE direction WHEN 'normal' THEN amount ELSE -amount END) FILTER(WHERE metric_type=$2),0) END weight FROM profit_facts WHERE legal_entity_id=$3 AND currency=$4 AND management_period=$5 AND (cardinality($6::uuid[])=0 OR sales_order_id=ANY($6)) AND ($7::uuid IS NULL OR customer_id=$7) AND ($8::uuid IS NULL OR sku_id=$8) AND ($9::uuid IS NULL OR brand_id=$9) AND ($10::uuid IS NULL OR salesperson_user_id=$10) AND ($11::uuid IS NULL OR business_unit_id=$11) AND ($12::uuid IS NULL OR department_id=$12) AND ($13::uuid IS NULL OR warehouse_id=$13) GROUP BY sales_order_id ORDER BY sales_order_id LIMIT $14")
        .bind(&basis).bind(metric).bind(batch.get::<Uuid,_>("legal_entity_id")).bind(batch.get::<String,_>("currency")).bind(batch.get::<String,_>("management_period")).bind(ids).bind(scoped_id("customerId")).bind(scoped_id("skuId")).bind(scoped_id("brandId")).bind(scoped_id("salespersonUserId")).bind(scoped_id("businessUnitId")).bind(scoped_id("departmentId")).bind(scoped_id("warehouseId")).bind(i64::try_from(max+1).unwrap_or(i64::MAX)).fetch_all(&mut **tx).await?;
    if rows.len() > max {
        return Err(DomainError::Invalid(
            "allocation target limit exceeded".into(),
        ));
    }
    Ok(rows
        .into_iter()
        .map(|r| (r.get("sales_order_id"), r.get("weight")))
        .collect())
}

async fn order_dimensions(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
) -> Result<sqlx::postgres::PgRow, DomainError> {
    sqlx::query("SELECT legal_entity_id,customer_id,brand_id,currency::text,salesperson_user_id,business_unit_id,department_id FROM sales_orders WHERE id=$1 AND EXISTS(SELECT 1 FROM profit_facts WHERE sales_order_id=$1 AND metric_type='net_revenue')").bind(id).fetch_optional(&mut **tx).await?.ok_or(DomainError::NotFoundOrForbidden)
}

async fn ensure_line_target_scope(
    tx: &mut Transaction<'_, Postgres>,
    order_id: Uuid,
    scope: &Value,
) -> Result<(), DomainError> {
    let scoped_id = |key: &str| {
        scope[key]
            .as_str()
            .and_then(|value| Uuid::parse_str(value).ok())
    };
    let matches: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM profit_facts WHERE sales_order_id=$1 AND metric_type='net_revenue' AND ($2::uuid IS NULL OR customer_id=$2) AND ($3::uuid IS NULL OR sku_id=$3) AND ($4::uuid IS NULL OR brand_id=$4) AND ($5::uuid IS NULL OR salesperson_user_id=$5) AND ($6::uuid IS NULL OR business_unit_id=$6) AND ($7::uuid IS NULL OR department_id=$7) AND ($8::uuid IS NULL OR warehouse_id=$8))")
        .bind(order_id).bind(scoped_id("customerId")).bind(scoped_id("skuId")).bind(scoped_id("brandId")).bind(scoped_id("salespersonUserId")).bind(scoped_id("businessUnitId")).bind(scoped_id("departmentId")).bind(scoped_id("warehouseId")).fetch_one(&mut **tx).await?;
    if matches {
        Ok(())
    } else {
        Err(DomainError::NotFoundOrForbidden)
    }
}

async fn event(
    tx: &mut Transaction<'_, Postgres>,
    batch: Uuid,
    kind: &str,
    version: i64,
    actor: Uuid,
    trace: Uuid,
    payload: Value,
) -> Result<(), DomainError> {
    event_with_id(
        tx,
        Uuid::new_v4(),
        batch,
        kind,
        version,
        actor,
        trace,
        payload,
    )
    .await
}
#[allow(clippy::too_many_arguments)]
async fn event_with_id(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    batch: Uuid,
    kind: &str,
    version: i64,
    actor: Uuid,
    trace: Uuid,
    payload: Value,
) -> Result<(), DomainError> {
    sqlx::query("INSERT INTO operational_adjustment_events(id,batch_id,event_type,batch_version,payload,actor_user_id,trace_id) VALUES($1,$2,$3,$4,$5,$6,$7)").bind(id).bind(batch).bind(kind).bind(version).bind(payload).bind(actor).bind(trace).execute(&mut **tx).await?;
    Ok(())
}
