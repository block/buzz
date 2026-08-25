use super::{
    common::authorize,
    model::{
        CommandResult, CreateGoodsReceipt, GoodsReceiptConfirmationLine,
        GoodsReceiptConfirmationPreview, GoodsReceiptDraftOptionLine, GoodsReceiptDraftOptions,
        GoodsReceiptView, VersionCommand,
    },
    purchasing::PurchasingService,
};
use crate::{
    b2::common::{
        begin_idempotent, finish_idempotent, money, next_number, record, request_hash, DomainError,
    },
    store::PgStore,
};
use chrono::{Duration, Utc};
use rust_decimal::Decimal;
use serde_json::json;
use sqlx::{Postgres, Row, Transaction};
use std::collections::BTreeSet;
use uuid::Uuid;

#[derive(Clone)]
pub struct ReceivingService {
    store: PgStore,
    purchasing: PurchasingService,
    receipt_prefix: String,
    payable_prefix: String,
}

impl ReceivingService {
    pub fn new(
        store: PgStore,
        purchasing: PurchasingService,
        receipt_prefix: String,
        payable_prefix: String,
    ) -> Self {
        Self {
            store,
            purchasing,
            receipt_prefix,
            payable_prefix,
        }
    }

    pub async fn create_receipt(
        &self,
        actor: Uuid,
        trace_id: Uuid,
        key: &str,
        input: &CreateGoodsReceipt,
    ) -> Result<CommandResult, DomainError> {
        if input.lines.is_empty() || input.lines.len() > 200 {
            return Err(DomainError::Invalid(
                "goods receipt requires 1-200 lines".into(),
            ));
        }
        let scope = self.purchasing.order_scope(input.purchase_order_id).await?;
        authorize(
            &self.store,
            actor,
            "goods_receipt:create",
            Some(scope.0),
            Some(input.warehouse_id),
            Some(scope.1),
            None,
            Some(scope.2),
        )
        .await?;
        let hash = request_hash(input)?;
        let mut tx = self.store.pool().begin().await?;
        if let Some(mut replay) =
            begin_idempotent::<CommandResult>(&mut tx, actor, "goods_receipt:create", key, &hash)
                .await?
        {
            replay.idempotent_replay = true;
            tx.commit().await?;
            return Ok(replay);
        }
        let order = sqlx::query("SELECT legal_entity_id,supplier_id,business_unit_id,currency::text,lifecycle_status,receiving_status FROM purchase_orders WHERE id=$1 FOR SHARE")
            .bind(input.purchase_order_id).fetch_optional(&mut *tx).await?.ok_or(DomainError::NotFoundOrForbidden)?;
        if order.get::<String, _>("lifecycle_status") != "confirmed"
            || order.get::<String, _>("receiving_status") == "cancelled"
        {
            return Err(DomainError::Invalid(
                "goods receipt requires an open confirmed purchase order".into(),
            ));
        }
        let valid_warehouse: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM business_warehouses WHERE id=$1 AND legal_entity_id=$2 AND status='active')")
            .bind(input.warehouse_id).bind(order.get::<Uuid,_>("legal_entity_id")).fetch_one(&mut *tx).await?;
        if !valid_warehouse {
            return Err(DomainError::NotFoundOrForbidden);
        }
        let mut seen = BTreeSet::new();
        for input_line in &input.lines {
            if !seen.insert(input_line.purchase_order_line_id) {
                return Err(DomainError::Invalid("duplicate purchase order line".into()));
            }
        }
        let line_ids = seen.iter().copied().collect::<Vec<_>>();
        let locked_lines = sqlx::query(
            "SELECT id FROM purchase_order_lines WHERE purchase_order_id=$1 AND id=ANY($2) ORDER BY id FOR UPDATE",
        )
        .bind(input.purchase_order_id)
        .bind(&line_ids)
        .fetch_all(&mut *tx)
        .await?;
        if locked_lines.len() != line_ids.len() {
            return Err(DomainError::NotFoundOrForbidden);
        }
        let mut line_rows = Vec::new();
        for input_line in &input.lines {
            let quantity = input_line
                .quantity
                .positive("quantity")
                .map_err(DomainError::Invalid)?;
            let line=sqlx::query("SELECT id,sku_id,warehouse_id,unit_of_measure_id,ordered_quantity,received_quantity,cancelled_quantity FROM purchase_order_lines WHERE id=$1 AND purchase_order_id=$2")
                .bind(input_line.purchase_order_line_id).bind(input.purchase_order_id).fetch_optional(&mut *tx).await?.ok_or(DomainError::NotFoundOrForbidden)?;
            if line.get::<Uuid, _>("warehouse_id") != input.warehouse_id {
                return Err(DomainError::Invalid(
                    "goods receipt cannot cross warehouses".into(),
                ));
            }
            let remaining = line.get::<Decimal, _>("ordered_quantity")
                - line.get::<Decimal, _>("received_quantity")
                - line.get::<Decimal, _>("cancelled_quantity");
            let draft_allocated: Decimal = sqlx::query_scalar(
                "SELECT COALESCE(sum(grl.received_quantity),0) FROM goods_receipt_lines grl JOIN goods_receipts gr ON gr.id=grl.goods_receipt_id WHERE grl.purchase_order_line_id=$1 AND gr.status='draft'",
            )
            .bind(input_line.purchase_order_line_id)
            .fetch_one(&mut *tx)
            .await?;
            if quantity > remaining - draft_allocated {
                return Err(DomainError::OverReceipt);
            }
            line_rows.push((line, quantity));
        }
        let id = Uuid::new_v4();
        let number = next_number(
            &mut tx,
            "goods_receipt",
            &self.receipt_prefix,
            id,
            crate::numbering::NumberingContext::new(
                order.get("legal_entity_id"),
                Some(order.get("business_unit_id")),
            ),
        )
        .await?;
        sqlx::query("INSERT INTO goods_receipts(id,goods_receipt_number,purchase_order_id,legal_entity_id,supplier_id,warehouse_id,receipt_date,currency,created_by_user_id,trace_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)")
            .bind(id).bind(&number).bind(input.purchase_order_id).bind(order.get::<Uuid,_>("legal_entity_id")).bind(order.get::<Uuid,_>("supplier_id")).bind(input.warehouse_id).bind(input.receipt_date).bind(order.get::<String,_>("currency")).bind(actor).bind(trace_id).execute(&mut *tx).await?;
        for (line, quantity) in line_rows {
            sqlx::query("INSERT INTO goods_receipt_lines(id,goods_receipt_id,purchase_order_line_id,sku_id,unit_of_measure_id,received_quantity,base_quantity) VALUES($1,$2,$3,$4,$5,$6,$6)").bind(Uuid::new_v4()).bind(id).bind(line.get::<Uuid,_>("id")).bind(line.get::<Uuid,_>("sku_id")).bind(line.get::<Uuid,_>("unit_of_measure_id")).bind(quantity).execute(&mut *tx).await?;
        }
        receipt_event(
            &mut tx,
            id,
            "created",
            1,
            actor,
            trace_id,
            json!({"purchaseOrderId":input.purchase_order_id}),
        )
        .await?;
        record(
            &mut tx,
            trace_id,
            actor,
            "GOODS_RECEIPT_CREATED",
            "goods_receipt_created",
            "goods_receipt",
            id,
            json!({"goodsReceiptNumber":number,"purchaseOrderId":input.purchase_order_id}),
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
        finish_idempotent(&mut tx, actor, "goods_receipt:create", key, &result).await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn confirm_receipt(
        &self,
        actor: Uuid,
        trace_id: Uuid,
        receipt_id: Uuid,
        key: &str,
        input: &VersionCommand,
    ) -> Result<CommandResult, DomainError> {
        let scope = self.receipt_scope(receipt_id).await?;
        authorize(
            &self.store,
            actor,
            "goods_receipt:confirm",
            Some(scope.0),
            Some(scope.2),
            Some(scope.1),
            None,
            None,
        )
        .await?;
        let hash = request_hash(input)?;
        let mut tx = self.store.pool().begin().await?;
        if let Some(mut replay) =
            begin_idempotent::<CommandResult>(&mut tx, actor, "goods_receipt:confirm", key, &hash)
                .await?
        {
            replay.idempotent_replay = true;
            tx.commit().await?;
            return Ok(replay);
        }
        let receipt=sqlx::query("SELECT goods_receipt_number,purchase_order_id,legal_entity_id,supplier_id,warehouse_id,receipt_date,currency::text,status,version FROM goods_receipts WHERE id=$1 FOR UPDATE").bind(receipt_id).fetch_optional(&mut *tx).await?.ok_or(DomainError::NotFoundOrForbidden)?;
        if receipt.get::<i64, _>("version") != input.expected_version {
            return Err(DomainError::VersionConflict);
        }
        if receipt.get::<String, _>("status") != "draft" {
            return Err(DomainError::Invalid(
                "only draft goods receipts can be confirmed".into(),
            ));
        }
        let order_id: Uuid = receipt.get("purchase_order_id");
        let order=sqlx::query("SELECT business_unit_id,payment_terms_days,payment_terms_snapshot,lifecycle_status FROM purchase_orders WHERE id=$1 FOR UPDATE").bind(order_id).fetch_one(&mut *tx).await?;
        if order.get::<String, _>("lifecycle_status") != "confirmed" {
            return Err(DomainError::Invalid(
                "purchase order is not open for receiving".into(),
            ));
        }
        let lines=sqlx::query("SELECT grl.id receipt_line_id,grl.purchase_order_line_id,grl.sku_id,grl.received_quantity,pol.ordered_quantity,pol.cancelled_quantity,pol.received_quantity po_received,pol.net_amount po_net,pol.tax_amount po_tax,pol.gross_amount po_gross,pol.received_net_amount,pol.received_tax_amount,pol.received_gross_amount FROM goods_receipt_lines grl JOIN purchase_order_lines pol ON pol.id=grl.purchase_order_line_id WHERE grl.goods_receipt_id=$1 ORDER BY grl.sku_id,grl.id FOR UPDATE OF grl,pol").bind(receipt_id).fetch_all(&mut *tx).await?;
        for line in &lines {
            let remaining = line.get::<Decimal, _>("ordered_quantity")
                - line.get::<Decimal, _>("po_received")
                - line.get::<Decimal, _>("cancelled_quantity");
            if line.get::<Decimal, _>("received_quantity") > remaining {
                return Err(DomainError::OverReceipt);
            }
        }
        for line in &lines {
            sqlx::query("INSERT INTO inventory_balances(legal_entity_id,warehouse_id,sku_id,on_hand_quantity,reserved_quantity,inventory_value,average_unit_cost) VALUES($1,$2,$3,0,0,0,NULL) ON CONFLICT DO NOTHING").bind(receipt.get::<Uuid,_>("legal_entity_id")).bind(receipt.get::<Uuid,_>("warehouse_id")).bind(line.get::<Uuid,_>("sku_id")).execute(&mut *tx).await?;
        }
        let mut locked = BTreeSet::new();
        for line in &lines {
            let sku = line.get::<Uuid, _>("sku_id");
            if locked.insert(sku) {
                sqlx::query("SELECT 1 FROM inventory_balances WHERE legal_entity_id=$1 AND warehouse_id=$2 AND sku_id=$3 FOR UPDATE").bind(receipt.get::<Uuid,_>("legal_entity_id")).bind(receipt.get::<Uuid,_>("warehouse_id")).bind(sku).fetch_one(&mut *tx).await?;
            }
        }
        let mut net_total = Decimal::ZERO;
        let mut tax_total = Decimal::ZERO;
        let mut gross_total = Decimal::ZERO;
        let mut cost_total = Decimal::ZERO;
        for line in &lines {
            let quantity: Decimal = line.get("received_quantity");
            let remaining = line.get::<Decimal, _>("ordered_quantity")
                - line.get::<Decimal, _>("po_received")
                - line.get::<Decimal, _>("cancelled_quantity");
            let final_receipt = quantity == remaining;
            let net = if final_receipt {
                line.get::<Decimal, _>("po_net") - line.get::<Decimal, _>("received_net_amount")
            } else {
                money(
                    line.get::<Decimal, _>("po_net") * quantity
                        / line.get::<Decimal, _>("ordered_quantity"),
                )
            };
            let tax = if final_receipt {
                line.get::<Decimal, _>("po_tax") - line.get::<Decimal, _>("received_tax_amount")
            } else {
                money(
                    line.get::<Decimal, _>("po_tax") * quantity
                        / line.get::<Decimal, _>("ordered_quantity"),
                )
            };
            let gross = if final_receipt {
                line.get::<Decimal, _>("po_gross") - line.get::<Decimal, _>("received_gross_amount")
            } else {
                money(net + tax)
            };
            let unit_cost = money(net / quantity);
            let movement_id = Uuid::new_v4();
            let balance=sqlx::query("SELECT on_hand_quantity,inventory_value FROM inventory_balances WHERE legal_entity_id=$1 AND warehouse_id=$2 AND sku_id=$3 FOR UPDATE").bind(receipt.get::<Uuid,_>("legal_entity_id")).bind(receipt.get::<Uuid,_>("warehouse_id")).bind(line.get::<Uuid,_>("sku_id")).fetch_one(&mut *tx).await?;
            let new_quantity = balance.get::<Decimal, _>("on_hand_quantity") + quantity;
            let new_value = money(balance.get::<Decimal, _>("inventory_value") + net);
            let average = money(new_value / new_quantity);
            sqlx::query("INSERT INTO inventory_movements(id,legal_entity_id,warehouse_id,sku_id,movement_type,quantity,unit_cost,total_cost,currency,source_type,source_id,source_line_id,business_date,created_by_user_id,trace_id) VALUES($1,$2,$3,$4,'purchase_receipt',$5,$6,$7,$8,'goods_receipt',$9,$10,$11,$12,$13)").bind(movement_id).bind(receipt.get::<Uuid,_>("legal_entity_id")).bind(receipt.get::<Uuid,_>("warehouse_id")).bind(line.get::<Uuid,_>("sku_id")).bind(quantity).bind(unit_cost).bind(net).bind(receipt.get::<String,_>("currency")).bind(receipt_id).bind(line.get::<Uuid,_>("receipt_line_id")).bind(receipt.get::<chrono::NaiveDate,_>("receipt_date")).bind(actor).bind(trace_id).execute(&mut *tx).await?;
            sqlx::query("UPDATE inventory_balances SET on_hand_quantity=$4,inventory_value=$5,average_unit_cost=$6,last_movement_id=$7 WHERE legal_entity_id=$1 AND warehouse_id=$2 AND sku_id=$3").bind(receipt.get::<Uuid,_>("legal_entity_id")).bind(receipt.get::<Uuid,_>("warehouse_id")).bind(line.get::<Uuid,_>("sku_id")).bind(new_quantity).bind(new_value).bind(average).bind(movement_id).execute(&mut *tx).await?;
            sqlx::query("UPDATE goods_receipt_lines SET net_amount=$2,tax_amount=$3,gross_amount=$4,provisional_unit_cost=$5,provisional_total_cost=$2,cost_snapshot_at=now(),inventory_movement_id=$6 WHERE id=$1").bind(line.get::<Uuid,_>("receipt_line_id")).bind(net).bind(tax).bind(gross).bind(unit_cost).bind(movement_id).execute(&mut *tx).await?;
            sqlx::query("UPDATE purchase_order_lines SET received_quantity=received_quantity+$2,received_net_amount=received_net_amount+$3,received_tax_amount=received_tax_amount+$4,received_gross_amount=received_gross_amount+$5,received_inventory_cost_amount=received_inventory_cost_amount+$3 WHERE id=$1").bind(line.get::<Uuid,_>("purchase_order_line_id")).bind(quantity).bind(net).bind(tax).bind(gross).execute(&mut *tx).await?;
            record(&mut tx,trace_id,actor,"PURCHASE_INVENTORY_MOVEMENT_POSTED","inventory_purchase_receipt_posted","inventory_movement",movement_id,json!({"goodsReceiptId":receipt_id,"quantity":quantity.to_string(),"provisionalCost":net.to_string()})).await?;
            net_total += net;
            tax_total += tax;
            gross_total += gross;
            cost_total += net;
        }
        let status = refresh_order(&mut tx, order_id, actor, trace_id).await?;
        sqlx::query("UPDATE goods_receipts SET status='confirmed',net_amount=$2,tax_amount=$3,gross_amount=$4,inventory_cost_amount=$5,confirmed_by_user_id=$6,confirmed_at=now(),trace_id=$7 WHERE id=$1").bind(receipt_id).bind(money(net_total)).bind(money(tax_total)).bind(money(gross_total)).bind(money(cost_total)).bind(actor).bind(trace_id).execute(&mut *tx).await?;
        let payable_id = Uuid::new_v4();
        let payable_number = next_number(
            &mut tx,
            "payable",
            &self.payable_prefix,
            payable_id,
            crate::numbering::NumberingContext::new(
                receipt.get("legal_entity_id"),
                Some(order.get("business_unit_id")),
            ),
        )
        .await?;
        let terms: i32 = order.get("payment_terms_days");
        let due_date = receipt
            .get::<chrono::NaiveDate, _>("receipt_date")
            .checked_add_signed(Duration::days(i64::from(terms)))
            .ok_or_else(|| DomainError::Invalid("payable due date overflow".into()))?;
        sqlx::query("INSERT INTO trade_payables(id,payable_number,legal_entity_id,supplier_id,purchase_order_id,goods_receipt_id,currency,original_amount,open_amount,recognized_at,due_date,payment_terms_days,payment_terms_snapshot,trace_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$8,now(),$9,$10,$11,$12)").bind(payable_id).bind(&payable_number).bind(receipt.get::<Uuid,_>("legal_entity_id")).bind(receipt.get::<Uuid,_>("supplier_id")).bind(order_id).bind(receipt_id).bind(receipt.get::<String,_>("currency")).bind(money(gross_total)).bind(due_date).bind(terms).bind(order.get::<serde_json::Value,_>("payment_terms_snapshot")).bind(trace_id).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO trade_payable_events(id,payable_id,event_type,amount,payload,actor_user_id,trace_id) VALUES($1,$2,'created',$3,$4,$5,$6)").bind(Uuid::new_v4()).bind(payable_id).bind(money(gross_total)).bind(json!({"goodsReceiptId":receipt_id})).bind(actor).bind(trace_id).execute(&mut *tx).await?;
        record(&mut tx,trace_id,actor,"TRADE_PAYABLE_CREATED","trade_payable_created","trade_payable",payable_id,json!({"goodsReceiptId":receipt_id,"amount":money(gross_total).to_string(),"dueDate":due_date})).await?;
        let version = input.expected_version + 1;
        receipt_event(&mut tx,receipt_id,"confirmed",version,actor,trace_id,json!({"inventoryCostAmount":money(cost_total).to_string(),"payableId":payable_id,"purchaseOrderStatus":status})).await?;
        record(&mut tx,trace_id,actor,"GOODS_RECEIPT_CONFIRMED","goods_receipt_confirmed","goods_receipt",receipt_id,json!({"version":version,"payableId":payable_id,"provisionalCost":money(cost_total).to_string()})).await?;
        let result = CommandResult {
            id: receipt_id,
            number: receipt.get("goods_receipt_number"),
            status: "confirmed".into(),
            version,
            trace_id,
            idempotent_replay: false,
        };
        finish_idempotent(&mut tx, actor, "goods_receipt:confirm", key, &result).await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn reverse_receipt(
        &self,
        actor: Uuid,
        trace_id: Uuid,
        receipt_id: Uuid,
        key: &str,
        input: &VersionCommand,
    ) -> Result<CommandResult, DomainError> {
        let scope = self.receipt_scope(receipt_id).await?;
        authorize(
            &self.store,
            actor,
            "goods_receipt:reverse",
            Some(scope.0),
            Some(scope.2),
            Some(scope.1),
            None,
            None,
        )
        .await?;
        let hash = request_hash(input)?;
        let mut tx = self.store.pool().begin().await?;
        if let Some(mut replay) =
            begin_idempotent::<CommandResult>(&mut tx, actor, "goods_receipt:reverse", key, &hash)
                .await?
        {
            replay.idempotent_replay = true;
            tx.commit().await?;
            return Ok(replay);
        }
        let receipt=sqlx::query("SELECT goods_receipt_number,purchase_order_id,legal_entity_id,warehouse_id,currency::text,receipt_date,status,version FROM goods_receipts WHERE id=$1 FOR UPDATE").bind(receipt_id).fetch_optional(&mut *tx).await?.ok_or(DomainError::NotFoundOrForbidden)?;
        if receipt.get::<i64, _>("version") != input.expected_version {
            return Err(DomainError::VersionConflict);
        }
        if receipt.get::<String, _>("status") != "confirmed" {
            return Err(DomainError::Invalid(
                "only confirmed goods receipts can be reversed".into(),
            ));
        }
        let payable = sqlx::query(
            "SELECT id,settled_amount FROM trade_payables WHERE goods_receipt_id=$1 FOR UPDATE",
        )
        .bind(receipt_id)
        .fetch_one(&mut *tx)
        .await?;
        if payable.get::<Decimal, _>("settled_amount") > Decimal::ZERO {
            return Err(DomainError::PayableAlreadySettled);
        }
        let lines=sqlx::query("SELECT grl.id receipt_line_id,grl.purchase_order_line_id,grl.sku_id,grl.received_quantity,grl.net_amount,grl.tax_amount,grl.gross_amount,grl.provisional_total_cost,grl.inventory_movement_id,m.posted_at FROM goods_receipt_lines grl JOIN inventory_movements m ON m.id=grl.inventory_movement_id WHERE grl.goods_receipt_id=$1 ORDER BY grl.sku_id,grl.id FOR UPDATE OF grl").bind(receipt_id).fetch_all(&mut *tx).await?;
        for line in &lines {
            let later:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM inventory_movements WHERE legal_entity_id=$1 AND warehouse_id=$2 AND sku_id=$3 AND posted_at>$4)").bind(receipt.get::<Uuid,_>("legal_entity_id")).bind(receipt.get::<Uuid,_>("warehouse_id")).bind(line.get::<Uuid,_>("sku_id")).bind(line.get::<chrono::DateTime<chrono::Utc>,_>("posted_at")).fetch_one(&mut *tx).await?;
            if later {
                return Err(DomainError::SubsequentInventoryMovementsExist);
            }
        }
        for line in &lines {
            let balance=sqlx::query("SELECT on_hand_quantity,inventory_value FROM inventory_balances WHERE legal_entity_id=$1 AND warehouse_id=$2 AND sku_id=$3 FOR UPDATE").bind(receipt.get::<Uuid,_>("legal_entity_id")).bind(receipt.get::<Uuid,_>("warehouse_id")).bind(line.get::<Uuid,_>("sku_id")).fetch_one(&mut *tx).await?;
            let quantity = line.get::<Decimal, _>("received_quantity");
            let cost = line.get::<Decimal, _>("provisional_total_cost");
            let new_quantity = balance.get::<Decimal, _>("on_hand_quantity") - quantity;
            let new_value = money(balance.get::<Decimal, _>("inventory_value") - cost);
            if new_quantity.is_sign_negative() || new_value.is_sign_negative() {
                return Err(DomainError::SubsequentInventoryMovementsExist);
            }
            let movement_id = Uuid::new_v4();
            let average = if new_quantity == Decimal::ZERO {
                None
            } else {
                Some(money(new_value / new_quantity))
            };
            sqlx::query("INSERT INTO inventory_movements(id,legal_entity_id,warehouse_id,sku_id,movement_type,quantity,unit_cost,total_cost,currency,source_type,source_id,source_line_id,business_date,reversal_of_movement_id,created_by_user_id,trace_id) VALUES($1,$2,$3,$4,'purchase_receipt_reversal',$5,$6,$7,$8,'goods_receipt_reversal',$9,$10,$11,$12,$13,$14)").bind(movement_id).bind(receipt.get::<Uuid,_>("legal_entity_id")).bind(receipt.get::<Uuid,_>("warehouse_id")).bind(line.get::<Uuid,_>("sku_id")).bind(-quantity).bind(money(cost/quantity)).bind(-cost).bind(receipt.get::<String,_>("currency")).bind(receipt_id).bind(line.get::<Uuid,_>("receipt_line_id")).bind(receipt.get::<chrono::NaiveDate,_>("receipt_date")).bind(line.get::<Uuid,_>("inventory_movement_id")).bind(actor).bind(trace_id).execute(&mut *tx).await?;
            sqlx::query("UPDATE inventory_balances SET on_hand_quantity=$4,inventory_value=$5,average_unit_cost=$6,last_movement_id=$7 WHERE legal_entity_id=$1 AND warehouse_id=$2 AND sku_id=$3").bind(receipt.get::<Uuid,_>("legal_entity_id")).bind(receipt.get::<Uuid,_>("warehouse_id")).bind(line.get::<Uuid,_>("sku_id")).bind(new_quantity).bind(new_value).bind(average).bind(movement_id).execute(&mut *tx).await?;
            sqlx::query("UPDATE purchase_order_lines SET received_quantity=received_quantity-$2,received_net_amount=received_net_amount-$3,received_tax_amount=received_tax_amount-$4,received_gross_amount=received_gross_amount-$5,received_inventory_cost_amount=received_inventory_cost_amount-$3 WHERE id=$1").bind(line.get::<Uuid,_>("purchase_order_line_id")).bind(quantity).bind(line.get::<Decimal,_>("net_amount")).bind(line.get::<Decimal,_>("tax_amount")).bind(line.get::<Decimal,_>("gross_amount")).execute(&mut *tx).await?;
            record(&mut tx,trace_id,actor,"PURCHASE_INVENTORY_MOVEMENT_REVERSED","inventory_purchase_receipt_reversed","inventory_movement",movement_id,json!({"goodsReceiptId":receipt_id,"reversalOfMovementId":line.get::<Uuid,_>("inventory_movement_id")})).await?;
        }
        sqlx::query("UPDATE trade_payables SET status='reversed',trace_id=$2 WHERE id=$1")
            .bind(payable.get::<Uuid, _>("id"))
            .bind(trace_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO trade_payable_events(id,payable_id,event_type,amount,actor_user_id,trace_id) SELECT $1,id,'reversed',original_amount,$2,$3 FROM trade_payables WHERE id=$4").bind(Uuid::new_v4()).bind(actor).bind(trace_id).bind(payable.get::<Uuid,_>("id")).execute(&mut *tx).await?;
        record(
            &mut tx,
            trace_id,
            actor,
            "TRADE_PAYABLE_REVERSED",
            "trade_payable_reversed",
            "trade_payable",
            payable.get("id"),
            json!({"goodsReceiptId":receipt_id}),
        )
        .await?;
        refresh_order(&mut tx, receipt.get("purchase_order_id"), actor, trace_id).await?;
        sqlx::query(
            "UPDATE goods_receipts SET status='reversed',reversed_at=now(),trace_id=$2 WHERE id=$1",
        )
        .bind(receipt_id)
        .bind(trace_id)
        .execute(&mut *tx)
        .await?;
        let version = input.expected_version + 1;
        receipt_event(
            &mut tx,
            receipt_id,
            "reversed",
            version,
            actor,
            trace_id,
            json!({"payableId":payable.get::<Uuid,_>("id")}),
        )
        .await?;
        record(
            &mut tx,
            trace_id,
            actor,
            "GOODS_RECEIPT_REVERSED",
            "goods_receipt_reversed",
            "goods_receipt",
            receipt_id,
            json!({"version":version}),
        )
        .await?;
        let result = CommandResult {
            id: receipt_id,
            number: receipt.get("goods_receipt_number"),
            status: "reversed".into(),
            version,
            trace_id,
            idempotent_replay: false,
        };
        finish_idempotent(&mut tx, actor, "goods_receipt:reverse", key, &result).await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn draft_options(
        &self,
        actor: Uuid,
        limit: i64,
    ) -> Result<GoodsReceiptDraftOptions, DomainError> {
        let snapshot = authorize(
            &self.store,
            actor,
            "goods_receipt:read",
            None,
            None,
            None,
            None,
            None,
        )
        .await?;
        let can_create = snapshot.permission_keys.contains("goods_receipt:create");
        let rows = sqlx::query_as::<_, GoodsReceiptDraftOptionLine>(
            "WITH options AS (SELECT o.order_date,o.id order_id,o.purchase_order_number order_number,sup.code supplier_code,sup.name supplier_name,o.currency::text currency,l.warehouse_id,w.code warehouse_code,w.name warehouse_name,l.id purchase_order_line_id,l.line_number,l.sku_id,sku.code sku_code,sku.name sku_name,u.code unit_code,u.name unit_name,l.ordered_quantity,l.received_quantity,l.cancelled_quantity,COALESCE((SELECT sum(grl.received_quantity) FROM goods_receipt_lines grl JOIN goods_receipts gr ON gr.id=grl.goods_receipt_id WHERE grl.purchase_order_line_id=l.id AND gr.status='draft'),0) draft_allocated_quantity,GREATEST(l.ordered_quantity-l.received_quantity-l.cancelled_quantity-COALESCE((SELECT sum(grl.received_quantity) FROM goods_receipt_lines grl JOIN goods_receipts gr ON gr.id=grl.goods_receipt_id WHERE grl.purchase_order_line_id=l.id AND gr.status='draft'),0),0) receivable_quantity FROM purchase_orders o JOIN business_suppliers sup ON sup.id=o.supplier_id JOIN purchase_order_lines l ON l.purchase_order_id=o.id JOIN business_warehouses w ON w.id=l.warehouse_id JOIN business_skus sku ON sku.id=l.sku_id JOIN business_units_of_measure u ON u.id=l.unit_of_measure_id WHERE o.legal_entity_id=ANY($1) AND o.supplier_id=ANY($2) AND o.business_unit_id=ANY($3) AND l.warehouse_id=ANY($4) AND o.lifecycle_status='confirmed' AND o.receiving_status<>'cancelled') SELECT order_id,order_number,supplier_code,supplier_name,currency,warehouse_id,warehouse_code,warehouse_name,purchase_order_line_id,line_number,sku_id,sku_code,sku_name,unit_code,unit_name,ordered_quantity,received_quantity,cancelled_quantity,draft_allocated_quantity,receivable_quantity FROM options WHERE receivable_quantity>0 ORDER BY order_date,order_number,warehouse_code,line_number LIMIT $5",
        )
        .bind(snapshot.scopes.legal_entity_ids.into_iter().collect::<Vec<_>>())
        .bind(snapshot.scopes.supplier_ids.into_iter().collect::<Vec<_>>())
        .bind(snapshot.scopes.business_unit_ids.into_iter().collect::<Vec<_>>())
        .bind(snapshot.scopes.warehouse_ids.into_iter().collect::<Vec<_>>())
        .bind(limit.clamp(1, 500))
        .fetch_all(self.store.pool())
        .await?;
        Ok(GoodsReceiptDraftOptions {
            can_create,
            data_as_of: Utc::now(),
            items: rows,
        })
    }

    pub async fn confirmation_preview(
        &self,
        actor: Uuid,
        receipt_id: Uuid,
    ) -> Result<GoodsReceiptConfirmationPreview, DomainError> {
        let receipt = sqlx::query(
            "SELECT gr.goods_receipt_number,gr.purchase_order_id,gr.legal_entity_id,gr.supplier_id,gr.warehouse_id,gr.receipt_date,gr.currency::text currency,gr.status,gr.version,o.purchase_order_number order_number,o.lifecycle_status,o.payment_terms_days,o.business_unit_id,sup.code supplier_code,sup.name supplier_name,w.code warehouse_code,w.name warehouse_name FROM goods_receipts gr JOIN purchase_orders o ON o.id=gr.purchase_order_id JOIN business_suppliers sup ON sup.id=gr.supplier_id JOIN business_warehouses w ON w.id=gr.warehouse_id WHERE gr.id=$1",
        )
        .bind(receipt_id)
        .fetch_optional(self.store.pool())
        .await?
        .ok_or(DomainError::NotFoundOrForbidden)?;
        let snapshot = authorize(
            &self.store,
            actor,
            "goods_receipt:read",
            Some(receipt.get("legal_entity_id")),
            Some(receipt.get("warehouse_id")),
            Some(receipt.get("supplier_id")),
            None,
            Some(receipt.get("business_unit_id")),
        )
        .await?;
        let rows = sqlx::query(
            "SELECT grl.purchase_order_line_id,grl.sku_id,sku.code sku_code,sku.name sku_name,grl.received_quantity,pol.ordered_quantity,pol.cancelled_quantity,pol.received_quantity po_received,pol.net_amount po_net,pol.tax_amount po_tax,pol.gross_amount po_gross,pol.received_net_amount,pol.received_tax_amount,pol.received_gross_amount,COALESCE(b.on_hand_quantity,0) current_on_hand_quantity,COALESCE(b.inventory_value,0) current_inventory_value,b.average_unit_cost current_average_unit_cost FROM goods_receipt_lines grl JOIN purchase_order_lines pol ON pol.id=grl.purchase_order_line_id JOIN business_skus sku ON sku.id=grl.sku_id LEFT JOIN inventory_balances b ON b.legal_entity_id=$2 AND b.warehouse_id=$3 AND b.sku_id=grl.sku_id WHERE grl.goods_receipt_id=$1 ORDER BY grl.id",
        )
        .bind(receipt_id)
        .bind(receipt.get::<Uuid, _>("legal_entity_id"))
        .bind(receipt.get::<Uuid, _>("warehouse_id"))
        .fetch_all(self.store.pool())
        .await?;
        let order_open = receipt.get::<String, _>("lifecycle_status") == "confirmed";
        let mut net_total = Decimal::ZERO;
        let mut tax_total = Decimal::ZERO;
        let mut gross_total = Decimal::ZERO;
        let lines = rows
            .into_iter()
            .map(|row| {
                let quantity: Decimal = row.get("received_quantity");
                let ordered: Decimal = row.get("ordered_quantity");
                let remaining = ordered
                    - row.get::<Decimal, _>("po_received")
                    - row.get::<Decimal, _>("cancelled_quantity");
                let final_receipt = quantity == remaining;
                let net = if final_receipt {
                    row.get::<Decimal, _>("po_net") - row.get::<Decimal, _>("received_net_amount")
                } else {
                    money(row.get::<Decimal, _>("po_net") * quantity / ordered)
                };
                let tax = if final_receipt {
                    row.get::<Decimal, _>("po_tax") - row.get::<Decimal, _>("received_tax_amount")
                } else {
                    money(row.get::<Decimal, _>("po_tax") * quantity / ordered)
                };
                let gross = if final_receipt {
                    row.get::<Decimal, _>("po_gross")
                        - row.get::<Decimal, _>("received_gross_amount")
                } else {
                    money(net + tax)
                };
                net_total += net;
                tax_total += tax;
                gross_total += gross;
                let current_quantity: Decimal = row.get("current_on_hand_quantity");
                let current_value: Decimal = row.get("current_inventory_value");
                let projected_quantity = current_quantity + quantity;
                let projected_value = money(current_value + net);
                let projected_average = money(projected_value / projected_quantity);
                let ready = order_open && quantity <= remaining;
                GoodsReceiptConfirmationLine {
                    purchase_order_line_id: row.get("purchase_order_line_id"),
                    sku_id: row.get("sku_id"),
                    sku_code: row.get("sku_code"),
                    sku_name: row.get("sku_name"),
                    received_quantity: quantity.into(),
                    order_remaining_quantity: remaining.into(),
                    provisional_unit_cost: money(net / quantity).into(),
                    provisional_inventory_cost: net.into(),
                    current_on_hand_quantity: current_quantity.into(),
                    current_inventory_value: current_value.into(),
                    current_average_unit_cost: row
                        .get::<Option<Decimal>, _>("current_average_unit_cost")
                        .map(Into::into),
                    projected_on_hand_quantity: projected_quantity.into(),
                    projected_inventory_value: projected_value.into(),
                    projected_average_unit_cost: projected_average.into(),
                    ready,
                    readiness: if ready {
                        "ready".into()
                    } else if !order_open {
                        "order_not_open".into()
                    } else {
                        "over_receipt".into()
                    },
                }
            })
            .collect::<Vec<_>>();
        let status: String = receipt.get("status");
        let all_ready = !lines.is_empty() && lines.iter().all(|line| line.ready);
        let has_permission = snapshot.permission_keys.contains("goods_receipt:confirm");
        let readiness = if status != "draft" {
            "receipt_not_draft"
        } else if !order_open {
            "order_not_open"
        } else if !all_ready {
            "over_receipt"
        } else if !has_permission {
            "permission_required"
        } else {
            "ready"
        };
        let receipt_date = receipt.get::<chrono::NaiveDate, _>("receipt_date");
        let due_date = receipt_date
            .checked_add_signed(Duration::days(i64::from(
                receipt.get::<i32, _>("payment_terms_days"),
            )))
            .ok_or_else(|| DomainError::Invalid("payable due date overflow".into()))?;
        Ok(GoodsReceiptConfirmationPreview {
            receipt_id,
            receipt_number: receipt.get("goods_receipt_number"),
            purchase_order_id: receipt.get("purchase_order_id"),
            order_number: receipt.get("order_number"),
            supplier_code: receipt.get("supplier_code"),
            supplier_name: receipt.get("supplier_name"),
            warehouse_code: receipt.get("warehouse_code"),
            warehouse_name: receipt.get("warehouse_name"),
            receipt_date,
            status,
            version: receipt.get("version"),
            currency: receipt.get("currency"),
            expected_inventory_cost: money(net_total).into(),
            expected_tax_amount: money(tax_total).into(),
            expected_payable_amount: money(gross_total).into(),
            expected_due_date: due_date,
            can_confirm: readiness == "ready",
            readiness: readiness.into(),
            inventory_as_of: Utc::now(),
            lines,
        })
    }

    pub async fn receipts(
        &self,
        actor: Uuid,
        supplier: Option<Uuid>,
        limit: i64,
    ) -> Result<Vec<GoodsReceiptView>, DomainError> {
        let snapshot = authorize(
            &self.store,
            actor,
            "goods_receipt:read",
            None,
            None,
            supplier,
            None,
            None,
        )
        .await?;
        Ok(sqlx::query_as::<_,GoodsReceiptView>("SELECT id,goods_receipt_number,purchase_order_id,legal_entity_id,supplier_id,warehouse_id,receipt_date,status,currency::text,gross_amount,inventory_cost_amount,updated_at,version FROM goods_receipts WHERE legal_entity_id=ANY($1) AND supplier_id=ANY($2) AND warehouse_id=ANY($3) AND ($4::uuid IS NULL OR supplier_id=$4) ORDER BY updated_at DESC LIMIT $5").bind(snapshot.scopes.legal_entity_ids.into_iter().collect::<Vec<_>>()).bind(snapshot.scopes.supplier_ids.into_iter().collect::<Vec<_>>()).bind(snapshot.scopes.warehouse_ids.into_iter().collect::<Vec<_>>()).bind(supplier).bind(limit.clamp(1,200)).fetch_all(self.store.pool()).await?)
    }
    async fn receipt_scope(&self, id: Uuid) -> Result<(Uuid, Uuid, Uuid), DomainError> {
        let row = sqlx::query(
            "SELECT legal_entity_id,supplier_id,warehouse_id FROM goods_receipts WHERE id=$1",
        )
        .bind(id)
        .fetch_optional(self.store.pool())
        .await?
        .ok_or(DomainError::NotFoundOrForbidden)?;
        Ok((
            row.get("legal_entity_id"),
            row.get("supplier_id"),
            row.get("warehouse_id"),
        ))
    }
}

async fn refresh_order(
    tx: &mut Transaction<'_, Postgres>,
    order_id: Uuid,
    actor: Uuid,
    trace_id: Uuid,
) -> Result<String, DomainError> {
    let row=sqlx::query("SELECT COALESCE(sum(ordered_quantity),0) ordered,COALESCE(sum(received_quantity),0) received,COALESCE(sum(cancelled_quantity),0) cancelled FROM purchase_order_lines WHERE purchase_order_id=$1").bind(order_id).fetch_one(&mut **tx).await?;
    let ordered: Decimal = row.get("ordered");
    let received: Decimal = row.get("received");
    let cancelled: Decimal = row.get("cancelled");
    let terminal = received + cancelled == ordered;
    let (lifecycle, receiving) = if terminal {
        if cancelled > Decimal::ZERO {
            ("completed", "cancelled")
        } else {
            ("completed", "received")
        }
    } else if received > Decimal::ZERO {
        ("confirmed", "partially_received")
    } else {
        ("confirmed", "unreceived")
    };
    sqlx::query("UPDATE purchase_orders SET lifecycle_status=$2,receiving_status=$3,completed_at=CASE WHEN $2='completed' THEN now() ELSE NULL END,updated_by_user_id=$4,trace_id=$5 WHERE id=$1").bind(order_id).bind(lifecycle).bind(receiving).bind(actor).bind(trace_id).execute(&mut **tx).await?;
    Ok(receiving.into())
}
async fn receipt_event(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    event_type: &str,
    version: i64,
    actor: Uuid,
    trace_id: Uuid,
    payload: serde_json::Value,
) -> Result<(), DomainError> {
    sqlx::query("INSERT INTO goods_receipt_events(id,goods_receipt_id,event_type,receipt_version,payload,actor_user_id,trace_id) VALUES($1,$2,$3,$4,$5,$6,$7)").bind(Uuid::new_v4()).bind(id).bind(event_type).bind(version).bind(payload).bind(actor).bind(trace_id).execute(&mut **tx).await?;
    Ok(())
}
