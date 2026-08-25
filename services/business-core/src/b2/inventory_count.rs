use super::{
    common::{
        authorize, begin_idempotent, finish_idempotent, money, next_number, record, request_hash,
        validate_currency,
    },
    model::{CommandResult, DecimalString, OptionalDecimalString, VersionCommand},
    DomainError,
};
use crate::store::PgStore;
use chrono::{Datelike, NaiveDate, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::Row;
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateInventoryCount {
    pub legal_entity_id: Uuid,
    pub warehouse_id: Uuid,
    pub count_date: NaiveDate,
    pub currency: String,
    #[serde(default)]
    pub business_note: Option<String>,
    pub sku_ids: Vec<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InventoryCountLineInput {
    pub count_line_id: Uuid,
    pub actual_on_hand_quantity: DecimalString,
    #[serde(default)]
    pub surplus_unit_cost: Option<DecimalString>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubmitInventoryCount {
    pub expected_version: i64,
    pub lines: Vec<InventoryCountLineInput>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct InventoryCountSummary {
    pub id: Uuid,
    pub count_number: String,
    pub legal_entity_id: Uuid,
    pub warehouse_id: Uuid,
    pub count_date: NaiveDate,
    pub currency: String,
    pub status: String,
    pub line_count: i64,
    pub variance_line_count: i64,
    #[sqlx(try_from = "Decimal")]
    pub variance_value: DecimalString,
    pub version: i64,
    pub updated_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct InventoryCountLineView {
    pub id: Uuid,
    pub sku_id: Uuid,
    pub sku_code: String,
    pub sku_name: String,
    #[sqlx(try_from = "Decimal")]
    pub snapshot_on_hand_quantity: DecimalString,
    #[sqlx(try_from = "Decimal")]
    pub snapshot_reserved_quantity: DecimalString,
    #[sqlx(try_from = "Decimal")]
    pub snapshot_quarantined_quantity: DecimalString,
    #[sqlx(try_from = "Option<Decimal>")]
    pub actual_on_hand_quantity: OptionalDecimalString,
    #[sqlx(try_from = "Option<Decimal>")]
    pub snapshot_average_unit_cost: OptionalDecimalString,
    #[sqlx(try_from = "Option<Decimal>")]
    pub surplus_unit_cost: OptionalDecimalString,
    #[sqlx(try_from = "Option<Decimal>")]
    pub variance_quantity: OptionalDecimalString,
    #[sqlx(try_from = "Option<Decimal>")]
    pub variance_value: OptionalDecimalString,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InventoryCountDetail {
    pub id: Uuid,
    pub count_number: String,
    pub legal_entity_id: Uuid,
    pub warehouse_id: Uuid,
    pub count_date: NaiveDate,
    pub currency: String,
    pub status: String,
    pub version: i64,
    pub lines: Vec<InventoryCountLineView>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct InventoryCountOption {
    pub legal_entity_id: Uuid,
    pub currency: String,
    pub warehouse_id: Uuid,
    pub warehouse_code: String,
    pub warehouse_name: String,
    pub sku_id: Uuid,
    pub sku_code: String,
    pub sku_name: String,
    #[sqlx(try_from = "Decimal")]
    pub on_hand_quantity: DecimalString,
    #[sqlx(try_from = "Decimal")]
    pub reserved_quantity: DecimalString,
    #[sqlx(try_from = "Decimal")]
    pub quarantined_quantity: DecimalString,
    #[sqlx(try_from = "Decimal")]
    pub inventory_value: DecimalString,
    #[sqlx(try_from = "Option<Decimal>")]
    pub average_unit_cost: OptionalDecimalString,
}

#[derive(Clone)]
pub struct InventoryCountService {
    store: PgStore,
    prefix: String,
}

impl InventoryCountService {
    pub fn new(store: PgStore, prefix: String) -> Self {
        Self { store, prefix }
    }

    pub async fn options(&self, actor: Uuid) -> Result<Vec<InventoryCountOption>, DomainError> {
        let scope = authorize(
            &self.store,
            actor,
            "inventory:read",
            None,
            None,
            None,
            None,
            None,
        )
        .await?;
        sqlx::query_as::<_, InventoryCountOption>("SELECT b.legal_entity_id,e.functional_currency::text currency,b.warehouse_id,w.code warehouse_code,w.name warehouse_name,b.sku_id,s.code sku_code,s.name sku_name,b.on_hand_quantity,b.reserved_quantity,b.quarantined_quantity,b.inventory_value,b.average_unit_cost FROM inventory_balances b JOIN business_legal_entities e ON e.id=b.legal_entity_id JOIN business_warehouses w ON w.id=b.warehouse_id JOIN business_skus s ON s.id=b.sku_id WHERE b.legal_entity_id=ANY($1) AND b.warehouse_id=ANY($2) AND NOT EXISTS(SELECT 1 FROM inventory_count_tasks t JOIN inventory_count_lines l ON l.inventory_count_id=t.id WHERE t.status IN ('counting','counted') AND t.legal_entity_id=b.legal_entity_id AND t.warehouse_id=b.warehouse_id AND l.sku_id=b.sku_id) ORDER BY w.code,s.code LIMIT 1000")
            .bind(scope.scopes.legal_entity_ids.into_iter().collect::<Vec<_>>())
            .bind(scope.scopes.warehouse_ids.into_iter().collect::<Vec<_>>())
            .fetch_all(self.store.pool()).await.map_err(Into::into)
    }

    pub async fn list(
        &self,
        actor: Uuid,
        limit: i64,
    ) -> Result<Vec<InventoryCountSummary>, DomainError> {
        let scope = authorize(
            &self.store,
            actor,
            "inventory:read",
            None,
            None,
            None,
            None,
            None,
        )
        .await?;
        sqlx::query_as::<_,InventoryCountSummary>("SELECT t.id,t.count_number,t.legal_entity_id,t.warehouse_id,t.count_date,t.currency::text currency,t.status,count(l.id) line_count,count(l.id) FILTER(WHERE COALESCE(l.variance_quantity,0)<>0) variance_line_count,COALESCE(sum(l.variance_value),0) variance_value,t.version,t.updated_at FROM inventory_count_tasks t JOIN inventory_count_lines l ON l.inventory_count_id=t.id WHERE t.legal_entity_id=ANY($1) AND t.warehouse_id=ANY($2) GROUP BY t.id ORDER BY t.count_date DESC,t.count_number DESC LIMIT $3").bind(scope.scopes.legal_entity_ids.into_iter().collect::<Vec<_>>()).bind(scope.scopes.warehouse_ids.into_iter().collect::<Vec<_>>()).bind(limit.clamp(1,500)).fetch_all(self.store.pool()).await.map_err(Into::into)
    }

    pub async fn detail(&self, actor: Uuid, id: Uuid) -> Result<InventoryCountDetail, DomainError> {
        let task=sqlx::query("SELECT count_number,legal_entity_id,warehouse_id,count_date,currency::text,status,version FROM inventory_count_tasks WHERE id=$1").bind(id).fetch_optional(self.store.pool()).await?.ok_or(DomainError::NotFoundOrForbidden)?;
        authorize(
            &self.store,
            actor,
            "inventory:read",
            Some(task.get("legal_entity_id")),
            Some(task.get("warehouse_id")),
            None,
            None,
            None,
        )
        .await?;
        let lines=sqlx::query_as::<_,InventoryCountLineView>("SELECT l.id,l.sku_id,s.code sku_code,s.name sku_name,l.snapshot_on_hand_quantity,l.snapshot_reserved_quantity,l.snapshot_quarantined_quantity,l.actual_on_hand_quantity,l.snapshot_average_unit_cost,l.surplus_unit_cost,l.variance_quantity,l.variance_value FROM inventory_count_lines l JOIN business_skus s ON s.id=l.sku_id WHERE l.inventory_count_id=$1 ORDER BY s.code,l.id").bind(id).fetch_all(self.store.pool()).await?;
        Ok(InventoryCountDetail {
            id,
            count_number: task.get("count_number"),
            legal_entity_id: task.get("legal_entity_id"),
            warehouse_id: task.get("warehouse_id"),
            count_date: task.get("count_date"),
            currency: task.get("currency"),
            status: task.get("status"),
            version: task.get("version"),
            lines,
        })
    }

    pub async fn create(
        &self,
        actor: Uuid,
        trace_id: Uuid,
        key: &str,
        input: &CreateInventoryCount,
    ) -> Result<CommandResult, DomainError> {
        validate_currency(&input.currency)?;
        validate_create(input)?;
        let scope = authorize(
            &self.store,
            actor,
            "inventory_opening:create",
            Some(input.legal_entity_id),
            Some(input.warehouse_id),
            None,
            None,
            None,
        )
        .await?;
        if !scope.scopes.warehouse_ids.contains(&input.warehouse_id) {
            return Err(DomainError::NotFoundOrForbidden);
        }
        let hash = request_hash(input)?;
        let mut tx = self.store.pool().begin().await?;
        if let Some(mut replay) =
            begin_idempotent::<CommandResult>(&mut tx, actor, "inventory_count:create", key, &hash)
                .await?
        {
            replay.idempotent_replay = true;
            tx.commit().await?;
            return Ok(replay);
        }
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(input.warehouse_id.to_string())
            .execute(&mut *tx)
            .await?;
        let functional_currency: String = sqlx::query_scalar(
            "SELECT functional_currency::text FROM business_legal_entities WHERE id=$1",
        )
        .bind(input.legal_entity_id)
        .fetch_one(&mut *tx)
        .await?;
        if input.currency != functional_currency {
            return Err(DomainError::Invalid(
                "inventory count currency must match the legal entity functional currency".into(),
            ));
        }
        let business_unit_id: Uuid = sqlx::query_scalar(
            "SELECT business_unit_id FROM business_warehouses WHERE id=$1 AND legal_entity_id=$2 AND status='active'",
        )
        .bind(input.warehouse_id)
        .bind(input.legal_entity_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(DomainError::NotFoundOrForbidden)?;
        let balances=sqlx::query("SELECT sku_id,on_hand_quantity,reserved_quantity,quarantined_quantity,inventory_value,average_unit_cost FROM inventory_balances WHERE legal_entity_id=$1 AND warehouse_id=$2 AND sku_id=ANY($3) ORDER BY sku_id FOR UPDATE").bind(input.legal_entity_id).bind(input.warehouse_id).bind(&input.sku_ids).fetch_all(&mut *tx).await?;
        if balances.len() != input.sku_ids.len() {
            return Err(DomainError::NotFoundOrForbidden);
        }
        let overlap:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM inventory_count_tasks t JOIN inventory_count_lines l ON l.inventory_count_id=t.id WHERE t.status IN ('counting','counted') AND t.legal_entity_id=$1 AND t.warehouse_id=$2 AND l.sku_id=ANY($3))").bind(input.legal_entity_id).bind(input.warehouse_id).bind(&input.sku_ids).fetch_one(&mut *tx).await?;
        if overlap {
            return Err(DomainError::Invalid(
                "inventory count scope is already frozen".into(),
            ));
        }
        let id = Uuid::new_v4();
        let number = next_number(
            &mut tx,
            "inventory_count",
            &self.prefix,
            id,
            crate::numbering::NumberingContext::new(input.legal_entity_id, Some(business_unit_id)),
        )
        .await?;
        sqlx::query("INSERT INTO inventory_count_tasks(id,count_number,legal_entity_id,warehouse_id,count_date,currency,business_note,created_by_user_id,trace_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)").bind(id).bind(&number).bind(input.legal_entity_id).bind(input.warehouse_id).bind(input.count_date).bind(&input.currency).bind(&input.business_note).bind(actor).bind(trace_id).execute(&mut *tx).await?;
        for balance in &balances {
            sqlx::query("INSERT INTO inventory_count_lines(id,inventory_count_id,sku_id,snapshot_on_hand_quantity,snapshot_reserved_quantity,snapshot_quarantined_quantity,snapshot_inventory_value,snapshot_average_unit_cost) VALUES($1,$2,$3,$4,$5,$6,$7,$8)").bind(Uuid::new_v4()).bind(id).bind(balance.get::<Uuid,_>("sku_id")).bind(balance.get::<Decimal,_>("on_hand_quantity")).bind(balance.get::<Decimal,_>("reserved_quantity")).bind(balance.get::<Decimal,_>("quarantined_quantity")).bind(balance.get::<Decimal,_>("inventory_value")).bind(balance.get::<Option<Decimal>,_>("average_unit_cost")).execute(&mut *tx).await?;
        }
        count_event(
            &mut tx,
            id,
            "created",
            1,
            actor,
            trace_id,
            json!({"lineCount":balances.len()}),
        )
        .await?;
        record(
            &mut tx,
            trace_id,
            actor,
            "INVENTORY_COUNT_CREATED",
            "inventory_count_created",
            "inventory_count",
            id,
            json!({"countNumber":number,"lineCount":balances.len()}),
        )
        .await?;
        let result = CommandResult {
            id,
            number,
            status: "counting".into(),
            version: 1,
            trace_id,
            idempotent_replay: false,
        };
        finish_idempotent(&mut tx, actor, "inventory_count:create", key, &result).await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn submit(
        &self,
        actor: Uuid,
        trace_id: Uuid,
        id: Uuid,
        key: &str,
        input: &SubmitInventoryCount,
    ) -> Result<CommandResult, DomainError> {
        self.pre_authorize(actor, id, "inventory_opening:create")
            .await?;
        let hash = request_hash(input)?;
        let mut tx = self.store.pool().begin().await?;
        if let Some(mut replay) =
            begin_idempotent::<CommandResult>(&mut tx, actor, "inventory_count:submit", key, &hash)
                .await?
        {
            replay.idempotent_replay = true;
            tx.commit().await?;
            return Ok(replay);
        }
        let task = sqlx::query(
            "SELECT count_number,status,version FROM inventory_count_tasks WHERE id=$1 FOR UPDATE",
        )
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
        check_task(&task, input.expected_version, "counting")?;
        let lines=sqlx::query("SELECT id,snapshot_on_hand_quantity,snapshot_reserved_quantity,snapshot_quarantined_quantity,snapshot_average_unit_cost FROM inventory_count_lines WHERE inventory_count_id=$1 ORDER BY id FOR UPDATE").bind(id).fetch_all(&mut *tx).await?;
        let values = input
            .lines
            .iter()
            .map(|line| (line.count_line_id, line))
            .collect::<BTreeMap<_, _>>();
        if values.len() != lines.len()
            || lines
                .iter()
                .any(|line| !values.contains_key(&line.get::<Uuid, _>("id")))
        {
            return Err(DomainError::Invalid(
                "every count line must be entered exactly once".into(),
            ));
        }
        for line in &lines {
            let value = values[&line.get::<Uuid, _>("id")];
            let actual = value
                .actual_on_hand_quantity
                .non_negative("actualOnHandQuantity")
                .map_err(DomainError::Invalid)?;
            let protected = line.get::<Decimal, _>("snapshot_reserved_quantity")
                + line.get::<Decimal, _>("snapshot_quarantined_quantity");
            if actual < protected {
                return Err(DomainError::Invalid(
                    "actual quantity cannot be lower than reserved plus quarantined quantity"
                        .into(),
                ));
            }
            let snapshot: Decimal = line.get("snapshot_on_hand_quantity");
            if actual > snapshot
                && line
                    .get::<Option<Decimal>, _>("snapshot_average_unit_cost")
                    .is_none()
                && value.surplus_unit_cost.is_none()
            {
                return Err(DomainError::MissingInventoryCost);
            }
            if value
                .surplus_unit_cost
                .is_some_and(|cost| cost.0 < Decimal::ZERO)
            {
                return Err(DomainError::Invalid(
                    "surplusUnitCost must be non-negative".into(),
                ));
            }
            sqlx::query("UPDATE inventory_count_lines SET actual_on_hand_quantity=$2,surplus_unit_cost=$3 WHERE id=$1").bind(line.get::<Uuid,_>("id")).bind(actual).bind(value.surplus_unit_cost.map(|cost|cost.0)).execute(&mut *tx).await?;
        }
        let version = input.expected_version + 1;
        sqlx::query("UPDATE inventory_count_tasks SET status='counted',counted_by_user_id=$2,counted_at=now(),version=$3,trace_id=$4 WHERE id=$1").bind(id).bind(actor).bind(version).bind(trace_id).execute(&mut *tx).await?;
        count_event(
            &mut tx,
            id,
            "counted",
            version,
            actor,
            trace_id,
            json!({"lineCount":lines.len()}),
        )
        .await?;
        record(
            &mut tx,
            trace_id,
            actor,
            "INVENTORY_COUNT_ENTERED",
            "inventory_count_entered",
            "inventory_count",
            id,
            json!({"version":version}),
        )
        .await?;
        let result = CommandResult {
            id,
            number: task.get("count_number"),
            status: "counted".into(),
            version,
            trace_id,
            idempotent_replay: false,
        };
        finish_idempotent(&mut tx, actor, "inventory_count:submit", key, &result).await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn post(
        &self,
        actor: Uuid,
        trace_id: Uuid,
        id: Uuid,
        key: &str,
        input: &VersionCommand,
    ) -> Result<CommandResult, DomainError> {
        self.pre_authorize(actor, id, "inventory_opening:post")
            .await?;
        let hash = request_hash(input)?;
        let mut tx = self.store.pool().begin().await?;
        if let Some(mut replay) =
            begin_idempotent::<CommandResult>(&mut tx, actor, "inventory_count:post", key, &hash)
                .await?
        {
            replay.idempotent_replay = true;
            tx.commit().await?;
            return Ok(replay);
        }
        let task=sqlx::query("SELECT count_number,legal_entity_id,warehouse_id,count_date,currency::text,status,version FROM inventory_count_tasks WHERE id=$1 FOR UPDATE").bind(id).fetch_one(&mut *tx).await?;
        check_task(&task, input.expected_version, "counted")?;
        sqlx::query("SELECT set_config('business.inventory_count_adjustment',$1,true)")
            .bind(id.to_string())
            .execute(&mut *tx)
            .await?;
        let lines=sqlx::query("SELECT id,sku_id,snapshot_on_hand_quantity,snapshot_reserved_quantity,snapshot_quarantined_quantity,snapshot_inventory_value,snapshot_average_unit_cost,actual_on_hand_quantity,surplus_unit_cost FROM inventory_count_lines WHERE inventory_count_id=$1 ORDER BY sku_id,id FOR UPDATE").bind(id).fetch_all(&mut *tx).await?;
        let mut variance_value_total = Decimal::ZERO;
        let mut variance_lines = 0usize;
        for line in &lines {
            let balance=sqlx::query("SELECT on_hand_quantity,reserved_quantity,quarantined_quantity,inventory_value,average_unit_cost FROM inventory_balances WHERE legal_entity_id=$1 AND warehouse_id=$2 AND sku_id=$3 FOR UPDATE").bind(task.get::<Uuid,_>("legal_entity_id")).bind(task.get::<Uuid,_>("warehouse_id")).bind(line.get::<Uuid,_>("sku_id")).fetch_one(&mut *tx).await?;
            ensure_snapshot(&balance, line)?;
            let actual = line
                .get::<Option<Decimal>, _>("actual_on_hand_quantity")
                .ok_or_else(|| {
                    DomainError::Invalid("count line is missing actual quantity".into())
                })?;
            let current: Decimal = balance.get("on_hand_quantity");
            let variance = actual - current;
            let mut movement = None;
            let variance_value = if variance == Decimal::ZERO {
                Decimal::ZERO
            } else {
                variance_lines += 1;
                let unit = balance
                    .get::<Option<Decimal>, _>("average_unit_cost")
                    .or(line.get::<Option<Decimal>, _>("surplus_unit_cost"))
                    .ok_or(DomainError::MissingInventoryCost)?;
                let value = if actual == Decimal::ZERO {
                    -balance.get::<Decimal, _>("inventory_value")
                } else {
                    money(unit * variance)
                };
                let movement_id = Uuid::new_v4();
                sqlx::query("INSERT INTO inventory_movements(id,legal_entity_id,warehouse_id,sku_id,movement_type,quantity,unit_cost,total_cost,currency,source_type,source_id,source_line_id,business_date,created_by_user_id,trace_id) VALUES($1,$2,$3,$4,'inventory_count_adjustment',$5,$6,$7,$8,'inventory_count',$9,$10,$11,$12,$13)").bind(movement_id).bind(task.get::<Uuid,_>("legal_entity_id")).bind(task.get::<Uuid,_>("warehouse_id")).bind(line.get::<Uuid,_>("sku_id")).bind(variance).bind(unit).bind(value).bind(task.get::<String,_>("currency")).bind(id).bind(line.get::<Uuid,_>("id")).bind(task.get::<NaiveDate,_>("count_date")).bind(actor).bind(trace_id).execute(&mut *tx).await?;
                movement = Some(movement_id);
                value
            };
            let new_value = if actual == Decimal::ZERO {
                Decimal::ZERO
            } else {
                money(balance.get::<Decimal, _>("inventory_value") + variance_value)
            };
            if new_value < Decimal::ZERO {
                return Err(DomainError::Invalid(
                    "count adjustment would make inventory value negative".into(),
                ));
            }
            if variance != Decimal::ZERO {
                let average = if actual == Decimal::ZERO {
                    None
                } else {
                    Some(money(new_value / actual))
                };
                sqlx::query("UPDATE inventory_balances SET on_hand_quantity=$4,inventory_value=$5,average_unit_cost=$6,last_movement_id=$7 WHERE legal_entity_id=$1 AND warehouse_id=$2 AND sku_id=$3").bind(task.get::<Uuid,_>("legal_entity_id")).bind(task.get::<Uuid,_>("warehouse_id")).bind(line.get::<Uuid,_>("sku_id")).bind(actual).bind(new_value).bind(average).bind(movement).execute(&mut *tx).await?;
            }
            sqlx::query("UPDATE inventory_count_lines SET variance_quantity=$2,variance_value=$3,inventory_movement_id=$4 WHERE id=$1").bind(line.get::<Uuid,_>("id")).bind(variance).bind(variance_value).bind(movement).execute(&mut *tx).await?;
            variance_value_total += variance_value;
        }
        let version = input.expected_version + 1;
        sqlx::query("UPDATE inventory_count_tasks SET status='posted',posted_by_user_id=$2,posted_at=now(),version=$3,trace_id=$4 WHERE id=$1").bind(id).bind(actor).bind(version).bind(trace_id).execute(&mut *tx).await?;
        count_event(&mut tx,id,"posted",version,actor,trace_id,json!({"varianceLineCount":variance_lines,"varianceValue":money(variance_value_total).to_string()})).await?;
        record(&mut tx,trace_id,actor,"INVENTORY_COUNT_POSTED","inventory_count_posted","inventory_count",id,json!({"version":version,"varianceLineCount":variance_lines,"varianceValue":money(variance_value_total).to_string()})).await?;
        let result = CommandResult {
            id,
            number: task.get("count_number"),
            status: "posted".into(),
            version,
            trace_id,
            idempotent_replay: false,
        };
        finish_idempotent(&mut tx, actor, "inventory_count:post", key, &result).await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn cancel(
        &self,
        actor: Uuid,
        trace_id: Uuid,
        id: Uuid,
        key: &str,
        input: &VersionCommand,
    ) -> Result<CommandResult, DomainError> {
        self.pre_authorize(actor, id, "inventory_opening:reverse")
            .await?;
        let hash = request_hash(input)?;
        let mut tx = self.store.pool().begin().await?;
        if let Some(mut replay) =
            begin_idempotent::<CommandResult>(&mut tx, actor, "inventory_count:cancel", key, &hash)
                .await?
        {
            replay.idempotent_replay = true;
            tx.commit().await?;
            return Ok(replay);
        }
        let task = sqlx::query(
            "SELECT count_number,status,version FROM inventory_count_tasks WHERE id=$1 FOR UPDATE",
        )
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
        if task.get::<i64, _>("version") != input.expected_version {
            return Err(DomainError::VersionConflict);
        }
        if !matches!(
            task.get::<String, _>("status").as_str(),
            "counting" | "counted"
        ) {
            return Err(DomainError::Invalid(
                "only active inventory counts can be cancelled".into(),
            ));
        }
        let version = input.expected_version + 1;
        sqlx::query("UPDATE inventory_count_tasks SET status='cancelled',cancelled_by_user_id=$2,cancelled_at=now(),version=$3,trace_id=$4 WHERE id=$1").bind(id).bind(actor).bind(version).bind(trace_id).execute(&mut *tx).await?;
        count_event(
            &mut tx,
            id,
            "cancelled",
            version,
            actor,
            trace_id,
            json!({}),
        )
        .await?;
        record(
            &mut tx,
            trace_id,
            actor,
            "INVENTORY_COUNT_CANCELLED",
            "inventory_count_cancelled",
            "inventory_count",
            id,
            json!({"version":version}),
        )
        .await?;
        let result = CommandResult {
            id,
            number: task.get("count_number"),
            status: "cancelled".into(),
            version,
            trace_id,
            idempotent_replay: false,
        };
        finish_idempotent(&mut tx, actor, "inventory_count:cancel", key, &result).await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn aging(
        &self,
        actor: Uuid,
        threshold_days: i32,
        limit: i64,
    ) -> Result<Value, DomainError> {
        let scope = authorize(
            &self.store,
            actor,
            "inventory:read",
            None,
            None,
            None,
            None,
            None,
        )
        .await?;
        let rows=sqlx::query("SELECT legal_entity_id,warehouse_id,sku_id,sku_code,sku_name,on_hand_quantity,reserved_quantity,quarantined_quantity,inventory_value,average_unit_cost,currency,last_issue_date,days_without_issue,aging_bucket FROM inventory_aging_current WHERE legal_entity_id=ANY($1) AND warehouse_id=ANY($2) AND days_without_issue>=$3 ORDER BY days_without_issue DESC,inventory_value DESC LIMIT $4").bind(scope.scopes.legal_entity_ids.into_iter().collect::<Vec<_>>()).bind(scope.scopes.warehouse_ids.into_iter().collect::<Vec<_>>()).bind(threshold_days.clamp(0,3650)).bind(limit.clamp(1,500)).fetch_all(self.store.pool()).await?;
        let items=rows.into_iter().map(|row|json!({"legalEntityId":row.get::<Uuid,_>("legal_entity_id"),"warehouseId":row.get::<Uuid,_>("warehouse_id"),"skuId":row.get::<Uuid,_>("sku_id"),"skuCode":row.get::<String,_>("sku_code"),"skuName":row.get::<String,_>("sku_name"),"onHandQuantity":row.get::<Decimal,_>("on_hand_quantity").to_string(),"reservedQuantity":row.get::<Decimal,_>("reserved_quantity").to_string(),"quarantinedQuantity":row.get::<Decimal,_>("quarantined_quantity").to_string(),"inventoryValue":row.get::<Decimal,_>("inventory_value").to_string(),"averageUnitCost":row.get::<Option<Decimal>,_>("average_unit_cost").map(|v|v.to_string()),"currency":row.get::<Option<String>,_>("currency"),"lastIssueDate":row.get::<Option<NaiveDate>,_>("last_issue_date"),"daysWithoutIssue":row.get::<i32,_>("days_without_issue"),"agingBucket":row.get::<String,_>("aging_bucket")})).collect::<Vec<_>>();
        Ok(
            json!({"items":items,"thresholdDays":threshold_days,"dataAsOf":Utc::now(),"warning":"库存库龄按最后一次出库日期计算，属于经营管理口径"}),
        )
    }

    pub async fn turnover(
        &self,
        actor: Uuid,
        period: &str,
        currency: &str,
    ) -> Result<Value, DomainError> {
        validate_currency(currency)?;
        let start = NaiveDate::parse_from_str(&format!("{period}-01"), "%Y-%m-%d")
            .map_err(|_| DomainError::Invalid("period must use YYYY-MM".into()))?;
        let end = if start.month() == 12 {
            NaiveDate::from_ymd_opt(start.year() + 1, 1, 1)
        } else {
            NaiveDate::from_ymd_opt(start.year(), start.month() + 1, 1)
        }
        .ok_or_else(|| DomainError::Invalid("invalid period".into()))?;
        let days = (end - start).num_days();
        let scope = authorize(
            &self.store,
            actor,
            "inventory:read",
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
        let cost:Decimal=sqlx::query_scalar("SELECT COALESCE(sum(l.total_cost),0) FROM shipment_lines l JOIN shipments s ON s.id=l.shipment_id WHERE s.status='confirmed' AND s.shipment_date>=$1 AND s.shipment_date<$2 AND s.currency=$3 AND s.legal_entity_id=ANY($4) AND s.warehouse_id=ANY($5)").bind(start).bind(end).bind(currency).bind(&entities).bind(&warehouses).fetch_one(self.store.pool()).await?;
        let value:Decimal=sqlx::query_scalar("SELECT COALESCE(sum(b.inventory_value),0) FROM inventory_balances b LEFT JOIN inventory_movements m ON m.id=b.last_movement_id WHERE b.legal_entity_id=ANY($1) AND b.warehouse_id=ANY($2) AND m.currency=$3").bind(&entities).bind(&warehouses).bind(currency).fetch_one(self.store.pool()).await?;
        let rate = if value == Decimal::ZERO {
            None
        } else {
            Some((cost / value).round_dp(8))
        };
        let days_value = rate
            .filter(|v| *v > Decimal::ZERO)
            .map(|v| (Decimal::from(days) / v).round_dp(2));
        Ok(
            json!({"managementPeriod":period,"currency":currency,"issuedProductCost":cost.to_string(),"endingInventoryValue":value.to_string(),"turnoverRate":rate.map(|v|v.to_string()),"turnoverDays":days_value.map(|v|v.to_string()),"dataAsOf":Utc::now(),"warning":"周转率使用当期销售出库成本/期末库存价值，属于经营管理近似口径"}),
        )
    }

    async fn pre_authorize(
        &self,
        actor: Uuid,
        id: Uuid,
        permission: &str,
    ) -> Result<(), DomainError> {
        let row = sqlx::query(
            "SELECT legal_entity_id,warehouse_id FROM inventory_count_tasks WHERE id=$1",
        )
        .bind(id)
        .fetch_optional(self.store.pool())
        .await?
        .ok_or(DomainError::NotFoundOrForbidden)?;
        authorize(
            &self.store,
            actor,
            permission,
            Some(row.get("legal_entity_id")),
            Some(row.get("warehouse_id")),
            None,
            None,
            None,
        )
        .await?;
        Ok(())
    }
}

fn validate_create(input: &CreateInventoryCount) -> Result<(), DomainError> {
    if input.sku_ids.is_empty() || input.sku_ids.len() > 500 {
        return Err(DomainError::Invalid(
            "inventory count requires 1-500 SKUs".into(),
        ));
    }
    if input
        .business_note
        .as_ref()
        .is_some_and(|note| note.len() > 1000)
    {
        return Err(DomainError::Invalid("businessNote is too long".into()));
    }
    let unique = input.sku_ids.iter().collect::<BTreeSet<_>>();
    if unique.len() != input.sku_ids.len() {
        return Err(DomainError::Invalid("duplicate inventory count SKU".into()));
    }
    Ok(())
}
fn check_task(row: &sqlx::postgres::PgRow, expected: i64, status: &str) -> Result<(), DomainError> {
    if row.get::<i64, _>("version") != expected {
        return Err(DomainError::VersionConflict);
    }
    if row.get::<String, _>("status") != status {
        return Err(DomainError::Invalid(format!(
            "inventory count must be {status}"
        )));
    }
    Ok(())
}
fn ensure_snapshot(
    balance: &sqlx::postgres::PgRow,
    line: &sqlx::postgres::PgRow,
) -> Result<(), DomainError> {
    for key in [
        "on_hand_quantity",
        "reserved_quantity",
        "quarantined_quantity",
        "inventory_value",
    ] {
        let snapshot_key = format!("snapshot_{key}");
        if balance.get::<Decimal, _>(key) != line.get::<Decimal, _>(snapshot_key.as_str()) {
            return Err(DomainError::Invalid(
                "inventory count snapshot changed while frozen".into(),
            ));
        }
    }
    Ok(())
}
async fn count_event(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: Uuid,
    event: &str,
    version: i64,
    actor: Uuid,
    trace: Uuid,
    payload: Value,
) -> Result<(), DomainError> {
    sqlx::query("INSERT INTO inventory_count_events(id,inventory_count_id,event_type,count_version,payload,actor_user_id,trace_id) VALUES($1,$2,$3,$4,$5,$6,$7)").bind(Uuid::new_v4()).bind(id).bind(event).bind(version).bind(payload).bind(actor).bind(trace).execute(&mut **tx).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn duplicate_skus_are_rejected() {
        let id = Uuid::new_v4();
        let input = CreateInventoryCount {
            legal_entity_id: Uuid::new_v4(),
            warehouse_id: Uuid::new_v4(),
            count_date: NaiveDate::from_ymd_opt(2026, 8, 22).unwrap(),
            currency: "CNY".into(),
            business_note: None,
            sku_ids: vec![id, id],
        };
        assert!(validate_create(&input).is_err());
    }
}
