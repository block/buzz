use super::{
    common::{authorize, begin_idempotent, finish_idempotent, money, record, request_hash},
    model::{CommandResult, DecimalString},
    DomainError,
};
use crate::store::PgStore;
use chrono::{NaiveDate, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::Row;
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InspectionLineInput {
    pub return_line_id: Uuid,
    pub accepted_quantity: DecimalString,
    pub scrap_quantity: DecimalString,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InspectSalesReturn {
    pub expected_version: i64,
    pub inspection_date: NaiveDate,
    #[serde(default)]
    pub inspection_note: Option<String>,
    pub lines: Vec<InspectionLineInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DispatchPurchaseReturn {
    pub expected_version: i64,
    pub dispatch_date: NaiveDate,
    pub carrier: String,
    pub tracking_number: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AcknowledgePurchaseReturn {
    pub expected_version: i64,
    pub acknowledged_date: NaiveDate,
    #[serde(default)]
    pub acknowledgment_note: Option<String>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct InspectionLineView {
    pub return_line_id: Uuid,
    pub sku_id: Uuid,
    pub sku_code: String,
    pub sku_name: String,
    #[sqlx(try_from = "Decimal")]
    pub quantity: DecimalString,
    #[sqlx(try_from = "Decimal")]
    pub unit_cost: DecimalString,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectionView {
    pub return_id: Uuid,
    pub return_number: String,
    pub version: i64,
    pub inspection_status: String,
    pub lines: Vec<InspectionLineView>,
}

#[derive(Clone)]
pub struct ReturnDispositionService {
    store: PgStore,
}

impl ReturnDispositionService {
    pub fn new(store: PgStore) -> Self {
        Self { store }
    }

    pub async fn sales_inspection(
        &self,
        actor: Uuid,
        id: Uuid,
    ) -> Result<InspectionView, DomainError> {
        let ret = sqlx::query("SELECT return_number,legal_entity_id,warehouse_id,customer_id,status,inspection_status,version FROM sales_returns WHERE id=$1")
            .bind(id).fetch_optional(self.store.pool()).await?.ok_or(DomainError::NotFoundOrForbidden)?;
        authorize(
            &self.store,
            actor,
            "sales_order:read",
            Some(ret.get("legal_entity_id")),
            Some(ret.get("warehouse_id")),
            Some(ret.get("customer_id")),
            None,
            None,
        )
        .await?;
        if ret.get::<String, _>("status") != "confirmed" {
            return Err(DomainError::Invalid(
                "only confirmed sales returns can be inspected".into(),
            ));
        }
        let lines=sqlx::query_as::<_,InspectionLineView>("SELECT l.id return_line_id,l.sku_id,s.code sku_code,s.name sku_name,l.quantity,l.unit_cost FROM sales_return_lines l JOIN business_skus s ON s.id=l.sku_id WHERE l.sales_return_id=$1 ORDER BY s.code,l.id").bind(id).fetch_all(self.store.pool()).await?;
        Ok(InspectionView {
            return_id: id,
            return_number: ret.get("return_number"),
            version: ret.get("version"),
            inspection_status: ret.get("inspection_status"),
            lines,
        })
    }

    pub async fn inspect_sales_return(
        &self,
        actor: Uuid,
        trace_id: Uuid,
        id: Uuid,
        key: &str,
        input: &InspectSalesReturn,
    ) -> Result<CommandResult, DomainError> {
        validate_inspection(input)?;
        let pre = sqlx::query(
            "SELECT legal_entity_id,warehouse_id,customer_id FROM sales_returns WHERE id=$1",
        )
        .bind(id)
        .fetch_optional(self.store.pool())
        .await?
        .ok_or(DomainError::NotFoundOrForbidden)?;
        authorize(
            &self.store,
            actor,
            "shipment:reverse",
            Some(pre.get("legal_entity_id")),
            Some(pre.get("warehouse_id")),
            Some(pre.get("customer_id")),
            None,
            None,
        )
        .await?;
        let hash = request_hash(input)?;
        let mut tx = self.store.pool().begin().await?;
        if let Some(mut replay) =
            begin_idempotent::<CommandResult>(&mut tx, actor, "sales_return:inspect", key, &hash)
                .await?
        {
            replay.idempotent_replay = true;
            tx.commit().await?;
            return Ok(replay);
        }
        let ret=sqlx::query("SELECT return_number,legal_entity_id,warehouse_id,return_date,currency::text,status,inspection_status,version FROM sales_returns WHERE id=$1 FOR UPDATE").bind(id).fetch_one(&mut *tx).await?;
        if ret.get::<i64, _>("version") != input.expected_version {
            return Err(DomainError::VersionConflict);
        }
        if ret.get::<String, _>("status") != "confirmed"
            || ret.get::<String, _>("inspection_status") != "pending"
        {
            return Err(DomainError::Invalid(
                "sales return is not pending inspection".into(),
            ));
        }
        if input.inspection_date < ret.get::<NaiveDate, _>("return_date") {
            return Err(DomainError::Invalid(
                "inspectionDate cannot precede returnDate".into(),
            ));
        }
        let lines=sqlx::query("SELECT id,sku_id,quantity,unit_cost,total_cost FROM sales_return_lines WHERE sales_return_id=$1 ORDER BY sku_id,id FOR UPDATE").bind(id).fetch_all(&mut *tx).await?;
        let requested = input
            .lines
            .iter()
            .map(|line| (line.return_line_id, line))
            .collect::<BTreeMap<_, _>>();
        if requested.len() != lines.len()
            || lines
                .iter()
                .any(|line| !requested.contains_key(&line.get::<Uuid, _>("id")))
        {
            return Err(DomainError::Invalid(
                "inspection must dispose every return line exactly once".into(),
            ));
        }
        let mut scrap_total = Decimal::ZERO;
        for line in &lines {
            let line_id: Uuid = line.get("id");
            let disposition = requested[&line_id];
            let accepted = disposition.accepted_quantity.0;
            let scrap = disposition.scrap_quantity.0;
            let quantity: Decimal = line.get("quantity");
            if accepted + scrap != quantity {
                return Err(DomainError::Invalid(
                    "acceptedQuantity plus scrapQuantity must equal returned quantity".into(),
                ));
            }
            let balance=sqlx::query("SELECT on_hand_quantity,reserved_quantity,quarantined_quantity,inventory_value FROM inventory_balances WHERE legal_entity_id=$1 AND warehouse_id=$2 AND sku_id=$3 FOR UPDATE").bind(ret.get::<Uuid,_>("legal_entity_id")).bind(ret.get::<Uuid,_>("warehouse_id")).bind(line.get::<Uuid,_>("sku_id")).fetch_one(&mut *tx).await?;
            if balance.get::<Decimal, _>("quarantined_quantity") < quantity {
                return Err(DomainError::Invalid(
                    "quarantined return stock is no longer available".into(),
                ));
            }
            let unit: Decimal = line.get("unit_cost");
            let scrap_cost = if scrap == quantity {
                line.get::<Decimal, _>("total_cost")
            } else {
                money(unit * scrap)
            };
            let new_qty = balance.get::<Decimal, _>("on_hand_quantity") - scrap;
            let new_quarantine = balance.get::<Decimal, _>("quarantined_quantity") - quantity;
            let new_value = money(balance.get::<Decimal, _>("inventory_value") - scrap_cost);
            if new_value < Decimal::ZERO
                || balance.get::<Decimal, _>("reserved_quantity") + new_quarantine > new_qty
            {
                return Err(DomainError::Invalid(
                    "inspection would violate inventory balance".into(),
                ));
            }
            let movement = if scrap > Decimal::ZERO {
                let movement_id = Uuid::new_v4();
                sqlx::query("INSERT INTO inventory_movements(id,legal_entity_id,warehouse_id,sku_id,movement_type,quantity,unit_cost,total_cost,currency,source_type,source_id,source_line_id,business_date,created_by_user_id,trace_id) VALUES($1,$2,$3,$4,'sales_return_scrap',$5,$6,$7,$8,'sales_return',$9,$10,$11,$12,$13)").bind(movement_id).bind(ret.get::<Uuid,_>("legal_entity_id")).bind(ret.get::<Uuid,_>("warehouse_id")).bind(line.get::<Uuid,_>("sku_id")).bind(-scrap).bind(unit).bind(-scrap_cost).bind(ret.get::<String,_>("currency")).bind(id).bind(line_id).bind(input.inspection_date).bind(actor).bind(trace_id).execute(&mut *tx).await?;
                Some(movement_id)
            } else {
                None
            };
            let average = if new_qty == Decimal::ZERO {
                None
            } else {
                Some(money(new_value / new_qty))
            };
            sqlx::query("UPDATE inventory_balances SET on_hand_quantity=$4,quarantined_quantity=$5,inventory_value=$6,average_unit_cost=$7,last_movement_id=COALESCE($8,last_movement_id) WHERE legal_entity_id=$1 AND warehouse_id=$2 AND sku_id=$3").bind(ret.get::<Uuid,_>("legal_entity_id")).bind(ret.get::<Uuid,_>("warehouse_id")).bind(line.get::<Uuid,_>("sku_id")).bind(new_qty).bind(new_quarantine).bind(new_value).bind(average).bind(movement).execute(&mut *tx).await?;
            sqlx::query("UPDATE sales_return_lines SET accepted_quantity=$2,scrap_quantity=$3,scrap_cost_amount=$4 WHERE id=$1").bind(line_id).bind(accepted).bind(scrap).bind(scrap_cost).execute(&mut *tx).await?;
            scrap_total += scrap_cost;
        }
        let version = input.expected_version + 1;
        sqlx::query("UPDATE sales_returns SET inspection_status='completed',inspection_date=$2,inspection_note=$3,scrap_cost_amount=$4,inspected_by_user_id=$5,inspected_at=now(),version=$6,trace_id=$7 WHERE id=$1").bind(id).bind(input.inspection_date).bind(&input.inspection_note).bind(money(scrap_total)).bind(actor).bind(version).bind(trace_id).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO sales_return_events(id,sales_return_id,event_type,return_version,payload,actor_user_id,trace_id) VALUES($1,$2,'inspected',$3,$4,$5,$6)").bind(Uuid::new_v4()).bind(id).bind(version).bind(json!({"scrapCostAmount":money(scrap_total).to_string()})).bind(actor).bind(trace_id).execute(&mut *tx).await?;
        record(
            &mut tx,
            trace_id,
            actor,
            "SALES_RETURN_INSPECTED",
            "sales_return_inspected",
            "sales_return",
            id,
            json!({"version":version,"scrapCostAmount":money(scrap_total).to_string()}),
        )
        .await?;
        let result = CommandResult {
            id,
            number: ret.get("return_number"),
            status: "completed".into(),
            version,
            trace_id,
            idempotent_replay: false,
        };
        finish_idempotent(&mut tx, actor, "sales_return:inspect", key, &result).await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn dispatch_purchase_return(
        &self,
        actor: Uuid,
        trace_id: Uuid,
        id: Uuid,
        key: &str,
        input: &DispatchPurchaseReturn,
    ) -> Result<CommandResult, DomainError> {
        if input.carrier.trim().is_empty()
            || input.carrier.len() > 120
            || input.tracking_number.trim().is_empty()
            || input.tracking_number.len() > 120
        {
            return Err(DomainError::Invalid(
                "carrier and trackingNumber are required".into(),
            ));
        }
        self.purchase_transition(
            actor,
            trace_id,
            id,
            key,
            input,
            "dispatch",
            Some((input.dispatch_date, &input.carrier, &input.tracking_number)),
            None,
        )
        .await
    }

    pub async fn acknowledge_purchase_return(
        &self,
        actor: Uuid,
        trace_id: Uuid,
        id: Uuid,
        key: &str,
        input: &AcknowledgePurchaseReturn,
    ) -> Result<CommandResult, DomainError> {
        if input
            .acknowledgment_note
            .as_ref()
            .is_some_and(|note| note.len() > 1000)
        {
            return Err(DomainError::Invalid(
                "acknowledgmentNote is too long".into(),
            ));
        }
        self.purchase_transition(
            actor,
            trace_id,
            id,
            key,
            input,
            "acknowledge",
            None,
            Some((
                input.acknowledged_date,
                input.acknowledgment_note.as_deref(),
            )),
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn purchase_transition<T: Serialize>(
        &self,
        actor: Uuid,
        trace_id: Uuid,
        id: Uuid,
        key: &str,
        input: &T,
        action: &str,
        dispatch: Option<(NaiveDate, &str, &str)>,
        acknowledgment: Option<(NaiveDate, Option<&str>)>,
    ) -> Result<CommandResult, DomainError> {
        let pre = sqlx::query(
            "SELECT legal_entity_id,warehouse_id,supplier_id FROM purchase_returns WHERE id=$1",
        )
        .bind(id)
        .fetch_optional(self.store.pool())
        .await?
        .ok_or(DomainError::NotFoundOrForbidden)?;
        authorize(
            &self.store,
            actor,
            "goods_receipt:reverse",
            Some(pre.get("legal_entity_id")),
            Some(pre.get("warehouse_id")),
            Some(pre.get("supplier_id")),
            None,
            None,
        )
        .await?;
        let command = if action == "dispatch" {
            "purchase_return:dispatch"
        } else {
            "purchase_return:acknowledge"
        };
        let hash = request_hash(input)?;
        let mut tx = self.store.pool().begin().await?;
        if let Some(mut replay) =
            begin_idempotent::<CommandResult>(&mut tx, actor, command, key, &hash).await?
        {
            replay.idempotent_replay = true;
            tx.commit().await?;
            return Ok(replay);
        }
        let ret=sqlx::query("SELECT return_number,return_date,status,logistics_status,dispatch_date,version FROM purchase_returns WHERE id=$1 FOR UPDATE").bind(id).fetch_one(&mut *tx).await?;
        let expected = input_version(input)?;
        if ret.get::<i64, _>("version") != expected {
            return Err(DomainError::VersionConflict);
        }
        if ret.get::<String, _>("status") != "confirmed" {
            return Err(DomainError::Invalid(
                "only confirmed purchase returns have logistics transitions".into(),
            ));
        }
        let (status, event, code, version) = if let Some((date, carrier, tracking)) = dispatch {
            if ret.get::<String, _>("logistics_status") != "not_dispatched"
                || date < ret.get::<NaiveDate, _>("return_date")
            {
                return Err(DomainError::Invalid(
                    "purchase return is not ready for dispatch".into(),
                ));
            }
            sqlx::query("UPDATE purchase_returns SET logistics_status='dispatched',dispatch_date=$2,carrier=$3,tracking_number=$4,dispatched_by_user_id=$5,dispatched_at=now(),version=$6,trace_id=$7 WHERE id=$1").bind(id).bind(date).bind(carrier.trim()).bind(tracking.trim()).bind(actor).bind(expected+1).bind(trace_id).execute(&mut *tx).await?;
            (
                "dispatched",
                "dispatched",
                "PURCHASE_RETURN_DISPATCHED",
                expected + 1,
            )
        } else if let Some((date, note)) = acknowledgment {
            if ret.get::<String, _>("logistics_status") != "dispatched"
                || date < ret.get::<NaiveDate, _>("dispatch_date")
            {
                return Err(DomainError::Invalid(
                    "supplier acknowledgment cannot precede dispatch".into(),
                ));
            }
            sqlx::query("UPDATE purchase_returns SET logistics_status='supplier_acknowledged',supplier_acknowledged_date=$2,supplier_acknowledgment_note=$3,supplier_acknowledged_by_user_id=$4,supplier_acknowledged_at=now(),version=$5,trace_id=$6 WHERE id=$1").bind(id).bind(date).bind(note).bind(actor).bind(expected+1).bind(trace_id).execute(&mut *tx).await?;
            (
                "supplier_acknowledged",
                "supplier_acknowledged",
                "PURCHASE_RETURN_SUPPLIER_ACKNOWLEDGED",
                expected + 1,
            )
        } else {
            return Err(DomainError::Invalid(
                "missing purchase return transition".into(),
            ));
        };
        sqlx::query("INSERT INTO purchase_return_events(id,purchase_return_id,event_type,return_version,payload,actor_user_id,trace_id) VALUES($1,$2,$3,$4,$5,$6,$7)").bind(Uuid::new_v4()).bind(id).bind(event).bind(version).bind(json!({"status":status})).bind(actor).bind(trace_id).execute(&mut *tx).await?;
        record(
            &mut tx,
            trace_id,
            actor,
            code,
            if action == "dispatch" {
                "purchase_return_dispatched"
            } else {
                "purchase_return_supplier_acknowledged"
            },
            "purchase_return",
            id,
            json!({"version":version}),
        )
        .await?;
        let result = CommandResult {
            id,
            number: ret.get("return_number"),
            status: status.into(),
            version,
            trace_id,
            idempotent_replay: false,
        };
        finish_idempotent(&mut tx, actor, command, key, &result).await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn analytics(
        &self,
        actor: Uuid,
        period: &str,
        currency: &str,
    ) -> Result<Value, DomainError> {
        let month = NaiveDate::parse_from_str(&format!("{period}-01"), "%Y-%m-%d")
            .map_err(|_| DomainError::Invalid("period must use YYYY-MM".into()))?;
        if currency.len() != 3 || !currency.bytes().all(|value| value.is_ascii_uppercase()) {
            return Err(DomainError::Invalid(
                "currency must be ISO 4217 uppercase".into(),
            ));
        }
        let scope = authorize(
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
        let rows=sqlx::query("SELECT legal_entity_id,currency,management_period,shipped_sales_amount,sales_return_count,sales_return_amount,sales_return_rate,return_loss_amount,scrap_cost_amount,received_purchase_amount,purchase_return_count,purchase_return_amount,purchase_return_rate FROM return_operating_metrics WHERE legal_entity_id=ANY($1) AND management_period=$2 AND currency=$3 ORDER BY legal_entity_id").bind(scope.scopes.legal_entity_ids.into_iter().collect::<Vec<_>>()).bind(month).bind(currency).fetch_all(self.store.pool()).await?;
        let items=rows.into_iter().map(|row|json!({"legalEntityId":row.get::<Uuid,_>("legal_entity_id"),"currency":row.get::<String,_>("currency"),"managementPeriod":row.get::<NaiveDate,_>("management_period").format("%Y-%m").to_string(),"shippedSalesAmount":row.get::<Decimal,_>("shipped_sales_amount").to_string(),"salesReturnCount":row.get::<i64,_>("sales_return_count"),"salesReturnAmount":row.get::<Decimal,_>("sales_return_amount").to_string(),"salesReturnRate":row.get::<Option<Decimal>,_>("sales_return_rate").map(|v|v.to_string()),"returnLossAmount":row.get::<Decimal,_>("return_loss_amount").to_string(),"scrapCostAmount":row.get::<Decimal,_>("scrap_cost_amount").to_string(),"receivedPurchaseAmount":row.get::<Decimal,_>("received_purchase_amount").to_string(),"purchaseReturnCount":row.get::<i64,_>("purchase_return_count"),"purchaseReturnAmount":row.get::<Decimal,_>("purchase_return_amount").to_string(),"purchaseReturnRate":row.get::<Option<Decimal>,_>("purchase_return_rate").map(|v|v.to_string())})).collect::<Vec<_>>();
        Ok(
            json!({"items":items,"managementPeriod":period,"currency":currency,"dataAsOf":Utc::now(),"warnings":["退货损失为经营管理口径，不是法定会计损失或总账金额"]}),
        )
    }
}

fn input_version<T: Serialize>(input: &T) -> Result<i64, DomainError> {
    serde_json::to_value(input)
        .map_err(|error| DomainError::Invalid(error.to_string()))?
        .get("expectedVersion")
        .and_then(Value::as_i64)
        .ok_or_else(|| DomainError::Invalid("expectedVersion is required".into()))
}

fn validate_inspection(input: &InspectSalesReturn) -> Result<(), DomainError> {
    if input.lines.is_empty() || input.lines.len() > 200 {
        return Err(DomainError::Invalid(
            "inspection requires 1-200 lines".into(),
        ));
    }
    if input
        .inspection_note
        .as_ref()
        .is_some_and(|note| note.len() > 1000)
    {
        return Err(DomainError::Invalid("inspectionNote is too long".into()));
    }
    let mut seen = BTreeSet::new();
    for line in &input.lines {
        if !seen.insert(line.return_line_id)
            || line.accepted_quantity.0 < Decimal::ZERO
            || line.scrap_quantity.0 < Decimal::ZERO
        {
            return Err(DomainError::Invalid(
                "invalid or duplicate inspection line".into(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inspection_rejects_duplicate_lines() {
        let id = Uuid::new_v4();
        let input = InspectSalesReturn {
            expected_version: 1,
            inspection_date: NaiveDate::from_ymd_opt(2026, 8, 22).unwrap(),
            inspection_note: None,
            lines: vec![
                InspectionLineInput {
                    return_line_id: id,
                    accepted_quantity: DecimalString(Decimal::ONE),
                    scrap_quantity: DecimalString(Decimal::ZERO),
                },
                InspectionLineInput {
                    return_line_id: id,
                    accepted_quantity: DecimalString(Decimal::ONE),
                    scrap_quantity: DecimalString(Decimal::ZERO),
                },
            ],
        };
        assert!(validate_inspection(&input).is_err());
    }
}
