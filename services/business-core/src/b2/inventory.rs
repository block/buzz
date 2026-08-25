use super::{
    common::{
        authorize, begin_idempotent, finish_idempotent, money, next_number, record, request_hash,
        validate_currency, DomainError,
    },
    model::{
        CommandResult, CreateInventoryOpening, DecimalString, InventoryBalanceView,
        InventoryMovementView, InventoryOpeningView, VersionCommand,
    },
};
use crate::store::PgStore;
use chrono::Duration;
use rust_decimal::Decimal;
use serde_json::{json, Value};
use sqlx::Row;
use std::collections::BTreeSet;
use uuid::Uuid;

#[derive(Clone)]
pub struct InventoryService {
    store: PgStore,
    opening_prefix: String,
    receivable_prefix: String,
}

impl InventoryService {
    pub fn new(store: PgStore, opening_prefix: String, receivable_prefix: String) -> Self {
        Self {
            store,
            opening_prefix,
            receivable_prefix,
        }
    }

    pub async fn create_opening(
        &self,
        actor: Uuid,
        trace_id: Uuid,
        key: &str,
        input: &CreateInventoryOpening,
    ) -> Result<CommandResult, DomainError> {
        validate_currency(&input.currency)?;
        if input.lines.is_empty() || input.lines.len() > 500 {
            return Err(DomainError::Invalid(
                "inventory opening requires 1-500 lines".into(),
            ));
        }
        let snapshot = authorize(
            &self.store,
            actor,
            "inventory_opening:create",
            Some(input.legal_entity_id),
            None,
            None,
            None,
            None,
        )
        .await?;
        if input
            .lines
            .iter()
            .any(|line| !snapshot.scopes.warehouse_ids.contains(&line.warehouse_id))
        {
            return Err(DomainError::NotFoundOrForbidden);
        }
        let hash = request_hash(input)?;
        let mut tx = self.store.pool().begin().await?;
        if let Some(mut replay) = begin_idempotent::<CommandResult>(
            &mut tx,
            actor,
            "inventory_opening:create",
            key,
            &hash,
        )
        .await?
        {
            replay.idempotent_replay = true;
            tx.commit().await?;
            return Ok(replay);
        }
        let mut stock_keys = BTreeSet::new();
        for line in &input.lines {
            if !stock_keys.insert((line.warehouse_id, line.sku_id)) {
                return Err(DomainError::Invalid("duplicate opening stock key".into()));
            }
            let quantity = line
                .quantity
                .positive("quantity")
                .map_err(DomainError::Invalid)?;
            let cost = line
                .unit_cost
                .non_negative("unitCost")
                .map_err(DomainError::Invalid)?;
            let row = sqlx::query("SELECT w.legal_entity_id,p.allow_zero_cost FROM business_warehouses w,business_skus s JOIN business_products p ON p.id=s.product_id WHERE w.id=$1 AND s.id=$2 AND w.status='active' AND s.status='active' AND p.status='active'")
                .bind(line.warehouse_id).bind(line.sku_id).fetch_optional(&mut *tx).await?.ok_or(DomainError::NotFoundOrForbidden)?;
            if row.get::<Uuid, _>("legal_entity_id") != input.legal_entity_id
                || (cost == Decimal::ZERO && !row.get::<bool, _>("allow_zero_cost"))
                || quantity.scale() > 6
            {
                return Err(DomainError::Invalid(
                    "opening line violates legal entity or zero-cost policy".into(),
                ));
            }
        }
        let id = Uuid::new_v4();
        let number = next_number(
            &mut tx,
            "opening",
            &self.opening_prefix,
            id,
            crate::numbering::NumberingContext::new(input.legal_entity_id, None),
        )
        .await?;
        sqlx::query("INSERT INTO inventory_opening_batches(id,batch_number,legal_entity_id,business_date,currency,created_by_user_id,trace_id) VALUES($1,$2,$3,$4,$5,$6,$7)")
            .bind(id).bind(&number).bind(input.legal_entity_id).bind(input.business_date).bind(&input.currency).bind(actor).bind(trace_id).execute(&mut *tx).await?;
        for (index, line) in input.lines.iter().enumerate() {
            let quantity = line.quantity.0;
            let cost = line.unit_cost.0;
            sqlx::query("INSERT INTO inventory_opening_lines(id,batch_id,line_number,warehouse_id,sku_id,quantity,unit_cost,total_cost) VALUES($1,$2,$3,$4,$5,$6,$7,$8)")
                .bind(Uuid::new_v4()).bind(id).bind((index + 1) as i32).bind(line.warehouse_id).bind(line.sku_id).bind(quantity).bind(cost).bind(money(quantity * cost)).execute(&mut *tx).await?;
        }
        record(
            &mut tx,
            trace_id,
            actor,
            "INVENTORY_OPENING_CREATED",
            "inventory_opening_created",
            "inventory_opening",
            id,
            json!({"batchNumber":number,"lineCount":input.lines.len()}),
        )
        .await?;
        let result = CommandResult {
            id,
            number,
            status: "draft".into(),
            version: 1,
            trace_id,
            idempotent_replay: false,
        };
        finish_idempotent(&mut tx, actor, "inventory_opening:create", key, &result).await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn post_opening(
        &self,
        actor: Uuid,
        trace_id: Uuid,
        batch_id: Uuid,
        key: &str,
        input: &VersionCommand,
    ) -> Result<CommandResult, DomainError> {
        let pre = sqlx::query("SELECT legal_entity_id FROM inventory_opening_batches WHERE id=$1")
            .bind(batch_id)
            .fetch_optional(self.store.pool())
            .await?
            .ok_or(DomainError::NotFoundOrForbidden)?;
        authorize(
            &self.store,
            actor,
            "inventory_opening:post",
            Some(pre.get("legal_entity_id")),
            None,
            None,
            None,
            None,
        )
        .await?;
        let hash = request_hash(input)?;
        let mut tx = self.store.pool().begin().await?;
        if let Some(mut replay) =
            begin_idempotent::<CommandResult>(&mut tx, actor, "inventory_opening:post", key, &hash)
                .await?
        {
            replay.idempotent_replay = true;
            tx.commit().await?;
            return Ok(replay);
        }
        let batch=sqlx::query("SELECT batch_number,legal_entity_id,business_date,currency,status,version FROM inventory_opening_batches WHERE id=$1 FOR UPDATE").bind(batch_id).fetch_one(&mut *tx).await?;
        if batch.get::<i64, _>("version") != input.expected_version {
            return Err(DomainError::VersionConflict);
        }
        if batch.get::<String, _>("status") != "draft" {
            return Err(DomainError::Invalid(
                "only draft opening batches can be posted".into(),
            ));
        }
        let lines=sqlx::query("SELECT id,warehouse_id,sku_id,quantity,unit_cost,total_cost FROM inventory_opening_lines WHERE batch_id=$1 ORDER BY warehouse_id,sku_id,id").bind(batch_id).fetch_all(&mut *tx).await?;
        for line in &lines {
            sqlx::query("INSERT INTO inventory_balances(legal_entity_id,warehouse_id,sku_id) VALUES($1,$2,$3) ON CONFLICT DO NOTHING").bind(batch.get::<Uuid,_>("legal_entity_id")).bind(line.get::<Uuid,_>("warehouse_id")).bind(line.get::<Uuid,_>("sku_id")).execute(&mut *tx).await?;
            let balance=sqlx::query("SELECT on_hand_quantity,inventory_value FROM inventory_balances WHERE legal_entity_id=$1 AND warehouse_id=$2 AND sku_id=$3 FOR UPDATE").bind(batch.get::<Uuid,_>("legal_entity_id")).bind(line.get::<Uuid,_>("warehouse_id")).bind(line.get::<Uuid,_>("sku_id")).fetch_one(&mut *tx).await?;
            let quantity: Decimal = line.get("quantity");
            let total: Decimal = line.get("total_cost");
            let new_quantity = balance.get::<Decimal, _>("on_hand_quantity") + quantity;
            let new_value = money(balance.get::<Decimal, _>("inventory_value") + total);
            let movement = Uuid::new_v4();
            sqlx::query("INSERT INTO inventory_movements(id,legal_entity_id,warehouse_id,sku_id,movement_type,quantity,unit_cost,total_cost,currency,source_type,source_id,source_line_id,business_date,created_by_user_id,trace_id) VALUES($1,$2,$3,$4,'opening_balance',$5,$6,$7,$8,'inventory_opening',$9,$10,$11,$12,$13)").bind(movement).bind(batch.get::<Uuid,_>("legal_entity_id")).bind(line.get::<Uuid,_>("warehouse_id")).bind(line.get::<Uuid,_>("sku_id")).bind(quantity).bind(line.get::<Decimal,_>("unit_cost")).bind(total).bind(batch.get::<String,_>("currency")).bind(batch_id).bind(line.get::<Uuid,_>("id")).bind(batch.get::<chrono::NaiveDate,_>("business_date")).bind(actor).bind(trace_id).execute(&mut *tx).await?;
            sqlx::query("UPDATE inventory_balances SET on_hand_quantity=$4,inventory_value=$5,average_unit_cost=$6,last_movement_id=$7 WHERE legal_entity_id=$1 AND warehouse_id=$2 AND sku_id=$3").bind(batch.get::<Uuid,_>("legal_entity_id")).bind(line.get::<Uuid,_>("warehouse_id")).bind(line.get::<Uuid,_>("sku_id")).bind(new_quantity).bind(new_value).bind(money(new_value/new_quantity)).bind(movement).execute(&mut *tx).await?;
            record(
                &mut tx,
                trace_id,
                actor,
                "INVENTORY_MOVEMENT_POSTED",
                "inventory_movement_posted",
                "inventory_movement",
                movement,
                json!({"movementType":"opening_balance","quantity":quantity.to_string()}),
            )
            .await?;
        }
        sqlx::query("UPDATE inventory_opening_batches SET status='posted',posted_by_user_id=$2,posted_at=now(),trace_id=$3 WHERE id=$1").bind(batch_id).bind(actor).bind(trace_id).execute(&mut *tx).await?;
        let version = input.expected_version + 1;
        record(
            &mut tx,
            trace_id,
            actor,
            "INVENTORY_OPENING_POSTED",
            "inventory_opening_posted",
            "inventory_opening",
            batch_id,
            json!({"version":version,"lineCount":lines.len()}),
        )
        .await?;
        let result = CommandResult {
            id: batch_id,
            number: batch.get("batch_number"),
            status: "posted".into(),
            version,
            trace_id,
            idempotent_replay: false,
        };
        finish_idempotent(&mut tx, actor, "inventory_opening:post", key, &result).await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn reverse_opening(
        &self,
        actor: Uuid,
        trace_id: Uuid,
        batch_id: Uuid,
        key: &str,
        input: &VersionCommand,
    ) -> Result<CommandResult, DomainError> {
        let pre = sqlx::query("SELECT legal_entity_id FROM inventory_opening_batches WHERE id=$1")
            .bind(batch_id)
            .fetch_optional(self.store.pool())
            .await?
            .ok_or(DomainError::NotFoundOrForbidden)?;
        authorize(
            &self.store,
            actor,
            "inventory_opening:reverse",
            Some(pre.get("legal_entity_id")),
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
            "inventory_opening:reverse",
            key,
            &hash,
        )
        .await?
        {
            replay.idempotent_replay = true;
            tx.commit().await?;
            return Ok(replay);
        }
        let batch=sqlx::query("SELECT batch_number,legal_entity_id,business_date,currency,status,version FROM inventory_opening_batches WHERE id=$1 FOR UPDATE").bind(batch_id).fetch_one(&mut *tx).await?;
        if batch.get::<i64, _>("version") != input.expected_version {
            return Err(DomainError::VersionConflict);
        }
        if batch.get::<String, _>("status") != "posted" {
            return Err(DomainError::Invalid(
                "only posted openings can be reversed".into(),
            ));
        }
        let movements=sqlx::query("SELECT m.id,m.warehouse_id,m.sku_id,m.quantity,m.unit_cost,m.total_cost,m.posted_at,l.id line_id FROM inventory_movements m JOIN inventory_opening_lines l ON l.id=m.source_line_id WHERE m.source_id=$1 AND m.movement_type='opening_balance' ORDER BY m.warehouse_id,m.sku_id,m.id").bind(batch_id).fetch_all(&mut *tx).await?;
        for movement in &movements {
            let later:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM inventory_movements WHERE legal_entity_id=$1 AND warehouse_id=$2 AND sku_id=$3 AND posted_at>$4)").bind(batch.get::<Uuid,_>("legal_entity_id")).bind(movement.get::<Uuid,_>("warehouse_id")).bind(movement.get::<Uuid,_>("sku_id")).bind(movement.get::<chrono::DateTime<chrono::Utc>,_>("posted_at")).fetch_one(&mut *tx).await?;
            if later {
                return Err(DomainError::Invalid(
                    "opening has subsequent inventory movements".into(),
                ));
            }
            let balance=sqlx::query("SELECT on_hand_quantity,reserved_quantity,inventory_value FROM inventory_balances WHERE legal_entity_id=$1 AND warehouse_id=$2 AND sku_id=$3 FOR UPDATE").bind(batch.get::<Uuid,_>("legal_entity_id")).bind(movement.get::<Uuid,_>("warehouse_id")).bind(movement.get::<Uuid,_>("sku_id")).fetch_one(&mut *tx).await?;
            let new_qty = balance.get::<Decimal, _>("on_hand_quantity")
                - movement.get::<Decimal, _>("quantity");
            if new_qty < balance.get::<Decimal, _>("reserved_quantity") {
                return Err(DomainError::Invalid(
                    "opening reversal would invalidate reservations".into(),
                ));
            }
            let new_value = money(
                balance.get::<Decimal, _>("inventory_value")
                    - movement.get::<Decimal, _>("total_cost"),
            );
            let reversal = Uuid::new_v4();
            sqlx::query("INSERT INTO inventory_movements(id,legal_entity_id,warehouse_id,sku_id,movement_type,quantity,unit_cost,total_cost,currency,source_type,source_id,source_line_id,business_date,reversal_of_movement_id,created_by_user_id,trace_id) VALUES($1,$2,$3,$4,'opening_balance_reversal',$5,$6,$7,$8,'inventory_opening_reversal',$9,$10,$11,$12,$13,$14)").bind(reversal).bind(batch.get::<Uuid,_>("legal_entity_id")).bind(movement.get::<Uuid,_>("warehouse_id")).bind(movement.get::<Uuid,_>("sku_id")).bind(-movement.get::<Decimal,_>("quantity")).bind(movement.get::<Decimal,_>("unit_cost")).bind(-movement.get::<Decimal,_>("total_cost")).bind(batch.get::<String,_>("currency")).bind(batch_id).bind(movement.get::<Uuid,_>("line_id")).bind(batch.get::<chrono::NaiveDate,_>("business_date")).bind(movement.get::<Uuid,_>("id")).bind(actor).bind(trace_id).execute(&mut *tx).await?;
            let average = if new_qty == Decimal::ZERO {
                None
            } else {
                Some(money(new_value / new_qty))
            };
            sqlx::query("UPDATE inventory_balances SET on_hand_quantity=$4,inventory_value=$5,average_unit_cost=$6,last_movement_id=$7 WHERE legal_entity_id=$1 AND warehouse_id=$2 AND sku_id=$3").bind(batch.get::<Uuid,_>("legal_entity_id")).bind(movement.get::<Uuid,_>("warehouse_id")).bind(movement.get::<Uuid,_>("sku_id")).bind(new_qty).bind(new_value).bind(average).bind(reversal).execute(&mut *tx).await?;
        }
        sqlx::query("UPDATE inventory_opening_batches SET status='reversed',reversed_at=now(),trace_id=$2 WHERE id=$1").bind(batch_id).bind(trace_id).execute(&mut *tx).await?;
        let version = input.expected_version + 1;
        record(
            &mut tx,
            trace_id,
            actor,
            "INVENTORY_OPENING_REVERSED",
            "inventory_opening_reversed",
            "inventory_opening",
            batch_id,
            json!({"version":version}),
        )
        .await?;
        let result = CommandResult {
            id: batch_id,
            number: batch.get("batch_number"),
            status: "reversed".into(),
            version,
            trace_id,
            idempotent_replay: false,
        };
        finish_idempotent(&mut tx, actor, "inventory_opening:reverse", key, &result).await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn confirm_shipment(
        &self,
        actor: Uuid,
        trace_id: Uuid,
        shipment_id: Uuid,
        key: &str,
        input: &VersionCommand,
    ) -> Result<CommandResult, DomainError> {
        let pre = sqlx::query(
            "SELECT legal_entity_id,warehouse_id,customer_id FROM shipments WHERE id=$1",
        )
        .bind(shipment_id)
        .fetch_optional(self.store.pool())
        .await?
        .ok_or(DomainError::NotFoundOrForbidden)?;
        authorize(
            &self.store,
            actor,
            "shipment:confirm",
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
            begin_idempotent::<CommandResult>(&mut tx, actor, "shipment:confirm", key, &hash)
                .await?
        {
            replay.idempotent_replay = true;
            tx.commit().await?;
            return Ok(replay);
        }
        let shipment=sqlx::query("SELECT shipment_number,sales_order_id,legal_entity_id,warehouse_id,customer_id,shipment_date,sales_amount,currency,status,version FROM shipments WHERE id=$1 FOR UPDATE").bind(shipment_id).fetch_one(&mut *tx).await?;
        if shipment.get::<i64, _>("version") != input.expected_version {
            return Err(DomainError::VersionConflict);
        }
        if shipment.get::<String, _>("status") != "draft" {
            return Err(DomainError::Invalid(
                "only draft shipments can be confirmed".into(),
            ));
        }
        let order=sqlx::query("SELECT hold_status,lifecycle_status,business_unit_id,payment_terms_days,payment_terms_snapshot FROM sales_orders WHERE id=$1 FOR UPDATE").bind(shipment.get::<Uuid,_>("sales_order_id")).fetch_one(&mut *tx).await?;
        if order.get::<String, _>("hold_status") != "none" {
            return Err(DomainError::OrderOnHold);
        }
        if order.get::<String, _>("lifecycle_status") != "confirmed" {
            return Err(DomainError::Invalid(
                "sales order is not fulfillable".into(),
            ));
        }
        let lines=sqlx::query("SELECT sl.id,sl.sales_order_line_id,sl.sku_id,sl.quantity,sl.sales_amount,sl.inventory_reservation_id,r.reserved_quantity,r.consumed_quantity,r.released_quantity FROM shipment_lines sl JOIN inventory_reservations r ON r.id=sl.inventory_reservation_id WHERE sl.shipment_id=$1 ORDER BY sl.sku_id,sl.id FOR UPDATE OF r").bind(shipment_id).fetch_all(&mut *tx).await?;
        let mut total_cost = Decimal::ZERO;
        for line in &lines {
            let quantity: Decimal = line.get("quantity");
            let reservation_open = line.get::<Decimal, _>("reserved_quantity")
                - line.get::<Decimal, _>("consumed_quantity")
                - line.get::<Decimal, _>("released_quantity");
            if quantity > reservation_open {
                return Err(DomainError::InsufficientStock(
                    json!({"reservationId":line.get::<Uuid,_>("inventory_reservation_id")}),
                ));
            }
            let balance=sqlx::query("SELECT on_hand_quantity,reserved_quantity,inventory_value,average_unit_cost FROM inventory_balances WHERE legal_entity_id=$1 AND warehouse_id=$2 AND sku_id=$3 FOR UPDATE").bind(shipment.get::<Uuid,_>("legal_entity_id")).bind(shipment.get::<Uuid,_>("warehouse_id")).bind(line.get::<Uuid,_>("sku_id")).fetch_one(&mut *tx).await?;
            let average = balance
                .get::<Option<Decimal>, _>("average_unit_cost")
                .ok_or(DomainError::MissingInventoryCost)?;
            if balance.get::<Decimal, _>("on_hand_quantity") < quantity
                || balance.get::<Decimal, _>("reserved_quantity") < quantity
            {
                return Err(DomainError::InsufficientStock(
                    json!({"skuId":line.get::<Uuid,_>("sku_id")}),
                ));
            }
            let cost = money(average * quantity);
            total_cost += cost;
            let movement = Uuid::new_v4();
            sqlx::query("INSERT INTO inventory_movements(id,legal_entity_id,warehouse_id,sku_id,movement_type,quantity,unit_cost,total_cost,currency,source_type,source_id,source_line_id,business_date,created_by_user_id,trace_id) VALUES($1,$2,$3,$4,'sales_shipment',$5,$6,$7,$8,'shipment',$9,$10,$11,$12,$13)").bind(movement).bind(shipment.get::<Uuid,_>("legal_entity_id")).bind(shipment.get::<Uuid,_>("warehouse_id")).bind(line.get::<Uuid,_>("sku_id")).bind(-quantity).bind(average).bind(-cost).bind(shipment.get::<String,_>("currency")).bind(shipment_id).bind(line.get::<Uuid,_>("id")).bind(shipment.get::<chrono::NaiveDate,_>("shipment_date")).bind(actor).bind(trace_id).execute(&mut *tx).await?;
            let new_qty = balance.get::<Decimal, _>("on_hand_quantity") - quantity;
            let new_value = if new_qty == Decimal::ZERO {
                Decimal::ZERO
            } else {
                money(balance.get::<Decimal, _>("inventory_value") - cost)
            };
            let new_average = if new_qty == Decimal::ZERO {
                None
            } else {
                Some(average)
            };
            sqlx::query("UPDATE inventory_balances SET on_hand_quantity=$4,reserved_quantity=reserved_quantity-$5,inventory_value=$6,average_unit_cost=$7,last_movement_id=$8 WHERE legal_entity_id=$1 AND warehouse_id=$2 AND sku_id=$3").bind(shipment.get::<Uuid,_>("legal_entity_id")).bind(shipment.get::<Uuid,_>("warehouse_id")).bind(line.get::<Uuid,_>("sku_id")).bind(new_qty).bind(quantity).bind(new_value).bind(new_average).bind(movement).execute(&mut *tx).await?;
            let consumed = line.get::<Decimal, _>("consumed_quantity") + quantity;
            let status = if consumed + line.get::<Decimal, _>("released_quantity")
                == line.get::<Decimal, _>("reserved_quantity")
            {
                "consumed"
            } else {
                "partially_consumed"
            };
            sqlx::query(
                "UPDATE inventory_reservations SET consumed_quantity=$2,status=$3 WHERE id=$1",
            )
            .bind(line.get::<Uuid, _>("inventory_reservation_id"))
            .bind(consumed)
            .bind(status)
            .execute(&mut *tx)
            .await?;
            sqlx::query("INSERT INTO inventory_reservation_events(id,reservation_id,event_type,quantity,actor_user_id,trace_id) VALUES($1,$2,'consumed',$3,$4,$5)").bind(Uuid::new_v4()).bind(line.get::<Uuid,_>("inventory_reservation_id")).bind(quantity).bind(actor).bind(trace_id).execute(&mut *tx).await?;
            sqlx::query("UPDATE sales_order_lines SET shipped_quantity=shipped_quantity+$2,reserved_quantity=reserved_quantity-$2 WHERE id=$1").bind(line.get::<Uuid,_>("sales_order_line_id")).bind(quantity).execute(&mut *tx).await?;
            sqlx::query("UPDATE shipment_lines SET unit_cost=$2,total_cost=$3,cost_snapshot_at=now() WHERE id=$1").bind(line.get::<Uuid,_>("id")).bind(average).bind(cost).execute(&mut *tx).await?;
            record(&mut tx,trace_id,actor,"INVENTORY_MOVEMENT_POSTED","inventory_movement_posted","inventory_movement",movement,json!({"movementType":"sales_shipment","quantity":(-quantity).to_string(),"totalCost":(-cost).to_string()})).await?;
        }
        let totals=sqlx::query("SELECT sum(ordered_quantity) ordered,sum(shipped_quantity) shipped,sum(cancelled_quantity) cancelled FROM sales_order_lines WHERE sales_order_id=$1").bind(shipment.get::<Uuid,_>("sales_order_id")).fetch_one(&mut *tx).await?;
        let complete: bool = totals.get::<Decimal, _>("ordered")
            == totals.get::<Decimal, _>("shipped") + totals.get::<Decimal, _>("cancelled");
        sqlx::query("UPDATE sales_orders SET fulfillment_status=CASE WHEN $2 THEN 'shipped' ELSE 'partially_shipped' END,lifecycle_status=CASE WHEN $2 THEN 'completed' ELSE lifecycle_status END,completed_at=CASE WHEN $2 THEN now() ELSE completed_at END,updated_by_user_id=$3,trace_id=$4 WHERE id=$1").bind(shipment.get::<Uuid,_>("sales_order_id")).bind(complete).bind(actor).bind(trace_id).execute(&mut *tx).await?;
        let receivable_id = Uuid::new_v4();
        let receivable_number = next_number(
            &mut tx,
            "receivable",
            &self.receivable_prefix,
            receivable_id,
            crate::numbering::NumberingContext::new(
                shipment.get("legal_entity_id"),
                Some(order.get("business_unit_id")),
            ),
        )
        .await?;
        let terms: i32 = order.get("payment_terms_days");
        let due = shipment.get::<chrono::NaiveDate, _>("shipment_date")
            + Duration::days(i64::from(terms));
        let sales_amount: Decimal = shipment.get("sales_amount");
        sqlx::query("INSERT INTO trade_receivables(id,receivable_number,legal_entity_id,customer_id,sales_order_id,shipment_id,currency,original_amount,open_amount,recognized_at,due_date,payment_terms_days,payment_terms_snapshot,trace_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$8,now(),$9,$10,$11,$12)").bind(receivable_id).bind(&receivable_number).bind(shipment.get::<Uuid,_>("legal_entity_id")).bind(shipment.get::<Uuid,_>("customer_id")).bind(shipment.get::<Uuid,_>("sales_order_id")).bind(shipment_id).bind(shipment.get::<String,_>("currency")).bind(sales_amount).bind(due).bind(terms).bind(order.get::<Value,_>("payment_terms_snapshot")).bind(trace_id).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO trade_receivable_events(id,receivable_id,event_type,amount,payload,actor_user_id,trace_id) VALUES($1,$2,'created',$3,$4,$5,$6)").bind(Uuid::new_v4()).bind(receivable_id).bind(sales_amount).bind(json!({"shipmentId":shipment_id,"dueDate":due})).bind(actor).bind(trace_id).execute(&mut *tx).await?;
        sqlx::query("UPDATE shipments SET status='confirmed',cost_amount=$2,confirmed_by_user_id=$3,confirmed_at=now(),trace_id=$4 WHERE id=$1").bind(shipment_id).bind(money(total_cost)).bind(actor).bind(trace_id).execute(&mut *tx).await?;
        let version = input.expected_version + 1;
        sqlx::query("INSERT INTO shipment_events(id,shipment_id,event_type,shipment_version,payload,actor_user_id,trace_id) VALUES($1,$2,'confirmed',$3,$4,$5,$6)").bind(Uuid::new_v4()).bind(shipment_id).bind(version).bind(json!({"costAmount":money(total_cost).to_string(),"receivableId":receivable_id})).bind(actor).bind(trace_id).execute(&mut *tx).await?;
        record(
            &mut tx,
            trace_id,
            actor,
            "SHIPMENT_CONFIRMED",
            "shipment_confirmed",
            "shipment",
            shipment_id,
            json!({"version":version,"costAmount":money(total_cost).to_string()}),
        )
        .await?;
        record(
            &mut tx,
            trace_id,
            actor,
            "RECEIVABLE_CREATED",
            "trade_receivable_created",
            "trade_receivable",
            receivable_id,
            json!({"receivableNumber":receivable_number,"amount":sales_amount.to_string()}),
        )
        .await?;
        let result = CommandResult {
            id: shipment_id,
            number: shipment.get("shipment_number"),
            status: "confirmed".into(),
            version,
            trace_id,
            idempotent_replay: false,
        };
        finish_idempotent(&mut tx, actor, "shipment:confirm", key, &result).await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn reverse_shipment(
        &self,
        actor: Uuid,
        trace_id: Uuid,
        shipment_id: Uuid,
        key: &str,
        input: &VersionCommand,
    ) -> Result<CommandResult, DomainError> {
        let pre = sqlx::query(
            "SELECT legal_entity_id,warehouse_id,customer_id FROM shipments WHERE id=$1",
        )
        .bind(shipment_id)
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
            begin_idempotent::<CommandResult>(&mut tx, actor, "shipment:reverse", key, &hash)
                .await?
        {
            replay.idempotent_replay = true;
            tx.commit().await?;
            return Ok(replay);
        }
        let shipment=sqlx::query("SELECT shipment_number,sales_order_id,legal_entity_id,warehouse_id,shipment_date,currency,status,version FROM shipments WHERE id=$1 FOR UPDATE").bind(shipment_id).fetch_one(&mut *tx).await?;
        if shipment.get::<i64, _>("version") != input.expected_version {
            return Err(DomainError::VersionConflict);
        }
        if shipment.get::<String, _>("status") != "confirmed" {
            return Err(DomainError::Invalid(
                "only confirmed shipments can be reversed".into(),
            ));
        }
        let receivable=sqlx::query("SELECT id,original_amount,settled_amount,status FROM trade_receivables WHERE shipment_id=$1 FOR UPDATE").bind(shipment_id).fetch_one(&mut *tx).await?;
        if receivable.get::<Decimal, _>("settled_amount") > Decimal::ZERO {
            return Err(DomainError::ReceivableAlreadySettled);
        }
        let lines=sqlx::query("SELECT sl.id,sl.sales_order_line_id,sl.sku_id,sl.quantity,sl.unit_cost,sl.total_cost,sl.inventory_reservation_id,r.consumed_quantity,r.released_quantity,r.reserved_quantity,m.id movement_id FROM shipment_lines sl JOIN inventory_reservations r ON r.id=sl.inventory_reservation_id JOIN inventory_movements m ON m.source_line_id=sl.id AND m.movement_type='sales_shipment' WHERE sl.shipment_id=$1 ORDER BY sl.sku_id,sl.id FOR UPDATE OF r").bind(shipment_id).fetch_all(&mut *tx).await?;
        for line in &lines {
            let quantity: Decimal = line.get("quantity");
            let cost: Decimal = line
                .get::<Option<Decimal>, _>("total_cost")
                .ok_or(DomainError::MissingInventoryCost)?;
            let unit_cost: Decimal = line
                .get::<Option<Decimal>, _>("unit_cost")
                .ok_or(DomainError::MissingInventoryCost)?;
            let balance=sqlx::query("SELECT on_hand_quantity,reserved_quantity,inventory_value FROM inventory_balances WHERE legal_entity_id=$1 AND warehouse_id=$2 AND sku_id=$3 FOR UPDATE").bind(shipment.get::<Uuid,_>("legal_entity_id")).bind(shipment.get::<Uuid,_>("warehouse_id")).bind(line.get::<Uuid,_>("sku_id")).fetch_one(&mut *tx).await?;
            let new_qty = balance.get::<Decimal, _>("on_hand_quantity") + quantity;
            let new_value = money(balance.get::<Decimal, _>("inventory_value") + cost);
            let reversal = Uuid::new_v4();
            sqlx::query("INSERT INTO inventory_movements(id,legal_entity_id,warehouse_id,sku_id,movement_type,quantity,unit_cost,total_cost,currency,source_type,source_id,source_line_id,business_date,reversal_of_movement_id,created_by_user_id,trace_id) VALUES($1,$2,$3,$4,'sales_shipment_reversal',$5,$6,$7,$8,'shipment_reversal',$9,$10,$11,$12,$13,$14)").bind(reversal).bind(shipment.get::<Uuid,_>("legal_entity_id")).bind(shipment.get::<Uuid,_>("warehouse_id")).bind(line.get::<Uuid,_>("sku_id")).bind(quantity).bind(unit_cost).bind(cost).bind(shipment.get::<String,_>("currency")).bind(shipment_id).bind(line.get::<Uuid,_>("id")).bind(shipment.get::<chrono::NaiveDate,_>("shipment_date")).bind(line.get::<Uuid,_>("movement_id")).bind(actor).bind(trace_id).execute(&mut *tx).await?;
            sqlx::query("UPDATE inventory_balances SET on_hand_quantity=$4,reserved_quantity=reserved_quantity+$5,inventory_value=$6,average_unit_cost=$7,last_movement_id=$8 WHERE legal_entity_id=$1 AND warehouse_id=$2 AND sku_id=$3").bind(shipment.get::<Uuid,_>("legal_entity_id")).bind(shipment.get::<Uuid,_>("warehouse_id")).bind(line.get::<Uuid,_>("sku_id")).bind(new_qty).bind(quantity).bind(new_value).bind(money(new_value/new_qty)).bind(reversal).execute(&mut *tx).await?;
            let consumed = line.get::<Decimal, _>("consumed_quantity") - quantity;
            let status = if consumed == Decimal::ZERO {
                "active"
            } else {
                "partially_consumed"
            };
            sqlx::query(
                "UPDATE inventory_reservations SET consumed_quantity=$2,status=$3 WHERE id=$1",
            )
            .bind(line.get::<Uuid, _>("inventory_reservation_id"))
            .bind(consumed)
            .bind(status)
            .execute(&mut *tx)
            .await?;
            sqlx::query("INSERT INTO inventory_reservation_events(id,reservation_id,event_type,quantity,payload,actor_user_id,trace_id) VALUES($1,$2,'consumption_reversed',$3,$4,$5,$6)").bind(Uuid::new_v4()).bind(line.get::<Uuid,_>("inventory_reservation_id")).bind(quantity).bind(json!({"shipmentId":shipment_id})).bind(actor).bind(trace_id).execute(&mut *tx).await?;
            sqlx::query("UPDATE sales_order_lines SET shipped_quantity=shipped_quantity-$2,reserved_quantity=reserved_quantity+$2 WHERE id=$1").bind(line.get::<Uuid,_>("sales_order_line_id")).bind(quantity).execute(&mut *tx).await?;
        }
        sqlx::query("UPDATE trade_receivables SET status='reversed',trace_id=$2 WHERE id=$1")
            .bind(receivable.get::<Uuid, _>("id"))
            .bind(trace_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO trade_receivable_events(id,receivable_id,event_type,amount,payload,actor_user_id,trace_id) VALUES($1,$2,'reversed',$3,$4,$5,$6)").bind(Uuid::new_v4()).bind(receivable.get::<Uuid,_>("id")).bind(receivable.get::<Decimal,_>("original_amount")).bind(json!({"shipmentId":shipment_id})).bind(actor).bind(trace_id).execute(&mut *tx).await?;
        sqlx::query("UPDATE sales_orders SET lifecycle_status='confirmed',fulfillment_status=CASE WHEN EXISTS(SELECT 1 FROM sales_order_lines WHERE sales_order_id=$1 AND shipped_quantity>0) THEN 'partially_shipped' ELSE 'reserved' END,completed_at=NULL,trace_id=$2 WHERE id=$1").bind(shipment.get::<Uuid,_>("sales_order_id")).bind(trace_id).execute(&mut *tx).await?;
        sqlx::query(
            "UPDATE shipments SET status='reversed',reversed_at=now(),trace_id=$2 WHERE id=$1",
        )
        .bind(shipment_id)
        .bind(trace_id)
        .execute(&mut *tx)
        .await?;
        let version = input.expected_version + 1;
        sqlx::query("INSERT INTO shipment_events(id,shipment_id,event_type,shipment_version,actor_user_id,trace_id) VALUES($1,$2,'reversed',$3,$4,$5)").bind(Uuid::new_v4()).bind(shipment_id).bind(version).bind(actor).bind(trace_id).execute(&mut *tx).await?;
        record(
            &mut tx,
            trace_id,
            actor,
            "SHIPMENT_REVERSED",
            "shipment_reversed",
            "shipment",
            shipment_id,
            json!({"version":version}),
        )
        .await?;
        record(
            &mut tx,
            trace_id,
            actor,
            "RECEIVABLE_REVERSED",
            "trade_receivable_reversed",
            "trade_receivable",
            receivable.get("id"),
            json!({"shipmentId":shipment_id}),
        )
        .await?;
        let result = CommandResult {
            id: shipment_id,
            number: shipment.get("shipment_number"),
            status: "reversed".into(),
            version,
            trace_id,
            idempotent_replay: false,
        };
        finish_idempotent(&mut tx, actor, "shipment:reverse", key, &result).await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn balances(
        &self,
        actor: Uuid,
        sku: Option<Uuid>,
        limit: i64,
    ) -> Result<Vec<InventoryBalanceView>, DomainError> {
        let snapshot = authorize(
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
        let rows=sqlx::query("SELECT legal_entity_id,warehouse_id,sku_id,on_hand_quantity,reserved_quantity,quarantined_quantity,on_hand_quantity-reserved_quantity-quarantined_quantity available_quantity,inventory_value,average_unit_cost,last_movement_id,updated_at,version FROM inventory_balances WHERE legal_entity_id=ANY($1) AND warehouse_id=ANY($2) AND ($3::uuid IS NULL OR sku_id=$3) ORDER BY updated_at DESC LIMIT $4").bind(snapshot.scopes.legal_entity_ids.into_iter().collect::<Vec<_>>()).bind(snapshot.scopes.warehouse_ids.into_iter().collect::<Vec<_>>()).bind(sku).bind(limit.clamp(1,500)).fetch_all(self.store.pool()).await?;
        Ok(rows
            .into_iter()
            .map(|row| InventoryBalanceView {
                legal_entity_id: row.get("legal_entity_id"),
                warehouse_id: row.get("warehouse_id"),
                sku_id: row.get("sku_id"),
                on_hand_quantity: DecimalString(row.get("on_hand_quantity")),
                reserved_quantity: DecimalString(row.get("reserved_quantity")),
                quarantined_quantity: DecimalString(row.get("quarantined_quantity")),
                available_quantity: DecimalString(row.get("available_quantity")),
                inventory_value: DecimalString(row.get("inventory_value")),
                average_unit_cost: row
                    .get::<Option<Decimal>, _>("average_unit_cost")
                    .map(DecimalString),
                last_movement_id: row.get("last_movement_id"),
                updated_at: row.get("updated_at"),
                version: row.get("version"),
            })
            .collect())
    }

    pub async fn openings(
        &self,
        actor: Uuid,
        limit: i64,
    ) -> Result<Vec<InventoryOpeningView>, DomainError> {
        let snapshot = authorize(
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
        let rows = sqlx::query_as::<_, InventoryOpeningView>("SELECT id,batch_number,legal_entity_id,business_date,currency::text,status,posted_at,reversed_at,version FROM inventory_opening_batches WHERE legal_entity_id=ANY($1) ORDER BY business_date DESC,id DESC LIMIT $2")
            .bind(snapshot.scopes.legal_entity_ids.into_iter().collect::<Vec<_>>())
            .bind(limit.clamp(1, 500))
            .fetch_all(self.store.pool())
            .await?;
        Ok(rows)
    }

    pub async fn movements(
        &self,
        actor: Uuid,
        sku_id: Option<Uuid>,
        limit: i64,
    ) -> Result<Vec<InventoryMovementView>, DomainError> {
        let snapshot = authorize(
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
        let rows = sqlx::query_as::<_, InventoryMovementView>("SELECT id,legal_entity_id,warehouse_id,sku_id,movement_type,quantity,unit_cost,total_cost,business_date,posted_at FROM inventory_movements WHERE legal_entity_id=ANY($1) AND warehouse_id=ANY($2) AND ($3::uuid IS NULL OR sku_id=$3) ORDER BY posted_at DESC,id DESC LIMIT $4")
            .bind(snapshot.scopes.legal_entity_ids.into_iter().collect::<Vec<_>>())
            .bind(snapshot.scopes.warehouse_ids.into_iter().collect::<Vec<_>>())
            .bind(sku_id)
            .bind(limit.clamp(1, 500))
            .fetch_all(self.store.pool())
            .await?;
        Ok(rows)
    }

    pub async fn reconcile(&self, actor: Uuid) -> Result<Vec<Value>, DomainError> {
        authorize(
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
        let rows=sqlx::query("SELECT to_jsonb(r) value FROM inventory_balance_reconciliation r WHERE on_hand_difference<>0 OR reserved_difference<>0 OR value_difference<>0").fetch_all(self.store.pool()).await?;
        Ok(rows.into_iter().map(|row| row.get("value")).collect())
    }
}
