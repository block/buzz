use crate::{
    model::{BootstrapRequest, BootstrapResponse, ScopeDimension},
    security::{safe_text, valid_code, valid_key},
    store::{audit, outbox, PgStore, StoreError},
};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

impl PgStore {
    pub async fn bootstrap(
        &self,
        actor: Uuid,
        trace_id: Uuid,
        input: &BootstrapRequest,
    ) -> Result<BootstrapResponse, StoreError> {
        validate(input)?;
        if !self.active_user(actor).await? {
            return Err(StoreError::NotFoundOrForbidden);
        }
        let mut tx = self.pool().begin().await?;
        let already_initialized: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM business_group_profile)")
                .fetch_one(&mut *tx)
                .await?;
        if already_initialized {
            return Err(StoreError::Conflict);
        }
        insert_master_data(&mut tx, input).await?;
        insert_authorization(&mut tx, actor, input).await?;
        audit(
            &mut tx,
            trace_id,
            actor,
            "business_core_bootstrapped",
            "group_profile",
            &input.group.id.to_string(),
            serde_json::json!({
                "legalEntities": input.legal_entities.len(),
                "warehouses": input.warehouses.len(),
                "customers": input.customers.len(),
                "suppliers": input.suppliers.len(),
                "skus": input.skus.len(),
                "roles": input.roles.len()
            }),
        )
        .await?;
        outbox(
            &mut tx,
            "business.master_data.bootstrapped",
            "group_profile",
            &input.group.id.to_string(),
            serde_json::json!({"version": 1}),
        )
        .await?;
        let authorization_revision = sqlx::query_scalar(
            "SELECT revision FROM business_authorization_revision WHERE singleton",
        )
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(BootstrapResponse {
            group_id: input.group.id,
            authorization_revision,
        })
    }
}

fn validate(input: &BootstrapRequest) -> Result<(), StoreError> {
    if !valid_code(&input.group.code)
        || !safe_text(&input.group.name, 160)
        || input.group.base_currency.len() != 3
        || !input
            .group
            .base_currency
            .bytes()
            .all(|value| value.is_ascii_uppercase())
        || !safe_text(&input.group.timezone, 64)
    {
        return Err(StoreError::Invalid("invalid group profile".into()));
    }
    if input.legal_entities.is_empty()
        || input.business_units.is_empty()
        || input.units_of_measure.is_empty()
        || input.roles.is_empty()
    {
        return Err(StoreError::Invalid(
            "legalEntities, businessUnits, unitsOfMeasure, and roles are required".into(),
        ));
    }
    let valid_named = |code: &str, name: &str, max| valid_code(code) && safe_text(name, max);
    if input
        .legal_entities
        .iter()
        .any(|item| !valid_named(&item.code, &item.name, 160))
        || input
            .business_units
            .iter()
            .any(|item| !valid_named(&item.code, &item.name, 160))
        || input
            .warehouses
            .iter()
            .any(|item| !valid_named(&item.code, &item.name, 160))
        || input
            .customers
            .iter()
            .any(|item| !valid_named(&item.code, &item.name, 200) || item.credit_limit_minor < 0)
        || input
            .suppliers
            .iter()
            .any(|item| !valid_named(&item.code, &item.name, 200))
        || input
            .products
            .iter()
            .any(|item| !valid_named(&item.code, &item.name, 200))
        || input
            .skus
            .iter()
            .any(|item| !valid_named(&item.code, &item.name, 200))
    {
        return Err(StoreError::Invalid(
            "invalid master-data code or name".into(),
        ));
    }
    if input.roles.iter().any(|role| {
        !valid_key(&role.role_key, 64)
            || !safe_text(&role.name, 120)
            || role
                .permission_keys
                .iter()
                .any(|permission| !valid_key(permission, 96))
    }) || input.assignment_policies.iter().any(|policy| {
        !valid_key(&policy.action_code, 96)
            || !valid_key(&policy.required_permission, 96)
            || policy.eligible_role_keys.is_empty()
    }) || input.approval_policies.iter().any(|policy| {
        !valid_key(&policy.action_code, 96)
            || !valid_key(&policy.required_permission, 96)
            || policy.eligible_role_keys.is_empty()
            || !(1..=10).contains(&policy.min_approvers)
            || policy.step_up_amount_minor.is_some_and(|value| value < 0)
    }) {
        return Err(StoreError::Invalid("invalid role or policy".into()));
    }
    Ok(())
}

async fn insert_master_data(
    tx: &mut Transaction<'_, Postgres>,
    input: &BootstrapRequest,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO business_group_profile(id,code,name,base_currency,timezone) VALUES($1,$2,$3,$4,$5)")
        .bind(input.group.id).bind(&input.group.code).bind(&input.group.name).bind(&input.group.base_currency).bind(&input.group.timezone).execute(&mut **tx).await?;
    for item in &input.legal_entities {
        sqlx::query("INSERT INTO business_legal_entities(id,code,name,country_code,functional_currency,registration_number) VALUES($1,$2,$3,$4,$5,$6)")
            .bind(item.id).bind(&item.code).bind(&item.name).bind(&item.country_code).bind(&item.functional_currency).bind(&item.registration_number).execute(&mut **tx).await?;
    }
    for item in &input.ledger_books {
        sqlx::query("INSERT INTO business_ledger_books(id,legal_entity_id,code,name,currency,fiscal_year_start_month,is_primary) VALUES($1,$2,$3,$4,$5,$6,$7)")
            .bind(item.id).bind(item.legal_entity_id).bind(&item.code).bind(&item.name).bind(&item.currency).bind(item.fiscal_year_start_month).bind(item.is_primary).execute(&mut **tx).await?;
    }
    for item in &input.business_units {
        sqlx::query("INSERT INTO business_units(id,legal_entity_id,code,name) VALUES($1,$2,$3,$4)")
            .bind(item.id)
            .bind(item.legal_entity_id)
            .bind(&item.code)
            .bind(&item.name)
            .execute(&mut **tx)
            .await?;
    }
    for item in &input.departments {
        sqlx::query(
            "INSERT INTO business_departments(id,business_unit_id,code,name) VALUES($1,$2,$3,$4)",
        )
        .bind(item.id)
        .bind(item.business_unit_id)
        .bind(&item.code)
        .bind(&item.name)
        .execute(&mut **tx)
        .await?;
    }
    for item in &input.units_of_measure {
        sqlx::query("INSERT INTO business_units_of_measure(id,code,name,precision_scale) VALUES($1,$2,$3,$4)")
            .bind(item.id).bind(&item.code).bind(&item.name).bind(item.precision_scale).execute(&mut **tx).await?;
    }
    for item in &input.product_categories {
        sqlx::query(
            "INSERT INTO business_product_categories(id,parent_id,code,name) VALUES($1,$2,$3,$4)",
        )
        .bind(item.id)
        .bind(item.parent_id)
        .bind(&item.code)
        .bind(&item.name)
        .execute(&mut **tx)
        .await?;
    }
    for item in &input.brands {
        sqlx::query("INSERT INTO business_brands(id,code,name) VALUES($1,$2,$3)")
            .bind(item.id)
            .bind(&item.code)
            .bind(&item.name)
            .execute(&mut **tx)
            .await?;
    }
    for item in &input.warehouses {
        sqlx::query("INSERT INTO business_warehouses(id,legal_entity_id,business_unit_id,code,name,address) VALUES($1,$2,$3,$4,$5,$6)")
            .bind(item.id).bind(item.legal_entity_id).bind(item.business_unit_id).bind(&item.code).bind(&item.name).bind(&item.address).execute(&mut **tx).await?;
    }
    for item in &input.customers {
        sqlx::query("INSERT INTO business_customers(id,legal_entity_id,business_unit_id,code,name,credit_currency,credit_limit_minor) VALUES($1,$2,$3,$4,$5,$6,$7)")
            .bind(item.id).bind(item.legal_entity_id).bind(item.business_unit_id).bind(&item.code).bind(&item.name).bind(&item.credit_currency).bind(item.credit_limit_minor).execute(&mut **tx).await?;
    }
    for item in &input.suppliers {
        sqlx::query("INSERT INTO business_suppliers(id,legal_entity_id,business_unit_id,code,name) VALUES($1,$2,$3,$4,$5)")
            .bind(item.id).bind(item.legal_entity_id).bind(item.business_unit_id).bind(&item.code).bind(&item.name).execute(&mut **tx).await?;
    }
    for item in &input.products {
        sqlx::query("INSERT INTO business_products(id,code,name,category_id,brand_id,base_uom_id) VALUES($1,$2,$3,$4,$5,$6)")
            .bind(item.id).bind(&item.code).bind(&item.name).bind(item.category_id).bind(item.brand_id).bind(item.base_uom_id).execute(&mut **tx).await?;
    }
    for item in &input.skus {
        sqlx::query(
            "INSERT INTO business_skus(id,product_id,code,name,barcode) VALUES($1,$2,$3,$4,$5)",
        )
        .bind(item.id)
        .bind(item.product_id)
        .bind(&item.code)
        .bind(&item.name)
        .bind(&item.barcode)
        .execute(&mut **tx)
        .await?;
    }
    for item in &input.salespeople {
        sqlx::query("INSERT INTO business_salespeople(id,enterprise_user_id,business_unit_id,department_id,code,name) VALUES($1,$2,$3,$4,$5,$6)")
            .bind(item.id).bind(item.enterprise_user_id).bind(item.business_unit_id).bind(item.department_id).bind(&item.code).bind(&item.name).execute(&mut **tx).await?;
    }
    Ok(())
}

async fn insert_authorization(
    tx: &mut Transaction<'_, Postgres>,
    actor: Uuid,
    input: &BootstrapRequest,
) -> Result<(), sqlx::Error> {
    for role in &input.roles {
        sqlx::query("INSERT INTO business_roles(id,role_key,name,description) VALUES($1,$2,$3,$4)")
            .bind(role.id)
            .bind(&role.role_key)
            .bind(&role.name)
            .bind(&role.description)
            .execute(&mut **tx)
            .await?;
        for permission in &role.permission_keys {
            sqlx::query(
                "INSERT INTO business_role_permissions(role_id,permission_key) VALUES($1,$2)",
            )
            .bind(role.id)
            .bind(permission)
            .execute(&mut **tx)
            .await?;
        }
    }
    for assignment in &input.user_roles {
        sqlx::query("INSERT INTO business_user_roles(enterprise_user_id,role_id,assigned_by) VALUES($1,$2,$3)")
            .bind(assignment.enterprise_user_id).bind(assignment.role_id).bind(actor).execute(&mut **tx).await?;
    }
    for grant in &input.scopes {
        insert_scope(
            tx,
            actor,
            grant.enterprise_user_id,
            grant.dimension,
            grant.resource_id,
        )
        .await?;
    }
    for policy in &input.assignment_policies {
        sqlx::query("INSERT INTO business_assignment_policies(action_code,required_permission,eligible_role_keys) VALUES($1,$2,$3)")
            .bind(&policy.action_code).bind(&policy.required_permission).bind(&policy.eligible_role_keys).execute(&mut **tx).await?;
    }
    for policy in &input.approval_policies {
        sqlx::query("INSERT INTO business_approval_policies(action_code,required_permission,eligible_role_keys,min_approvers,allow_self_approval,require_distinct_business_unit,step_up_amount_minor) VALUES($1,$2,$3,$4,$5,$6,$7)")
            .bind(&policy.action_code).bind(&policy.required_permission).bind(&policy.eligible_role_keys).bind(policy.min_approvers).bind(policy.allow_self_approval).bind(policy.require_distinct_business_unit).bind(policy.step_up_amount_minor).execute(&mut **tx).await?;
    }
    Ok(())
}

async fn insert_scope(
    tx: &mut Transaction<'_, Postgres>,
    actor: Uuid,
    user: Uuid,
    dimension: ScopeDimension,
    resource: Uuid,
) -> Result<(), sqlx::Error> {
    let sql = match dimension {
        ScopeDimension::LegalEntity => "INSERT INTO business_legal_entity_scopes(enterprise_user_id,legal_entity_id,granted_by) VALUES($1,$2,$3)",
        ScopeDimension::Warehouse => "INSERT INTO business_warehouse_scopes(enterprise_user_id,warehouse_id,granted_by) VALUES($1,$2,$3)",
        ScopeDimension::Customer => "INSERT INTO business_customer_scopes(enterprise_user_id,customer_id,granted_by) VALUES($1,$2,$3)",
        ScopeDimension::Supplier => "INSERT INTO business_supplier_scopes(enterprise_user_id,supplier_id,granted_by) VALUES($1,$2,$3)",
        ScopeDimension::Brand => "INSERT INTO business_brand_scopes(enterprise_user_id,brand_id,granted_by) VALUES($1,$2,$3)",
        ScopeDimension::BusinessUnit => "INSERT INTO business_unit_scopes(enterprise_user_id,business_unit_id,granted_by) VALUES($1,$2,$3)",
    };
    sqlx::query(sql)
        .bind(user)
        .bind(resource)
        .bind(actor)
        .execute(&mut **tx)
        .await?;
    Ok(())
}
