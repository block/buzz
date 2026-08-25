use crate::{
    model::AuthorizationSnapshot,
    store::{audit, outbox, PgStore},
};
use rust_decimal::{Decimal, RoundingStrategy};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum DomainError {
    #[error("not found or forbidden")]
    NotFoundOrForbidden,
    #[error("version conflict")]
    VersionConflict,
    #[error("idempotency key was reused with a different request")]
    IdempotencyConflict,
    #[error("insufficient stock")]
    InsufficientStock(Value),
    #[error("inventory cost is missing")]
    MissingInventoryCost,
    #[error("order is on manual review hold")]
    OrderOnHold,
    #[error("receivable has already been settled")]
    ReceivableAlreadySettled,
    #[error("allocation exceeds an open balance")]
    OverAllocation,
    #[error("receipt quantity exceeds purchase order remainder")]
    OverReceipt,
    #[error("subsequent inventory movements exist")]
    SubsequentInventoryMovementsExist,
    #[error("payable has active settlement")]
    PayableAlreadySettled,
    #[error("allocation preview is stale")]
    StalePreview,
    #[error("invalid input: {0}")]
    Invalid(String),
    #[error("database error")]
    Database(#[from] sqlx::Error),
    #[error("serialization error")]
    Serialization(#[from] serde_json::Error),
}

pub fn money(value: Decimal) -> Decimal {
    value.round_dp_with_strategy(6, RoundingStrategy::MidpointAwayFromZero)
}

pub fn validate_currency(value: &str) -> Result<(), DomainError> {
    if value.len() == 3 && value.bytes().all(|byte| byte.is_ascii_uppercase()) {
        Ok(())
    } else {
        Err(DomainError::Invalid(
            "currency must be ISO-4217 uppercase".into(),
        ))
    }
}

pub fn validate_idempotency_key(value: &str) -> Result<(), DomainError> {
    if (8..=128).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:-".contains(&byte))
    {
        Ok(())
    } else {
        Err(DomainError::Invalid("invalid Idempotency-Key".into()))
    }
}

pub fn request_hash<T: Serialize>(input: &T) -> Result<String, DomainError> {
    Ok(hex::encode(Sha256::digest(serde_json::to_vec(input)?)))
}

// The six optional arguments intentionally mirror B1's fixed scope dimensions.
#[allow(clippy::too_many_arguments)]
pub async fn authorize(
    store: &PgStore,
    actor: Uuid,
    permission: &str,
    legal_entity: Option<Uuid>,
    warehouse: Option<Uuid>,
    customer: Option<Uuid>,
    brand: Option<Uuid>,
    business_unit: Option<Uuid>,
) -> Result<AuthorizationSnapshot, DomainError> {
    let snapshot = store
        .snapshot(actor)
        .await
        .map_err(|_| DomainError::NotFoundOrForbidden)?;
    let allowed = snapshot.permission_keys.contains(permission)
        && legal_entity.is_none_or(|id| snapshot.scopes.legal_entity_ids.contains(&id))
        && warehouse.is_none_or(|id| snapshot.scopes.warehouse_ids.contains(&id))
        && customer.is_none_or(|id| snapshot.scopes.customer_ids.contains(&id))
        && brand.is_none_or(|id| snapshot.scopes.brand_ids.contains(&id))
        && business_unit.is_none_or(|id| snapshot.scopes.business_unit_ids.contains(&id));
    if allowed {
        Ok(snapshot)
    } else {
        Err(DomainError::NotFoundOrForbidden)
    }
}

pub async fn begin_idempotent<T: DeserializeOwned>(
    tx: &mut Transaction<'_, Postgres>,
    actor: Uuid,
    operation: &str,
    key: &str,
    hash: &str,
) -> Result<Option<T>, DomainError> {
    validate_idempotency_key(key)?;
    let inserted = sqlx::query(
        "INSERT INTO business_command_idempotency(actor_user_id,operation,idempotency_key,request_hash) VALUES($1,$2,$3,$4) ON CONFLICT DO NOTHING RETURNING actor_user_id",
    )
    .bind(actor)
    .bind(operation)
    .bind(key)
    .bind(hash)
    .fetch_optional(&mut **tx)
    .await?;
    if inserted.is_some() {
        return Ok(None);
    }
    let row = sqlx::query(
        "SELECT request_hash,response FROM business_command_idempotency WHERE actor_user_id=$1 AND operation=$2 AND idempotency_key=$3 FOR UPDATE",
    )
    .bind(actor)
    .bind(operation)
    .bind(key)
    .fetch_one(&mut **tx)
    .await?;
    if row.get::<String, _>("request_hash") != hash {
        return Err(DomainError::IdempotencyConflict);
    }
    let response = row
        .get::<Option<Value>, _>("response")
        .ok_or(DomainError::IdempotencyConflict)?;
    Ok(Some(serde_json::from_value(response)?))
}

pub async fn finish_idempotent<T: Serialize>(
    tx: &mut Transaction<'_, Postgres>,
    actor: Uuid,
    operation: &str,
    key: &str,
    result: &T,
) -> Result<(), DomainError> {
    sqlx::query(
        "UPDATE business_command_idempotency SET response=$4,completed_at=now() WHERE actor_user_id=$1 AND operation=$2 AND idempotency_key=$3",
    )
    .bind(actor)
    .bind(operation)
    .bind(key)
    .bind(serde_json::to_value(result)?)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

// Audit and outbox share one explicit envelope so no required attribution is
// hidden behind defaults at a transaction call site.
#[allow(clippy::too_many_arguments)]
pub async fn record(
    tx: &mut Transaction<'_, Postgres>,
    trace_id: Uuid,
    actor: Uuid,
    audit_operation: &str,
    topic: &str,
    aggregate_type: &str,
    aggregate_id: Uuid,
    details: Value,
) -> Result<(), DomainError> {
    audit(
        tx,
        trace_id,
        actor,
        audit_operation,
        aggregate_type,
        &aggregate_id.to_string(),
        details.clone(),
    )
    .await?;
    outbox(
        tx,
        topic,
        aggregate_type,
        &aggregate_id.to_string(),
        details,
    )
    .await?;
    Ok(())
}

pub async fn next_number(
    tx: &mut Transaction<'_, Postgres>,
    sequence: &str,
    prefix: &str,
    aggregate_id: Uuid,
    context: crate::numbering::NumberingContext,
) -> Result<String, DomainError> {
    let sql = match sequence {
        "sales_order" => "SELECT nextval('business_sales_order_number_seq')",
        "shipment" => "SELECT nextval('business_shipment_number_seq')",
        "receivable" => "SELECT nextval('business_receivable_number_seq')",
        "receipt" => "SELECT nextval('business_customer_receipt_number_seq')",
        "opening" => "SELECT nextval('business_inventory_opening_number_seq')",
        "purchase_order" => "SELECT nextval('business_purchase_order_number_seq')",
        "goods_receipt" => "SELECT nextval('business_goods_receipt_number_seq')",
        "payable" => "SELECT nextval('business_trade_payable_number_seq')",
        "supplier_payment" => "SELECT nextval('business_supplier_payment_number_seq')",
        "sales_return" => "SELECT nextval('business_sales_return_number_seq')",
        "purchase_return" => "SELECT nextval('business_purchase_return_number_seq')",
        "inventory_count" => "SELECT nextval('business_inventory_count_number_seq')",
        "purchase_requisition" => "SELECT nextval('business_purchase_requisition_number_seq')",
        "profit_adjustment" => "SELECT nextval('business_profit_adjustment_number_seq')",
        "management_report" => "SELECT nextval('business_management_report_number_seq')",
        _ => return Err(DomainError::Invalid("unsupported number sequence".into())),
    };
    // Keep the legacy sequence moving even while a governed rule is active so
    // disabling that rule can safely fall back without reusing an old number.
    let fallback_number: i64 = sqlx::query_scalar(sql).fetch_one(&mut **tx).await?;
    let configured = sqlx::query(
        "SELECT id,segments,reset_period,scope_dimension,status FROM business_numbering_rules WHERE record_type=$1",
    )
    .bind(sequence)
    .fetch_optional(&mut **tx)
    .await?;
    if let Some(rule) = configured {
        let rule_id: Uuid = rule.get("id");
        let status: String = rule.get("status");
        if status == "disabled" {
            let rendered = format!(
                "{}-{}-{fallback_number:06}",
                prefix,
                chrono::Utc::now().format("%Y%m")
            );
            sqlx::query(
                "INSERT INTO business_numbering_issuances(rule_id,record_type,aggregate_id,rendered_number,source,scope_key,period_key,sequence_value) VALUES($1,$2,$3,$4,'fallback','global','*',$5)",
            )
            .bind(rule_id)
            .bind(sequence)
            .bind(aggregate_id)
            .bind(&rendered)
            .bind(fallback_number)
            .execute(&mut **tx)
            .await?;
            return Ok(rendered);
        }
        let reset_period: String = rule.get("reset_period");
        let scope_dimension: String = rule.get("scope_dimension");
        let (scope_key, scope_code) = match scope_dimension.as_str() {
            "global" => ("global".into(), None),
            "legal_entity" => {
                let id = context.legal_entity_id.ok_or_else(|| {
                    DomainError::Invalid("legal entity numbering context is required".into())
                })?;
                let code: String = sqlx::query_scalar(
                    "SELECT code FROM business_legal_entities WHERE id=$1 AND status='active'",
                )
                .bind(id)
                .fetch_optional(&mut **tx)
                .await?
                .ok_or(DomainError::NotFoundOrForbidden)?;
                (id.to_string(), Some(code))
            }
            "business_unit" => {
                let id = context.business_unit_id.ok_or_else(|| {
                    DomainError::Invalid("business unit numbering context is required".into())
                })?;
                let code: String = sqlx::query_scalar(
                    "SELECT code FROM business_units WHERE id=$1 AND status='active'",
                )
                .bind(id)
                .fetch_optional(&mut **tx)
                .await?
                .ok_or(DomainError::NotFoundOrForbidden)?;
                (id.to_string(), Some(code))
            }
            _ => return Err(DomainError::Invalid("unsupported sequence scope".into())),
        };
        let today = chrono::Utc::now().date_naive();
        let period_key = crate::numbering::period_key(&reset_period, today)?;
        let number: i64 = sqlx::query_scalar(
            "INSERT INTO business_numbering_sequence_pools(rule_id,scope_key,period_key,current_value) VALUES($1,$2,$3,1) ON CONFLICT(rule_id,scope_key,period_key) DO UPDATE SET current_value=business_numbering_sequence_pools.current_value+1,updated_at=now() RETURNING current_value",
        )
        .bind(rule_id)
        .bind(&scope_key)
        .bind(&period_key)
        .fetch_one(&mut **tx)
        .await?;
        let segments = serde_json::from_value::<Vec<crate::numbering::NumberSegment>>(
            rule.get::<serde_json::Value, _>("segments"),
        )?;
        let rendered =
            crate::numbering::render_number(&segments, today, number, scope_code.as_deref())?;
        sqlx::query(
            "INSERT INTO business_numbering_issuances(rule_id,record_type,aggregate_id,rendered_number,source,scope_key,scope_code,period_key,sequence_value) VALUES($1,$2,$3,$4,'governed',$5,$6,$7,$8)",
        )
        .bind(rule_id)
        .bind(sequence)
        .bind(aggregate_id)
        .bind(&rendered)
        .bind(scope_key)
        .bind(scope_code)
        .bind(period_key)
        .bind(number)
        .execute(&mut **tx)
        .await?;
        return Ok(rendered);
    }
    Ok(format!(
        "{}-{}-{fallback_number:06}",
        prefix,
        chrono::Utc::now().format("%Y%m")
    ))
}
