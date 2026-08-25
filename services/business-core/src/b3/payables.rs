use super::{
    common::authorize,
    model::{
        ApplySupplierPayment, CommandResult, CreateSupplierPayment, PayableView,
        ReversePayableAllocation, SupplierPaymentView, VersionCommand,
    },
};
use crate::{
    b2::common::{
        begin_idempotent, finish_idempotent, next_number, record, request_hash, validate_currency,
        DomainError,
    },
    store::PgStore,
};
use rust_decimal::Decimal;
use serde_json::json;
use sqlx::Row;
use std::collections::BTreeSet;
use uuid::Uuid;

#[derive(Clone)]
pub struct PayablesService {
    store: PgStore,
    payment_prefix: String,
}

impl PayablesService {
    pub fn new(store: PgStore, payment_prefix: String) -> Self {
        Self {
            store,
            payment_prefix,
        }
    }

    pub async fn create_payment(
        &self,
        actor: Uuid,
        trace_id: Uuid,
        key: &str,
        input: &CreateSupplierPayment,
    ) -> Result<CommandResult, DomainError> {
        validate_currency(&input.currency)?;
        let amount = input
            .amount
            .positive("amount")
            .map_err(DomainError::Invalid)?;
        if !matches!(
            input.payment_method.as_str(),
            "bank_transfer" | "cash" | "card" | "other"
        ) {
            return Err(DomainError::Invalid("invalid paymentMethod".into()));
        }
        authorize(
            &self.store,
            actor,
            "supplier_payment:create",
            Some(input.legal_entity_id),
            None,
            Some(input.supplier_id),
            None,
            None,
        )
        .await?;
        let hash = request_hash(input)?;
        let mut tx = self.store.pool().begin().await?;
        if let Some(mut replay) =
            begin_idempotent::<CommandResult>(&mut tx, actor, "supplier_payment:create", key, &hash)
                .await?
        {
            replay.idempotent_replay = true;
            tx.commit().await?;
            return Ok(replay);
        }
        let supplier_business_unit: Uuid = sqlx::query_scalar("SELECT business_unit_id FROM business_suppliers WHERE id=$1 AND legal_entity_id=$2 AND status='active'")
            .bind(input.supplier_id).bind(input.legal_entity_id).fetch_optional(&mut *tx).await?.ok_or(DomainError::NotFoundOrForbidden)?;
        let id = Uuid::new_v4();
        let number = next_number(
            &mut tx,
            "supplier_payment",
            &self.payment_prefix,
            id,
            crate::numbering::NumberingContext::new(
                input.legal_entity_id,
                Some(supplier_business_unit),
            ),
        )
        .await?;
        sqlx::query("INSERT INTO supplier_payments(id,supplier_payment_number,legal_entity_id,supplier_id,currency,payment_date,amount,payment_method,external_reference,business_note,created_by_user_id,trace_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)")
            .bind(id).bind(&number).bind(input.legal_entity_id).bind(input.supplier_id).bind(&input.currency).bind(input.payment_date).bind(amount).bind(&input.payment_method).bind(&input.external_reference).bind(&input.business_note).bind(actor).bind(trace_id).execute(&mut *tx).await?;
        payment_event(&mut tx, id, "created", amount, actor, trace_id, json!({})).await?;
        record(&mut tx,trace_id,actor,"SUPPLIER_PAYMENT_CREATED","supplier_payment_created","supplier_payment",id,json!({"supplierPaymentNumber":number,"amount":amount.to_string(),"currency":input.currency})).await?;
        let result = CommandResult {
            id,
            number,
            status: "draft".into(),
            version: 1,
            trace_id,
            idempotent_replay: false,
        };
        finish_idempotent(&mut tx, actor, "supplier_payment:create", key, &result).await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn confirm_payment(
        &self,
        actor: Uuid,
        trace_id: Uuid,
        payment_id: Uuid,
        key: &str,
        input: &VersionCommand,
    ) -> Result<CommandResult, DomainError> {
        let scope = self.payment_scope(payment_id).await?;
        authorize(
            &self.store,
            actor,
            "supplier_payment:confirm",
            Some(scope.0),
            None,
            Some(scope.1),
            None,
            None,
        )
        .await?;
        let hash = request_hash(input)?;
        let mut tx = self.store.pool().begin().await?;
        if let Some(mut replay) = begin_idempotent::<CommandResult>(
            &mut tx,
            actor,
            "supplier_payment:confirm",
            key,
            &hash,
        )
        .await?
        {
            replay.idempotent_replay = true;
            tx.commit().await?;
            return Ok(replay);
        }
        let row=sqlx::query("SELECT supplier_payment_number,amount,status,version FROM supplier_payments WHERE id=$1 FOR UPDATE").bind(payment_id).fetch_optional(&mut *tx).await?.ok_or(DomainError::NotFoundOrForbidden)?;
        if row.get::<i64, _>("version") != input.expected_version {
            return Err(DomainError::VersionConflict);
        }
        if row.get::<String, _>("status") != "draft" {
            return Err(DomainError::Invalid(
                "only draft supplier payments can be confirmed".into(),
            ));
        }
        let amount: Decimal = row.get("amount");
        sqlx::query("UPDATE supplier_payments SET status='confirmed',unapplied_amount=amount,confirmed_by_user_id=$2,confirmed_at=now(),trace_id=$3 WHERE id=$1").bind(payment_id).bind(actor).bind(trace_id).execute(&mut *tx).await?;
        payment_event(
            &mut tx,
            payment_id,
            "confirmed",
            amount,
            actor,
            trace_id,
            json!({}),
        )
        .await?;
        let version = input.expected_version + 1;
        record(
            &mut tx,
            trace_id,
            actor,
            "SUPPLIER_PAYMENT_CONFIRMED",
            "supplier_payment_confirmed",
            "supplier_payment",
            payment_id,
            json!({"version":version}),
        )
        .await?;
        let result = CommandResult {
            id: payment_id,
            number: row.get("supplier_payment_number"),
            status: "confirmed".into(),
            version,
            trace_id,
            idempotent_replay: false,
        };
        finish_idempotent(&mut tx, actor, "supplier_payment:confirm", key, &result).await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn apply_payment(
        &self,
        actor: Uuid,
        trace_id: Uuid,
        payment_id: Uuid,
        key: &str,
        input: &ApplySupplierPayment,
    ) -> Result<CommandResult, DomainError> {
        if input.allocations.is_empty() || input.allocations.len() > 200 {
            return Err(DomainError::Invalid("allocations are required".into()));
        }
        let scope = self.payment_scope(payment_id).await?;
        authorize(
            &self.store,
            actor,
            "payable_allocation:create",
            Some(scope.0),
            None,
            Some(scope.1),
            None,
            None,
        )
        .await?;
        let hash = request_hash(input)?;
        let mut tx = self.store.pool().begin().await?;
        if let Some(mut replay) = begin_idempotent::<CommandResult>(
            &mut tx,
            actor,
            "payable_allocation:create",
            key,
            &hash,
        )
        .await?
        {
            replay.idempotent_replay = true;
            tx.commit().await?;
            return Ok(replay);
        }
        let payment=sqlx::query("SELECT supplier_payment_number,legal_entity_id,supplier_id,currency::text,amount,allocated_amount,unapplied_amount,status,version FROM supplier_payments WHERE id=$1 FOR UPDATE").bind(payment_id).fetch_optional(&mut *tx).await?.ok_or(DomainError::NotFoundOrForbidden)?;
        if payment.get::<i64, _>("version") != input.expected_payment_version {
            return Err(DomainError::VersionConflict);
        }
        if !matches!(
            payment.get::<String, _>("status").as_str(),
            "confirmed" | "partially_allocated"
        ) {
            return Err(DomainError::Invalid(
                "supplier payment is not allocatable".into(),
            ));
        }
        let mut ids = BTreeSet::new();
        let mut requested = Decimal::ZERO;
        for allocation in &input.allocations {
            if !ids.insert(allocation.payable_id) {
                return Err(DomainError::Invalid("duplicate payable allocation".into()));
            }
            requested += allocation
                .amount
                .positive("amount")
                .map_err(DomainError::Invalid)?;
        }
        if requested > payment.get::<Decimal, _>("unapplied_amount") {
            return Err(DomainError::OverAllocation);
        }
        let ordered: Vec<Uuid> = ids.into_iter().collect();
        let rows=sqlx::query("SELECT id,legal_entity_id,supplier_id,currency::text,open_amount,status FROM trade_payables WHERE id=ANY($1) ORDER BY id FOR UPDATE").bind(&ordered).fetch_all(&mut *tx).await?;
        if rows.len() != ordered.len() {
            return Err(DomainError::NotFoundOrForbidden);
        }
        for row in rows {
            let allocation = input
                .allocations
                .iter()
                .find(|a| a.payable_id == row.get::<Uuid, _>("id"))
                .ok_or(DomainError::NotFoundOrForbidden)?;
            let amount = allocation.amount.0;
            if row.get::<Uuid, _>("legal_entity_id") != payment.get::<Uuid, _>("legal_entity_id")
                || row.get::<Uuid, _>("supplier_id") != payment.get::<Uuid, _>("supplier_id")
                || row.get::<String, _>("currency") != payment.get::<String, _>("currency")
            {
                return Err(DomainError::NotFoundOrForbidden);
            }
            if row.get::<String, _>("status") == "reversed"
                || amount > row.get::<Decimal, _>("open_amount")
            {
                return Err(DomainError::OverAllocation);
            }
            let id = Uuid::new_v4();
            sqlx::query("INSERT INTO payable_allocations(id,supplier_payment_id,payable_id,amount,created_by_user_id,trace_id) VALUES($1,$2,$3,$4,$5,$6)").bind(id).bind(payment_id).bind(allocation.payable_id).bind(amount).bind(actor).bind(trace_id).execute(&mut *tx).await?;
            let open = row.get::<Decimal, _>("open_amount") - amount;
            let settled: Decimal =
                sqlx::query_scalar("SELECT settled_amount+$2 FROM trade_payables WHERE id=$1")
                    .bind(allocation.payable_id)
                    .bind(amount)
                    .fetch_one(&mut *tx)
                    .await?;
            let status = if open == Decimal::ZERO {
                "settled"
            } else {
                "partially_settled"
            };
            sqlx::query("UPDATE trade_payables SET settled_amount=$2,open_amount=$3,status=$4,trace_id=$5 WHERE id=$1").bind(allocation.payable_id).bind(settled).bind(open).bind(status).bind(trace_id).execute(&mut *tx).await?;
            sqlx::query("INSERT INTO trade_payable_events(id,payable_id,event_type,amount,payload,actor_user_id,trace_id) VALUES($1,$2,'allocation_applied',$3,$4,$5,$6)").bind(Uuid::new_v4()).bind(allocation.payable_id).bind(amount).bind(json!({"supplierPaymentId":payment_id,"allocationId":id})).bind(actor).bind(trace_id).execute(&mut *tx).await?;
            record(&mut tx,trace_id,actor,"PAYABLE_ALLOCATION_APPLIED","payable_allocation_applied","payable_allocation",id,json!({"payableId":allocation.payable_id,"supplierPaymentId":payment_id,"amount":amount.to_string()})).await?;
        }
        let allocated = payment.get::<Decimal, _>("allocated_amount") + requested;
        let unapplied = payment.get::<Decimal, _>("unapplied_amount") - requested;
        let status = if unapplied == Decimal::ZERO {
            "fully_allocated"
        } else {
            "partially_allocated"
        };
        sqlx::query("UPDATE supplier_payments SET allocated_amount=$2,unapplied_amount=$3,status=$4,trace_id=$5 WHERE id=$1").bind(payment_id).bind(allocated).bind(unapplied).bind(status).bind(trace_id).execute(&mut *tx).await?;
        payment_event(
            &mut tx,
            payment_id,
            "allocated",
            requested,
            actor,
            trace_id,
            json!({"allocationCount":input.allocations.len()}),
        )
        .await?;
        let version = input.expected_payment_version + 1;
        let result = CommandResult {
            id: payment_id,
            number: payment.get("supplier_payment_number"),
            status: status.into(),
            version,
            trace_id,
            idempotent_replay: false,
        };
        finish_idempotent(&mut tx, actor, "payable_allocation:create", key, &result).await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn reverse_allocation(
        &self,
        actor: Uuid,
        trace_id: Uuid,
        allocation_id: Uuid,
        key: &str,
        input: &ReversePayableAllocation,
    ) -> Result<CommandResult, DomainError> {
        let pre=sqlx::query("SELECT p.legal_entity_id,p.supplier_id FROM payable_allocations a JOIN trade_payables p ON p.id=a.payable_id WHERE a.id=$1").bind(allocation_id).fetch_optional(self.store.pool()).await?.ok_or(DomainError::NotFoundOrForbidden)?;
        authorize(
            &self.store,
            actor,
            "payable_allocation:reverse",
            Some(pre.get("legal_entity_id")),
            None,
            Some(pre.get("supplier_id")),
            None,
            None,
        )
        .await?;
        let hash = request_hash(input)?;
        let mut tx = self.store.pool().begin().await?;
        if let Some(mut replay) = begin_idempotent::<CommandResult>(
            &mut tx,
            actor,
            "payable_allocation:reverse",
            key,
            &hash,
        )
        .await?
        {
            replay.idempotent_replay = true;
            tx.commit().await?;
            return Ok(replay);
        }
        let allocation=sqlx::query("SELECT supplier_payment_id,payable_id,amount,allocation_type FROM payable_allocations WHERE id=$1 FOR SHARE").bind(allocation_id).fetch_one(&mut *tx).await?;
        if allocation.get::<String, _>("allocation_type") != "apply" {
            return Err(DomainError::Invalid("allocation cannot be reversed".into()));
        }
        let already: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM payable_allocations WHERE reverses_allocation_id=$1)",
        )
        .bind(allocation_id)
        .fetch_one(&mut *tx)
        .await?;
        if already {
            return Err(DomainError::Invalid(
                "allocation is already reversed".into(),
            ));
        }
        let payment=sqlx::query("SELECT supplier_payment_number,allocated_amount,unapplied_amount,status,version FROM supplier_payments WHERE id=$1 FOR UPDATE").bind(allocation.get::<Uuid,_>("supplier_payment_id")).fetch_one(&mut *tx).await?;
        let payable=sqlx::query("SELECT settled_amount,open_amount,status,version FROM trade_payables WHERE id=$1 FOR UPDATE").bind(allocation.get::<Uuid,_>("payable_id")).fetch_one(&mut *tx).await?;
        if payment.get::<i64, _>("version") != input.expected_payment_version
            || payable.get::<i64, _>("version") != input.expected_payable_version
        {
            return Err(DomainError::VersionConflict);
        }
        if payment.get::<String, _>("status") == "reversed"
            || payable.get::<String, _>("status") == "reversed"
        {
            return Err(DomainError::Invalid(
                "reversed records cannot be adjusted".into(),
            ));
        }
        let amount: Decimal = allocation.get("amount");
        let reversal = Uuid::new_v4();
        sqlx::query("INSERT INTO payable_allocations(id,supplier_payment_id,payable_id,allocation_type,amount,reverses_allocation_id,created_by_user_id,trace_id) VALUES($1,$2,$3,'reversal',$4,$5,$6,$7)").bind(reversal).bind(allocation.get::<Uuid,_>("supplier_payment_id")).bind(allocation.get::<Uuid,_>("payable_id")).bind(amount).bind(allocation_id).bind(actor).bind(trace_id).execute(&mut *tx).await?;
        let allocated = payment.get::<Decimal, _>("allocated_amount") - amount;
        let unapplied = payment.get::<Decimal, _>("unapplied_amount") + amount;
        let payment_status = if allocated == Decimal::ZERO {
            "confirmed"
        } else {
            "partially_allocated"
        };
        sqlx::query("UPDATE supplier_payments SET allocated_amount=$2,unapplied_amount=$3,status=$4,trace_id=$5 WHERE id=$1").bind(allocation.get::<Uuid,_>("supplier_payment_id")).bind(allocated).bind(unapplied).bind(payment_status).bind(trace_id).execute(&mut *tx).await?;
        let settled = payable.get::<Decimal, _>("settled_amount") - amount;
        let open = payable.get::<Decimal, _>("open_amount") + amount;
        let payable_status = if settled == Decimal::ZERO {
            "open"
        } else {
            "partially_settled"
        };
        sqlx::query("UPDATE trade_payables SET settled_amount=$2,open_amount=$3,status=$4,trace_id=$5 WHERE id=$1").bind(allocation.get::<Uuid,_>("payable_id")).bind(settled).bind(open).bind(payable_status).bind(trace_id).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO trade_payable_events(id,payable_id,event_type,amount,payload,actor_user_id,trace_id) VALUES($1,$2,'allocation_reversed',$3,$4,$5,$6)").bind(Uuid::new_v4()).bind(allocation.get::<Uuid,_>("payable_id")).bind(amount).bind(json!({"supplierPaymentId":allocation.get::<Uuid,_>("supplier_payment_id"),"allocationId":allocation_id,"reversalId":reversal})).bind(actor).bind(trace_id).execute(&mut *tx).await?;
        record(
            &mut tx,
            trace_id,
            actor,
            "PAYABLE_ALLOCATION_REVERSED",
            "payable_allocation_reversed",
            "payable_allocation",
            reversal,
            json!({"reversesAllocationId":allocation_id,"amount":amount.to_string()}),
        )
        .await?;
        let version = input.expected_payment_version + 1;
        let result = CommandResult {
            id: allocation.get("supplier_payment_id"),
            number: payment.get("supplier_payment_number"),
            status: payment_status.into(),
            version,
            trace_id,
            idempotent_replay: false,
        };
        finish_idempotent(&mut tx, actor, "payable_allocation:reverse", key, &result).await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn reverse_payment(
        &self,
        actor: Uuid,
        trace_id: Uuid,
        payment_id: Uuid,
        key: &str,
        input: &VersionCommand,
    ) -> Result<CommandResult, DomainError> {
        let scope = self.payment_scope(payment_id).await?;
        authorize(
            &self.store,
            actor,
            "supplier_payment:reverse",
            Some(scope.0),
            None,
            Some(scope.1),
            None,
            None,
        )
        .await?;
        let hash = request_hash(input)?;
        let mut tx = self.store.pool().begin().await?;
        if let Some(mut replay) = begin_idempotent::<CommandResult>(
            &mut tx,
            actor,
            "supplier_payment:reverse",
            key,
            &hash,
        )
        .await?
        {
            replay.idempotent_replay = true;
            tx.commit().await?;
            return Ok(replay);
        }
        let row=sqlx::query("SELECT supplier_payment_number,amount,allocated_amount,status,version FROM supplier_payments WHERE id=$1 FOR UPDATE").bind(payment_id).fetch_one(&mut *tx).await?;
        if row.get::<i64, _>("version") != input.expected_version {
            return Err(DomainError::VersionConflict);
        }
        if row.get::<Decimal, _>("allocated_amount") != Decimal::ZERO {
            return Err(DomainError::Invalid(
                "reverse allocations before reversing supplier payment".into(),
            ));
        }
        if !matches!(
            row.get::<String, _>("status").as_str(),
            "confirmed" | "partially_allocated"
        ) {
            return Err(DomainError::Invalid(
                "supplier payment cannot be reversed".into(),
            ));
        }
        sqlx::query("UPDATE supplier_payments SET status='reversed',unapplied_amount=0,reversed_at=now(),trace_id=$2 WHERE id=$1").bind(payment_id).bind(trace_id).execute(&mut *tx).await?;
        payment_event(
            &mut tx,
            payment_id,
            "reversed",
            row.get("amount"),
            actor,
            trace_id,
            json!({}),
        )
        .await?;
        let version = input.expected_version + 1;
        record(
            &mut tx,
            trace_id,
            actor,
            "SUPPLIER_PAYMENT_REVERSED",
            "supplier_payment_reversed",
            "supplier_payment",
            payment_id,
            json!({"version":version}),
        )
        .await?;
        let result = CommandResult {
            id: payment_id,
            number: row.get("supplier_payment_number"),
            status: "reversed".into(),
            version,
            trace_id,
            idempotent_replay: false,
        };
        finish_idempotent(&mut tx, actor, "supplier_payment:reverse", key, &result).await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn payables(
        &self,
        actor: Uuid,
        supplier: Option<Uuid>,
        limit: i64,
    ) -> Result<Vec<PayableView>, DomainError> {
        let snapshot = authorize(
            &self.store,
            actor,
            "payable:read",
            None,
            None,
            supplier,
            None,
            None,
        )
        .await?;
        Ok(sqlx::query_as::<_,PayableView>("SELECT id,payable_number,legal_entity_id,supplier_id,purchase_order_id,goods_receipt_id,currency::text,original_amount,settled_amount,open_amount,due_date,status,(open_amount>0 AND CURRENT_DATE>due_date) is_overdue,GREATEST(CURRENT_DATE-due_date,0) overdue_days,updated_at,version FROM trade_payables WHERE legal_entity_id=ANY($1) AND supplier_id=ANY($2) AND ($3::uuid IS NULL OR supplier_id=$3) ORDER BY due_date,id LIMIT $4").bind(snapshot.scopes.legal_entity_ids.into_iter().collect::<Vec<_>>()).bind(snapshot.scopes.supplier_ids.into_iter().collect::<Vec<_>>()).bind(supplier).bind(limit.clamp(1,200)).fetch_all(self.store.pool()).await?)
    }
    pub async fn payments(
        &self,
        actor: Uuid,
        supplier: Option<Uuid>,
        limit: i64,
    ) -> Result<Vec<SupplierPaymentView>, DomainError> {
        let snapshot = authorize(
            &self.store,
            actor,
            "supplier_payment:read",
            None,
            None,
            supplier,
            None,
            None,
        )
        .await?;
        Ok(sqlx::query_as::<_,SupplierPaymentView>("SELECT id,supplier_payment_number,legal_entity_id,supplier_id,currency::text,payment_date,amount,allocated_amount,unapplied_amount,status,updated_at,version FROM supplier_payments WHERE legal_entity_id=ANY($1) AND supplier_id=ANY($2) AND ($3::uuid IS NULL OR supplier_id=$3) ORDER BY updated_at DESC LIMIT $4").bind(snapshot.scopes.legal_entity_ids.into_iter().collect::<Vec<_>>()).bind(snapshot.scopes.supplier_ids.into_iter().collect::<Vec<_>>()).bind(supplier).bind(limit.clamp(1,200)).fetch_all(self.store.pool()).await?)
    }
    pub async fn reconcile(&self, actor: Uuid) -> Result<serde_json::Value, DomainError> {
        authorize(
            &self.store,
            actor,
            "payable:read",
            None,
            None,
            None,
            None,
            None,
        )
        .await?;
        let payable=sqlx::query("SELECT payable_id,expected_settled,actual_settled,settled_difference,expected_open,actual_open,open_difference,source_event_count,last_event_at FROM payable_balance_reconciliation WHERE settled_difference<>0 OR open_difference<>0").fetch_all(self.store.pool()).await?;
        let payments=sqlx::query("SELECT supplier_payment_id,expected_allocated,actual_allocated,allocated_difference,expected_unapplied,actual_unapplied,unapplied_difference,source_event_count,last_event_at FROM supplier_payment_reconciliation WHERE allocated_difference<>0 OR unapplied_difference<>0").fetch_all(self.store.pool()).await?;
        Ok(
            json!({"consistent":payable.is_empty()&&payments.is_empty(),"payableDifferenceCount":payable.len(),"supplierPaymentDifferenceCount":payments.len()}),
        )
    }
    async fn payment_scope(&self, id: Uuid) -> Result<(Uuid, Uuid), DomainError> {
        let row =
            sqlx::query("SELECT legal_entity_id,supplier_id FROM supplier_payments WHERE id=$1")
                .bind(id)
                .fetch_optional(self.store.pool())
                .await?
                .ok_or(DomainError::NotFoundOrForbidden)?;
        Ok((row.get("legal_entity_id"), row.get("supplier_id")))
    }
}

async fn payment_event(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: Uuid,
    event_type: &str,
    amount: Decimal,
    actor: Uuid,
    trace_id: Uuid,
    payload: serde_json::Value,
) -> Result<(), DomainError> {
    sqlx::query("INSERT INTO supplier_payment_events(id,supplier_payment_id,event_type,amount,payload,actor_user_id,trace_id) VALUES($1,$2,$3,$4,$5,$6,$7)").bind(Uuid::new_v4()).bind(id).bind(event_type).bind(amount).bind(payload).bind(actor).bind(trace_id).execute(&mut **tx).await?;
    Ok(())
}
