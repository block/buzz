use super::{
    common::{
        authorize, begin_idempotent, finish_idempotent, money, next_number, record, request_hash,
        validate_currency, DomainError,
    },
    model::{
        CommandResult, CreateSalesOrder, CreateShipment, ReplaceSalesOrderDraft,
        SalesOrderConfirmationLine, SalesOrderConfirmationPreview, SalesOrderLineInput,
        SalesOrderSummary, ShipmentConfirmationLine, ShipmentConfirmationPreview,
        ShipmentDraftOptionLine, ShipmentDraftOptions, ShipmentView, VersionCommand,
    },
};
use crate::store::PgStore;
use chrono::{Duration, Utc};
use rust_decimal::Decimal;
use serde_json::json;
use sqlx::{Postgres, Row, Transaction};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

#[derive(Clone)]
pub struct SalesService {
    store: PgStore,
    order_prefix: String,
    shipment_prefix: String,
    default_payment_terms_days: i32,
}

struct LineAmount {
    quantity: Decimal,
    unit_price: Decimal,
    discount: Decimal,
    net: Decimal,
    tax_rate: Decimal,
    tax: Decimal,
    gross: Decimal,
}

impl SalesService {
    pub fn new(
        store: PgStore,
        order_prefix: String,
        shipment_prefix: String,
        default_payment_terms_days: i32,
    ) -> Self {
        Self {
            store,
            order_prefix,
            shipment_prefix,
            default_payment_terms_days,
        }
    }

    pub async fn create_order(
        &self,
        actor: Uuid,
        trace_id: Uuid,
        key: &str,
        input: &CreateSalesOrder,
    ) -> Result<CommandResult, DomainError> {
        validate_currency(&input.currency)?;
        validate_order_input(
            &input.lines,
            input.requested_delivery_date,
            input.order_date,
        )?;
        let snapshot = authorize(
            &self.store,
            actor,
            "sales_order:create",
            Some(input.legal_entity_id),
            None,
            Some(input.customer_id),
            input.brand_id,
            Some(input.business_unit_id),
        )
        .await?;
        for line in &input.lines {
            if !snapshot.scopes.warehouse_ids.contains(&line.warehouse_id)
                || line
                    .business_unit_id
                    .is_some_and(|id| !snapshot.scopes.business_unit_ids.contains(&id))
                || line
                    .brand_id
                    .is_some_and(|id| !snapshot.scopes.brand_ids.contains(&id))
            {
                return Err(DomainError::NotFoundOrForbidden);
            }
        }
        let hash = request_hash(input)?;
        let mut tx = self.store.pool().begin().await?;
        if let Some(mut replay) =
            begin_idempotent::<CommandResult>(&mut tx, actor, "sales_order:create", key, &hash)
                .await?
        {
            replay.idempotent_replay = true;
            tx.commit().await?;
            return Ok(replay);
        }
        let customer_terms = validate_order_master_data(
            &mut tx,
            input.legal_entity_id,
            input.customer_id,
            input.business_unit_id,
            &input.lines,
        )
        .await?;
        let terms = input
            .payment_terms_days
            .unwrap_or(customer_terms.unwrap_or(self.default_payment_terms_days));
        if !(0..=3650).contains(&terms) {
            return Err(DomainError::Invalid("invalid paymentTermsDays".into()));
        }
        let amounts = calculate_lines(&input.lines)?;
        let subtotal = amounts
            .iter()
            .map(|line| line.quantity * line.unit_price)
            .sum();
        let discount = amounts.iter().map(|line| line.discount).sum();
        let net = amounts.iter().map(|line| line.net).sum();
        let tax = amounts.iter().map(|line| line.tax).sum();
        let gross = amounts.iter().map(|line| line.gross).sum();
        let id = Uuid::new_v4();
        let number = next_number(
            &mut tx,
            "sales_order",
            &self.order_prefix,
            id,
            crate::numbering::NumberingContext::new(
                input.legal_entity_id,
                Some(input.business_unit_id),
            ),
        )
        .await?;
        sqlx::query("INSERT INTO sales_orders(id,order_number,legal_entity_id,customer_id,salesperson_user_id,business_unit_id,department_id,brand_id,currency,order_date,requested_delivery_date,payment_terms_days,payment_terms_snapshot,subtotal_amount,discount_amount,net_amount,tax_amount,gross_amount,customer_reference,business_note,created_by_user_id,updated_by_user_id,trace_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21,$21,$22)")
            .bind(id).bind(&number).bind(input.legal_entity_id).bind(input.customer_id)
            .bind(input.salesperson_user_id.unwrap_or(actor)).bind(input.business_unit_id)
            .bind(input.department_id).bind(input.brand_id).bind(&input.currency).bind(input.order_date)
            .bind(input.requested_delivery_date).bind(terms)
            .bind(json!({"days":terms,"basis":"shipment_date"}))
            .bind(money(subtotal)).bind(money(discount)).bind(money(net)).bind(money(tax)).bind(money(gross))
            .bind(&input.customer_reference).bind(&input.business_note).bind(actor).bind(trace_id)
            .execute(&mut *tx).await?;
        insert_order_lines(&mut tx, id, input.business_unit_id, &input.lines, &amounts).await?;
        sqlx::query("INSERT INTO sales_order_events(id,sales_order_id,event_type,order_version,payload,actor_user_id,trace_id) VALUES($1,$2,'created',1,$3,$4,$5)")
            .bind(Uuid::new_v4()).bind(id).bind(json!({"grossAmount":money(gross).to_string(),"currency":input.currency})).bind(actor).bind(trace_id)
            .execute(&mut *tx).await?;
        record(&mut tx, trace_id, actor, "SALES_ORDER_CREATED", "sales_order_created", "sales_order", id, json!({"orderNumber":number,"grossAmount":money(gross).to_string(),"currency":input.currency})).await?;
        let result = CommandResult {
            id,
            number,
            status: "draft".into(),
            version: 1,
            trace_id,
            idempotent_replay: false,
        };
        finish_idempotent(&mut tx, actor, "sales_order:create", key, &result).await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn replace_order_draft(
        &self,
        actor: Uuid,
        trace_id: Uuid,
        order_id: Uuid,
        key: &str,
        input: &ReplaceSalesOrderDraft,
    ) -> Result<CommandResult, DomainError> {
        validate_currency(&input.currency)?;
        validate_order_input(
            &input.lines,
            input.requested_delivery_date,
            input.order_date,
        )?;
        let current = self.order_scope(order_id).await?;
        let snapshot = authorize(
            &self.store,
            actor,
            "sales_order:update_draft",
            Some(current.0),
            None,
            Some(input.customer_id),
            input.brand_id,
            Some(input.business_unit_id),
        )
        .await?;
        for line in &input.lines {
            if !snapshot.scopes.warehouse_ids.contains(&line.warehouse_id) {
                return Err(DomainError::NotFoundOrForbidden);
            }
        }
        let hash = request_hash(input)?;
        let mut tx = self.store.pool().begin().await?;
        if let Some(mut replay) = begin_idempotent::<CommandResult>(
            &mut tx,
            actor,
            "sales_order:update_draft",
            key,
            &hash,
        )
        .await?
        {
            replay.idempotent_replay = true;
            tx.commit().await?;
            return Ok(replay);
        }
        let row = sqlx::query("SELECT order_number,legal_entity_id,lifecycle_status,version FROM sales_orders WHERE id=$1 FOR UPDATE")
            .bind(order_id).fetch_optional(&mut *tx).await?.ok_or(DomainError::NotFoundOrForbidden)?;
        if row.get::<String, _>("lifecycle_status") != "draft"
            || row.get::<i64, _>("version") != input.expected_version
        {
            return Err(DomainError::VersionConflict);
        }
        let legal_entity: Uuid = row.get("legal_entity_id");
        let customer_terms = validate_order_master_data(
            &mut tx,
            legal_entity,
            input.customer_id,
            input.business_unit_id,
            &input.lines,
        )
        .await?;
        let terms = input
            .payment_terms_days
            .unwrap_or(customer_terms.unwrap_or(self.default_payment_terms_days));
        let amounts = calculate_lines(&input.lines)?;
        let subtotal: Decimal = amounts
            .iter()
            .map(|line| line.quantity * line.unit_price)
            .sum();
        let discount: Decimal = amounts.iter().map(|line| line.discount).sum();
        let net: Decimal = amounts.iter().map(|line| line.net).sum();
        let tax: Decimal = amounts.iter().map(|line| line.tax).sum();
        let gross: Decimal = amounts.iter().map(|line| line.gross).sum();
        sqlx::query("DELETE FROM sales_order_lines WHERE sales_order_id=$1")
            .bind(order_id)
            .execute(&mut *tx)
            .await?;
        insert_order_lines(
            &mut tx,
            order_id,
            input.business_unit_id,
            &input.lines,
            &amounts,
        )
        .await?;
        sqlx::query("UPDATE sales_orders SET customer_id=$2,business_unit_id=$3,department_id=$4,brand_id=$5,currency=$6,order_date=$7,requested_delivery_date=$8,payment_terms_days=$9,payment_terms_snapshot=$10,subtotal_amount=$11,discount_amount=$12,net_amount=$13,tax_amount=$14,gross_amount=$15,customer_reference=$16,business_note=$17,updated_by_user_id=$18,trace_id=$19 WHERE id=$1")
            .bind(order_id).bind(input.customer_id).bind(input.business_unit_id).bind(input.department_id).bind(input.brand_id).bind(&input.currency).bind(input.order_date).bind(input.requested_delivery_date).bind(terms).bind(json!({"days":terms,"basis":"shipment_date"})).bind(money(subtotal)).bind(money(discount)).bind(money(net)).bind(money(tax)).bind(money(gross)).bind(&input.customer_reference).bind(&input.business_note).bind(actor).bind(trace_id).execute(&mut *tx).await?;
        let version = input.expected_version + 1;
        sqlx::query("INSERT INTO sales_order_events(id,sales_order_id,event_type,order_version,payload,actor_user_id,trace_id) VALUES($1,$2,'draft_updated',$3,$4,$5,$6)")
            .bind(Uuid::new_v4()).bind(order_id).bind(version).bind(json!({"grossAmount":money(gross).to_string()})).bind(actor).bind(trace_id).execute(&mut *tx).await?;
        record(
            &mut tx,
            trace_id,
            actor,
            "SALES_ORDER_UPDATED",
            "sales_order_updated",
            "sales_order",
            order_id,
            json!({"version":version}),
        )
        .await?;
        let result = CommandResult {
            id: order_id,
            number: row.get("order_number"),
            status: "draft".into(),
            version,
            trace_id,
            idempotent_replay: false,
        };
        finish_idempotent(&mut tx, actor, "sales_order:update_draft", key, &result).await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn confirm_order(
        &self,
        actor: Uuid,
        trace_id: Uuid,
        order_id: Uuid,
        key: &str,
        input: &VersionCommand,
    ) -> Result<CommandResult, DomainError> {
        let scope = self.order_scope(order_id).await?;
        authorize(
            &self.store,
            actor,
            "sales_order:confirm",
            Some(scope.0),
            None,
            Some(scope.1),
            None,
            Some(scope.2),
        )
        .await?;
        let hash = request_hash(input)?;
        let mut tx = self.store.pool().begin().await?;
        if let Some(mut replay) =
            begin_idempotent::<CommandResult>(&mut tx, actor, "sales_order:confirm", key, &hash)
                .await?
        {
            replay.idempotent_replay = true;
            tx.commit().await?;
            return Ok(replay);
        }
        let order=sqlx::query("SELECT order_number,legal_entity_id,lifecycle_status,version FROM sales_orders WHERE id=$1 FOR UPDATE").bind(order_id).fetch_one(&mut *tx).await?;
        if order.get::<i64, _>("version") != input.expected_version {
            return Err(DomainError::VersionConflict);
        }
        if order.get::<String, _>("lifecycle_status") != "draft" {
            return Err(DomainError::Invalid(
                "only draft orders can be confirmed".into(),
            ));
        }
        let legal_entity: Uuid = order.get("legal_entity_id");
        let lines=sqlx::query("SELECT id,warehouse_id,sku_id,ordered_quantity FROM sales_order_lines WHERE sales_order_id=$1 ORDER BY warehouse_id,sku_id,id").bind(order_id).fetch_all(&mut *tx).await?;
        let mut required = BTreeMap::<(Uuid, Uuid), Decimal>::new();
        for line in &lines {
            *required
                .entry((line.get("warehouse_id"), line.get("sku_id")))
                .or_default() += line.get::<Decimal, _>("ordered_quantity");
        }
        let mut shortages = Vec::new();
        for ((warehouse, sku), quantity) in &required {
            let balance=sqlx::query("SELECT on_hand_quantity,reserved_quantity,quarantined_quantity FROM inventory_balances WHERE legal_entity_id=$1 AND warehouse_id=$2 AND sku_id=$3 FOR UPDATE")
                .bind(legal_entity).bind(warehouse).bind(sku).fetch_optional(&mut *tx).await?;
            let available = balance
                .as_ref()
                .map(|row| {
                    row.get::<Decimal, _>("on_hand_quantity")
                        - row.get::<Decimal, _>("reserved_quantity")
                        - row.get::<Decimal, _>("quarantined_quantity")
                })
                .unwrap_or(Decimal::ZERO);
            if available < *quantity {
                shortages.push(json!({"skuId":sku,"warehouseId":warehouse,"requiredQuantity":quantity.to_string(),"availableQuantity":available.to_string()}));
            }
        }
        if !shortages.is_empty() {
            return Err(DomainError::InsufficientStock(
                json!({"shortages":shortages}),
            ));
        }
        for ((warehouse, sku), quantity) in &required {
            sqlx::query("UPDATE inventory_balances SET reserved_quantity=reserved_quantity+$4 WHERE legal_entity_id=$1 AND warehouse_id=$2 AND sku_id=$3")
                .bind(legal_entity).bind(warehouse).bind(sku).bind(quantity).execute(&mut *tx).await?;
        }
        for line in &lines {
            let reservation_id = Uuid::new_v4();
            let line_id: Uuid = line.get("id");
            let quantity: Decimal = line.get("ordered_quantity");
            sqlx::query("INSERT INTO inventory_reservations(id,sales_order_id,sales_order_line_id,legal_entity_id,warehouse_id,sku_id,reserved_quantity,trace_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8)")
                .bind(reservation_id).bind(order_id).bind(line_id).bind(legal_entity).bind(line.get::<Uuid,_>("warehouse_id")).bind(line.get::<Uuid,_>("sku_id")).bind(quantity).bind(trace_id).execute(&mut *tx).await?;
            sqlx::query("INSERT INTO inventory_reservation_events(id,reservation_id,event_type,quantity,actor_user_id,trace_id) VALUES($1,$2,'reserved',$3,$4,$5)")
                .bind(Uuid::new_v4()).bind(reservation_id).bind(quantity).bind(actor).bind(trace_id).execute(&mut *tx).await?;
            sqlx::query(
                "UPDATE sales_order_lines SET reserved_quantity=ordered_quantity WHERE id=$1",
            )
            .bind(line_id)
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query("UPDATE sales_orders SET lifecycle_status='confirmed',fulfillment_status='reserved',confirmed_at=now(),updated_by_user_id=$2,trace_id=$3 WHERE id=$1").bind(order_id).bind(actor).bind(trace_id).execute(&mut *tx).await?;
        let version = input.expected_version + 1;
        sqlx::query("INSERT INTO sales_order_events(id,sales_order_id,event_type,order_version,actor_user_id,trace_id) VALUES($1,$2,'confirmed',$3,$4,$5)").bind(Uuid::new_v4()).bind(order_id).bind(version).bind(actor).bind(trace_id).execute(&mut *tx).await?;
        record(
            &mut tx,
            trace_id,
            actor,
            "SALES_ORDER_CONFIRMED",
            "sales_order_confirmed",
            "sales_order",
            order_id,
            json!({"version":version}),
        )
        .await?;
        record(
            &mut tx,
            trace_id,
            actor,
            "INVENTORY_RESERVED",
            "inventory_reserved",
            "sales_order",
            order_id,
            json!({"lineCount":lines.len()}),
        )
        .await?;
        let result = CommandResult {
            id: order_id,
            number: order.get("order_number"),
            status: "confirmed".into(),
            version,
            trace_id,
            idempotent_replay: false,
        };
        finish_idempotent(&mut tx, actor, "sales_order:confirm", key, &result).await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn set_hold(
        &self,
        actor: Uuid,
        trace_id: Uuid,
        order_id: Uuid,
        key: &str,
        input: &VersionCommand,
        place: bool,
    ) -> Result<CommandResult, DomainError> {
        let scope = self.order_scope(order_id).await?;
        let permission = if place {
            "sales_order:place_hold"
        } else {
            "sales_order:release_hold"
        };
        authorize(
            &self.store,
            actor,
            permission,
            Some(scope.0),
            None,
            Some(scope.1),
            None,
            Some(scope.2),
        )
        .await?;
        if input
            .reason_code
            .as_deref()
            .is_none_or(|value| value.is_empty() || value.len() > 64)
        {
            return Err(DomainError::Invalid("reasonCode is required".into()));
        }
        let operation = if place {
            "sales_order:place_hold"
        } else {
            "sales_order:release_hold"
        };
        let hash = request_hash(input)?;
        let mut tx = self.store.pool().begin().await?;
        if let Some(mut replay) =
            begin_idempotent::<CommandResult>(&mut tx, actor, operation, key, &hash).await?
        {
            replay.idempotent_replay = true;
            tx.commit().await?;
            return Ok(replay);
        }
        let row=sqlx::query("SELECT order_number,lifecycle_status,hold_status,version FROM sales_orders WHERE id=$1 FOR UPDATE").bind(order_id).fetch_one(&mut *tx).await?;
        if row.get::<i64, _>("version") != input.expected_version {
            return Err(DomainError::VersionConflict);
        }
        if row.get::<String, _>("lifecycle_status") != "confirmed" {
            return Err(DomainError::Invalid(
                "hold applies only to confirmed orders".into(),
            ));
        }
        let expected = if place { "none" } else { "manual_review_hold" };
        if row.get::<String, _>("hold_status") != expected {
            return Err(DomainError::Invalid(
                "hold transition is not allowed".into(),
            ));
        }
        let status = if place { "manual_review_hold" } else { "none" };
        sqlx::query(
            "UPDATE sales_orders SET hold_status=$2,updated_by_user_id=$3,trace_id=$4 WHERE id=$1",
        )
        .bind(order_id)
        .bind(status)
        .bind(actor)
        .bind(trace_id)
        .execute(&mut *tx)
        .await?;
        let version = input.expected_version + 1;
        let event = if place {
            "manual_review_hold_placed"
        } else {
            "manual_review_hold_released"
        };
        let audit = if place {
            "SALES_ORDER_HOLD_PLACED"
        } else {
            "SALES_ORDER_HOLD_RELEASED"
        };
        sqlx::query("INSERT INTO sales_order_events(id,sales_order_id,event_type,order_version,payload,actor_user_id,trace_id) VALUES($1,$2,$3,$4,$5,$6,$7)").bind(Uuid::new_v4()).bind(order_id).bind(event).bind(version).bind(json!({"reasonCode":input.reason_code})).bind(actor).bind(trace_id).execute(&mut *tx).await?;
        record(
            &mut tx,
            trace_id,
            actor,
            audit,
            event,
            "sales_order",
            order_id,
            json!({"reasonCode":input.reason_code,"version":version}),
        )
        .await?;
        let result = CommandResult {
            id: order_id,
            number: row.get("order_number"),
            status: status.into(),
            version,
            trace_id,
            idempotent_replay: false,
        };
        finish_idempotent(&mut tx, actor, operation, key, &result).await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn cancel_remaining(
        &self,
        actor: Uuid,
        trace_id: Uuid,
        order_id: Uuid,
        key: &str,
        input: &VersionCommand,
    ) -> Result<CommandResult, DomainError> {
        let scope = self.order_scope(order_id).await?;
        authorize(
            &self.store,
            actor,
            "sales_order:cancel",
            Some(scope.0),
            None,
            Some(scope.1),
            None,
            Some(scope.2),
        )
        .await?;
        let hash = request_hash(input)?;
        let mut tx = self.store.pool().begin().await?;
        if let Some(mut replay) =
            begin_idempotent::<CommandResult>(&mut tx, actor, "sales_order:cancel", key, &hash)
                .await?
        {
            replay.idempotent_replay = true;
            tx.commit().await?;
            return Ok(replay);
        }
        let order=sqlx::query("SELECT order_number,lifecycle_status,legal_entity_id,version FROM sales_orders WHERE id=$1 FOR UPDATE").bind(order_id).fetch_one(&mut *tx).await?;
        if order.get::<i64, _>("version") != input.expected_version {
            return Err(DomainError::VersionConflict);
        }
        let lifecycle: String = order.get("lifecycle_status");
        if !matches!(lifecycle.as_str(), "draft" | "confirmed") {
            return Err(DomainError::Invalid(
                "order has no cancellable quantity".into(),
            ));
        }
        let lines=sqlx::query("SELECT l.id,l.ordered_quantity,l.shipped_quantity,r.id reservation_id,r.warehouse_id,r.sku_id,r.reserved_quantity,r.consumed_quantity,r.released_quantity FROM sales_order_lines l LEFT JOIN inventory_reservations r ON r.sales_order_line_id=l.id WHERE l.sales_order_id=$1 ORDER BY r.warehouse_id,r.sku_id,l.id FOR UPDATE OF l").bind(order_id).fetch_all(&mut *tx).await?;
        let total_shipped: Decimal = lines
            .iter()
            .map(|row| row.get::<Decimal, _>("shipped_quantity"))
            .sum();
        let total_ordered: Decimal = lines
            .iter()
            .map(|row| row.get::<Decimal, _>("ordered_quantity"))
            .sum();
        if total_shipped == total_ordered {
            return Err(DomainError::Invalid(
                "fully shipped order cannot be cancelled".into(),
            ));
        }
        for row in &lines {
            let ordered: Decimal = row.get("ordered_quantity");
            let shipped: Decimal = row.get("shipped_quantity");
            sqlx::query("UPDATE sales_order_lines SET cancelled_quantity=$2,reserved_quantity=0 WHERE id=$1").bind(row.get::<Uuid,_>("id")).bind(ordered-shipped).execute(&mut *tx).await?;
            if let Some(reservation) = row.get::<Option<Uuid>, _>("reservation_id") {
                let open: Decimal = row.get::<Decimal, _>("reserved_quantity")
                    - row.get::<Decimal, _>("consumed_quantity")
                    - row.get::<Decimal, _>("released_quantity");
                if open > Decimal::ZERO {
                    sqlx::query("SELECT 1 FROM inventory_balances WHERE legal_entity_id=$1 AND warehouse_id=$2 AND sku_id=$3 FOR UPDATE").bind(order.get::<Uuid,_>("legal_entity_id")).bind(row.get::<Uuid,_>("warehouse_id")).bind(row.get::<Uuid,_>("sku_id")).fetch_one(&mut *tx).await?;
                    sqlx::query("UPDATE inventory_balances SET reserved_quantity=reserved_quantity-$4 WHERE legal_entity_id=$1 AND warehouse_id=$2 AND sku_id=$3").bind(order.get::<Uuid,_>("legal_entity_id")).bind(row.get::<Uuid,_>("warehouse_id")).bind(row.get::<Uuid,_>("sku_id")).bind(open).execute(&mut *tx).await?;
                    sqlx::query("UPDATE inventory_reservations SET released_quantity=released_quantity+$2,status='released' WHERE id=$1").bind(reservation).bind(open).execute(&mut *tx).await?;
                    sqlx::query("INSERT INTO inventory_reservation_events(id,reservation_id,event_type,quantity,actor_user_id,trace_id) VALUES($1,$2,'released',$3,$4,$5)").bind(Uuid::new_v4()).bind(reservation).bind(open).bind(actor).bind(trace_id).execute(&mut *tx).await?;
                }
            }
        }
        let status = if total_shipped == Decimal::ZERO {
            "cancelled"
        } else {
            "completed"
        };
        sqlx::query("UPDATE sales_orders SET lifecycle_status=$2,fulfillment_status='cancelled',cancelled_at=now(),completed_at=CASE WHEN $2='completed' THEN now() ELSE completed_at END,updated_by_user_id=$3,trace_id=$4 WHERE id=$1").bind(order_id).bind(status).bind(actor).bind(trace_id).execute(&mut *tx).await?;
        let version = input.expected_version + 1;
        sqlx::query("INSERT INTO sales_order_events(id,sales_order_id,event_type,order_version,payload,actor_user_id,trace_id) VALUES($1,$2,'cancelled',$3,$4,$5,$6)").bind(Uuid::new_v4()).bind(order_id).bind(version).bind(json!({"shippedQuantity":total_shipped.to_string()})).bind(actor).bind(trace_id).execute(&mut *tx).await?;
        record(
            &mut tx,
            trace_id,
            actor,
            "SALES_ORDER_CANCELLED",
            "sales_order_cancelled",
            "sales_order",
            order_id,
            json!({"version":version,"releasedQuantity":(total_ordered-total_shipped).to_string()}),
        )
        .await?;
        let result = CommandResult {
            id: order_id,
            number: order.get("order_number"),
            status: status.into(),
            version,
            trace_id,
            idempotent_replay: false,
        };
        finish_idempotent(&mut tx, actor, "sales_order:cancel", key, &result).await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn create_shipment(
        &self,
        actor: Uuid,
        trace_id: Uuid,
        key: &str,
        input: &CreateShipment,
    ) -> Result<CommandResult, DomainError> {
        if input.lines.is_empty() {
            return Err(DomainError::Invalid("shipment lines are required".into()));
        }
        let scope = self.order_scope(input.sales_order_id).await?;
        authorize(
            &self.store,
            actor,
            "shipment:create",
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
            begin_idempotent::<CommandResult>(&mut tx, actor, "shipment:create", key, &hash).await?
        {
            replay.idempotent_replay = true;
            tx.commit().await?;
            return Ok(replay);
        }
        let order=sqlx::query("SELECT legal_entity_id,customer_id,business_unit_id,currency,lifecycle_status,hold_status FROM sales_orders WHERE id=$1 FOR SHARE").bind(input.sales_order_id).fetch_one(&mut *tx).await?;
        if order.get::<String, _>("lifecycle_status") != "confirmed" {
            return Err(DomainError::Invalid(
                "shipment requires a confirmed order".into(),
            ));
        }
        if order.get::<String, _>("hold_status") != "none" {
            return Err(DomainError::OrderOnHold);
        }
        let warehouse_legal: Option<Uuid> = sqlx::query_scalar(
            "SELECT legal_entity_id FROM business_warehouses WHERE id=$1 AND status='active'",
        )
        .bind(input.warehouse_id)
        .fetch_optional(&mut *tx)
        .await?;
        if warehouse_legal != Some(order.get("legal_entity_id")) {
            return Err(DomainError::NotFoundOrForbidden);
        }
        let mut seen = BTreeSet::new();
        let mut line_rows = Vec::new();
        let mut sales_total = Decimal::ZERO;
        for input_line in &input.lines {
            if !seen.insert(input_line.sales_order_line_id) {
                return Err(DomainError::Invalid("duplicate shipment order line".into()));
            }
            let quantity = input_line
                .quantity
                .positive("quantity")
                .map_err(DomainError::Invalid)?;
            let row=sqlx::query("SELECT l.id,l.sku_id,l.warehouse_id,l.ordered_quantity,l.shipped_quantity,l.cancelled_quantity,l.net_amount,r.id reservation_id,r.reserved_quantity,r.consumed_quantity,r.released_quantity FROM sales_order_lines l JOIN inventory_reservations r ON r.sales_order_line_id=l.id WHERE l.id=$1 AND l.sales_order_id=$2 FOR SHARE OF l FOR UPDATE OF r").bind(input_line.sales_order_line_id).bind(input.sales_order_id).fetch_optional(&mut *tx).await?.ok_or(DomainError::NotFoundOrForbidden)?;
            if row.get::<Uuid, _>("warehouse_id") != input.warehouse_id {
                return Err(DomainError::Invalid(
                    "shipment cannot cross warehouses".into(),
                ));
            }
            let remaining: Decimal = row.get::<Decimal, _>("ordered_quantity")
                - row.get::<Decimal, _>("shipped_quantity")
                - row.get::<Decimal, _>("cancelled_quantity");
            let reservation_open: Decimal = row.get::<Decimal, _>("reserved_quantity")
                - row.get::<Decimal, _>("consumed_quantity")
                - row.get::<Decimal, _>("released_quantity");
            let draft_allocated: Decimal = sqlx::query_scalar("SELECT COALESCE(sum(sl.quantity),0) FROM shipment_lines sl JOIN shipments s ON s.id=sl.shipment_id WHERE sl.sales_order_line_id=$1 AND s.status='draft'")
                .bind(input_line.sales_order_line_id)
                .fetch_one(&mut *tx)
                .await?;
            if quantity > remaining || quantity > reservation_open - draft_allocated {
                return Err(DomainError::Invalid(
                    "shipment quantity exceeds reserved remainder".into(),
                ));
            }
            let sales = money(
                row.get::<Decimal, _>("net_amount") * quantity
                    / row.get::<Decimal, _>("ordered_quantity"),
            );
            sales_total += sales;
            line_rows.push((row, quantity, sales));
        }
        let id = Uuid::new_v4();
        let number = next_number(
            &mut tx,
            "shipment",
            &self.shipment_prefix,
            id,
            crate::numbering::NumberingContext::new(
                order.get("legal_entity_id"),
                Some(order.get("business_unit_id")),
            ),
        )
        .await?;
        sqlx::query("INSERT INTO shipments(id,shipment_number,sales_order_id,legal_entity_id,warehouse_id,customer_id,shipment_date,sales_amount,currency,created_by_user_id,trace_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)").bind(id).bind(&number).bind(input.sales_order_id).bind(order.get::<Uuid,_>("legal_entity_id")).bind(input.warehouse_id).bind(order.get::<Uuid,_>("customer_id")).bind(input.shipment_date).bind(money(sales_total)).bind(order.get::<String,_>("currency")).bind(actor).bind(trace_id).execute(&mut *tx).await?;
        for (row, quantity, sales) in line_rows {
            sqlx::query("INSERT INTO shipment_lines(id,shipment_id,sales_order_line_id,sku_id,quantity,sales_amount,inventory_reservation_id) VALUES($1,$2,$3,$4,$5,$6,$7)").bind(Uuid::new_v4()).bind(id).bind(row.get::<Uuid,_>("id")).bind(row.get::<Uuid,_>("sku_id")).bind(quantity).bind(sales).bind(row.get::<Uuid,_>("reservation_id")).execute(&mut *tx).await?;
        }
        sqlx::query("INSERT INTO shipment_events(id,shipment_id,event_type,shipment_version,payload,actor_user_id,trace_id) VALUES($1,$2,'created',1,$3,$4,$5)").bind(Uuid::new_v4()).bind(id).bind(json!({"salesAmount":money(sales_total).to_string()})).bind(actor).bind(trace_id).execute(&mut *tx).await?;
        record(&mut tx,trace_id,actor,"SHIPMENT_CREATED","shipment_created","shipment",id,json!({"salesOrderId":input.sales_order_id,"salesAmount":money(sales_total).to_string()})).await?;
        let result = CommandResult {
            id,
            number,
            status: "draft".into(),
            version: 1,
            trace_id,
            idempotent_replay: false,
        };
        finish_idempotent(&mut tx, actor, "shipment:create", key, &result).await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn list_orders(
        &self,
        actor: Uuid,
        limit: i64,
    ) -> Result<Vec<SalesOrderSummary>, DomainError> {
        let snapshot = authorize(
            &self.store,
            actor,
            "sales_order:read",
            None,
            None,
            None,
            None,
            None,
        )
        .await?;
        let rows=sqlx::query_as::<_,SalesOrderSummary>("SELECT id,order_number,legal_entity_id,customer_id,currency::text,lifecycle_status,hold_status,fulfillment_status,gross_amount,order_date,updated_at,version FROM sales_orders WHERE legal_entity_id=ANY($1) AND customer_id=ANY($2) AND business_unit_id=ANY($3) ORDER BY updated_at DESC LIMIT $4").bind(snapshot.scopes.legal_entity_ids.into_iter().collect::<Vec<_>>()).bind(snapshot.scopes.customer_ids.into_iter().collect::<Vec<_>>()).bind(snapshot.scopes.business_unit_ids.into_iter().collect::<Vec<_>>()).bind(limit.clamp(1,200)).fetch_all(self.store.pool()).await?;
        Ok(rows)
    }

    pub async fn confirmation_preview(
        &self,
        actor: Uuid,
        order_id: Uuid,
    ) -> Result<SalesOrderConfirmationPreview, DomainError> {
        let scope = self.order_scope(order_id).await?;
        let snapshot = authorize(
            &self.store,
            actor,
            "sales_order:read",
            Some(scope.0),
            None,
            Some(scope.1),
            None,
            Some(scope.2),
        )
        .await?;
        let order = sqlx::query(
            "SELECT order_number,lifecycle_status,version FROM sales_orders WHERE id=$1",
        )
        .bind(order_id)
        .fetch_one(self.store.pool())
        .await?;
        let rows = sqlx::query(
            "SELECT l.sku_id,s.code sku_code,s.name sku_name,l.warehouse_id,w.code warehouse_code,w.name warehouse_name,sum(l.ordered_quantity) required_quantity,COALESCE(b.on_hand_quantity,0) on_hand_quantity,COALESCE(b.reserved_quantity,0) reserved_quantity,COALESCE(b.on_hand_quantity-b.reserved_quantity-b.quarantined_quantity,0) available_quantity FROM sales_order_lines l JOIN business_skus s ON s.id=l.sku_id JOIN business_warehouses w ON w.id=l.warehouse_id LEFT JOIN inventory_balances b ON b.legal_entity_id=$2 AND b.warehouse_id=l.warehouse_id AND b.sku_id=l.sku_id WHERE l.sales_order_id=$1 GROUP BY l.sku_id,s.code,s.name,l.warehouse_id,w.code,w.name,b.on_hand_quantity,b.reserved_quantity,b.quarantined_quantity ORDER BY w.code,s.code",
        )
        .bind(order_id)
        .bind(scope.0)
        .fetch_all(self.store.pool())
        .await?;
        let lines = rows
            .into_iter()
            .map(|row| {
                let required: Decimal = row.get("required_quantity");
                let available: Decimal = row.get("available_quantity");
                SalesOrderConfirmationLine {
                    sku_id: row.get("sku_id"),
                    sku_code: row.get("sku_code"),
                    sku_name: row.get("sku_name"),
                    warehouse_id: row.get("warehouse_id"),
                    warehouse_code: row.get("warehouse_code"),
                    warehouse_name: row.get("warehouse_name"),
                    required_quantity: required.into(),
                    on_hand_quantity: row.get::<Decimal, _>("on_hand_quantity").into(),
                    reserved_quantity: row.get::<Decimal, _>("reserved_quantity").into(),
                    available_quantity: available.into(),
                    expected_reserved_quantity: required.min(available).max(Decimal::ZERO).into(),
                    shortage_quantity: (required - available).max(Decimal::ZERO).into(),
                }
            })
            .collect::<Vec<_>>();
        let all_available = !lines.is_empty()
            && lines
                .iter()
                .all(|line| line.shortage_quantity.0 == Decimal::ZERO);
        let lifecycle_status: String = order.get("lifecycle_status");
        let has_permission = snapshot.permission_keys.contains("sales_order:confirm");
        let readiness = if lifecycle_status != "draft" {
            "order_not_draft"
        } else if !has_permission {
            "permission_required"
        } else if !all_available {
            "insufficient_stock"
        } else {
            "ready"
        };
        Ok(SalesOrderConfirmationPreview {
            order_id,
            order_number: order.get("order_number"),
            lifecycle_status,
            version: order.get("version"),
            can_confirm: readiness == "ready",
            readiness: readiness.into(),
            all_available,
            inventory_as_of: Utc::now(),
            lines,
        })
    }

    pub async fn shipments(
        &self,
        actor: Uuid,
        shipment_id: Option<Uuid>,
        limit: i64,
    ) -> Result<Vec<ShipmentView>, DomainError> {
        let snapshot = authorize(
            &self.store,
            actor,
            "sales_order:read",
            None,
            None,
            None,
            None,
            None,
        )
        .await?;
        let rows = sqlx::query_as::<_, ShipmentView>(
            "SELECT s.id,s.shipment_number,s.sales_order_id,s.warehouse_id,s.shipment_date,s.status,s.confirmed_at,s.updated_at,s.version FROM shipments s JOIN sales_orders o ON o.id=s.sales_order_id WHERE o.legal_entity_id=ANY($1) AND s.warehouse_id=ANY($2) AND ($3::uuid IS NULL OR s.id=$3) ORDER BY s.shipment_date DESC,s.id DESC LIMIT $4",
        )
        .bind(snapshot.scopes.legal_entity_ids.into_iter().collect::<Vec<_>>())
        .bind(snapshot.scopes.warehouse_ids.into_iter().collect::<Vec<_>>())
        .bind(shipment_id)
        .bind(limit.clamp(1, 500))
        .fetch_all(self.store.pool())
        .await?;
        Ok(rows)
    }

    pub async fn shipment_draft_options(
        &self,
        actor: Uuid,
        limit: i64,
    ) -> Result<ShipmentDraftOptions, DomainError> {
        let snapshot = authorize(
            &self.store,
            actor,
            "sales_order:read",
            None,
            None,
            None,
            None,
            None,
        )
        .await?;
        let can_create = snapshot.permission_keys.contains("shipment:create");
        let rows = sqlx::query_as::<_, ShipmentDraftOptionLine>(
            "WITH options AS (SELECT o.order_date,o.id order_id,o.order_number,c.code customer_code,c.name customer_name,o.currency::text currency,l.warehouse_id,w.code warehouse_code,w.name warehouse_name,l.id sales_order_line_id,l.line_number,l.sku_id,s.code sku_code,s.name sku_name,l.ordered_quantity,l.shipped_quantity,r.reserved_quantity-r.consumed_quantity-r.released_quantity reservation_open_quantity,COALESCE((SELECT sum(sl.quantity) FROM shipment_lines sl JOIN shipments sh ON sh.id=sl.shipment_id WHERE sl.sales_order_line_id=l.id AND sh.status='draft'),0) draft_allocated_quantity,GREATEST(LEAST(l.ordered_quantity-l.shipped_quantity-l.cancelled_quantity,r.reserved_quantity-r.consumed_quantity-r.released_quantity)-COALESCE((SELECT sum(sl.quantity) FROM shipment_lines sl JOIN shipments sh ON sh.id=sl.shipment_id WHERE sl.sales_order_line_id=l.id AND sh.status='draft'),0),0) shippable_quantity FROM sales_orders o JOIN business_customers c ON c.id=o.customer_id JOIN sales_order_lines l ON l.sales_order_id=o.id JOIN business_warehouses w ON w.id=l.warehouse_id JOIN business_skus s ON s.id=l.sku_id JOIN inventory_reservations r ON r.sales_order_line_id=l.id WHERE o.legal_entity_id=ANY($1) AND o.customer_id=ANY($2) AND o.business_unit_id=ANY($3) AND l.warehouse_id=ANY($4) AND o.lifecycle_status='confirmed' AND o.hold_status='none') SELECT order_id,order_number,customer_code,customer_name,currency,warehouse_id,warehouse_code,warehouse_name,sales_order_line_id,line_number,sku_id,sku_code,sku_name,ordered_quantity,shipped_quantity,reservation_open_quantity,draft_allocated_quantity,shippable_quantity FROM options WHERE shippable_quantity>0 ORDER BY order_date,order_number,warehouse_code,line_number LIMIT $5",
        )
        .bind(snapshot.scopes.legal_entity_ids.into_iter().collect::<Vec<_>>())
        .bind(snapshot.scopes.customer_ids.into_iter().collect::<Vec<_>>())
        .bind(snapshot.scopes.business_unit_ids.into_iter().collect::<Vec<_>>())
        .bind(snapshot.scopes.warehouse_ids.into_iter().collect::<Vec<_>>())
        .bind(limit.clamp(1, 500))
        .fetch_all(self.store.pool())
        .await?;
        Ok(ShipmentDraftOptions {
            can_create,
            data_as_of: Utc::now(),
            items: rows,
        })
    }

    pub async fn shipment_confirmation_preview(
        &self,
        actor: Uuid,
        shipment_id: Uuid,
    ) -> Result<ShipmentConfirmationPreview, DomainError> {
        let shipment = sqlx::query(
            "SELECT sh.shipment_number,sh.sales_order_id,sh.legal_entity_id,sh.warehouse_id,sh.customer_id,sh.shipment_date,sh.sales_amount,sh.currency::text currency,sh.status,sh.version,o.order_number,o.lifecycle_status,o.hold_status,o.payment_terms_days,o.business_unit_id,c.code customer_code,c.name customer_name,w.code warehouse_code,w.name warehouse_name FROM shipments sh JOIN sales_orders o ON o.id=sh.sales_order_id JOIN business_customers c ON c.id=sh.customer_id JOIN business_warehouses w ON w.id=sh.warehouse_id WHERE sh.id=$1",
        )
        .bind(shipment_id)
        .fetch_optional(self.store.pool())
        .await?
        .ok_or(DomainError::NotFoundOrForbidden)?;
        let snapshot = authorize(
            &self.store,
            actor,
            "sales_order:read",
            Some(shipment.get("legal_entity_id")),
            Some(shipment.get("warehouse_id")),
            Some(shipment.get("customer_id")),
            None,
            Some(shipment.get("business_unit_id")),
        )
        .await?;
        let rows = sqlx::query(
            "SELECT sl.sales_order_line_id,sl.sku_id,s.code sku_code,s.name sku_name,sl.quantity,r.reserved_quantity-r.consumed_quantity-r.released_quantity reservation_open_quantity,COALESCE(b.on_hand_quantity,0) on_hand_quantity,COALESCE(b.reserved_quantity,0) reserved_quantity,b.average_unit_cost FROM shipment_lines sl JOIN business_skus s ON s.id=sl.sku_id JOIN inventory_reservations r ON r.id=sl.inventory_reservation_id LEFT JOIN inventory_balances b ON b.legal_entity_id=$2 AND b.warehouse_id=$3 AND b.sku_id=sl.sku_id WHERE sl.shipment_id=$1 ORDER BY sl.id",
        )
        .bind(shipment_id)
        .bind(shipment.get::<Uuid, _>("legal_entity_id"))
        .bind(shipment.get::<Uuid, _>("warehouse_id"))
        .fetch_all(self.store.pool())
        .await?;
        let mut total_cost = Decimal::ZERO;
        let mut all_costed = !rows.is_empty();
        let lines = rows
            .into_iter()
            .map(|row| {
                let quantity: Decimal = row.get("quantity");
                let reservation_open: Decimal = row.get("reservation_open_quantity");
                let on_hand: Decimal = row.get("on_hand_quantity");
                let reserved: Decimal = row.get("reserved_quantity");
                let average: Option<Decimal> = row.get("average_unit_cost");
                let expected_cost = average.map(|value| money(value * quantity));
                if let Some(value) = expected_cost {
                    total_cost += value;
                } else {
                    all_costed = false;
                }
                let readiness = if average.is_none() {
                    "missing_inventory_cost"
                } else if quantity > reservation_open || quantity > on_hand || quantity > reserved {
                    "insufficient_inventory"
                } else {
                    "ready"
                };
                ShipmentConfirmationLine {
                    sales_order_line_id: row.get("sales_order_line_id"),
                    sku_id: row.get("sku_id"),
                    sku_code: row.get("sku_code"),
                    sku_name: row.get("sku_name"),
                    quantity: quantity.into(),
                    reservation_open_quantity: reservation_open.into(),
                    on_hand_quantity: on_hand.into(),
                    reserved_quantity: reserved.into(),
                    average_unit_cost: average.map(Into::into),
                    expected_cost_amount: expected_cost.map(Into::into),
                    ready: readiness == "ready",
                    readiness: readiness.into(),
                }
            })
            .collect::<Vec<_>>();
        let status: String = shipment.get("status");
        let hold_status: String = shipment.get("hold_status");
        let lifecycle_status: String = shipment.get("lifecycle_status");
        let has_inventory = !lines.is_empty() && lines.iter().all(|line| line.ready);
        let has_permission = snapshot.permission_keys.contains("shipment:confirm");
        let readiness = if status != "draft" {
            "shipment_not_draft"
        } else if hold_status != "none" {
            "order_on_hold"
        } else if lifecycle_status != "confirmed" {
            "order_not_fulfillable"
        } else if lines.iter().any(|line| line.average_unit_cost.is_none()) {
            "missing_inventory_cost"
        } else if !has_inventory {
            "insufficient_inventory"
        } else if !has_permission {
            "permission_required"
        } else {
            "ready"
        };
        let shipment_date = shipment.get::<chrono::NaiveDate, _>("shipment_date");
        let sales_amount = shipment.get::<Decimal, _>("sales_amount");
        Ok(ShipmentConfirmationPreview {
            shipment_id,
            shipment_number: shipment.get("shipment_number"),
            sales_order_id: shipment.get("sales_order_id"),
            order_number: shipment.get("order_number"),
            customer_code: shipment.get("customer_code"),
            customer_name: shipment.get("customer_name"),
            warehouse_code: shipment.get("warehouse_code"),
            warehouse_name: shipment.get("warehouse_name"),
            shipment_date,
            status,
            version: shipment.get("version"),
            currency: shipment.get("currency"),
            sales_amount: sales_amount.into(),
            expected_cost_amount: all_costed.then(|| money(total_cost).into()),
            expected_receivable_amount: sales_amount.into(),
            expected_due_date: shipment_date
                + Duration::days(i64::from(shipment.get::<i32, _>("payment_terms_days"))),
            can_confirm: readiness == "ready",
            readiness: readiness.into(),
            inventory_as_of: Utc::now(),
            lines,
        })
    }

    async fn order_scope(&self, id: Uuid) -> Result<(Uuid, Uuid, Uuid), DomainError> {
        let row = sqlx::query(
            "SELECT legal_entity_id,customer_id,business_unit_id FROM sales_orders WHERE id=$1",
        )
        .bind(id)
        .fetch_optional(self.store.pool())
        .await?
        .ok_or(DomainError::NotFoundOrForbidden)?;
        Ok((
            row.get("legal_entity_id"),
            row.get("customer_id"),
            row.get("business_unit_id"),
        ))
    }
}

fn validate_order_input(
    lines: &[SalesOrderLineInput],
    delivery: Option<chrono::NaiveDate>,
    order_date: chrono::NaiveDate,
) -> Result<(), DomainError> {
    if lines.is_empty() || lines.len() > 100 {
        return Err(DomainError::Invalid(
            "sales order requires 1-100 lines".into(),
        ));
    }
    if delivery.is_some_and(|date| date < order_date) {
        return Err(DomainError::Invalid(
            "requestedDeliveryDate precedes orderDate".into(),
        ));
    }
    Ok(())
}

fn calculate_lines(lines: &[SalesOrderLineInput]) -> Result<Vec<LineAmount>, DomainError> {
    lines
        .iter()
        .map(|line| {
            let quantity = line
                .quantity
                .positive("quantity")
                .map_err(DomainError::Invalid)?;
            let unit_price = line
                .unit_price
                .non_negative("unitPrice")
                .map_err(DomainError::Invalid)?;
            let discount = line
                .discount_amount
                .non_negative("discountAmount")
                .map_err(DomainError::Invalid)?;
            let tax_rate = line
                .tax_rate
                .non_negative("taxRate")
                .map_err(DomainError::Invalid)?;
            if tax_rate > Decimal::ONE {
                return Err(DomainError::Invalid("taxRate must not exceed 1".into()));
            }
            let subtotal = money(quantity * unit_price);
            if discount > subtotal {
                return Err(DomainError::Invalid(
                    "discount exceeds line subtotal".into(),
                ));
            }
            let net = money(subtotal - discount);
            let tax = money(net * tax_rate);
            Ok(LineAmount {
                quantity,
                unit_price,
                discount,
                net,
                tax_rate,
                tax,
                gross: money(net + tax),
            })
        })
        .collect()
}

async fn validate_order_master_data(
    tx: &mut Transaction<'_, Postgres>,
    legal: Uuid,
    customer: Uuid,
    business_unit: Uuid,
    lines: &[SalesOrderLineInput],
) -> Result<Option<i32>, DomainError> {
    let customer_row=sqlx::query("SELECT payment_terms_days FROM business_customers WHERE id=$1 AND legal_entity_id=$2 AND status='active'").bind(customer).bind(legal).fetch_optional(&mut **tx).await?.ok_or(DomainError::NotFoundOrForbidden)?;
    let unit_ok:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM business_units WHERE id=$1 AND legal_entity_id=$2 AND status='active')").bind(business_unit).bind(legal).fetch_one(&mut **tx).await?;
    if !unit_ok {
        return Err(DomainError::NotFoundOrForbidden);
    }
    for line in lines {
        let row=sqlx::query("SELECT w.legal_entity_id,s.status sku_status,p.base_uom_id,p.brand_id,p.status product_status FROM business_warehouses w,business_skus s JOIN business_products p ON p.id=s.product_id WHERE w.id=$1 AND s.id=$2 AND w.status='active'").bind(line.warehouse_id).bind(line.sku_id).fetch_optional(&mut **tx).await?.ok_or(DomainError::NotFoundOrForbidden)?;
        if row.get::<Uuid, _>("legal_entity_id") != legal
            || row.get::<String, _>("sku_status") != "active"
            || row.get::<String, _>("product_status") != "active"
            || row.get::<Uuid, _>("base_uom_id") != line.unit_of_measure_id
            || line
                .brand_id
                .is_some_and(|id| Some(id) != row.get::<Option<Uuid>, _>("brand_id"))
        {
            return Err(DomainError::NotFoundOrForbidden);
        }
    }
    Ok(Some(customer_row.get("payment_terms_days")))
}

async fn insert_order_lines(
    tx: &mut Transaction<'_, Postgres>,
    order_id: Uuid,
    header_business_unit: Uuid,
    lines: &[SalesOrderLineInput],
    amounts: &[LineAmount],
) -> Result<(), DomainError> {
    for (index, (line, amount)) in lines.iter().zip(amounts).enumerate() {
        sqlx::query("INSERT INTO sales_order_lines(id,sales_order_id,line_number,sku_id,warehouse_id,unit_of_measure_id,ordered_quantity,unit_price,discount_amount,net_amount,tax_rate,tax_amount,gross_amount,business_unit_id,department_id,brand_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16)").bind(Uuid::new_v4()).bind(order_id).bind((index+1)as i32).bind(line.sku_id).bind(line.warehouse_id).bind(line.unit_of_measure_id).bind(amount.quantity).bind(amount.unit_price).bind(amount.discount).bind(amount.net).bind(amount.tax_rate).bind(amount.tax).bind(amount.gross).bind(line.business_unit_id.unwrap_or(header_business_unit)).bind(line.department_id).bind(line.brand_id).execute(&mut **tx).await?;
    }
    Ok(())
}
