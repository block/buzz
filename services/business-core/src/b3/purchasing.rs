use super::{
    common::authorize,
    model::{
        CommandResult, CreatePurchaseOrder, PurchaseOrderConfirmationLine,
        PurchaseOrderConfirmationPreview, PurchaseOrderDraftLineView, PurchaseOrderDraftView,
        PurchaseOrderEntryOptions, PurchaseOrderLineInput, PurchaseOrderView,
        ReplacePurchaseOrderDraft, VersionCommand,
    },
};
use crate::{
    b2::common::{
        begin_idempotent, finish_idempotent, money, next_number, record, request_hash,
        validate_currency, DomainError,
    },
    store::PgStore,
};
use chrono::Utc;
use rust_decimal::Decimal;
use serde_json::json;
use sqlx::{Postgres, Row, Transaction};
use std::collections::BTreeSet;
use uuid::Uuid;

#[derive(Clone)]
pub struct PurchasingService {
    store: PgStore,
    order_prefix: String,
    default_terms_days: i32,
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

impl PurchasingService {
    pub fn new(store: PgStore, order_prefix: String, default_terms_days: i32) -> Self {
        Self {
            store,
            order_prefix,
            default_terms_days,
        }
    }

    pub async fn create_order(
        &self,
        actor: Uuid,
        trace_id: Uuid,
        key: &str,
        input: &CreatePurchaseOrder,
    ) -> Result<CommandResult, DomainError> {
        validate_order(input)?;
        self.authorize_input(actor, "purchase_order:create", input)
            .await?;
        let hash = request_hash(input)?;
        let mut tx = self.store.pool().begin().await?;
        if let Some(mut replay) =
            begin_idempotent::<CommandResult>(&mut tx, actor, "purchase_order:create", key, &hash)
                .await?
        {
            replay.idempotent_replay = true;
            tx.commit().await?;
            return Ok(replay);
        }
        let supplier_terms = validate_master_data(&mut tx, input).await?;
        let terms = input
            .payment_terms_days
            .unwrap_or(supplier_terms.unwrap_or(self.default_terms_days));
        if !(0..=3650).contains(&terms) {
            return Err(DomainError::Invalid("invalid paymentTermsDays".into()));
        }
        let amounts = calculate_lines(&input.lines)?;
        let (subtotal, discount, net, tax, gross) = totals(&amounts);
        let id = Uuid::new_v4();
        let number = next_number(
            &mut tx,
            "purchase_order",
            &self.order_prefix,
            id,
            crate::numbering::NumberingContext::new(
                input.legal_entity_id,
                Some(input.business_unit_id),
            ),
        )
        .await?;
        sqlx::query("INSERT INTO purchase_orders(id,purchase_order_number,legal_entity_id,supplier_id,buyer_user_id,business_unit_id,department_id,brand_id,currency,order_date,expected_delivery_date,payment_terms_days,payment_terms_snapshot,subtotal_amount,discount_amount,net_amount,tax_amount,gross_amount,supplier_reference,business_note,created_by_user_id,updated_by_user_id,trace_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21,$21,$22)")
            .bind(id).bind(&number).bind(input.legal_entity_id).bind(input.supplier_id)
            .bind(input.buyer_user_id.unwrap_or(actor)).bind(input.business_unit_id)
            .bind(input.department_id).bind(input.brand_id).bind(&input.currency)
            .bind(input.order_date).bind(input.expected_delivery_date).bind(terms)
            .bind(json!({"days":terms,"basis":"goods_receipt_date"}))
            .bind(subtotal).bind(discount).bind(net).bind(tax).bind(gross)
            .bind(&input.supplier_reference).bind(&input.business_note).bind(actor).bind(trace_id)
            .execute(&mut *tx).await?;
        insert_lines(&mut tx, id, input.business_unit_id, &input.lines, &amounts).await?;
        event(
            &mut tx,
            id,
            "created",
            1,
            actor,
            trace_id,
            json!({"grossAmount":gross.to_string()}),
        )
        .await?;
        record(&mut tx, trace_id, actor, "PURCHASE_ORDER_CREATED", "purchase_order_created", "purchase_order", id, json!({"purchaseOrderNumber":number,"grossAmount":gross.to_string(),"currency":input.currency})).await?;
        let result = CommandResult {
            id,
            number,
            status: "draft".into(),
            version: 1,
            trace_id,
            idempotent_replay: false,
        };
        finish_idempotent(&mut tx, actor, "purchase_order:create", key, &result).await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn replace_draft(
        &self,
        actor: Uuid,
        trace_id: Uuid,
        order_id: Uuid,
        key: &str,
        input: &ReplacePurchaseOrderDraft,
    ) -> Result<CommandResult, DomainError> {
        validate_order(&input.order)?;
        let (legal_entity, _, _) = self.order_scope(order_id).await?;
        if legal_entity != input.order.legal_entity_id {
            return Err(DomainError::NotFoundOrForbidden);
        }
        self.authorize_input(actor, "purchase_order:update_draft", &input.order)
            .await?;
        let hash = request_hash(input)?;
        let mut tx = self.store.pool().begin().await?;
        if let Some(mut replay) = begin_idempotent::<CommandResult>(
            &mut tx,
            actor,
            "purchase_order:update_draft",
            key,
            &hash,
        )
        .await?
        {
            replay.idempotent_replay = true;
            tx.commit().await?;
            return Ok(replay);
        }
        let row=sqlx::query("SELECT purchase_order_number,lifecycle_status,version FROM purchase_orders WHERE id=$1 FOR UPDATE").bind(order_id).fetch_optional(&mut *tx).await?.ok_or(DomainError::NotFoundOrForbidden)?;
        if row.get::<String, _>("lifecycle_status") != "draft"
            || row.get::<i64, _>("version") != input.expected_version
        {
            return Err(DomainError::VersionConflict);
        }
        let supplier_terms = validate_master_data(&mut tx, &input.order).await?;
        let terms = input
            .order
            .payment_terms_days
            .unwrap_or(supplier_terms.unwrap_or(self.default_terms_days));
        let amounts = calculate_lines(&input.order.lines)?;
        let (subtotal, discount, net, tax, gross) = totals(&amounts);
        sqlx::query("DELETE FROM purchase_order_lines WHERE purchase_order_id=$1")
            .bind(order_id)
            .execute(&mut *tx)
            .await?;
        insert_lines(
            &mut tx,
            order_id,
            input.order.business_unit_id,
            &input.order.lines,
            &amounts,
        )
        .await?;
        sqlx::query("UPDATE purchase_orders SET supplier_id=$2,buyer_user_id=$3,business_unit_id=$4,department_id=$5,brand_id=$6,currency=$7,order_date=$8,expected_delivery_date=$9,payment_terms_days=$10,payment_terms_snapshot=$11,subtotal_amount=$12,discount_amount=$13,net_amount=$14,tax_amount=$15,gross_amount=$16,supplier_reference=$17,business_note=$18,updated_by_user_id=$19,trace_id=$20 WHERE id=$1")
            .bind(order_id).bind(input.order.supplier_id).bind(input.order.buyer_user_id.unwrap_or(actor))
            .bind(input.order.business_unit_id).bind(input.order.department_id).bind(input.order.brand_id)
            .bind(&input.order.currency).bind(input.order.order_date).bind(input.order.expected_delivery_date)
            .bind(terms).bind(json!({"days":terms,"basis":"goods_receipt_date"}))
            .bind(subtotal).bind(discount).bind(net).bind(tax).bind(gross)
            .bind(&input.order.supplier_reference).bind(&input.order.business_note).bind(actor).bind(trace_id)
            .execute(&mut *tx).await?;
        let version = input.expected_version + 1;
        event(
            &mut tx,
            order_id,
            "draft_updated",
            version,
            actor,
            trace_id,
            json!({"grossAmount":gross.to_string()}),
        )
        .await?;
        record(
            &mut tx,
            trace_id,
            actor,
            "PURCHASE_ORDER_UPDATED",
            "purchase_order_updated",
            "purchase_order",
            order_id,
            json!({"version":version}),
        )
        .await?;
        let result = CommandResult {
            id: order_id,
            number: row.get("purchase_order_number"),
            status: "draft".into(),
            version,
            trace_id,
            idempotent_replay: false,
        };
        finish_idempotent(&mut tx, actor, "purchase_order:update_draft", key, &result).await?;
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
            "purchase_order:confirm",
            Some(scope.0),
            None,
            Some(scope.1),
            None,
            Some(scope.2),
        )
        .await?;
        self.transition(actor, trace_id, order_id, key, input, false)
            .await
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
            "purchase_order:cancel_remaining",
            Some(scope.0),
            None,
            Some(scope.1),
            None,
            Some(scope.2),
        )
        .await?;
        self.transition(actor, trace_id, order_id, key, input, true)
            .await
    }

    async fn transition(
        &self,
        actor: Uuid,
        trace_id: Uuid,
        order_id: Uuid,
        key: &str,
        input: &VersionCommand,
        cancel: bool,
    ) -> Result<CommandResult, DomainError> {
        let operation = if cancel {
            "purchase_order:cancel_remaining"
        } else {
            "purchase_order:confirm"
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
        let row=sqlx::query("SELECT purchase_order_number,lifecycle_status,version FROM purchase_orders WHERE id=$1 FOR UPDATE").bind(order_id).fetch_one(&mut *tx).await?;
        if row.get::<i64, _>("version") != input.expected_version {
            return Err(DomainError::VersionConflict);
        }
        let current: String = row.get("lifecycle_status");
        let (status, receiving, event_type, audit, topic) = if cancel {
            if !matches!(current.as_str(), "draft" | "confirmed") {
                return Err(DomainError::Invalid(
                    "purchase order has no cancellable remainder".into(),
                ));
            }
            let lines=sqlx::query("SELECT id,ordered_quantity,received_quantity FROM purchase_order_lines WHERE purchase_order_id=$1 FOR UPDATE").bind(order_id).fetch_all(&mut *tx).await?;
            let received: Decimal = lines
                .iter()
                .map(|r| r.get::<Decimal, _>("received_quantity"))
                .sum();
            let ordered: Decimal = lines
                .iter()
                .map(|r| r.get::<Decimal, _>("ordered_quantity"))
                .sum();
            if received == ordered {
                return Err(DomainError::Invalid(
                    "fully received purchase order cannot be cancelled".into(),
                ));
            }
            for line in lines {
                sqlx::query("UPDATE purchase_order_lines SET cancelled_quantity=ordered_quantity-received_quantity WHERE id=$1").bind(line.get::<Uuid,_>("id")).execute(&mut *tx).await?;
            }
            if received == Decimal::ZERO {
                (
                    "cancelled",
                    "cancelled",
                    "remaining_cancelled",
                    "PURCHASE_ORDER_REMAINING_CANCELLED",
                    "purchase_order_remaining_cancelled",
                )
            } else {
                (
                    "completed",
                    "cancelled",
                    "remaining_cancelled",
                    "PURCHASE_ORDER_REMAINING_CANCELLED",
                    "purchase_order_remaining_cancelled",
                )
            }
        } else {
            if current != "draft" {
                return Err(DomainError::Invalid(
                    "only draft purchase orders can be confirmed".into(),
                ));
            }
            validate_confirmation_master_data(&mut tx, order_id).await?;
            (
                "confirmed",
                "unreceived",
                "confirmed",
                "PURCHASE_ORDER_CONFIRMED",
                "purchase_order_confirmed",
            )
        };
        sqlx::query("UPDATE purchase_orders SET lifecycle_status=$2,receiving_status=$3,confirmed_at=CASE WHEN $2='confirmed' THEN now() ELSE confirmed_at END,cancelled_at=CASE WHEN $2='cancelled' THEN now() ELSE cancelled_at END,completed_at=CASE WHEN $2='completed' THEN now() ELSE completed_at END,updated_by_user_id=$4,trace_id=$5 WHERE id=$1").bind(order_id).bind(status).bind(receiving).bind(actor).bind(trace_id).execute(&mut *tx).await?;
        let version = input.expected_version + 1;
        event(
            &mut tx,
            order_id,
            event_type,
            version,
            actor,
            trace_id,
            json!({"version":version}),
        )
        .await?;
        record(
            &mut tx,
            trace_id,
            actor,
            audit,
            topic,
            "purchase_order",
            order_id,
            json!({"version":version}),
        )
        .await?;
        let result = CommandResult {
            id: order_id,
            number: row.get("purchase_order_number"),
            status: status.into(),
            version,
            trace_id,
            idempotent_replay: false,
        };
        finish_idempotent(&mut tx, actor, operation, key, &result).await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn orders(
        &self,
        actor: Uuid,
        supplier_id: Option<Uuid>,
        limit: i64,
    ) -> Result<Vec<PurchaseOrderView>, DomainError> {
        let snapshot = authorize(
            &self.store,
            actor,
            "purchase_order:read",
            None,
            None,
            supplier_id,
            None,
            None,
        )
        .await?;
        Ok(sqlx::query_as::<_,PurchaseOrderView>("SELECT id,purchase_order_number,legal_entity_id,supplier_id,currency::text,lifecycle_status,receiving_status,gross_amount,order_date,updated_at,version FROM purchase_orders WHERE legal_entity_id=ANY($1) AND supplier_id=ANY($2) AND ($3::uuid IS NULL OR supplier_id=$3) ORDER BY updated_at DESC LIMIT $4")
            .bind(snapshot.scopes.legal_entity_ids.into_iter().collect::<Vec<_>>()).bind(snapshot.scopes.supplier_ids.into_iter().collect::<Vec<_>>()).bind(supplier_id).bind(limit.clamp(1,200)).fetch_all(self.store.pool()).await?)
    }

    pub async fn entry_options(
        &self,
        actor: Uuid,
        order_id: Option<Uuid>,
    ) -> Result<PurchaseOrderEntryOptions, DomainError> {
        let snapshot = authorize(
            &self.store,
            actor,
            "purchase_order:read",
            None,
            None,
            None,
            None,
            None,
        )
        .await?;
        let can_create = snapshot.permission_keys.contains("purchase_order:create");
        let Some(order_id) = order_id else {
            return Ok(PurchaseOrderEntryOptions {
                can_create,
                can_update: false,
                data_as_of: Utc::now(),
                draft: None,
            });
        };
        let order = sqlx::query(
            "SELECT id,purchase_order_number,legal_entity_id,supplier_id,business_unit_id,currency::text currency,order_date,expected_delivery_date,payment_terms_days,supplier_reference,business_note,lifecycle_status,version FROM purchase_orders WHERE id=$1 AND legal_entity_id=ANY($2) AND supplier_id=ANY($3) AND business_unit_id=ANY($4)",
        )
        .bind(order_id)
        .bind(snapshot.scopes.legal_entity_ids.into_iter().collect::<Vec<_>>())
        .bind(snapshot.scopes.supplier_ids.into_iter().collect::<Vec<_>>())
        .bind(snapshot.scopes.business_unit_ids.into_iter().collect::<Vec<_>>())
        .fetch_optional(self.store.pool())
        .await?
        .ok_or(DomainError::NotFoundOrForbidden)?;
        let line_rows = sqlx::query(
            "SELECT sku_id,warehouse_id,unit_of_measure_id,ordered_quantity,unit_price,discount_amount,tax_rate FROM purchase_order_lines WHERE purchase_order_id=$1 ORDER BY line_number",
        )
        .bind(order_id)
        .fetch_all(self.store.pool())
        .await?;
        let lines = line_rows
            .into_iter()
            .map(|line| PurchaseOrderDraftLineView {
                sku_id: line.get("sku_id"),
                warehouse_id: line.get("warehouse_id"),
                unit_of_measure_id: line.get("unit_of_measure_id"),
                quantity: line.get::<Decimal, _>("ordered_quantity").into(),
                unit_price: line.get::<Decimal, _>("unit_price").into(),
                discount_amount: line.get::<Decimal, _>("discount_amount").into(),
                tax_rate: line.get::<Decimal, _>("tax_rate").into(),
            })
            .collect();
        let lifecycle_status = order.get::<String, _>("lifecycle_status");
        let can_update = lifecycle_status == "draft"
            && snapshot
                .permission_keys
                .contains("purchase_order:update_draft");
        Ok(PurchaseOrderEntryOptions {
            can_create,
            can_update,
            data_as_of: Utc::now(),
            draft: Some(PurchaseOrderDraftView {
                id: order.get("id"),
                purchase_order_number: order.get("purchase_order_number"),
                legal_entity_id: order.get("legal_entity_id"),
                supplier_id: order.get("supplier_id"),
                business_unit_id: order.get("business_unit_id"),
                currency: order.get("currency"),
                order_date: order.get("order_date"),
                expected_delivery_date: order.get("expected_delivery_date"),
                payment_terms_days: order.get("payment_terms_days"),
                supplier_reference: order.get("supplier_reference"),
                business_note: order.get("business_note"),
                lifecycle_status,
                version: order.get("version"),
                lines,
            }),
        })
    }

    pub async fn confirmation_preview(
        &self,
        actor: Uuid,
        order_id: Uuid,
    ) -> Result<PurchaseOrderConfirmationPreview, DomainError> {
        let snapshot = authorize(
            &self.store,
            actor,
            "purchase_order:read",
            None,
            None,
            None,
            None,
            None,
        )
        .await?;
        let order = sqlx::query(
            "SELECT o.purchase_order_number,o.currency::text currency,o.order_date,o.expected_delivery_date,o.payment_terms_days,o.lifecycle_status,o.version,o.subtotal_amount,o.discount_amount,o.net_amount,o.tax_amount,o.gross_amount,s.code supplier_code,s.name supplier_name,s.status supplier_status,bu.status business_unit_status FROM purchase_orders o JOIN business_suppliers s ON s.id=o.supplier_id JOIN business_units bu ON bu.id=o.business_unit_id WHERE o.id=$1 AND o.legal_entity_id=ANY($2) AND o.supplier_id=ANY($3) AND o.business_unit_id=ANY($4)",
        )
        .bind(order_id)
        .bind(snapshot.scopes.legal_entity_ids.into_iter().collect::<Vec<_>>())
        .bind(snapshot.scopes.supplier_ids.into_iter().collect::<Vec<_>>())
        .bind(snapshot.scopes.business_unit_ids.into_iter().collect::<Vec<_>>())
        .fetch_optional(self.store.pool())
        .await?
        .ok_or(DomainError::NotFoundOrForbidden)?;
        let line_rows = sqlx::query(
            "SELECT l.line_number,sku.code sku_code,sku.name sku_name,w.code warehouse_code,w.name warehouse_name,u.code unit_code,u.name unit_name,l.ordered_quantity,l.unit_price,l.discount_amount,l.net_amount,l.tax_rate,l.tax_amount,l.gross_amount,(sku.status='active' AND p.status='active' AND w.status='active' AND u.status='active' AND w.legal_entity_id=o.legal_entity_id AND p.base_uom_id=l.unit_of_measure_id AND l.ordered_quantity>0 AND l.unit_price>=0 AND l.discount_amount>=0 AND l.discount_amount<=l.ordered_quantity*l.unit_price AND l.tax_rate>=0 AND l.tax_rate<=1) ready FROM purchase_order_lines l JOIN purchase_orders o ON o.id=l.purchase_order_id JOIN business_skus sku ON sku.id=l.sku_id JOIN business_products p ON p.id=sku.product_id JOIN business_warehouses w ON w.id=l.warehouse_id JOIN business_units_of_measure u ON u.id=l.unit_of_measure_id WHERE l.purchase_order_id=$1 ORDER BY l.line_number",
        )
        .bind(order_id)
        .fetch_all(self.store.pool())
        .await?;
        let warehouse_count = line_rows
            .iter()
            .map(|row| row.get::<String, _>("warehouse_code"))
            .collect::<BTreeSet<_>>()
            .len() as i64;
        let lines = line_rows
            .into_iter()
            .map(|line| {
                let ready = line.get::<bool, _>("ready");
                PurchaseOrderConfirmationLine {
                    line_number: line.get("line_number"),
                    sku_code: line.get("sku_code"),
                    sku_name: line.get("sku_name"),
                    warehouse_code: line.get("warehouse_code"),
                    warehouse_name: line.get("warehouse_name"),
                    unit_code: line.get("unit_code"),
                    unit_name: line.get("unit_name"),
                    ordered_quantity: line.get::<Decimal, _>("ordered_quantity").into(),
                    unit_price: line.get::<Decimal, _>("unit_price").into(),
                    discount_amount: line.get::<Decimal, _>("discount_amount").into(),
                    net_amount: line.get::<Decimal, _>("net_amount").into(),
                    tax_rate: line.get::<Decimal, _>("tax_rate").into(),
                    tax_amount: line.get::<Decimal, _>("tax_amount").into(),
                    gross_amount: line.get::<Decimal, _>("gross_amount").into(),
                    ready,
                    readiness: if ready {
                        "ready"
                    } else {
                        "master_data_inactive"
                    }
                    .into(),
                }
            })
            .collect::<Vec<_>>();
        let lifecycle_status = order.get::<String, _>("lifecycle_status");
        let supplier_active = order.get::<String, _>("supplier_status") == "active";
        let business_unit_active = order.get::<String, _>("business_unit_status") == "active";
        let lines_ready = !lines.is_empty() && lines.iter().all(|line| line.ready);
        let has_permission = snapshot.permission_keys.contains("purchase_order:confirm");
        let readiness = if lifecycle_status != "draft" {
            "order_not_draft"
        } else if !supplier_active {
            "supplier_inactive"
        } else if !business_unit_active || !lines_ready {
            "line_incomplete"
        } else if !has_permission {
            "permission_required"
        } else {
            "ready"
        };
        Ok(PurchaseOrderConfirmationPreview {
            order_id,
            order_number: order.get("purchase_order_number"),
            supplier_code: order.get("supplier_code"),
            supplier_name: order.get("supplier_name"),
            currency: order.get("currency"),
            order_date: order.get("order_date"),
            expected_delivery_date: order.get("expected_delivery_date"),
            payment_terms_days: order.get("payment_terms_days"),
            lifecycle_status,
            version: order.get("version"),
            subtotal_amount: order.get::<Decimal, _>("subtotal_amount").into(),
            discount_amount: order.get::<Decimal, _>("discount_amount").into(),
            net_amount: order.get::<Decimal, _>("net_amount").into(),
            tax_amount: order.get::<Decimal, _>("tax_amount").into(),
            gross_amount: order.get::<Decimal, _>("gross_amount").into(),
            warehouse_count,
            can_confirm: readiness == "ready",
            readiness: readiness.into(),
            checked_at: Utc::now(),
            lines,
        })
    }

    pub(crate) async fn order_scope(&self, id: Uuid) -> Result<(Uuid, Uuid, Uuid), DomainError> {
        let row = sqlx::query(
            "SELECT legal_entity_id,supplier_id,business_unit_id FROM purchase_orders WHERE id=$1",
        )
        .bind(id)
        .fetch_optional(self.store.pool())
        .await?
        .ok_or(DomainError::NotFoundOrForbidden)?;
        Ok((
            row.get("legal_entity_id"),
            row.get("supplier_id"),
            row.get("business_unit_id"),
        ))
    }

    async fn authorize_input(
        &self,
        actor: Uuid,
        permission: &str,
        input: &CreatePurchaseOrder,
    ) -> Result<(), DomainError> {
        let snapshot = authorize(
            &self.store,
            actor,
            permission,
            Some(input.legal_entity_id),
            None,
            Some(input.supplier_id),
            input.brand_id,
            Some(input.business_unit_id),
        )
        .await?;
        for line in &input.lines {
            if !snapshot.scopes.warehouse_ids.contains(&line.warehouse_id)
                || line
                    .brand_id
                    .is_some_and(|id| !snapshot.scopes.brand_ids.contains(&id))
                || line
                    .business_unit_id
                    .is_some_and(|id| !snapshot.scopes.business_unit_ids.contains(&id))
            {
                return Err(DomainError::NotFoundOrForbidden);
            }
        }
        Ok(())
    }
}

fn validate_order(input: &CreatePurchaseOrder) -> Result<(), DomainError> {
    validate_currency(&input.currency)?;
    if input.lines.is_empty() || input.lines.len() > 200 {
        return Err(DomainError::Invalid(
            "purchase order requires 1-200 lines".into(),
        ));
    }
    if input
        .expected_delivery_date
        .is_some_and(|d| d < input.order_date)
    {
        return Err(DomainError::Invalid(
            "expectedDeliveryDate precedes orderDate".into(),
        ));
    }
    if input.business_note.as_ref().is_some_and(|v| v.len() > 1000) {
        return Err(DomainError::Invalid("businessNote is too long".into()));
    }
    let mut seen = BTreeSet::new();
    for line in &input.lines {
        line.quantity
            .positive("quantity")
            .map_err(DomainError::Invalid)?;
        line.unit_price
            .non_negative("unitPrice")
            .map_err(DomainError::Invalid)?;
        line.discount_amount
            .non_negative("discountAmount")
            .map_err(DomainError::Invalid)?;
        let tax = line
            .tax_rate
            .non_negative("taxRate")
            .map_err(DomainError::Invalid)?;
        if tax > Decimal::ONE {
            return Err(DomainError::Invalid("taxRate must not exceed 1".into()));
        }
        if !seen.insert((line.sku_id, line.warehouse_id)) {
            return Err(DomainError::Invalid(
                "duplicate SKU and warehouse line".into(),
            ));
        }
    }
    Ok(())
}

fn calculate_lines(lines: &[PurchaseOrderLineInput]) -> Result<Vec<LineAmount>, DomainError> {
    lines
        .iter()
        .map(|line| {
            let quantity = line.quantity.0;
            let unit_price = line.unit_price.0;
            let discount = line.discount_amount.0;
            let subtotal = money(quantity * unit_price);
            if discount > subtotal {
                return Err(DomainError::Invalid(
                    "discount exceeds line subtotal".into(),
                ));
            }
            let net = money(subtotal - discount);
            let tax_rate = line.tax_rate.0;
            let tax = money(net * tax_rate);
            Ok(LineAmount {
                quantity,
                unit_price,
                discount: money(discount),
                net,
                tax_rate,
                tax,
                gross: money(net + tax),
            })
        })
        .collect()
}

fn totals(lines: &[LineAmount]) -> (Decimal, Decimal, Decimal, Decimal, Decimal) {
    (
        money(lines.iter().map(|l| l.quantity * l.unit_price).sum()),
        money(lines.iter().map(|l| l.discount).sum()),
        money(lines.iter().map(|l| l.net).sum()),
        money(lines.iter().map(|l| l.tax).sum()),
        money(lines.iter().map(|l| l.gross).sum()),
    )
}

async fn validate_master_data(
    tx: &mut Transaction<'_, Postgres>,
    input: &CreatePurchaseOrder,
) -> Result<Option<i32>, DomainError> {
    let supplier=sqlx::query("SELECT payment_terms_days FROM business_suppliers WHERE id=$1 AND legal_entity_id=$2 AND business_unit_id=$3 AND status='active'").bind(input.supplier_id).bind(input.legal_entity_id).bind(input.business_unit_id).fetch_optional(&mut **tx).await?.ok_or(DomainError::NotFoundOrForbidden)?;
    for line in &input.lines {
        let valid:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM business_skus s JOIN business_products p ON p.id=s.product_id JOIN business_warehouses w ON w.id=$2 WHERE s.id=$1 AND s.status='active' AND p.status='active' AND p.base_uom_id=$3 AND w.status='active' AND w.legal_entity_id=$4)").bind(line.sku_id).bind(line.warehouse_id).bind(line.unit_of_measure_id).bind(input.legal_entity_id).fetch_one(&mut **tx).await?;
        if !valid {
            return Err(DomainError::Invalid(
                "UOM_CONVERSION_NOT_SUPPORTED or inactive SKU/warehouse".into(),
            ));
        }
    }
    Ok(supplier.get("payment_terms_days"))
}

async fn validate_confirmation_master_data(
    tx: &mut Transaction<'_, Postgres>,
    order_id: Uuid,
) -> Result<(), DomainError> {
    let ready: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM purchase_orders o JOIN business_suppliers sup ON sup.id=o.supplier_id JOIN business_units bu ON bu.id=o.business_unit_id WHERE o.id=$1 AND sup.status='active' AND bu.status='active') AND EXISTS(SELECT 1 FROM purchase_order_lines WHERE purchase_order_id=$1) AND NOT EXISTS(SELECT 1 FROM purchase_order_lines l JOIN purchase_orders o ON o.id=l.purchase_order_id JOIN business_skus sku ON sku.id=l.sku_id JOIN business_products p ON p.id=sku.product_id JOIN business_warehouses w ON w.id=l.warehouse_id JOIN business_units_of_measure u ON u.id=l.unit_of_measure_id WHERE l.purchase_order_id=$1 AND (sku.status<>'active' OR p.status<>'active' OR w.status<>'active' OR u.status<>'active' OR w.legal_entity_id<>o.legal_entity_id OR p.base_uom_id<>l.unit_of_measure_id OR l.ordered_quantity<=0 OR l.unit_price<0 OR l.discount_amount<0 OR l.discount_amount>l.ordered_quantity*l.unit_price OR l.tax_rate<0 OR l.tax_rate>1))",
    )
    .bind(order_id)
    .fetch_one(&mut **tx)
    .await?;
    if !ready {
        return Err(DomainError::Invalid(
            "purchase order supplier, business unit, warehouse, SKU or line data is not ready for confirmation".into(),
        ));
    }
    Ok(())
}

async fn insert_lines(
    tx: &mut Transaction<'_, Postgres>,
    order_id: Uuid,
    default_bu: Uuid,
    inputs: &[PurchaseOrderLineInput],
    amounts: &[LineAmount],
) -> Result<(), DomainError> {
    for (index, (input, amount)) in inputs.iter().zip(amounts).enumerate() {
        sqlx::query("INSERT INTO purchase_order_lines(id,purchase_order_id,line_number,sku_id,warehouse_id,unit_of_measure_id,ordered_quantity,unit_price,discount_amount,net_amount,tax_rate,tax_amount,gross_amount,provisional_inventory_cost_amount,business_unit_id,department_id,brand_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$10,$14,$15,$16)").bind(Uuid::new_v4()).bind(order_id).bind(i32::try_from(index+1).map_err(|_|DomainError::Invalid("too many lines".into()))?).bind(input.sku_id).bind(input.warehouse_id).bind(input.unit_of_measure_id).bind(amount.quantity).bind(amount.unit_price).bind(amount.discount).bind(amount.net).bind(amount.tax_rate).bind(amount.tax).bind(amount.gross).bind(input.business_unit_id.unwrap_or(default_bu)).bind(input.department_id).bind(input.brand_id).execute(&mut **tx).await?;
    }
    Ok(())
}

async fn event(
    tx: &mut Transaction<'_, Postgres>,
    order_id: Uuid,
    event_type: &str,
    version: i64,
    actor: Uuid,
    trace_id: Uuid,
    payload: serde_json::Value,
) -> Result<(), DomainError> {
    sqlx::query("INSERT INTO purchase_order_events(id,purchase_order_id,event_type,order_version,payload,actor_user_id,trace_id) VALUES($1,$2,$3,$4,$5,$6,$7)").bind(Uuid::new_v4()).bind(order_id).bind(event_type).bind(version).bind(payload).bind(actor).bind(trace_id).execute(&mut **tx).await?;
    Ok(())
}
