use crate::{
    b2::common::{begin_idempotent, finish_idempotent, record, request_hash, DomainError},
    model::AuthorizationSnapshot,
    store::PgStore,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{AssertSqlSafe, Row};
use std::str::FromStr;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreMasterType {
    LegalEntity,
    BusinessUnit,
    Customer,
    Supplier,
    Warehouse,
}

impl CoreMasterType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LegalEntity => "legal_entity",
            Self::BusinessUnit => "business_unit",
            Self::Customer => "customer",
            Self::Supplier => "supplier",
            Self::Warehouse => "warehouse",
        }
    }
}

impl FromStr for CoreMasterType {
    type Err = DomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "legal_entity" => Ok(Self::LegalEntity),
            "business_unit" => Ok(Self::BusinessUnit),
            "customer" => Ok(Self::Customer),
            "supplier" => Ok(Self::Supplier),
            "warehouse" => Ok(Self::Warehouse),
            _ => Err(DomainError::Invalid(
                "unsupported core master data type".into(),
            )),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveCoreMasterData {
    pub resource_type: String,
    pub code: String,
    pub name: String,
    #[serde(default)]
    pub legal_entity_id: Option<Uuid>,
    #[serde(default)]
    pub business_unit_id: Option<Uuid>,
    #[serde(default)]
    pub country_code: Option<String>,
    #[serde(default)]
    pub functional_currency: Option<String>,
    #[serde(default)]
    pub registration_number: Option<String>,
    #[serde(default)]
    pub address: Option<String>,
    #[serde(default)]
    pub credit_currency: Option<String>,
    #[serde(default)]
    pub credit_limit_minor: Option<i64>,
    #[serde(default)]
    pub payment_terms_days: Option<i32>,
    #[serde(default)]
    pub expected_version: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChangeCoreMasterStatus {
    pub status: String,
    pub expected_version: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoreMasterCommandResult {
    pub id: Uuid,
    pub resource_type: String,
    pub code: String,
    pub status: String,
    pub version: i64,
    pub trace_id: Uuid,
    pub idempotent_replay: bool,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct CoreMasterRecord {
    pub resource_type: String,
    pub id: Uuid,
    pub code: String,
    pub name: String,
    pub status: String,
    pub legal_entity_id: Option<Uuid>,
    pub legal_entity_code: Option<String>,
    pub legal_entity_name: Option<String>,
    pub business_unit_id: Option<Uuid>,
    pub business_unit_code: Option<String>,
    pub business_unit_name: Option<String>,
    pub country_code: Option<String>,
    pub functional_currency: Option<String>,
    pub registration_number: Option<String>,
    pub address: Option<String>,
    pub credit_currency: Option<String>,
    pub credit_limit_minor: Option<i64>,
    pub payment_terms_days: Option<i32>,
    pub version: i64,
    pub updated_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoreMasterList {
    pub items: Vec<CoreMasterRecord>,
    pub can_manage: bool,
    pub data_as_of: chrono::DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImpactItem {
    pub code: String,
    pub label: String,
    pub count: i64,
    pub blocking: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DisableImpact {
    pub resource_type: String,
    pub id: Uuid,
    pub code: String,
    pub name: String,
    pub status: String,
    pub version: i64,
    pub can_disable: bool,
    pub impacts: Vec<ImpactItem>,
    pub checked_at: chrono::DateTime<Utc>,
}

#[derive(Clone)]
pub struct CoreMasterDataService {
    store: PgStore,
}

impl CoreMasterDataService {
    pub fn new(store: PgStore) -> Self {
        Self { store }
    }

    async fn snapshot(
        &self,
        actor: Uuid,
        permission: &str,
    ) -> Result<AuthorizationSnapshot, DomainError> {
        let snapshot = self
            .store
            .snapshot(actor)
            .await
            .map_err(|_| DomainError::NotFoundOrForbidden)?;
        if snapshot.permission_keys.contains(permission) {
            Ok(snapshot)
        } else {
            Err(DomainError::NotFoundOrForbidden)
        }
    }

    pub async fn list(
        &self,
        actor: Uuid,
        resource_type: Option<CoreMasterType>,
        limit: i64,
    ) -> Result<CoreMasterList, DomainError> {
        let snapshot = self.snapshot(actor, "business_master_data:read").await?;
        let entities = snapshot
            .scopes
            .legal_entity_ids
            .iter()
            .copied()
            .collect::<Vec<_>>();
        let units = snapshot
            .scopes
            .business_unit_ids
            .iter()
            .copied()
            .collect::<Vec<_>>();
        let warehouses = snapshot
            .scopes
            .warehouse_ids
            .iter()
            .copied()
            .collect::<Vec<_>>();
        let customers = snapshot
            .scopes
            .customer_ids
            .iter()
            .copied()
            .collect::<Vec<_>>();
        let suppliers = snapshot
            .scopes
            .supplier_ids
            .iter()
            .copied()
            .collect::<Vec<_>>();
        let items=sqlx::query_as::<_,CoreMasterRecord>("SELECT resource_type,id,code,name,status,legal_entity_id,legal_entity_code,legal_entity_name,business_unit_id,business_unit_code,business_unit_name,country_code,functional_currency,registration_number,address,credit_currency,credit_limit_minor,payment_terms_days,version,updated_at FROM core_master_data_maintenance WHERE ($1::text IS NULL OR resource_type=$1) AND legal_entity_id=ANY($2) AND (business_unit_id IS NULL OR business_unit_id=ANY($3)) AND (resource_type<>'warehouse' OR id=ANY($4)) AND (resource_type<>'customer' OR id=ANY($5)) AND (resource_type<>'supplier' OR id=ANY($6)) ORDER BY CASE resource_type WHEN 'legal_entity' THEN 0 WHEN 'business_unit' THEN 1 WHEN 'customer' THEN 2 WHEN 'supplier' THEN 3 ELSE 4 END,code LIMIT $7")
            .bind(resource_type.map(CoreMasterType::as_str)).bind(entities).bind(units).bind(warehouses).bind(customers).bind(suppliers).bind(limit.clamp(1,1000)).fetch_all(self.store.pool()).await?;
        Ok(CoreMasterList {
            items,
            can_manage: snapshot
                .permission_keys
                .contains("business_master_data:manage"),
            data_as_of: Utc::now(),
        })
    }

    pub async fn save(
        &self,
        actor: Uuid,
        trace_id: Uuid,
        id: Option<Uuid>,
        key: &str,
        input: &SaveCoreMasterData,
    ) -> Result<CoreMasterCommandResult, DomainError> {
        let kind = CoreMasterType::from_str(&input.resource_type)?;
        validate(input, kind, id.is_some())?;
        let snapshot = self.snapshot(actor, "business_master_data:manage").await?;
        let hash = request_hash(&(id, input))?;
        let mut tx = self.store.pool().begin().await?;
        if let Some(mut replay) = begin_idempotent::<CoreMasterCommandResult>(
            &mut tx,
            actor,
            "core_master_data:save",
            key,
            &hash,
        )
        .await?
        {
            replay.idempotent_replay = true;
            tx.commit().await?;
            return Ok(replay);
        }
        let target_id = id.unwrap_or_else(Uuid::new_v4);
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!("{}:{target_id}", kind.as_str()))
            .execute(&mut *tx)
            .await?;
        if let Some(existing_id) = id {
            let current=sqlx::query("SELECT code,status,version,legal_entity_id,business_unit_id FROM core_master_data_maintenance WHERE resource_type=$1 AND id=$2").bind(kind.as_str()).bind(existing_id).fetch_optional(&mut *tx).await?.ok_or(DomainError::NotFoundOrForbidden)?;
            if current.get::<i64, _>("version") != input.expected_version.unwrap_or(0) {
                return Err(DomainError::VersionConflict);
            }
            self.ensure_scope(
                &snapshot,
                kind,
                current.get("legal_entity_id"),
                current.get("business_unit_id"),
                existing_id,
            )?;
            update_record(&mut tx, kind, existing_id, input, actor, trace_id).await?;
        } else {
            if input.expected_version.is_some() {
                return Err(DomainError::VersionConflict);
            }
            if input
                .legal_entity_id
                .is_some_and(|value| !snapshot.scopes.legal_entity_ids.contains(&value))
                || input
                    .business_unit_id
                    .is_some_and(|value| !snapshot.scopes.business_unit_ids.contains(&value))
            {
                return Err(DomainError::NotFoundOrForbidden);
            }
            ensure_parents(&mut tx, kind, input).await?;
            insert_record(&mut tx, kind, target_id, input).await?;
            grant_creator_scope(&mut tx, kind, target_id, input, actor).await?;
        }
        let row=sqlx::query("SELECT code,status,version,legal_entity_id,business_unit_id FROM core_master_data_maintenance WHERE resource_type=$1 AND id=$2").bind(kind.as_str()).bind(target_id).fetch_one(&mut *tx).await?;
        if id.is_some() {
            self.ensure_scope(
                &snapshot,
                kind,
                row.get("legal_entity_id"),
                row.get("business_unit_id"),
                target_id,
            )?;
        }
        let version: i64 = row.get("version");
        record(&mut tx,trace_id,actor,"CORE_MASTER_DATA_SAVED","core_master_data_saved",kind.as_str(),target_id,json!({"resourceType":kind.as_str(),"code":row.get::<String,_>("code"),"version":version,"mode":if id.is_some(){"update"}else{"create"}})).await?;
        let result = CoreMasterCommandResult {
            id: target_id,
            resource_type: kind.as_str().into(),
            code: row.get("code"),
            status: row.get("status"),
            version,
            trace_id,
            idempotent_replay: false,
        };
        finish_idempotent(&mut tx, actor, "core_master_data:save", key, &result).await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn impact(
        &self,
        actor: Uuid,
        kind: CoreMasterType,
        id: Uuid,
    ) -> Result<DisableImpact, DomainError> {
        let snapshot = self.snapshot(actor, "business_master_data:read").await?;
        let row=sqlx::query("SELECT code,name,status,version,legal_entity_id,business_unit_id FROM core_master_data_maintenance WHERE resource_type=$1 AND id=$2").bind(kind.as_str()).bind(id).fetch_optional(self.store.pool()).await?.ok_or(DomainError::NotFoundOrForbidden)?;
        self.ensure_scope(
            &snapshot,
            kind,
            row.get("legal_entity_id"),
            row.get("business_unit_id"),
            id,
        )?;
        let impacts = load_impacts(self.store.pool(), kind, id).await?;
        Ok(DisableImpact {
            resource_type: kind.as_str().into(),
            id,
            code: row.get("code"),
            name: row.get("name"),
            status: row.get("status"),
            version: row.get("version"),
            can_disable: !impacts.iter().any(|item| item.blocking && item.count > 0),
            impacts,
            checked_at: Utc::now(),
        })
    }

    pub async fn change_status(
        &self,
        actor: Uuid,
        trace_id: Uuid,
        kind: CoreMasterType,
        id: Uuid,
        key: &str,
        input: &ChangeCoreMasterStatus,
    ) -> Result<CoreMasterCommandResult, DomainError> {
        if !matches!(input.status.as_str(), "active" | "disabled") {
            return Err(DomainError::Invalid(
                "status must be active or disabled".into(),
            ));
        }
        let snapshot = self.snapshot(actor, "business_master_data:manage").await?;
        let hash = request_hash(&(kind.as_str(), id, input))?;
        let mut tx = self.store.pool().begin().await?;
        if let Some(mut replay) = begin_idempotent::<CoreMasterCommandResult>(
            &mut tx,
            actor,
            "core_master_data:status",
            key,
            &hash,
        )
        .await?
        {
            replay.idempotent_replay = true;
            tx.commit().await?;
            return Ok(replay);
        }
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!("{}:{id}", kind.as_str()))
            .execute(&mut *tx)
            .await?;
        let row=sqlx::query("SELECT code,status,version,legal_entity_id,business_unit_id FROM core_master_data_maintenance WHERE resource_type=$1 AND id=$2").bind(kind.as_str()).bind(id).fetch_optional(&mut *tx).await?.ok_or(DomainError::NotFoundOrForbidden)?;
        self.ensure_scope(
            &snapshot,
            kind,
            row.get("legal_entity_id"),
            row.get("business_unit_id"),
            id,
        )?;
        if row.get::<i64, _>("version") != input.expected_version {
            return Err(DomainError::VersionConflict);
        }
        if input.status == "disabled" {
            let impacts = load_impacts(self.store.pool(), kind, id).await?;
            if impacts.iter().any(|item| item.blocking && item.count > 0) {
                return Err(DomainError::Invalid(
                    "master data has blocking operational impacts".into(),
                ));
            }
        }
        update_status(&mut tx, kind, id, &input.status).await?;
        let version = input.expected_version + 1;
        record(
            &mut tx,
            trace_id,
            actor,
            "CORE_MASTER_DATA_STATUS_CHANGED",
            "core_master_data_status_changed",
            kind.as_str(),
            id,
            json!({"status":input.status,"version":version}),
        )
        .await?;
        let result = CoreMasterCommandResult {
            id,
            resource_type: kind.as_str().into(),
            code: row.get("code"),
            status: input.status.clone(),
            version,
            trace_id,
            idempotent_replay: false,
        };
        finish_idempotent(&mut tx, actor, "core_master_data:status", key, &result).await?;
        tx.commit().await?;
        Ok(result)
    }

    fn ensure_scope(
        &self,
        s: &AuthorizationSnapshot,
        k: CoreMasterType,
        legal: Option<Uuid>,
        unit: Option<Uuid>,
        id: Uuid,
    ) -> Result<(), DomainError> {
        let ok = legal.is_some_and(|v| s.scopes.legal_entity_ids.contains(&v))
            && unit.is_none_or(|v| s.scopes.business_unit_ids.contains(&v))
            && match k {
                CoreMasterType::Warehouse => s.scopes.warehouse_ids.contains(&id),
                CoreMasterType::Customer => s.scopes.customer_ids.contains(&id),
                CoreMasterType::Supplier => s.scopes.supplier_ids.contains(&id),
                _ => true,
            };
        if ok {
            Ok(())
        } else {
            Err(DomainError::NotFoundOrForbidden)
        }
    }
}

fn validate(i: &SaveCoreMasterData, k: CoreMasterType, updating: bool) -> Result<(), DomainError> {
    let code_ok = (2..=32).contains(&i.code.len())
        && i.code
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b"_-".contains(&b));
    if !code_ok || i.name.trim().is_empty() || i.name.chars().count() > 200 {
        return Err(DomainError::Invalid("code or name is invalid".into()));
    }
    if updating && i.expected_version.is_none() {
        return Err(DomainError::Invalid("expectedVersion is required".into()));
    }
    if i.payment_terms_days
        .is_some_and(|v| !(0..=3650).contains(&v))
        || i.credit_limit_minor.is_some_and(|v| v < 0)
    {
        return Err(DomainError::Invalid(
            "terms or credit limit is invalid".into(),
        ));
    }
    match k {
        CoreMasterType::LegalEntity => {
            currency(i.functional_currency.as_deref())?;
            if i.country_code
                .as_deref()
                .is_none_or(|v| v.len() != 2 || !v.bytes().all(|b| b.is_ascii_uppercase()))
            {
                return Err(DomainError::Invalid("countryCode is invalid".into()));
            }
        }
        CoreMasterType::BusinessUnit => {
            if i.legal_entity_id.is_none() {
                return Err(DomainError::Invalid("legalEntityId is required".into()));
            }
        }
        CoreMasterType::Customer => {
            if i.legal_entity_id.is_none() || i.business_unit_id.is_none() {
                return Err(DomainError::Invalid(
                    "legalEntityId and businessUnitId are required".into(),
                ));
            }
            currency(i.credit_currency.as_deref())?;
        }
        CoreMasterType::Supplier | CoreMasterType::Warehouse => {
            if i.legal_entity_id.is_none() || i.business_unit_id.is_none() {
                return Err(DomainError::Invalid(
                    "legalEntityId and businessUnitId are required".into(),
                ));
            }
        }
    }
    Ok(())
}
fn currency(v: Option<&str>) -> Result<(), DomainError> {
    if v.is_some_and(|v| v.len() == 3 && v.bytes().all(|b| b.is_ascii_uppercase())) {
        Ok(())
    } else {
        Err(DomainError::Invalid("currency is invalid".into()))
    }
}

async fn ensure_parents(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    k: CoreMasterType,
    i: &SaveCoreMasterData,
) -> Result<(), DomainError> {
    if k != CoreMasterType::LegalEntity {
        let legal = i
            .legal_entity_id
            .ok_or_else(|| DomainError::Invalid("legalEntityId is required".into()))?;
        let unit = i.business_unit_id;
        let ok:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM business_legal_entities e WHERE e.id=$1 AND e.status='active' AND ($2::uuid IS NULL OR EXISTS(SELECT 1 FROM business_units u WHERE u.id=$2 AND u.legal_entity_id=e.id AND u.status='active')))").bind(legal).bind(unit).fetch_one(&mut **tx).await?;
        if !ok {
            return Err(DomainError::NotFoundOrForbidden);
        }
    }
    Ok(())
}

async fn insert_record(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    k: CoreMasterType,
    id: Uuid,
    i: &SaveCoreMasterData,
) -> Result<(), DomainError> {
    match k {
        CoreMasterType::LegalEntity => {
            sqlx::query("INSERT INTO business_legal_entities(id,code,name,country_code,functional_currency,registration_number) VALUES($1,$2,$3,$4,$5,$6)").bind(id).bind(&i.code).bind(i.name.trim()).bind(i.country_code.as_deref()).bind(i.functional_currency.as_deref()).bind(&i.registration_number).execute(&mut **tx).await?;
        }
        CoreMasterType::BusinessUnit => {
            sqlx::query(
                "INSERT INTO business_units(id,legal_entity_id,code,name) VALUES($1,$2,$3,$4)",
            )
            .bind(id)
            .bind(i.legal_entity_id)
            .bind(&i.code)
            .bind(i.name.trim())
            .execute(&mut **tx)
            .await?;
        }
        CoreMasterType::Customer => {
            sqlx::query("INSERT INTO business_customers(id,legal_entity_id,business_unit_id,code,name,credit_currency,credit_limit_minor,payment_terms_days) VALUES($1,$2,$3,$4,$5,$6,$7,$8)").bind(id).bind(i.legal_entity_id).bind(i.business_unit_id).bind(&i.code).bind(i.name.trim()).bind(i.credit_currency.as_deref()).bind(i.credit_limit_minor.unwrap_or(0)).bind(i.payment_terms_days.unwrap_or(30)).execute(&mut **tx).await?;
        }
        CoreMasterType::Supplier => {
            sqlx::query("INSERT INTO business_suppliers(id,legal_entity_id,business_unit_id,code,name,payment_terms_days) VALUES($1,$2,$3,$4,$5,$6)").bind(id).bind(i.legal_entity_id).bind(i.business_unit_id).bind(&i.code).bind(i.name.trim()).bind(i.payment_terms_days.unwrap_or(30)).execute(&mut **tx).await?;
        }
        CoreMasterType::Warehouse => {
            sqlx::query("INSERT INTO business_warehouses(id,legal_entity_id,business_unit_id,code,name,address) VALUES($1,$2,$3,$4,$5,$6)").bind(id).bind(i.legal_entity_id).bind(i.business_unit_id).bind(&i.code).bind(i.name.trim()).bind(&i.address).execute(&mut **tx).await?;
        }
    }
    Ok(())
}

async fn update_record(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    k: CoreMasterType,
    id: Uuid,
    i: &SaveCoreMasterData,
    actor: Uuid,
    trace: Uuid,
) -> Result<(), DomainError> {
    let _ = (actor, trace);
    match k {
        CoreMasterType::LegalEntity => {
            sqlx::query("UPDATE business_legal_entities SET name=$2,country_code=$3,functional_currency=$4,registration_number=$5,version=version+1,updated_at=now() WHERE id=$1").bind(id).bind(i.name.trim()).bind(i.country_code.as_deref()).bind(i.functional_currency.as_deref()).bind(&i.registration_number).execute(&mut **tx).await?;
        }
        CoreMasterType::BusinessUnit => {
            sqlx::query(
                "UPDATE business_units SET name=$2,version=version+1,updated_at=now() WHERE id=$1",
            )
            .bind(id)
            .bind(i.name.trim())
            .execute(&mut **tx)
            .await?;
        }
        CoreMasterType::Customer => {
            sqlx::query("UPDATE business_customers SET name=$2,credit_currency=$3,credit_limit_minor=$4,payment_terms_days=$5,version=version+1,updated_at=now() WHERE id=$1").bind(id).bind(i.name.trim()).bind(i.credit_currency.as_deref()).bind(i.credit_limit_minor.unwrap_or(0)).bind(i.payment_terms_days.unwrap_or(30)).execute(&mut **tx).await?;
        }
        CoreMasterType::Supplier => {
            sqlx::query("UPDATE business_suppliers SET name=$2,payment_terms_days=$3,version=version+1,updated_at=now() WHERE id=$1").bind(id).bind(i.name.trim()).bind(i.payment_terms_days.unwrap_or(30)).execute(&mut **tx).await?;
        }
        CoreMasterType::Warehouse => {
            sqlx::query("UPDATE business_warehouses SET name=$2,address=$3,version=version+1,updated_at=now() WHERE id=$1").bind(id).bind(i.name.trim()).bind(&i.address).execute(&mut **tx).await?;
        }
    }
    Ok(())
}

async fn update_status(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    k: CoreMasterType,
    id: Uuid,
    status: &str,
) -> Result<(), DomainError> {
    let table = match k {
        CoreMasterType::LegalEntity => "business_legal_entities",
        CoreMasterType::BusinessUnit => "business_units",
        CoreMasterType::Customer => "business_customers",
        CoreMasterType::Supplier => "business_suppliers",
        CoreMasterType::Warehouse => "business_warehouses",
    };
    let sql =
        format!("UPDATE {table} SET status=$2,version=version+1,updated_at=now() WHERE id=$1");
    sqlx::query(AssertSqlSafe(sql))
        .bind(id)
        .bind(status)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn grant_creator_scope(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    k: CoreMasterType,
    id: Uuid,
    i: &SaveCoreMasterData,
    actor: Uuid,
) -> Result<(), DomainError> {
    if let Some(le) = if k == CoreMasterType::LegalEntity {
        Some(id)
    } else {
        i.legal_entity_id
    } {
        sqlx::query("INSERT INTO business_legal_entity_scopes(enterprise_user_id,legal_entity_id,granted_by) VALUES($1,$2,$1) ON CONFLICT DO NOTHING").bind(actor).bind(le).execute(&mut **tx).await?;
    }
    if let Some(bu) = if k == CoreMasterType::BusinessUnit {
        Some(id)
    } else {
        i.business_unit_id
    } {
        sqlx::query("INSERT INTO business_unit_scopes(enterprise_user_id,business_unit_id,granted_by) VALUES($1,$2,$1) ON CONFLICT DO NOTHING").bind(actor).bind(bu).execute(&mut **tx).await?;
    }
    let table = match k {
        CoreMasterType::Warehouse => Some("business_warehouse_scopes"),
        CoreMasterType::Customer => Some("business_customer_scopes"),
        CoreMasterType::Supplier => Some("business_supplier_scopes"),
        _ => None,
    };
    if let Some(table) = table {
        let column = format!("{}_id", k.as_str());
        let sql=format!("INSERT INTO {table}(enterprise_user_id,{column},granted_by) VALUES($1,$2,$1) ON CONFLICT DO NOTHING");
        sqlx::query(AssertSqlSafe(sql))
            .bind(actor)
            .bind(id)
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

async fn load_impacts(
    pool: &sqlx::PgPool,
    k: CoreMasterType,
    id: Uuid,
) -> Result<Vec<ImpactItem>, DomainError> {
    let queries:&[(&str,&str,&str,bool)]=match k{
CoreMasterType::LegalEntity=>&[("active_units","启用中的经营主体","SELECT count(*) FROM business_units WHERE legal_entity_id=$1 AND status='active'",true),("open_sales","未完成销售订单","SELECT count(*) FROM sales_orders WHERE legal_entity_id=$1 AND lifecycle_status IN ('draft','confirmed')",true),("open_purchase","未完成采购订单","SELECT count(*) FROM purchase_orders WHERE legal_entity_id=$1 AND lifecycle_status IN ('draft','confirmed')",true),("stock","存在库存的商品仓位","SELECT count(*) FROM inventory_balances WHERE legal_entity_id=$1 AND (on_hand_quantity<>0 OR reserved_quantity<>0 OR quarantined_quantity<>0)",true)],
CoreMasterType::BusinessUnit=>&[("active_warehouses","启用中的仓库","SELECT count(*) FROM business_warehouses WHERE business_unit_id=$1 AND status='active'",true),("active_partners","启用中的客户或供应商","SELECT (SELECT count(*) FROM business_customers WHERE business_unit_id=$1 AND status='active')+(SELECT count(*) FROM business_suppliers WHERE business_unit_id=$1 AND status='active')",true),("open_orders","未完成销售或采购订单","SELECT (SELECT count(*) FROM sales_orders WHERE business_unit_id=$1 AND lifecycle_status IN ('draft','confirmed'))+(SELECT count(*) FROM purchase_orders WHERE business_unit_id=$1 AND lifecycle_status IN ('draft','confirmed'))",true)],
CoreMasterType::Customer=>&[("open_orders","未完成销售订单","SELECT count(*) FROM sales_orders WHERE customer_id=$1 AND lifecycle_status IN ('draft','confirmed')",true),("open_receivables","未结经营应收","SELECT count(*) FROM trade_receivables WHERE customer_id=$1 AND status IN ('open','partially_settled')",false)],
CoreMasterType::Supplier=>&[("open_orders","未完成采购订单","SELECT count(*) FROM purchase_orders WHERE supplier_id=$1 AND lifecycle_status IN ('draft','confirmed')",true),("open_payables","未结经营应付","SELECT count(*) FROM trade_payables WHERE supplier_id=$1 AND status IN ('open','partially_settled')",false),("inbound_lines","仍有在途数量的采购行","SELECT count(*) FROM purchase_order_lines l JOIN purchase_orders o ON o.id=l.purchase_order_id WHERE o.supplier_id=$1 AND o.lifecycle_status='confirmed' AND l.ordered_quantity>l.received_quantity+l.cancelled_quantity",true)],
CoreMasterType::Warehouse=>&[("stock","存在余额的库存记录","SELECT count(*) FROM inventory_balances WHERE warehouse_id=$1 AND (on_hand_quantity<>0 OR reserved_quantity<>0 OR quarantined_quantity<>0)",true),("sales_demand","未完成销售订单行","SELECT count(*) FROM sales_order_lines l JOIN sales_orders o ON o.id=l.sales_order_id WHERE l.warehouse_id=$1 AND o.lifecycle_status IN ('draft','confirmed') AND l.ordered_quantity>l.shipped_quantity+l.cancelled_quantity",true),("purchase_inbound","未完成采购订单行","SELECT count(*) FROM purchase_order_lines l JOIN purchase_orders o ON o.id=l.purchase_order_id WHERE l.warehouse_id=$1 AND o.lifecycle_status='confirmed' AND l.ordered_quantity>l.received_quantity+l.cancelled_quantity",true),("inventory_counts","进行中的盘点任务","SELECT count(*) FROM inventory_count_tasks WHERE warehouse_id=$1 AND status IN ('counting','counted')",true)]};
    let mut out = Vec::with_capacity(queries.len());
    for (code, label, sql, blocking) in queries {
        let count: i64 = sqlx::query_scalar(*sql).bind(id).fetch_one(pool).await?;
        out.push(ImpactItem {
            code: (*code).into(),
            label: (*label).into(),
            count,
            blocking: *blocking,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_lowercase_code() {
        let i = SaveCoreMasterData {
            resource_type: "warehouse".into(),
            code: "bad".into(),
            name: "A".into(),
            legal_entity_id: Some(Uuid::nil()),
            business_unit_id: Some(Uuid::nil()),
            country_code: None,
            functional_currency: None,
            registration_number: None,
            address: None,
            credit_currency: None,
            credit_limit_minor: None,
            payment_terms_days: None,
            expected_version: None,
        };
        assert!(validate(&i, CoreMasterType::Warehouse, false).is_err());
    }
}
