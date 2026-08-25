use super::{
    common::{authorize, period},
    model::{CommandResult, GenerateReportSnapshot, OrderProfitView},
};
use crate::{
    b2::{
        common::{
            begin_idempotent, finish_idempotent, next_number, record, request_hash,
            validate_currency,
        },
        DomainError,
    },
    store::PgStore,
};
use chrono::Utc;
use rust_decimal::Decimal;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::Row;
use uuid::Uuid;

#[derive(Clone)]
pub struct ProfitReportingService {
    store: PgStore,
    snapshot_prefix: String,
    max_rows: usize,
    worker_enabled: bool,
    stale_after_minutes: i64,
}

impl ProfitReportingService {
    pub fn new(
        store: PgStore,
        snapshot_prefix: String,
        max_rows: usize,
        worker_enabled: bool,
        stale_after_minutes: i64,
    ) -> Self {
        Self {
            store,
            snapshot_prefix,
            max_rows,
            worker_enabled,
            stale_after_minutes,
        }
    }

    pub async fn order_profits(
        &self,
        actor: Uuid,
        order_id: Option<Uuid>,
        order_number: Option<&str>,
        period_filter: Option<&str>,
        limit: i64,
    ) -> Result<Value, DomainError> {
        if order_number.is_some_and(|value| {
            value.is_empty()
                || value.len() > 64
                || !value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        }) {
            return Err(DomainError::Invalid("invalid sales order number".into()));
        }
        if let Some(value) = period_filter {
            period(value)?;
        }
        let snapshot = authorize(
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
        let rows=sqlx::query_as::<_,OrderProfitView>("SELECT p.sales_order_id,o.order_number,p.legal_entity_id,p.customer_id,o.brand_id,o.business_unit_id,o.salesperson_user_id,p.currency::text,p.net_revenue,p.product_cost,p.gross_profit,p.outbound_freight,p.sales_commission,p.platform_fee,p.customer_rebate,p.supplier_rebate,p.other_direct_cost,p.contribution_profit,p.allocated_operating_expense,p.management_operating_profit,p.gross_margin_rate,p.contribution_margin_rate,p.management_operating_margin_rate,CASE WHEN $8 AND p.data_as_of>=now()-($9*interval '1 minute') THEN p.data_quality_status ELSE 'partial' END data_quality_status,p.data_as_of,p.last_fact_sequence FROM order_profit_current p JOIN sales_orders o ON o.id=p.sales_order_id WHERE p.legal_entity_id=ANY($1) AND p.customer_id=ANY($2) AND ($3::uuid IS NULL OR p.sales_order_id=$3) AND ($4::text IS NULL OR EXISTS(SELECT 1 FROM profit_facts f WHERE f.sales_order_id=p.sales_order_id AND f.management_period=$4)) AND NOT EXISTS(SELECT 1 FROM profit_facts f WHERE f.sales_order_id=p.sales_order_id AND ((f.brand_id IS NOT NULL AND NOT(f.brand_id=ANY($5))) OR (f.business_unit_id IS NOT NULL AND NOT(f.business_unit_id=ANY($6))) OR (f.warehouse_id IS NOT NULL AND NOT(f.warehouse_id=ANY($7))))) AND ($10::text IS NULL OR o.order_number=$10) ORDER BY p.data_as_of DESC,p.sales_order_id LIMIT $11")
            .bind(snapshot.scopes.legal_entity_ids.into_iter().collect::<Vec<_>>()).bind(snapshot.scopes.customer_ids.into_iter().collect::<Vec<_>>()).bind(order_id).bind(period_filter).bind(snapshot.scopes.brand_ids.into_iter().collect::<Vec<_>>()).bind(snapshot.scopes.business_unit_ids.into_iter().collect::<Vec<_>>()).bind(snapshot.scopes.warehouse_ids.into_iter().collect::<Vec<_>>()).bind(self.worker_enabled).bind(self.stale_after_minutes).bind(order_number).bind(limit.clamp(1,200)).fetch_all(self.store.pool()).await?;
        if (order_id.is_some() || order_number.is_some()) && rows.is_empty() {
            return Err(DomainError::NotFoundOrForbidden);
        }
        Ok(
            json!({"items":rows,"dataAsOf":Utc::now(),"source":"business-core-b4","ruleVersion":"management-profit-v1","sourceWatermark":self.watermark().await?,"warnings":if self.worker_enabled{vec!["经营管理口径，不是法定利润、会计凭证或应纳税所得额"]}else{vec!["PROFIT_PROJECTION_WORKER_DISABLED","经营管理口径，不是法定利润、会计凭证或应纳税所得额"]}}),
        )
    }

    pub async fn profitability(
        &self,
        actor: Uuid,
        period_value: &str,
        currency: &str,
        dimension_one: &str,
        dimension_two: Option<&str>,
        limit: i64,
    ) -> Result<Value, DomainError> {
        period(period_value)?;
        validate_currency(currency)?;
        let snapshot = authorize(
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
        validate_dimension(dimension_one)?;
        if let Some(value) = dimension_two {
            validate_dimension(value)?;
        }
        if dimension_two == Some(dimension_one) {
            return Err(DomainError::Invalid(
                "profit dimensions must be distinct".into(),
            ));
        }
        let rows = sqlx::query("SELECT CASE $8 WHEN 'group' THEN NULL WHEN 'legal_entity' THEN legal_entity_id WHEN 'customer' THEN customer_id WHEN 'sku' THEN sku_id WHEN 'product_category' THEN product_category_id WHEN 'brand' THEN brand_id WHEN 'salesperson' THEN salesperson_user_id WHEN 'business_unit' THEN business_unit_id WHEN 'department' THEN department_id WHEN 'warehouse' THEN warehouse_id WHEN 'sales_order' THEN sales_order_id END::text dimension_one_id,CASE $9 WHEN '' THEN NULL WHEN 'legal_entity' THEN legal_entity_id WHEN 'customer' THEN customer_id WHEN 'sku' THEN sku_id WHEN 'product_category' THEN product_category_id WHEN 'brand' THEN brand_id WHEN 'salesperson' THEN salesperson_user_id WHEN 'business_unit' THEN business_unit_id WHEN 'department' THEN department_id WHEN 'warehouse' THEN warehouse_id WHEN 'sales_order' THEN sales_order_id END::text dimension_two_id,COALESCE(sum(CASE direction WHEN 'normal' THEN amount ELSE -amount END) FILTER(WHERE metric_type='net_revenue'),0)::numeric(24,6) net_revenue,COALESCE(sum(CASE direction WHEN 'normal' THEN amount ELSE -amount END) FILTER(WHERE metric_type='product_cost'),0)::numeric(24,6) product_cost,COALESCE(sum(CASE direction WHEN 'normal' THEN amount ELSE -amount END) FILTER(WHERE metric_type IN ('outbound_freight','sales_commission','platform_fee','customer_rebate','other_direct_cost')),0)::numeric(24,6) direct_costs,COALESCE(sum(CASE direction WHEN 'normal' THEN amount ELSE -amount END) FILTER(WHERE metric_type='supplier_rebate'),0)::numeric(24,6) supplier_rebate,COALESCE(sum(CASE direction WHEN 'normal' THEN amount ELSE -amount END) FILTER(WHERE metric_type='allocated_operating_expense'),0)::numeric(24,6) operating_expense,max(fact_sequence) source_watermark,max(data_as_of) data_as_of FROM profit_facts WHERE management_period=$1 AND currency=$2 AND legal_entity_id=ANY($3) AND customer_id=ANY($4) AND (brand_id IS NULL OR brand_id=ANY($5)) AND business_unit_id=ANY($6) AND (warehouse_id IS NULL OR warehouse_id=ANY($7)) GROUP BY 1,2 ORDER BY net_revenue DESC,1 LIMIT $10")
            .bind(period_value)
            .bind(currency)
            .bind(
                snapshot
                    .scopes
                    .legal_entity_ids
                    .into_iter()
                    .collect::<Vec<_>>(),
            )
            .bind(snapshot.scopes.customer_ids.into_iter().collect::<Vec<_>>())
            .bind(snapshot.scopes.brand_ids.into_iter().collect::<Vec<_>>())
            .bind(
                snapshot
                    .scopes
                    .business_unit_ids
                    .into_iter()
                    .collect::<Vec<_>>(),
            )
            .bind(snapshot.scopes.warehouse_ids.into_iter().collect::<Vec<_>>())
            .bind(dimension_one)
            .bind(dimension_two.unwrap_or(""))
            .bind(limit.clamp(1, i64::try_from(self.max_rows).unwrap_or(i64::MAX)))
            .fetch_all(self.store.pool())
            .await?;
        let items=rows.into_iter().map(|row|{
            let revenue:Decimal=row.get("net_revenue");let cost:Decimal=row.get("product_cost");let direct:Decimal=row.get("direct_costs");let rebate:Decimal=row.get("supplier_rebate");let operating:Decimal=row.get("operating_expense");let gross=revenue-cost;let contribution=gross-direct+rebate;let management=contribution-operating;
            let data_as_of=row.get::<chrono::DateTime<Utc>,_>("data_as_of");
            let complete=self.worker_enabled && Utc::now().signed_duration_since(data_as_of).num_minutes()<=self.stale_after_minutes;
            json!({"dimensionOne":dimension_one,"dimensionOneId":row.get::<Option<String>,_>("dimension_one_id"),"dimensionTwo":dimension_two,"dimensionTwoId":row.get::<Option<String>,_>("dimension_two_id"),"currency":currency,"netRevenue":revenue.to_string(),"productCost":cost.to_string(),"grossProfit":gross.to_string(),"contributionProfit":contribution.to_string(),"managementOperatingProfit":management.to_string(),"managementOperatingMarginRate":if revenue==Decimal::ZERO{Value::Null}else{json!((management/revenue).round_dp(8).to_string())},"dataQualityStatus":if complete{"complete"}else{"partial"},"sourceWatermark":row.get::<i64,_>("source_watermark"),"dataAsOf":data_as_of})
        }).collect::<Vec<_>>();
        Ok(
            json!({"items":items,"managementPeriod":period_value,"currency":currency,"dimensions":[dimension_one,dimension_two.unwrap_or("")],"dataAsOf":Utc::now(),"ruleVersion":"management-profit-v1","warnings":["经营管理口径，不是法定利润"]}),
        )
    }

    pub async fn management_report(
        &self,
        actor: Uuid,
        period_value: &str,
        currency: &str,
    ) -> Result<Value, DomainError> {
        let authorization = authorize(
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
        let dimension = self
            .profitability(actor, period_value, currency, "group", None, 1)
            .await?;
        let row=dimension["items"].as_array().and_then(|items|items.first()).cloned().unwrap_or_else(||json!({"netRevenue":"0","productCost":"0","grossProfit":"0","contributionProfit":"0","managementOperatingProfit":"0","dataQualityStatus":"partial"}));
        let source_watermark = row.get("sourceWatermark").cloned().unwrap_or(json!(0));
        let row_complete = row.get("dataQualityStatus").and_then(Value::as_str) == Some("complete");
        let unallocated:Decimal=sqlx::query_scalar("SELECT COALESCE(sum(l.amount),0) FROM operational_adjustment_lines l JOIN operational_adjustment_batches b ON b.id=l.batch_id WHERE b.management_period=$1 AND b.currency=$2 AND b.status IN ('draft','previewed') AND b.legal_entity_id=ANY($3) AND (l.customer_id IS NULL OR l.customer_id=ANY($4)) AND (l.brand_id IS NULL OR l.brand_id=ANY($5)) AND (l.business_unit_id IS NULL OR l.business_unit_id=ANY($6)) AND (l.warehouse_id IS NULL OR l.warehouse_id=ANY($7))").bind(period_value).bind(currency).bind(authorization.scopes.legal_entity_ids.iter().copied().collect::<Vec<_>>()).bind(authorization.scopes.customer_ids.iter().copied().collect::<Vec<_>>()).bind(authorization.scopes.brand_ids.iter().copied().collect::<Vec<_>>()).bind(authorization.scopes.business_unit_ids.iter().copied().collect::<Vec<_>>()).bind(authorization.scopes.warehouse_ids.iter().copied().collect::<Vec<_>>()).fetch_one(self.store.pool()).await?;
        Ok(
            json!({"reportType":"management_profit_statement","managementPeriod":period_value,"currency":currency,"rows":row,"unallocatedOperatingExpense":unallocated.to_string(),"dataQualityStatus":if unallocated>Decimal::ZERO||!row_complete{"partial"}else{"complete"},"sourceWatermark":source_watermark,"dataAsOf":Utc::now(),"warnings":["该报表基于业务经营数据和管理调整，不是法定会计报表，尚未与总账、发票和税务申报勾稽。"]}),
        )
    }

    pub async fn profit_change(
        &self,
        actor: Uuid,
        base_from: chrono::NaiveDate,
        base_to: chrono::NaiveDate,
        comparison_from: chrono::NaiveDate,
        comparison_to: chrono::NaiveDate,
        currency: &str,
    ) -> Result<Value, DomainError> {
        validate_currency(currency)?;
        if base_from > base_to || comparison_from > comparison_to {
            return Err(DomainError::Invalid(
                "profit change date range is reversed".into(),
            ));
        }
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
        let legal_entities = scope
            .scopes
            .legal_entity_ids
            .into_iter()
            .collect::<Vec<_>>();
        let customers = scope.scopes.customer_ids.into_iter().collect::<Vec<_>>();
        let brands = scope.scopes.brand_ids.into_iter().collect::<Vec<_>>();
        let business_units = scope
            .scopes
            .business_unit_ids
            .into_iter()
            .collect::<Vec<_>>();
        let warehouses = scope.scopes.warehouse_ids.into_iter().collect::<Vec<_>>();
        let base = aggregate_period(
            self.store.pool(),
            base_from,
            base_to,
            currency,
            &legal_entities,
            &customers,
            &brands,
            &business_units,
            &warehouses,
        )
        .await?;
        let comparison = aggregate_period(
            self.store.pool(),
            comparison_from,
            comparison_to,
            currency,
            &legal_entities,
            &customers,
            &brands,
            &business_units,
            &warehouses,
        )
        .await?;
        let base_profit: Decimal = base.get("management_operating_profit");
        let comparison_profit: Decimal = comparison.get("management_operating_profit");
        Ok(json!({
            "basePeriod":{"from":base_from,"to":base_to,"managementOperatingProfit":base_profit.to_string()},
            "comparisonPeriod":{"from":comparison_from,"to":comparison_to,"managementOperatingProfit":comparison_profit.to_string()},
            "change":(comparison_profit-base_profit).to_string(),
            "components":{
                "revenue":(comparison.get::<Decimal,_>("net_revenue")-base.get::<Decimal,_>("net_revenue")).to_string(),
                "productCost":(comparison.get::<Decimal,_>("product_cost")-base.get::<Decimal,_>("product_cost")).to_string(),
                "directCosts":(comparison.get::<Decimal,_>("direct_costs")-base.get::<Decimal,_>("direct_costs")).to_string(),
                "supplierRebate":(comparison.get::<Decimal,_>("supplier_rebate")-base.get::<Decimal,_>("supplier_rebate")).to_string(),
                "operatingExpense":(comparison.get::<Decimal,_>("operating_expense")-base.get::<Decimal,_>("operating_expense")).to_string()
            },
            "unexplainedDifference":"0.00","currency":currency,"ruleVersion":"management-profit-v1",
            "sourceWatermark":comparison.get::<i64,_>("source_watermark").max(base.get::<i64,_>("source_watermark")),
            "dataQualityStatus":if self.worker_enabled{"complete"}else{"partial"},"dataAsOf":Utc::now()
        }))
    }

    pub async fn generate_snapshot(
        &self,
        actor: Uuid,
        trace_id: Uuid,
        key: &str,
        input: &GenerateReportSnapshot,
    ) -> Result<CommandResult, DomainError> {
        period(&input.management_period)?;
        validate_currency(&input.currency)?;
        if !matches!(
            input.report_type.as_str(),
            "management_profit_statement" | "profitability_by_dimension"
        ) {
            return Err(DomainError::Invalid("unsupported report type".into()));
        }
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
        let legal_entities = if input.legal_entity_ids.is_empty() {
            auth.scopes
                .legal_entity_ids
                .iter()
                .copied()
                .collect::<Vec<_>>()
        } else {
            if input
                .legal_entity_ids
                .iter()
                .any(|id| !auth.scopes.legal_entity_ids.contains(id))
            {
                return Err(DomainError::NotFoundOrForbidden);
            }
            input.legal_entity_ids.clone()
        };
        let customers = auth.scopes.customer_ids.iter().copied().collect::<Vec<_>>();
        let brands = auth.scopes.brand_ids.iter().copied().collect::<Vec<_>>();
        let business_units = auth
            .scopes
            .business_unit_ids
            .iter()
            .copied()
            .collect::<Vec<_>>();
        let warehouses = auth
            .scopes
            .warehouse_ids
            .iter()
            .copied()
            .collect::<Vec<_>>();
        let hash = request_hash(input)?;
        let mut tx = self.store.pool().begin().await?;
        if let Some(mut replay) = begin_idempotent::<CommandResult>(
            &mut tx,
            actor,
            "management_report:generate_snapshot",
            key,
            &hash,
        )
        .await?
        {
            replay.idempotent_replay = true;
            tx.commit().await?;
            return Ok(replay);
        }
        let watermark:i64=sqlx::query_scalar("SELECT COALESCE(max(fact_sequence),0) FROM profit_facts WHERE management_period=$1 AND currency=$2 AND legal_entity_id=ANY($3) AND customer_id=ANY($4) AND (brand_id IS NULL OR brand_id=ANY($5)) AND business_unit_id=ANY($6) AND (warehouse_id IS NULL OR warehouse_id=ANY($7))").bind(&input.management_period).bind(&input.currency).bind(&legal_entities).bind(&customers).bind(&brands).bind(&business_units).bind(&warehouses).fetch_one(&mut *tx).await?;
        let components=sqlx::query("SELECT metric_type,COALESCE(sum(CASE direction WHEN 'normal' THEN amount ELSE -amount END),0)::numeric(24,6) amount,count(*) fact_count FROM profit_facts WHERE management_period=$1 AND currency=$2 AND legal_entity_id=ANY($3) AND fact_sequence<=$4 AND customer_id=ANY($5) AND (brand_id IS NULL OR brand_id=ANY($6)) AND business_unit_id=ANY($7) AND (warehouse_id IS NULL OR warehouse_id=ANY($8)) GROUP BY metric_type ORDER BY metric_type").bind(&input.management_period).bind(&input.currency).bind(&legal_entities).bind(watermark).bind(&customers).bind(&brands).bind(&business_units).bind(&warehouses).fetch_all(&mut *tx).await?;
        let amounts:Vec<Value>=components.into_iter().map(|row|json!({"metricType":row.get::<String,_>("metric_type"),"amount":row.get::<Decimal,_>("amount").to_string(),"factCount":row.get::<i64,_>("fact_count")})).collect();
        let facts_fresh: bool = sqlx::query_scalar("SELECT $1 AND NOT EXISTS(SELECT 1 FROM profit_projection_failures WHERE status='pending') AND COALESCE(max(data_as_of)>=now()-($2*interval '1 minute'),false) FROM profit_facts WHERE management_period=$3 AND currency=$4 AND legal_entity_id=ANY($5) AND customer_id=ANY($6) AND (brand_id IS NULL OR brand_id=ANY($7)) AND business_unit_id=ANY($8) AND (warehouse_id IS NULL OR warehouse_id=ANY($9))")
            .bind(self.worker_enabled).bind(self.stale_after_minutes).bind(&input.management_period).bind(&input.currency).bind(&legal_entities).bind(&customers).bind(&brands).bind(&business_units).bind(&warehouses).fetch_one(&mut *tx).await?;
        let has_unallocated: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM operational_adjustment_lines l JOIN operational_adjustment_batches b ON b.id=l.batch_id WHERE b.management_period=$1 AND b.currency=$2 AND b.status IN ('draft','previewed') AND b.legal_entity_id=ANY($3) AND (l.customer_id IS NULL OR l.customer_id=ANY($4)) AND (l.brand_id IS NULL OR l.brand_id=ANY($5)) AND (l.business_unit_id IS NULL OR l.business_unit_id=ANY($6)) AND (l.warehouse_id IS NULL OR l.warehouse_id=ANY($7)))")
            .bind(&input.management_period).bind(&input.currency).bind(&legal_entities).bind(&customers).bind(&brands).bind(&business_units).bind(&warehouses).fetch_one(&mut *tx).await?;
        let data_quality_status = if facts_fresh && !has_unallocated {
            "complete"
        } else {
            "partial"
        };
        let scope = json!({"legalEntityIds":legal_entities,"customerIds":customers,"brandIds":brands,"businessUnitIds":business_units,"warehouseIds":warehouses});
        let scope_hash = hex::encode(Sha256::digest(serde_json::to_vec(&scope)?));
        let source_hash = hex::encode(Sha256::digest(serde_json::to_vec(
            &json!({"scope":scope,"watermark":watermark,"amounts":amounts}),
        )?));
        let existing=sqlx::query("SELECT id,snapshot_number,version FROM management_report_snapshots WHERE report_type=$1 AND management_period=$2 AND currency=$3 AND scope_hash=$4 AND rule_version='management-profit-v1' AND source_watermark=$5").bind(&input.report_type).bind(&input.management_period).bind(&input.currency).bind(&scope_hash).bind(watermark).fetch_optional(&mut *tx).await?;
        if let Some(row) = existing {
            let result = CommandResult {
                id: row.get("id"),
                number: row.get("snapshot_number"),
                status: "generated".into(),
                version: row.get("version"),
                trace_id,
                idempotent_replay: true,
            };
            finish_idempotent(
                &mut tx,
                actor,
                "management_report:generate_snapshot",
                key,
                &result,
            )
            .await?;
            tx.commit().await?;
            return Ok(result);
        }
        let id = Uuid::new_v4();
        let number = next_number(
            &mut tx,
            "management_report",
            &self.snapshot_prefix,
            id,
            crate::numbering::NumberingContext::default(),
        )
        .await?;
        sqlx::query("INSERT INTO management_report_snapshots(id,snapshot_number,report_type,management_period,currency,scope,scope_hash,rule_version,source_watermark,source_hash,generated_by_user_id,supersedes_snapshot_id,data_as_of,trace_id) VALUES($1,$2,$3,$4,$5,$6,$7,'management-profit-v1',$8,$9,$10,$11,now(),$12)").bind(id).bind(&number).bind(&input.report_type).bind(&input.management_period).bind(&input.currency).bind(&scope).bind(&scope_hash).bind(watermark).bind(&source_hash).bind(actor).bind(input.supersedes_snapshot_id).bind(trace_id).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO management_report_snapshot_rows(id,snapshot_id,row_key,amounts,data_quality_status,warnings) VALUES($1,$2,'management_profit_statement',$3,$4,$5)").bind(Uuid::new_v4()).bind(id).bind(json!({"components":amounts})).bind(data_quality_status).bind(json!(["经营管理口径，不是法定利润"])).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO management_report_snapshot_evidence(id,snapshot_id,evidence_type,source_watermark,source_hash) VALUES($1,$2,'profit_facts',$3,$4)").bind(Uuid::new_v4()).bind(id).bind(watermark).bind(&source_hash).execute(&mut *tx).await?;
        record(&mut tx,trace_id,actor,"MANAGEMENT_REPORT_SNAPSHOT_GENERATED","management_report_snapshot_generated","management_report_snapshot",id,json!({"snapshotNumber":number,"managementPeriod":input.management_period,"currency":input.currency,"sourceWatermark":watermark})).await?;
        let result = CommandResult {
            id,
            number,
            status: "generated".into(),
            version: 1,
            trace_id,
            idempotent_replay: false,
        };
        finish_idempotent(
            &mut tx,
            actor,
            "management_report:generate_snapshot",
            key,
            &result,
        )
        .await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn snapshots(
        &self,
        actor: Uuid,
        id: Option<Uuid>,
        limit: i64,
    ) -> Result<Value, DomainError> {
        let authorization = authorize(
            &self.store,
            actor,
            "management_report:read_snapshot",
            None,
            None,
            None,
            None,
            None,
        )
        .await?;
        let rows=sqlx::query("SELECT s.id,s.snapshot_number,s.report_type,s.management_period,s.currency::text,s.scope,s.rule_version,s.source_watermark,s.source_hash,s.status,s.generated_at,s.data_as_of,s.version,COALESCE(jsonb_agg(jsonb_build_object('rowKey',r.row_key,'amounts',r.amounts,'dataQualityStatus',r.data_quality_status,'warnings',r.warnings)) FILTER(WHERE r.id IS NOT NULL),'[]') rows FROM management_report_snapshots s LEFT JOIN management_report_snapshot_rows r ON r.snapshot_id=s.id WHERE ($1::uuid IS NULL OR s.id=$1) GROUP BY s.id ORDER BY s.generated_at DESC LIMIT 100").bind(id).fetch_all(self.store.pool()).await?;
        let rows = rows
            .into_iter()
            .filter(|row| {
                let scope: Value = row.get("scope");
                [
                    ("legalEntityIds", &authorization.scopes.legal_entity_ids),
                    ("customerIds", &authorization.scopes.customer_ids),
                    ("brandIds", &authorization.scopes.brand_ids),
                    ("businessUnitIds", &authorization.scopes.business_unit_ids),
                    ("warehouseIds", &authorization.scopes.warehouse_ids),
                ]
                .into_iter()
                .all(|(key, allowed)| {
                    scope
                        .get(key)
                        .and_then(Value::as_array)
                        .is_some_and(|values| {
                            values
                                .iter()
                                .filter_map(Value::as_str)
                                .filter_map(|value| Uuid::parse_str(value).ok())
                                .all(|value| allowed.contains(&value))
                        })
                })
            })
            .take(limit.clamp(1, 100) as usize)
            .collect::<Vec<_>>();
        if id.is_some() && rows.is_empty() {
            return Err(DomainError::NotFoundOrForbidden);
        }
        Ok(
            json!({"items":rows.into_iter().map(|r|json!({"id":r.get::<Uuid,_>("id"),"snapshotNumber":r.get::<String,_>("snapshot_number"),"reportType":r.get::<String,_>("report_type"),"managementPeriod":r.get::<String,_>("management_period"),"currency":r.get::<String,_>("currency"),"scope":r.get::<Value,_>("scope"),"ruleVersion":r.get::<String,_>("rule_version"),"sourceWatermark":r.get::<i64,_>("source_watermark"),"sourceHash":r.get::<String,_>("source_hash"),"status":r.get::<String,_>("status"),"generatedAt":r.get::<chrono::DateTime<Utc>,_>("generated_at"),"dataAsOf":r.get::<chrono::DateTime<Utc>,_>("data_as_of"),"version":r.get::<i64,_>("version"),"rows":r.get::<Value,_>("rows"),"boundary":"not_statutory_financial_statement"})).collect::<Vec<_>>() }),
        )
    }

    pub async fn evidence(
        &self,
        actor: Uuid,
        order_id: Uuid,
        limit: i64,
    ) -> Result<Value, DomainError> {
        let scope = sqlx::query("SELECT legal_entity_id,customer_id,brand_id,business_unit_id FROM sales_orders WHERE id=$1")
            .bind(order_id)
            .fetch_optional(self.store.pool())
            .await?
            .ok_or(DomainError::NotFoundOrForbidden)?;
        let authorization = authorize(
            &self.store,
            actor,
            "profit:read_detail",
            Some(scope.get("legal_entity_id")),
            None,
            Some(scope.get("customer_id")),
            scope.get("brand_id"),
            Some(scope.get("business_unit_id")),
        )
        .await?;
        let outside_scope: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM profit_facts WHERE sales_order_id=$1 AND ((brand_id IS NOT NULL AND NOT(brand_id=ANY($2))) OR (business_unit_id IS NOT NULL AND NOT(business_unit_id=ANY($3))) OR (warehouse_id IS NOT NULL AND NOT(warehouse_id=ANY($4)))))")
            .bind(order_id)
            .bind(authorization.scopes.brand_ids.iter().copied().collect::<Vec<_>>())
            .bind(authorization.scopes.business_unit_ids.iter().copied().collect::<Vec<_>>())
            .bind(authorization.scopes.warehouse_ids.iter().copied().collect::<Vec<_>>())
            .fetch_one(self.store.pool()).await?;
        if outside_scope {
            return Err(DomainError::NotFoundOrForbidden);
        }
        let rows=sqlx::query("SELECT id,fact_sequence,metric_type,direction,amount,currency::text,shipment_id,shipment_line_id,source_type,source_id,source_line_id,source_event_id,source_event_version,business_date,management_period,data_as_of,trace_id FROM profit_facts WHERE sales_order_id=$1 ORDER BY fact_sequence LIMIT $2").bind(order_id).bind(limit.clamp(1,500)).fetch_all(self.store.pool()).await?;
        Ok(
            json!({"salesOrderId":order_id,"facts":rows.into_iter().map(|r|json!({"id":r.get::<Uuid,_>("id"),"factSequence":r.get::<i64,_>("fact_sequence"),"metricType":r.get::<String,_>("metric_type"),"direction":r.get::<String,_>("direction"),"amount":r.get::<Decimal,_>("amount").to_string(),"currency":r.get::<String,_>("currency"),"shipmentId":r.get::<Option<Uuid>,_>("shipment_id"),"shipmentLineId":r.get::<Option<Uuid>,_>("shipment_line_id"),"sourceType":r.get::<String,_>("source_type"),"sourceId":r.get::<Uuid,_>("source_id"),"sourceLineId":r.get::<Uuid,_>("source_line_id"),"sourceEventId":r.get::<Uuid,_>("source_event_id"),"sourceEventVersion":r.get::<i64,_>("source_event_version"),"businessDate":r.get::<chrono::NaiveDate,_>("business_date"),"managementPeriod":r.get::<String,_>("management_period"),"dataAsOf":r.get::<chrono::DateTime<Utc>,_>("data_as_of"),"traceId":r.get::<Uuid,_>("trace_id")})).collect::<Vec<_>>() }),
        )
    }

    async fn watermark(&self) -> Result<i64, DomainError> {
        Ok(
            sqlx::query_scalar("SELECT COALESCE(max(fact_sequence),0) FROM profit_facts")
                .fetch_one(self.store.pool())
                .await?,
        )
    }
}

#[allow(clippy::too_many_arguments)]
async fn aggregate_period(
    pool: &sqlx::PgPool,
    from: chrono::NaiveDate,
    to: chrono::NaiveDate,
    currency: &str,
    legal_entities: &[Uuid],
    customers: &[Uuid],
    brands: &[Uuid],
    business_units: &[Uuid],
    warehouses: &[Uuid],
) -> Result<sqlx::postgres::PgRow, DomainError> {
    Ok(sqlx::query("SELECT COALESCE(sum(CASE direction WHEN 'normal' THEN amount ELSE -amount END) FILTER(WHERE metric_type='net_revenue'),0)::numeric(24,6) net_revenue,COALESCE(sum(CASE direction WHEN 'normal' THEN amount ELSE -amount END) FILTER(WHERE metric_type='product_cost'),0)::numeric(24,6) product_cost,COALESCE(sum(CASE direction WHEN 'normal' THEN amount ELSE -amount END) FILTER(WHERE metric_type IN ('outbound_freight','sales_commission','platform_fee','customer_rebate','other_direct_cost')),0)::numeric(24,6) direct_costs,COALESCE(sum(CASE direction WHEN 'normal' THEN amount ELSE -amount END) FILTER(WHERE metric_type='supplier_rebate'),0)::numeric(24,6) supplier_rebate,COALESCE(sum(CASE direction WHEN 'normal' THEN amount ELSE -amount END) FILTER(WHERE metric_type='allocated_operating_expense'),0)::numeric(24,6) operating_expense,COALESCE(sum(CASE direction WHEN 'normal' THEN amount ELSE -amount END) FILTER(WHERE metric_type='net_revenue'),0)-COALESCE(sum(CASE direction WHEN 'normal' THEN amount ELSE -amount END) FILTER(WHERE metric_type='product_cost'),0)-COALESCE(sum(CASE direction WHEN 'normal' THEN amount ELSE -amount END) FILTER(WHERE metric_type IN ('outbound_freight','sales_commission','platform_fee','customer_rebate','other_direct_cost')),0)+COALESCE(sum(CASE direction WHEN 'normal' THEN amount ELSE -amount END) FILTER(WHERE metric_type='supplier_rebate'),0)-COALESCE(sum(CASE direction WHEN 'normal' THEN amount ELSE -amount END) FILTER(WHERE metric_type='allocated_operating_expense'),0) management_operating_profit,COALESCE(max(fact_sequence),0) source_watermark FROM profit_facts WHERE business_date BETWEEN $1 AND $2 AND currency=$3 AND legal_entity_id=ANY($4) AND customer_id=ANY($5) AND (brand_id IS NULL OR brand_id=ANY($6)) AND business_unit_id=ANY($7) AND (warehouse_id IS NULL OR warehouse_id=ANY($8))")
        .bind(from).bind(to).bind(currency).bind(legal_entities).bind(customers).bind(brands).bind(business_units).bind(warehouses).fetch_one(pool).await?)
}

fn validate_dimension(value: &str) -> Result<(), DomainError> {
    if matches!(
        value,
        "group"
            | "legal_entity"
            | "customer"
            | "sku"
            | "product_category"
            | "brand"
            | "salesperson"
            | "business_unit"
            | "department"
            | "warehouse"
            | "sales_order"
    ) {
        Ok(())
    } else {
        Err(DomainError::Invalid(
            "unsupported profitability dimension".into(),
        ))
    }
}
