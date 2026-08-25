use super::{
    common::{
        authorize, begin_idempotent, finish_idempotent, next_number, record, request_hash,
        validate_currency, DomainError,
    },
    model::{
        ApplyReceipt, CommandResult, CreateCustomerReceipt, ReceiptView, ReceivableView,
        ReverseAllocation, VersionCommand,
    },
};
use crate::store::PgStore;
use rust_decimal::Decimal;
use serde_json::{json, Value};
use sqlx::Row;
use std::collections::BTreeSet;
use uuid::Uuid;

#[derive(Clone)]
pub struct SettlementService {
    store: PgStore,
    receipt_prefix: String,
}

impl SettlementService {
    pub fn new(store: PgStore, receipt_prefix: String) -> Self {
        Self {
            store,
            receipt_prefix,
        }
    }

    pub async fn create_receipt(
        &self,
        actor: Uuid,
        trace_id: Uuid,
        key: &str,
        input: &CreateCustomerReceipt,
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
            "customer_receipt:create",
            Some(input.legal_entity_id),
            None,
            Some(input.customer_id),
            None,
            None,
        )
        .await?;
        let hash = request_hash(input)?;
        let mut tx = self.store.pool().begin().await?;
        if let Some(mut replay) =
            begin_idempotent::<CommandResult>(&mut tx, actor, "customer_receipt:create", key, &hash)
                .await?
        {
            replay.idempotent_replay = true;
            tx.commit().await?;
            return Ok(replay);
        }
        let customer_business_unit: Uuid = sqlx::query_scalar("SELECT business_unit_id FROM business_customers WHERE id=$1 AND legal_entity_id=$2 AND status='active'")
            .bind(input.customer_id).bind(input.legal_entity_id).fetch_optional(&mut *tx).await?.ok_or(DomainError::NotFoundOrForbidden)?;
        let id = Uuid::new_v4();
        let number = next_number(
            &mut tx,
            "receipt",
            &self.receipt_prefix,
            id,
            crate::numbering::NumberingContext::new(
                input.legal_entity_id,
                Some(customer_business_unit),
            ),
        )
        .await?;
        sqlx::query("INSERT INTO customer_receipts(id,receipt_number,legal_entity_id,customer_id,currency,receipt_date,amount,payment_method,external_reference,business_note,created_by_user_id,trace_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)")
            .bind(id).bind(&number).bind(input.legal_entity_id).bind(input.customer_id).bind(&input.currency).bind(input.receipt_date).bind(amount).bind(&input.payment_method).bind(&input.external_reference).bind(&input.business_note).bind(actor).bind(trace_id).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO customer_receipt_events(id,receipt_id,event_type,amount,actor_user_id,trace_id) VALUES($1,$2,'created',$3,$4,$5)")
            .bind(Uuid::new_v4()).bind(id).bind(amount).bind(actor).bind(trace_id).execute(&mut *tx).await?;
        record(
            &mut tx,
            trace_id,
            actor,
            "CUSTOMER_RECEIPT_CREATED",
            "customer_receipt_created",
            "customer_receipt",
            id,
            json!({"receiptNumber":number,"amount":amount.to_string(),"currency":input.currency}),
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
        finish_idempotent(&mut tx, actor, "customer_receipt:create", key, &result).await?;
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
            "customer_receipt:confirm",
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
            "customer_receipt:confirm",
            key,
            &hash,
        )
        .await?
        {
            replay.idempotent_replay = true;
            tx.commit().await?;
            return Ok(replay);
        }
        let receipt=sqlx::query("SELECT receipt_number,amount,status,version FROM customer_receipts WHERE id=$1 FOR UPDATE").bind(receipt_id).fetch_one(&mut *tx).await?;
        if receipt.get::<i64, _>("version") != input.expected_version {
            return Err(DomainError::VersionConflict);
        }
        if receipt.get::<String, _>("status") != "draft" {
            return Err(DomainError::Invalid(
                "only draft receipts can be confirmed".into(),
            ));
        }
        let amount: Decimal = receipt.get("amount");
        sqlx::query("UPDATE customer_receipts SET status='confirmed',unapplied_amount=amount,confirmed_by_user_id=$2,confirmed_at=now(),trace_id=$3 WHERE id=$1").bind(receipt_id).bind(actor).bind(trace_id).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO customer_receipt_events(id,receipt_id,event_type,amount,actor_user_id,trace_id) VALUES($1,$2,'confirmed',$3,$4,$5)").bind(Uuid::new_v4()).bind(receipt_id).bind(amount).bind(actor).bind(trace_id).execute(&mut *tx).await?;
        let version = input.expected_version + 1;
        record(
            &mut tx,
            trace_id,
            actor,
            "CUSTOMER_RECEIPT_CONFIRMED",
            "customer_receipt_confirmed",
            "customer_receipt",
            receipt_id,
            json!({"version":version,"amount":amount.to_string()}),
        )
        .await?;
        let result = CommandResult {
            id: receipt_id,
            number: receipt.get("receipt_number"),
            status: "confirmed".into(),
            version,
            trace_id,
            idempotent_replay: false,
        };
        finish_idempotent(&mut tx, actor, "customer_receipt:confirm", key, &result).await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn apply_receipt(
        &self,
        actor: Uuid,
        trace_id: Uuid,
        receipt_id: Uuid,
        key: &str,
        input: &ApplyReceipt,
    ) -> Result<CommandResult, DomainError> {
        if input.allocations.is_empty() || input.allocations.len() > 100 {
            return Err(DomainError::Invalid("allocations are required".into()));
        }
        let scope = self.receipt_scope(receipt_id).await?;
        authorize(
            &self.store,
            actor,
            "receivable_allocation:create",
            Some(scope.0),
            None,
            Some(scope.1),
            None,
            None,
        )
        .await?;
        let mut seen = BTreeSet::new();
        for allocation in &input.allocations {
            if !seen.insert(allocation.receivable_id) {
                return Err(DomainError::Invalid(
                    "duplicate receivable allocation".into(),
                ));
            }
            allocation
                .amount
                .positive("allocation amount")
                .map_err(DomainError::Invalid)?;
        }
        let hash = request_hash(input)?;
        let mut tx = self.store.pool().begin().await?;
        if let Some(mut replay) = begin_idempotent::<CommandResult>(
            &mut tx,
            actor,
            "receivable_allocation:create",
            key,
            &hash,
        )
        .await?
        {
            replay.idempotent_replay = true;
            tx.commit().await?;
            return Ok(replay);
        }
        let receipt=sqlx::query("SELECT receipt_number,legal_entity_id,customer_id,currency,amount,allocated_amount,unapplied_amount,status,version FROM customer_receipts WHERE id=$1 FOR UPDATE").bind(receipt_id).fetch_one(&mut *tx).await?;
        if receipt.get::<i64, _>("version") != input.expected_receipt_version {
            return Err(DomainError::VersionConflict);
        }
        if !matches!(
            receipt.get::<String, _>("status").as_str(),
            "confirmed" | "partially_allocated"
        ) {
            return Err(DomainError::Invalid(
                "receipt has no unapplied balance".into(),
            ));
        }
        let mut total = Decimal::ZERO;
        let mut locked = Vec::new();
        let mut sorted = input.allocations.clone();
        sorted.sort_by_key(|allocation| allocation.receivable_id);
        for allocation in sorted {
            let amount = allocation.amount.0;
            let row=sqlx::query("SELECT id,legal_entity_id,customer_id,currency,open_amount,settled_amount,original_amount,status FROM trade_receivables WHERE id=$1 FOR UPDATE").bind(allocation.receivable_id).fetch_optional(&mut *tx).await?.ok_or(DomainError::NotFoundOrForbidden)?;
            if row.get::<Uuid, _>("legal_entity_id") != receipt.get::<Uuid, _>("legal_entity_id")
                || row.get::<Uuid, _>("customer_id") != receipt.get::<Uuid, _>("customer_id")
                || row.get::<String, _>("currency") != receipt.get::<String, _>("currency")
            {
                return Err(DomainError::NotFoundOrForbidden);
            }
            if !matches!(
                row.get::<String, _>("status").as_str(),
                "open" | "partially_settled"
            ) || amount > row.get::<Decimal, _>("open_amount")
            {
                return Err(DomainError::OverAllocation);
            }
            total += amount;
            locked.push((row, amount));
        }
        if total > receipt.get::<Decimal, _>("unapplied_amount") {
            return Err(DomainError::OverAllocation);
        }
        for (row, amount) in &locked {
            let id = Uuid::new_v4();
            sqlx::query("INSERT INTO receivable_allocations(id,receipt_id,receivable_id,amount,created_by_user_id,trace_id) VALUES($1,$2,$3,$4,$5,$6)").bind(id).bind(receipt_id).bind(row.get::<Uuid,_>("id")).bind(amount).bind(actor).bind(trace_id).execute(&mut *tx).await?;
            let open = row.get::<Decimal, _>("open_amount") - *amount;
            let settled = row.get::<Decimal, _>("settled_amount") + *amount;
            let status = if open == Decimal::ZERO {
                "settled"
            } else {
                "partially_settled"
            };
            sqlx::query("UPDATE trade_receivables SET open_amount=$2,settled_amount=$3,status=$4,trace_id=$5 WHERE id=$1").bind(row.get::<Uuid,_>("id")).bind(open).bind(settled).bind(status).bind(trace_id).execute(&mut *tx).await?;
            sqlx::query("INSERT INTO trade_receivable_events(id,receivable_id,event_type,amount,payload,actor_user_id,trace_id) VALUES($1,$2,'allocation_applied',$3,$4,$5,$6)").bind(Uuid::new_v4()).bind(row.get::<Uuid,_>("id")).bind(amount).bind(json!({"receiptId":receipt_id,"allocationId":id})).bind(actor).bind(trace_id).execute(&mut *tx).await?;
            record(&mut tx,trace_id,actor,"RECEIVABLE_ALLOCATION_APPLIED","receivable_allocation_applied","receivable_allocation",id,json!({"receiptId":receipt_id,"receivableId":row.get::<Uuid,_>("id"),"amount":amount.to_string()})).await?;
        }
        let allocated = receipt.get::<Decimal, _>("allocated_amount") + total;
        let unapplied = receipt.get::<Decimal, _>("unapplied_amount") - total;
        let status = if unapplied == Decimal::ZERO {
            "fully_allocated"
        } else {
            "partially_allocated"
        };
        sqlx::query("UPDATE customer_receipts SET allocated_amount=$2,unapplied_amount=$3,status=$4,trace_id=$5 WHERE id=$1").bind(receipt_id).bind(allocated).bind(unapplied).bind(status).bind(trace_id).execute(&mut *tx).await?;
        let version = input.expected_receipt_version + 1;
        let result = CommandResult {
            id: receipt_id,
            number: receipt.get("receipt_number"),
            status: status.into(),
            version,
            trace_id,
            idempotent_replay: false,
        };
        finish_idempotent(&mut tx, actor, "receivable_allocation:create", key, &result).await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn reverse_allocation(
        &self,
        actor: Uuid,
        trace_id: Uuid,
        allocation_id: Uuid,
        key: &str,
        input: &ReverseAllocation,
    ) -> Result<CommandResult, DomainError> {
        let pre=sqlx::query("SELECT r.legal_entity_id,r.customer_id FROM receivable_allocations a JOIN trade_receivables r ON r.id=a.receivable_id WHERE a.id=$1").bind(allocation_id).fetch_optional(self.store.pool()).await?.ok_or(DomainError::NotFoundOrForbidden)?;
        authorize(
            &self.store,
            actor,
            "receivable_allocation:reverse",
            Some(pre.get("legal_entity_id")),
            None,
            Some(pre.get("customer_id")),
            None,
            None,
        )
        .await?;
        let hash = request_hash(input)?;
        let mut tx = self.store.pool().begin().await?;
        if let Some(mut replay) = begin_idempotent::<CommandResult>(
            &mut tx,
            actor,
            "receivable_allocation:reverse",
            key,
            &hash,
        )
        .await?
        {
            replay.idempotent_replay = true;
            tx.commit().await?;
            return Ok(replay);
        }
        let allocation=sqlx::query("SELECT id,receipt_id,receivable_id,amount,allocation_type,status FROM receivable_allocations WHERE id=$1 FOR SHARE").bind(allocation_id).fetch_one(&mut *tx).await?;
        if allocation.get::<String, _>("allocation_type") != "apply"
            || allocation.get::<String, _>("status") != "active"
        {
            return Err(DomainError::Invalid("allocation cannot be reversed".into()));
        }
        let already: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM receivable_allocations WHERE reverses_allocation_id=$1)",
        )
        .bind(allocation_id)
        .fetch_one(&mut *tx)
        .await?;
        if already {
            return Err(DomainError::Invalid(
                "allocation is already reversed".into(),
            ));
        }
        let receipt=sqlx::query("SELECT receipt_number,amount,allocated_amount,unapplied_amount,status,version FROM customer_receipts WHERE id=$1 FOR UPDATE").bind(allocation.get::<Uuid,_>("receipt_id")).fetch_one(&mut *tx).await?;
        let receivable=sqlx::query("SELECT original_amount,settled_amount,open_amount,status,version FROM trade_receivables WHERE id=$1 FOR UPDATE").bind(allocation.get::<Uuid,_>("receivable_id")).fetch_one(&mut *tx).await?;
        if receipt.get::<i64, _>("version") != input.expected_receipt_version
            || receivable.get::<i64, _>("version") != input.expected_receivable_version
        {
            return Err(DomainError::VersionConflict);
        }
        if receipt.get::<String, _>("status") == "reversed"
            || receivable.get::<String, _>("status") == "reversed"
        {
            return Err(DomainError::Invalid(
                "reversed records cannot be allocated".into(),
            ));
        }
        let amount: Decimal = allocation.get("amount");
        let reversal = Uuid::new_v4();
        sqlx::query("INSERT INTO receivable_allocations(id,receipt_id,receivable_id,allocation_type,amount,reverses_allocation_id,created_by_user_id,trace_id) VALUES($1,$2,$3,'reversal',$4,$5,$6,$7)").bind(reversal).bind(allocation.get::<Uuid,_>("receipt_id")).bind(allocation.get::<Uuid,_>("receivable_id")).bind(amount).bind(allocation_id).bind(actor).bind(trace_id).execute(&mut *tx).await?;
        let receipt_allocated = receipt.get::<Decimal, _>("allocated_amount") - amount;
        let receipt_unapplied = receipt.get::<Decimal, _>("unapplied_amount") + amount;
        let receipt_status = if receipt_allocated == Decimal::ZERO {
            "confirmed"
        } else {
            "partially_allocated"
        };
        sqlx::query("UPDATE customer_receipts SET allocated_amount=$2,unapplied_amount=$3,status=$4,trace_id=$5 WHERE id=$1").bind(allocation.get::<Uuid,_>("receipt_id")).bind(receipt_allocated).bind(receipt_unapplied).bind(receipt_status).bind(trace_id).execute(&mut *tx).await?;
        let settled = receivable.get::<Decimal, _>("settled_amount") - amount;
        let open = receivable.get::<Decimal, _>("open_amount") + amount;
        let receivable_status = if settled == Decimal::ZERO {
            "open"
        } else {
            "partially_settled"
        };
        sqlx::query("UPDATE trade_receivables SET settled_amount=$2,open_amount=$3,status=$4,trace_id=$5 WHERE id=$1").bind(allocation.get::<Uuid,_>("receivable_id")).bind(settled).bind(open).bind(receivable_status).bind(trace_id).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO trade_receivable_events(id,receivable_id,event_type,amount,payload,actor_user_id,trace_id) VALUES($1,$2,'allocation_reversed',$3,$4,$5,$6)").bind(Uuid::new_v4()).bind(allocation.get::<Uuid,_>("receivable_id")).bind(amount).bind(json!({"receiptId":allocation.get::<Uuid,_>("receipt_id"),"allocationId":allocation_id,"reversalId":reversal})).bind(actor).bind(trace_id).execute(&mut *tx).await?;
        record(
            &mut tx,
            trace_id,
            actor,
            "RECEIVABLE_ALLOCATION_REVERSED",
            "receivable_allocation_reversed",
            "receivable_allocation",
            reversal,
            json!({"reversesAllocationId":allocation_id,"amount":amount.to_string()}),
        )
        .await?;
        let version = input.expected_receipt_version + 1;
        let result = CommandResult {
            id: allocation.get("receipt_id"),
            number: receipt.get("receipt_number"),
            status: receipt_status.into(),
            version,
            trace_id,
            idempotent_replay: false,
        };
        finish_idempotent(
            &mut tx,
            actor,
            "receivable_allocation:reverse",
            key,
            &result,
        )
        .await?;
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
            "customer_receipt:reverse",
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
            "customer_receipt:reverse",
            key,
            &hash,
        )
        .await?
        {
            replay.idempotent_replay = true;
            tx.commit().await?;
            return Ok(replay);
        }
        let receipt=sqlx::query("SELECT receipt_number,amount,allocated_amount,status,version FROM customer_receipts WHERE id=$1 FOR UPDATE").bind(receipt_id).fetch_one(&mut *tx).await?;
        if receipt.get::<i64, _>("version") != input.expected_version {
            return Err(DomainError::VersionConflict);
        }
        if receipt.get::<Decimal, _>("allocated_amount") != Decimal::ZERO {
            return Err(DomainError::Invalid(
                "reverse allocations before reversing receipt".into(),
            ));
        }
        if !matches!(
            receipt.get::<String, _>("status").as_str(),
            "confirmed" | "partially_allocated"
        ) {
            return Err(DomainError::Invalid("receipt cannot be reversed".into()));
        }
        sqlx::query("UPDATE customer_receipts SET status='reversed',unapplied_amount=0,reversed_at=now(),trace_id=$2 WHERE id=$1").bind(receipt_id).bind(trace_id).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO customer_receipt_events(id,receipt_id,event_type,amount,actor_user_id,trace_id) VALUES($1,$2,'reversed',$3,$4,$5)").bind(Uuid::new_v4()).bind(receipt_id).bind(receipt.get::<Decimal,_>("amount")).bind(actor).bind(trace_id).execute(&mut *tx).await?;
        let version = input.expected_version + 1;
        record(
            &mut tx,
            trace_id,
            actor,
            "CUSTOMER_RECEIPT_REVERSED",
            "customer_receipt_reversed",
            "customer_receipt",
            receipt_id,
            json!({"version":version}),
        )
        .await?;
        let result = CommandResult {
            id: receipt_id,
            number: receipt.get("receipt_number"),
            status: "reversed".into(),
            version,
            trace_id,
            idempotent_replay: false,
        };
        finish_idempotent(&mut tx, actor, "customer_receipt:reverse", key, &result).await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn receivables(
        &self,
        actor: Uuid,
        customer: Option<Uuid>,
        limit: i64,
    ) -> Result<Vec<ReceivableView>, DomainError> {
        let snapshot = authorize(
            &self.store,
            actor,
            "receivable:read",
            None,
            None,
            customer,
            None,
            None,
        )
        .await?;
        let rows=sqlx::query_as::<_,ReceivableView>("SELECT id,receivable_number,legal_entity_id,customer_id,sales_order_id,shipment_id,currency::text,original_amount,settled_amount,open_amount,due_date,status,(open_amount>0 AND current_date>due_date) is_overdue,GREATEST(current_date-due_date,0)::int overdue_days,updated_at,version FROM trade_receivables WHERE legal_entity_id=ANY($1) AND customer_id=ANY($2) AND ($3::uuid IS NULL OR customer_id=$3) ORDER BY due_date,id LIMIT $4").bind(snapshot.scopes.legal_entity_ids.into_iter().collect::<Vec<_>>()).bind(snapshot.scopes.customer_ids.into_iter().collect::<Vec<_>>()).bind(customer).bind(limit.clamp(1,500)).fetch_all(self.store.pool()).await?;
        Ok(rows)
    }

    pub async fn receipt(&self, actor: Uuid, id: Uuid) -> Result<ReceiptView, DomainError> {
        let scope = self.receipt_scope(id).await?;
        authorize(
            &self.store,
            actor,
            "customer_receipt:read",
            Some(scope.0),
            None,
            Some(scope.1),
            None,
            None,
        )
        .await?;
        sqlx::query_as::<_,ReceiptView>("SELECT id,receipt_number,legal_entity_id,customer_id,currency::text,receipt_date,amount,allocated_amount,unapplied_amount,status,updated_at,version FROM customer_receipts WHERE id=$1").bind(id).fetch_optional(self.store.pool()).await?.ok_or(DomainError::NotFoundOrForbidden)
    }

    pub async fn receipts(
        &self,
        actor: Uuid,
        customer: Option<Uuid>,
        limit: i64,
    ) -> Result<Vec<ReceiptView>, DomainError> {
        let snapshot = authorize(
            &self.store,
            actor,
            "customer_receipt:read",
            None,
            None,
            customer,
            None,
            None,
        )
        .await?;
        let rows=sqlx::query_as::<_,ReceiptView>("SELECT id,receipt_number,legal_entity_id,customer_id,currency::text,receipt_date,amount,allocated_amount,unapplied_amount,status,updated_at,version FROM customer_receipts WHERE legal_entity_id=ANY($1) AND customer_id=ANY($2) AND ($3::uuid IS NULL OR customer_id=$3) ORDER BY receipt_date DESC,id DESC LIMIT $4").bind(snapshot.scopes.legal_entity_ids.into_iter().collect::<Vec<_>>()).bind(snapshot.scopes.customer_ids.into_iter().collect::<Vec<_>>()).bind(customer).bind(limit.clamp(1,500)).fetch_all(self.store.pool()).await?;
        Ok(rows)
    }

    pub async fn reconcile(&self, actor: Uuid) -> Result<Vec<Value>, DomainError> {
        authorize(
            &self.store,
            actor,
            "receivable:read",
            None,
            None,
            None,
            None,
            None,
        )
        .await?;
        let rows=sqlx::query("SELECT to_jsonb(r) value FROM receivable_balance_reconciliation r WHERE settled_difference<>0 OR open_difference<>0").fetch_all(self.store.pool()).await?;
        Ok(rows.into_iter().map(|row| row.get("value")).collect())
    }

    async fn receipt_scope(&self, id: Uuid) -> Result<(Uuid, Uuid), DomainError> {
        let row =
            sqlx::query("SELECT legal_entity_id,customer_id FROM customer_receipts WHERE id=$1")
                .bind(id)
                .fetch_optional(self.store.pool())
                .await?
                .ok_or(DomainError::NotFoundOrForbidden)?;
        Ok((row.get("legal_entity_id"), row.get("customer_id")))
    }
}
