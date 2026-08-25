use crate::{
    b2::common::{begin_idempotent, finish_idempotent, record, request_hash, DomainError},
    model::AuthorizationSnapshot,
    store::PgStore,
};
use chrono::Utc;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{AssertSqlSafe, Row};
use std::str::FromStr;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProductMasterType {
    UnitOfMeasure,
    ProductCategory,
    Brand,
    Product,
    Sku,
    UomConversion,
}

impl ProductMasterType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UnitOfMeasure => "unit_of_measure",
            Self::ProductCategory => "product_category",
            Self::Brand => "brand",
            Self::Product => "product",
            Self::Sku => "sku",
            Self::UomConversion => "uom_conversion",
        }
    }
}

impl FromStr for ProductMasterType {
    type Err = DomainError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "unit_of_measure" => Ok(Self::UnitOfMeasure),
            "product_category" => Ok(Self::ProductCategory),
            "brand" => Ok(Self::Brand),
            "product" => Ok(Self::Product),
            "sku" => Ok(Self::Sku),
            "uom_conversion" => Ok(Self::UomConversion),
            _ => Err(DomainError::Invalid(
                "unsupported product master data type".into(),
            )),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveProductMasterData {
    pub resource_type: String,
    pub code: String,
    pub name: String,
    #[serde(default)]
    pub parent_category_id: Option<Uuid>,
    #[serde(default)]
    pub category_id: Option<Uuid>,
    #[serde(default)]
    pub brand_id: Option<Uuid>,
    #[serde(default)]
    pub base_uom_id: Option<Uuid>,
    #[serde(default)]
    pub product_id: Option<Uuid>,
    #[serde(default)]
    pub unit_of_measure_id: Option<Uuid>,
    #[serde(default)]
    pub barcode: Option<String>,
    #[serde(default)]
    pub precision_scale: Option<i16>,
    #[serde(default)]
    pub allow_zero_cost: Option<bool>,
    #[serde(default)]
    pub factor_to_base: Option<Decimal>,
    #[serde(default)]
    pub usage_scope: Option<String>,
    #[serde(default)]
    pub expected_version: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChangeProductMasterStatus {
    pub status: String,
    pub expected_version: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductMasterCommandResult {
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
pub struct ProductMasterRecord {
    pub resource_type: String,
    pub id: Uuid,
    pub code: String,
    pub name: String,
    pub status: String,
    pub product_id: Option<Uuid>,
    pub product_code: Option<String>,
    pub product_name: Option<String>,
    pub category_id: Option<Uuid>,
    pub category_code: Option<String>,
    pub category_name: Option<String>,
    pub parent_category_id: Option<Uuid>,
    pub parent_category_code: Option<String>,
    pub parent_category_name: Option<String>,
    pub brand_id: Option<Uuid>,
    pub brand_code: Option<String>,
    pub brand_name: Option<String>,
    pub unit_of_measure_id: Option<Uuid>,
    pub unit_of_measure_code: Option<String>,
    pub unit_of_measure_name: Option<String>,
    pub barcode: Option<String>,
    pub precision_scale: Option<i16>,
    pub allow_zero_cost: Option<bool>,
    pub factor_to_base: Option<Decimal>,
    pub usage_scope: Option<String>,
    pub version: i64,
    pub updated_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductMasterList {
    pub items: Vec<ProductMasterRecord>,
    pub can_manage: bool,
    pub data_as_of: chrono::DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductImpactItem {
    pub code: String,
    pub label: String,
    pub count: i64,
    pub blocking: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductDisableImpact {
    pub resource_type: String,
    pub id: Uuid,
    pub code: String,
    pub name: String,
    pub status: String,
    pub version: i64,
    pub can_disable: bool,
    pub impacts: Vec<ProductImpactItem>,
    pub checked_at: chrono::DateTime<Utc>,
}

#[derive(Clone)]
pub struct ProductMasterService {
    store: PgStore,
}

impl ProductMasterService {
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
        resource_type: Option<ProductMasterType>,
        limit: i64,
    ) -> Result<ProductMasterList, DomainError> {
        let snapshot = self.snapshot(actor, "business_product_master:read").await?;
        let brands = snapshot
            .scopes
            .brand_ids
            .iter()
            .copied()
            .collect::<Vec<_>>();
        let items=sqlx::query_as::<_,ProductMasterRecord>("SELECT resource_type,id,code,name,status,product_id,product_code,product_name,category_id,category_code,category_name,parent_category_id,parent_category_code,parent_category_name,brand_id,brand_code,brand_name,unit_of_measure_id,unit_of_measure_code,unit_of_measure_name,barcode,precision_scale,allow_zero_cost,factor_to_base,usage_scope,version,updated_at FROM product_master_data_maintenance WHERE ($1::text IS NULL OR resource_type=$1) AND (brand_id IS NULL OR brand_id=ANY($2)) ORDER BY CASE resource_type WHEN 'product_category' THEN 0 WHEN 'brand' THEN 1 WHEN 'unit_of_measure' THEN 2 WHEN 'product' THEN 3 WHEN 'sku' THEN 4 ELSE 5 END,code LIMIT $3")
            .bind(resource_type.map(ProductMasterType::as_str)).bind(brands).bind(limit.clamp(1,2000)).fetch_all(self.store.pool()).await?;
        Ok(ProductMasterList {
            items,
            can_manage: snapshot
                .permission_keys
                .contains("business_product_master:manage"),
            data_as_of: Utc::now(),
        })
    }

    pub async fn save(
        &self,
        actor: Uuid,
        trace_id: Uuid,
        id: Option<Uuid>,
        key: &str,
        input: &SaveProductMasterData,
    ) -> Result<ProductMasterCommandResult, DomainError> {
        let kind = ProductMasterType::from_str(&input.resource_type)?;
        validate(input, kind, id.is_some())?;
        let snapshot = self
            .snapshot(actor, "business_product_master:manage")
            .await?;
        let hash = request_hash(&(id, input))?;
        let mut tx = self.store.pool().begin().await?;
        if let Some(mut replay) = begin_idempotent::<ProductMasterCommandResult>(
            &mut tx,
            actor,
            "product_master_data:save",
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
        if id.is_some() {
            let row = load_record(&mut tx, kind, target_id)
                .await?
                .ok_or(DomainError::NotFoundOrForbidden)?;
            if row.get::<i64, _>("version") != input.expected_version.unwrap_or(0) {
                return Err(DomainError::VersionConflict);
            }
            ensure_brand_scope(&snapshot, row.get("brand_id"))?;
            update_record(&mut tx, kind, target_id, input).await?;
        } else {
            if input.expected_version.is_some() {
                return Err(DomainError::VersionConflict);
            }
            ensure_inputs_accessible(&mut tx, &snapshot, kind, input).await?;
            insert_record(&mut tx, kind, target_id, input).await?;
            if kind == ProductMasterType::Brand {
                sqlx::query("INSERT INTO business_brand_scopes(enterprise_user_id,brand_id,granted_by) VALUES($1,$2,$1) ON CONFLICT DO NOTHING").bind(actor).bind(target_id).execute(&mut *tx).await?;
            }
        }
        let row = load_record(&mut tx, kind, target_id)
            .await?
            .ok_or(DomainError::NotFoundOrForbidden)?;
        ensure_brand_scope(&snapshot, row.get("brand_id")).or_else(|error| {
            if kind == ProductMasterType::Brand && id.is_none() {
                Ok(())
            } else {
                Err(error)
            }
        })?;
        let version: i64 = row.get("version");
        let code: String = row.get("code");
        let status: String = row.get("status");
        record(&mut tx,trace_id,actor,"PRODUCT_MASTER_DATA_SAVED","product_master_data_saved",kind.as_str(),target_id,json!({"resourceType":kind.as_str(),"code":code,"version":version,"mode":if id.is_some(){"update"}else{"create"}})).await?;
        let result = ProductMasterCommandResult {
            id: target_id,
            resource_type: kind.as_str().into(),
            code,
            status,
            version,
            trace_id,
            idempotent_replay: false,
        };
        finish_idempotent(&mut tx, actor, "product_master_data:save", key, &result).await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn impact(
        &self,
        actor: Uuid,
        kind: ProductMasterType,
        id: Uuid,
    ) -> Result<ProductDisableImpact, DomainError> {
        let snapshot = self.snapshot(actor, "business_product_master:read").await?;
        let row=sqlx::query("SELECT code,name,status,version,brand_id FROM product_master_data_maintenance WHERE resource_type=$1 AND id=$2").bind(kind.as_str()).bind(id).fetch_optional(self.store.pool()).await?.ok_or(DomainError::NotFoundOrForbidden)?;
        ensure_brand_scope(&snapshot, row.get("brand_id"))?;
        let impacts = load_impacts(self.store.pool(), kind, id).await?;
        Ok(ProductDisableImpact {
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
        kind: ProductMasterType,
        id: Uuid,
        key: &str,
        input: &ChangeProductMasterStatus,
    ) -> Result<ProductMasterCommandResult, DomainError> {
        if !matches!(input.status.as_str(), "active" | "disabled") {
            return Err(DomainError::Invalid(
                "status must be active or disabled".into(),
            ));
        }
        let snapshot = self
            .snapshot(actor, "business_product_master:manage")
            .await?;
        let hash = request_hash(&(kind.as_str(), id, input))?;
        let mut tx = self.store.pool().begin().await?;
        if let Some(mut replay) = begin_idempotent::<ProductMasterCommandResult>(
            &mut tx,
            actor,
            "product_master_data:status",
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
        let row = load_record(&mut tx, kind, id)
            .await?
            .ok_or(DomainError::NotFoundOrForbidden)?;
        ensure_brand_scope(&snapshot, row.get("brand_id"))?;
        if row.get::<i64, _>("version") != input.expected_version {
            return Err(DomainError::VersionConflict);
        }
        if input.status == "disabled" {
            let impacts = load_impacts(self.store.pool(), kind, id).await?;
            if impacts.iter().any(|item| item.blocking && item.count > 0) {
                return Err(DomainError::Invalid(
                    "product master data has blocking operational impacts".into(),
                ));
            }
        } else {
            ensure_enable_dependencies(&mut tx, kind, id).await?;
        }
        update_status(&mut tx, kind, id, &input.status).await?;
        let version = input.expected_version + 1;
        let code: String = row.get("code");
        record(
            &mut tx,
            trace_id,
            actor,
            "PRODUCT_MASTER_DATA_STATUS_CHANGED",
            "product_master_data_status_changed",
            kind.as_str(),
            id,
            json!({"status":input.status,"version":version}),
        )
        .await?;
        let result = ProductMasterCommandResult {
            id,
            resource_type: kind.as_str().into(),
            code,
            status: input.status.clone(),
            version,
            trace_id,
            idempotent_replay: false,
        };
        finish_idempotent(&mut tx, actor, "product_master_data:status", key, &result).await?;
        tx.commit().await?;
        Ok(result)
    }
}

fn ensure_brand_scope(
    snapshot: &AuthorizationSnapshot,
    brand: Option<Uuid>,
) -> Result<(), DomainError> {
    if brand.is_none_or(|id| snapshot.scopes.brand_ids.contains(&id)) {
        Ok(())
    } else {
        Err(DomainError::NotFoundOrForbidden)
    }
}

fn validate(
    input: &SaveProductMasterData,
    kind: ProductMasterType,
    updating: bool,
) -> Result<(), DomainError> {
    if kind != ProductMasterType::UomConversion {
        let code_ok = (2..=32).contains(&input.code.len())
            && input
                .code
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b"_-".contains(&b));
        if !code_ok || input.name.trim().is_empty() || input.name.chars().count() > 200 {
            return Err(DomainError::Invalid("code or name is invalid".into()));
        }
    }
    if updating && input.expected_version.is_none() {
        return Err(DomainError::Invalid("expectedVersion is required".into()));
    }
    match kind {
        ProductMasterType::UnitOfMeasure => {
            if input.precision_scale.is_none_or(|v| !(0..=6).contains(&v)) {
                return Err(DomainError::Invalid("precisionScale is invalid".into()));
            }
        }
        ProductMasterType::ProductCategory => {}
        ProductMasterType::Brand => {}
        ProductMasterType::Product => {
            if input.category_id.is_none() || input.base_uom_id.is_none() {
                return Err(DomainError::Invalid(
                    "categoryId and baseUomId are required".into(),
                ));
            }
        }
        ProductMasterType::Sku => {
            if input.product_id.is_none() {
                return Err(DomainError::Invalid("productId is required".into()));
            }
        }
        ProductMasterType::UomConversion => {
            if input.product_id.is_none()
                || input.unit_of_measure_id.is_none()
                || input.factor_to_base.is_none_or(|v| v <= Decimal::ZERO)
                || input
                    .usage_scope
                    .as_deref()
                    .is_none_or(|v| !matches!(v, "sales" | "purchase" | "both"))
            {
                return Err(DomainError::Invalid(
                    "product, unit, positive factor and usageScope are required".into(),
                ));
            }
        }
    }
    if input.barcode.as_deref().is_some_and(|value| {
        value.is_empty()
            || value.chars().count() > 64
            || !value
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || "-_".contains(ch))
    }) {
        return Err(DomainError::Invalid("barcode is invalid".into()));
    }
    Ok(())
}

async fn ensure_inputs_accessible(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    snapshot: &AuthorizationSnapshot,
    kind: ProductMasterType,
    input: &SaveProductMasterData,
) -> Result<(), DomainError> {
    if let Some(brand) = input.brand_id {
        ensure_brand_scope(snapshot, Some(brand))?;
    }
    let valid=match kind{
        ProductMasterType::UnitOfMeasure|ProductMasterType::Brand=>true,
        ProductMasterType::ProductCategory=>match input.parent_category_id{Some(id)=>sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM business_product_categories WHERE id=$1 AND status='active')").bind(id).fetch_one(&mut **tx).await?,None=>true},
        ProductMasterType::Product=>sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM business_product_categories c,business_units_of_measure u WHERE c.id=$1 AND c.status='active' AND u.id=$2 AND u.status='active' AND ($3::uuid IS NULL OR EXISTS(SELECT 1 FROM business_brands b WHERE b.id=$3 AND b.status='active')))").bind(input.category_id).bind(input.base_uom_id).bind(input.brand_id).fetch_one(&mut **tx).await?,
        ProductMasterType::Sku=>sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM business_products p WHERE p.id=$1 AND p.status='active' AND (p.brand_id IS NULL OR p.brand_id=ANY($2)))").bind(input.product_id).bind(snapshot.scopes.brand_ids.iter().copied().collect::<Vec<_>>()).fetch_one(&mut **tx).await?,
        ProductMasterType::UomConversion=>sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM business_products p JOIN business_units_of_measure u ON u.id=$2 WHERE p.id=$1 AND p.status='active' AND u.status='active' AND u.id<>p.base_uom_id AND (p.brand_id IS NULL OR p.brand_id=ANY($3)))").bind(input.product_id).bind(input.unit_of_measure_id).bind(snapshot.scopes.brand_ids.iter().copied().collect::<Vec<_>>()).fetch_one(&mut **tx).await?,
    };
    if valid {
        Ok(())
    } else {
        Err(DomainError::NotFoundOrForbidden)
    }
}

async fn insert_record(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    kind: ProductMasterType,
    id: Uuid,
    input: &SaveProductMasterData,
) -> Result<(), DomainError> {
    match kind {
        ProductMasterType::UnitOfMeasure => {
            sqlx::query("INSERT INTO business_units_of_measure(id,code,name,precision_scale) VALUES($1,$2,$3,$4)").bind(id).bind(&input.code).bind(input.name.trim()).bind(input.precision_scale.unwrap_or(2)).execute(&mut **tx).await?;
        }
        ProductMasterType::ProductCategory => {
            sqlx::query("INSERT INTO business_product_categories(id,parent_id,code,name) VALUES($1,$2,$3,$4)").bind(id).bind(input.parent_category_id).bind(&input.code).bind(input.name.trim()).execute(&mut **tx).await?;
        }
        ProductMasterType::Brand => {
            sqlx::query("INSERT INTO business_brands(id,code,name) VALUES($1,$2,$3)")
                .bind(id)
                .bind(&input.code)
                .bind(input.name.trim())
                .execute(&mut **tx)
                .await?;
        }
        ProductMasterType::Product => {
            sqlx::query("INSERT INTO business_products(id,code,name,category_id,brand_id,base_uom_id,allow_zero_cost) VALUES($1,$2,$3,$4,$5,$6,$7)").bind(id).bind(&input.code).bind(input.name.trim()).bind(input.category_id).bind(input.brand_id).bind(input.base_uom_id).bind(input.allow_zero_cost.unwrap_or(false)).execute(&mut **tx).await?;
        }
        ProductMasterType::Sku => {
            sqlx::query(
                "INSERT INTO business_skus(id,product_id,code,name,barcode) VALUES($1,$2,$3,$4,$5)",
            )
            .bind(id)
            .bind(input.product_id)
            .bind(&input.code)
            .bind(input.name.trim())
            .bind(input.barcode.as_deref().filter(|v| !v.is_empty()))
            .execute(&mut **tx)
            .await?;
        }
        ProductMasterType::UomConversion => {
            sqlx::query("INSERT INTO business_product_uom_conversions(id,product_id,unit_of_measure_id,factor_to_base,usage_scope) VALUES($1,$2,$3,$4,$5)").bind(id).bind(input.product_id).bind(input.unit_of_measure_id).bind(input.factor_to_base).bind(input.usage_scope.as_deref().unwrap_or("both")).execute(&mut **tx).await?;
        }
    }
    Ok(())
}

async fn update_record(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    kind: ProductMasterType,
    id: Uuid,
    input: &SaveProductMasterData,
) -> Result<(), DomainError> {
    match kind {
        ProductMasterType::UnitOfMeasure => {
            sqlx::query("UPDATE business_units_of_measure SET name=$2 WHERE id=$1")
                .bind(id)
                .bind(input.name.trim())
                .execute(&mut **tx)
                .await?;
        }
        ProductMasterType::ProductCategory => {
            sqlx::query("UPDATE business_product_categories SET name=$2 WHERE id=$1")
                .bind(id)
                .bind(input.name.trim())
                .execute(&mut **tx)
                .await?;
        }
        ProductMasterType::Brand => {
            sqlx::query("UPDATE business_brands SET name=$2 WHERE id=$1")
                .bind(id)
                .bind(input.name.trim())
                .execute(&mut **tx)
                .await?;
        }
        ProductMasterType::Product => {
            sqlx::query("UPDATE business_products SET name=$2,allow_zero_cost=$3 WHERE id=$1")
                .bind(id)
                .bind(input.name.trim())
                .bind(input.allow_zero_cost.unwrap_or(false))
                .execute(&mut **tx)
                .await?;
        }
        ProductMasterType::Sku => {
            sqlx::query("UPDATE business_skus SET name=$2,barcode=$3 WHERE id=$1")
                .bind(id)
                .bind(input.name.trim())
                .bind(input.barcode.as_deref().filter(|v| !v.is_empty()))
                .execute(&mut **tx)
                .await?;
        }
        ProductMasterType::UomConversion => {
            sqlx::query("UPDATE business_product_uom_conversions SET factor_to_base=$2,usage_scope=$3 WHERE id=$1").bind(id).bind(input.factor_to_base).bind(input.usage_scope.as_deref().unwrap_or("both")).execute(&mut **tx).await?;
        }
    }
    Ok(())
}

async fn load_record(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    kind: ProductMasterType,
    id: Uuid,
) -> Result<Option<sqlx::postgres::PgRow>, DomainError> {
    Ok(sqlx::query("SELECT code,status,version,brand_id FROM product_master_data_maintenance WHERE resource_type=$1 AND id=$2").bind(kind.as_str()).bind(id).fetch_optional(&mut **tx).await?)
}

async fn update_status(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    kind: ProductMasterType,
    id: Uuid,
    status: &str,
) -> Result<(), DomainError> {
    let table = match kind {
        ProductMasterType::UnitOfMeasure => "business_units_of_measure",
        ProductMasterType::ProductCategory => "business_product_categories",
        ProductMasterType::Brand => "business_brands",
        ProductMasterType::Product => "business_products",
        ProductMasterType::Sku => "business_skus",
        ProductMasterType::UomConversion => "business_product_uom_conversions",
    };
    sqlx::query(AssertSqlSafe(format!(
        "UPDATE {table} SET status=$2 WHERE id=$1"
    )))
    .bind(id)
    .bind(status)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn ensure_enable_dependencies(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    kind: ProductMasterType,
    id: Uuid,
) -> Result<(), DomainError> {
    let valid=match kind{
        ProductMasterType::UnitOfMeasure|ProductMasterType::Brand=>true,
        ProductMasterType::ProductCategory=>sqlx::query_scalar("SELECT parent_id IS NULL OR EXISTS(SELECT 1 FROM business_product_categories p WHERE p.id=c.parent_id AND p.status='active') FROM business_product_categories c WHERE c.id=$1").bind(id).fetch_one(&mut **tx).await?,
        ProductMasterType::Product=>sqlx::query_scalar("SELECT c.status='active' AND u.status='active' AND (b.id IS NULL OR b.status='active') FROM business_products p JOIN business_product_categories c ON c.id=p.category_id JOIN business_units_of_measure u ON u.id=p.base_uom_id LEFT JOIN business_brands b ON b.id=p.brand_id WHERE p.id=$1").bind(id).fetch_one(&mut **tx).await?,
        ProductMasterType::Sku=>sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM business_skus s JOIN business_products p ON p.id=s.product_id WHERE s.id=$1 AND p.status='active')").bind(id).fetch_one(&mut **tx).await?,
        ProductMasterType::UomConversion=>sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM business_product_uom_conversions x JOIN business_products p ON p.id=x.product_id JOIN business_units_of_measure u ON u.id=x.unit_of_measure_id WHERE x.id=$1 AND p.status='active' AND u.status='active')").bind(id).fetch_one(&mut **tx).await?,
    };
    if valid {
        Ok(())
    } else {
        Err(DomainError::Invalid(
            "parent product master data must be active".into(),
        ))
    }
}

async fn load_impacts(
    pool: &sqlx::PgPool,
    kind: ProductMasterType,
    id: Uuid,
) -> Result<Vec<ProductImpactItem>, DomainError> {
    let queries:&[(&str,&str,&str,bool)]=match kind{
        ProductMasterType::UnitOfMeasure=>&[("active_products","使用该基础单位的启用商品","SELECT count(*) FROM business_products WHERE base_uom_id=$1 AND status='active'",true),("active_conversions","使用该单位的启用换算","SELECT count(*) FROM business_product_uom_conversions WHERE unit_of_measure_id=$1 AND status='active'",true)],
        ProductMasterType::ProductCategory=>&[("active_children","启用中的子分类","SELECT count(*) FROM business_product_categories WHERE parent_id=$1 AND status='active'",true),("active_products","分类下的启用商品","SELECT count(*) FROM business_products WHERE category_id=$1 AND status='active'",true)],
        ProductMasterType::Brand=>&[("active_products","品牌下的启用商品","SELECT count(*) FROM business_products WHERE brand_id=$1 AND status='active'",true),("open_orders","品牌关联的未完成订单","SELECT (SELECT count(*) FROM sales_orders WHERE brand_id=$1 AND lifecycle_status IN ('draft','confirmed'))+(SELECT count(*) FROM purchase_orders WHERE brand_id=$1 AND lifecycle_status IN ('draft','confirmed'))",true)],
        ProductMasterType::Product=>&[("active_skus","启用中的 SKU","SELECT count(*) FROM business_skus WHERE product_id=$1 AND status='active'",true),("stock","商品 SKU 存在库存余额","SELECT count(*) FROM inventory_balances b JOIN business_skus s ON s.id=b.sku_id WHERE s.product_id=$1 AND (b.on_hand_quantity<>0 OR b.reserved_quantity<>0 OR b.quarantined_quantity<>0)",true),("open_orders","商品 SKU 存在未完成订单行","SELECT (SELECT count(*) FROM sales_order_lines l JOIN sales_orders o ON o.id=l.sales_order_id JOIN business_skus s ON s.id=l.sku_id WHERE s.product_id=$1 AND o.lifecycle_status IN ('draft','confirmed'))+(SELECT count(*) FROM purchase_order_lines l JOIN purchase_orders o ON o.id=l.purchase_order_id JOIN business_skus s ON s.id=l.sku_id WHERE s.product_id=$1 AND o.lifecycle_status IN ('draft','confirmed'))",true)],
        ProductMasterType::Sku=>&[("stock","存在库存余额","SELECT count(*) FROM inventory_balances WHERE sku_id=$1 AND (on_hand_quantity<>0 OR reserved_quantity<>0 OR quarantined_quantity<>0)",true),("sales_demand","未完成销售订单行","SELECT count(*) FROM sales_order_lines l JOIN sales_orders o ON o.id=l.sales_order_id WHERE l.sku_id=$1 AND o.lifecycle_status IN ('draft','confirmed')",true),("purchase_inbound","未完成采购订单行","SELECT count(*) FROM purchase_order_lines l JOIN purchase_orders o ON o.id=l.purchase_order_id WHERE l.sku_id=$1 AND o.lifecycle_status IN ('draft','confirmed')",true)],
        ProductMasterType::UomConversion=>&[("open_sales","使用该换算单位的未完成销售行","SELECT count(*) FROM business_product_uom_conversions x JOIN business_skus s ON s.product_id=x.product_id JOIN sales_order_lines l ON l.sku_id=s.id AND l.unit_of_measure_id=x.unit_of_measure_id JOIN sales_orders o ON o.id=l.sales_order_id WHERE x.id=$1 AND o.lifecycle_status IN ('draft','confirmed')",true),("open_purchase","使用该换算单位的未完成采购行","SELECT count(*) FROM business_product_uom_conversions x JOIN business_skus s ON s.product_id=x.product_id JOIN purchase_order_lines l ON l.sku_id=s.id AND l.unit_of_measure_id=x.unit_of_measure_id JOIN purchase_orders o ON o.id=l.purchase_order_id WHERE x.id=$1 AND o.lifecycle_status IN ('draft','confirmed')",true)],
    };
    let mut out = Vec::with_capacity(queries.len());
    for (code, label, sql, blocking) in queries {
        let count: i64 = sqlx::query_scalar(*sql).bind(id).fetch_one(pool).await?;
        out.push(ProductImpactItem {
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
    fn validates_conversion_factor() {
        let input = SaveProductMasterData {
            resource_type: "uom_conversion".into(),
            code: "".into(),
            name: "".into(),
            parent_category_id: None,
            category_id: None,
            brand_id: None,
            base_uom_id: None,
            product_id: Some(Uuid::nil()),
            unit_of_measure_id: Some(Uuid::nil()),
            barcode: None,
            precision_scale: None,
            allow_zero_cost: None,
            factor_to_base: Some(Decimal::ZERO),
            usage_scope: Some("both".into()),
            expected_version: None,
        };
        assert!(validate(&input, ProductMasterType::UomConversion, false).is_err());
    }
}
