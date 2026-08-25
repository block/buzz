#![forbid(unsafe_code)]

use business_auth_gateway::Store;
use business_iam::{Capability, DataScope};
use serde_json::{json, Value};
use sqlx::{postgres::PgPoolOptions, PgPool, Postgres, Transaction};
use uuid::Uuid;

const USAGE: &str = "\
business-iam-admin commands:
  principal-upsert <human|independent_agent|proxy_agent> <external-id> <display-name>
  principal-disable <external-id>
  permission-grant <external-id> <capability> [data-scope-json]
  permission-revoke <external-id> <capability>
  role-upsert <role-code> <role-name>
  role-permission-grant <role-code> <capability> [data-scope-json]
  role-permission-revoke <role-code> <capability>
  role-assign <external-id> <role-code>
  role-unassign <external-id> <role-code>
  principals-list

Required environment:
  BUSINESS_IAM_ADMIN_DATABASE_URL
  BUSINESS_IAM_ADMIN_ACTOR";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.is_empty() || args[0] == "--help" || args[0] == "-h" {
        println!("{USAGE}");
        return Ok(());
    }
    if args[0] == "--version" {
        println!("business-iam-admin {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let database_url = required_env("BUSINESS_IAM_ADMIN_DATABASE_URL")?;
    let actor = required_env("BUSINESS_IAM_ADMIN_ACTOR")?;
    validate_text(&actor, 1, 200, "BUSINESS_IAM_ADMIN_ACTOR")?;
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url)
        .await?;
    Store::migrate(&pool).await?;

    match args.as_slice() {
        [command, kind, external_id, display_name] if command == "principal-upsert" => {
            principal_upsert(&pool, &actor, kind, external_id, display_name).await?
        }
        [command, external_id] if command == "principal-disable" => {
            principal_disable(&pool, &actor, external_id).await?
        }
        [command, external_id, capability] if command == "permission-grant" => {
            permission_grant(
                &pool,
                &actor,
                external_id,
                capability,
                DataScope::Unrestricted,
            )
            .await?
        }
        [command, external_id, capability, scope] if command == "permission-grant" => {
            permission_grant(&pool, &actor, external_id, capability, parse_scope(scope)?).await?
        }
        [command, external_id, capability] if command == "permission-revoke" => {
            permission_revoke(&pool, &actor, external_id, capability).await?
        }
        [command, code, name] if command == "role-upsert" => {
            role_upsert(&pool, &actor, code, name).await?
        }
        [command, code, capability] if command == "role-permission-grant" => {
            role_permission_grant(&pool, &actor, code, capability, DataScope::Unrestricted).await?
        }
        [command, code, capability, scope] if command == "role-permission-grant" => {
            role_permission_grant(&pool, &actor, code, capability, parse_scope(scope)?).await?
        }
        [command, code, capability] if command == "role-permission-revoke" => {
            role_permission_revoke(&pool, &actor, code, capability).await?
        }
        [command, external_id, code] if command == "role-assign" => {
            role_assignment(&pool, &actor, external_id, code, true).await?
        }
        [command, external_id, code] if command == "role-unassign" => {
            role_assignment(&pool, &actor, external_id, code, false).await?
        }
        [command] if command == "principals-list" => principals_list(&pool).await?,
        _ => return Err(format!("invalid command\n\n{USAGE}").into()),
    }
    Ok(())
}

fn required_env(name: &str) -> Result<String, Box<dyn std::error::Error>> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("{name} is required").into())
}

fn validate_text(
    value: &str,
    min: usize,
    max: usize,
    field: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let length = value.chars().count();
    if !(min..=max).contains(&length) || value.chars().any(char::is_control) {
        return Err(format!("{field} is invalid").into());
    }
    Ok(())
}

fn parse_scope(value: &str) -> Result<DataScope, Box<dyn std::error::Error>> {
    let scope = serde_json::from_str::<DataScope>(value)?;
    if let DataScope::Restricted(dimensions) = &scope {
        if dimensions.is_empty()
            || dimensions.iter().any(|(name, values)| {
                name.is_empty()
                    || name.len() > 128
                    || values.is_empty()
                    || values
                        .iter()
                        .any(|value| value.is_empty() || value.len() > 200)
            })
        {
            return Err("restricted data scope is empty or invalid".into());
        }
    }
    Ok(scope)
}

async fn principal_upsert(
    pool: &PgPool,
    actor: &str,
    kind: &str,
    external_id: &str,
    display_name: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    if !matches!(kind, "human" | "independent_agent" | "proxy_agent") {
        return Err("principal kind is invalid".into());
    }
    validate_text(external_id, 1, 200, "external-id")?;
    validate_text(display_name, 1, 200, "display-name")?;
    let mut tx = pool.begin().await?;
    let id = Uuid::new_v4();
    let principal_id: Uuid = sqlx::query_scalar(
        "INSERT INTO business_iam.principals(id,kind,external_id,display_name)
         VALUES($1,$2,$3,$4)
         ON CONFLICT(kind,external_id) DO UPDATE SET
           display_name=EXCLUDED.display_name,status='active',disabled_at=NULL,
           updated_at=now(),version=business_iam.principals.version+1
         RETURNING id",
    )
    .bind(id)
    .bind(kind)
    .bind(external_id)
    .bind(display_name)
    .fetch_one(&mut *tx)
    .await?;
    audit(
        &mut tx,
        actor,
        "principal_upsert",
        json!({"principalId":principal_id,"kind":kind,"externalId":external_id}),
    )
    .await?;
    tx.commit().await?;
    println!("{}", json!({"principalId":principal_id,"status":"active"}));
    Ok(())
}

async fn principal_disable(
    pool: &PgPool,
    actor: &str,
    external_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    validate_text(external_id, 1, 200, "external-id")?;
    let mut tx = pool.begin().await?;
    let changed = sqlx::query(
        "UPDATE business_iam.principals SET
           status='disabled',disabled_at=now(),updated_at=now(),version=version+1
         WHERE external_id=$1 AND status='active'",
    )
    .bind(external_id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if changed != 1 {
        return Err("active principal not found or external id is ambiguous".into());
    }
    audit(
        &mut tx,
        actor,
        "principal_disable",
        json!({"externalId":external_id}),
    )
    .await?;
    tx.commit().await?;
    println!("{}", json!({"externalId":external_id,"status":"disabled"}));
    Ok(())
}

async fn permission_grant(
    pool: &PgPool,
    actor: &str,
    external_id: &str,
    capability: &str,
    scope: DataScope,
) -> Result<(), Box<dyn std::error::Error>> {
    let capability = Capability::parse(capability.to_owned())?;
    let scope_json = serde_json::to_value(scope)?;
    let mut tx = pool.begin().await?;
    let changed = sqlx::query(
        "INSERT INTO business_iam.principal_permissions(principal_id,permission_id,data_scope,reason)
         SELECT principal.id,permission.id,$3,'business-iam-admin'
         FROM business_iam.principals principal,business_iam.permissions permission
         WHERE principal.external_id=$1 AND principal.status='active'
           AND permission.capability=$2 AND permission.status='active'
         ON CONFLICT(principal_id,permission_id) DO UPDATE SET
           data_scope=EXCLUDED.data_scope,valid_from=now(),valid_until=NULL,
           reason=EXCLUDED.reason",
    )
    .bind(external_id)
    .bind(capability.as_str())
    .bind(&scope_json)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if changed != 1 {
        return Err("active principal or permission not found".into());
    }
    audit(
        &mut tx,
        actor,
        "permission_grant",
        json!({"externalId":external_id,"capability":capability.as_str(),"dataScope":scope_json}),
    )
    .await?;
    tx.commit().await?;
    println!(
        "{}",
        json!({"externalId":external_id,"capability":capability.as_str(),"granted":true})
    );
    Ok(())
}

async fn permission_revoke(
    pool: &PgPool,
    actor: &str,
    external_id: &str,
    capability: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let capability = Capability::parse(capability.to_owned())?;
    let mut tx = pool.begin().await?;
    let changed = sqlx::query(
        "DELETE FROM business_iam.principal_permissions grant_row
         USING business_iam.principals principal,business_iam.permissions permission
         WHERE grant_row.principal_id=principal.id AND grant_row.permission_id=permission.id
           AND principal.external_id=$1 AND permission.capability=$2",
    )
    .bind(external_id)
    .bind(capability.as_str())
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if changed != 1 {
        return Err("principal permission not found".into());
    }
    audit(
        &mut tx,
        actor,
        "permission_revoke",
        json!({"externalId":external_id,"capability":capability.as_str()}),
    )
    .await?;
    tx.commit().await?;
    println!(
        "{}",
        json!({"externalId":external_id,"capability":capability.as_str(),"granted":false})
    );
    Ok(())
}

async fn role_upsert(
    pool: &PgPool,
    actor: &str,
    code: &str,
    name: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    validate_code(code, "role-code")?;
    validate_text(name, 1, 200, "role-name")?;
    let mut tx = pool.begin().await?;
    let role_id: Uuid = sqlx::query_scalar(
        "INSERT INTO business_iam.roles(id,code,name) VALUES($1,$2,$3)
         ON CONFLICT(code) DO UPDATE SET name=EXCLUDED.name,status='active',
           updated_at=now(),version=business_iam.roles.version+1 RETURNING id",
    )
    .bind(Uuid::new_v4())
    .bind(code)
    .bind(name)
    .fetch_one(&mut *tx)
    .await?;
    audit(
        &mut tx,
        actor,
        "role_upsert",
        json!({"roleId":role_id,"code":code}),
    )
    .await?;
    tx.commit().await?;
    println!("{}", json!({"roleId":role_id,"code":code}));
    Ok(())
}

async fn role_permission_grant(
    pool: &PgPool,
    actor: &str,
    code: &str,
    capability: &str,
    scope: DataScope,
) -> Result<(), Box<dyn std::error::Error>> {
    validate_code(code, "role-code")?;
    let capability = Capability::parse(capability.to_owned())?;
    let scope_json = serde_json::to_value(scope)?;
    let mut tx = pool.begin().await?;
    let changed = sqlx::query(
        "INSERT INTO business_iam.role_permissions(role_id,permission_id,data_scope)
         SELECT role.id,permission.id,$3
         FROM business_iam.roles role,business_iam.permissions permission
         WHERE role.code=$1 AND role.status='active'
           AND permission.capability=$2 AND permission.status='active'
         ON CONFLICT(role_id,permission_id) DO UPDATE SET data_scope=EXCLUDED.data_scope",
    )
    .bind(code)
    .bind(capability.as_str())
    .bind(&scope_json)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if changed != 1 {
        return Err("active role or permission not found".into());
    }
    audit(
        &mut tx,
        actor,
        "role_permission_grant",
        json!({"role":code,"capability":capability.as_str(),"dataScope":scope_json}),
    )
    .await?;
    tx.commit().await?;
    println!(
        "{}",
        json!({"role":code,"capability":capability.as_str(),"granted":true})
    );
    Ok(())
}

async fn role_permission_revoke(
    pool: &PgPool,
    actor: &str,
    code: &str,
    capability: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    validate_code(code, "role-code")?;
    let capability = Capability::parse(capability.to_owned())?;
    let mut tx = pool.begin().await?;
    let changed = sqlx::query(
        "DELETE FROM business_iam.role_permissions grant_row
         USING business_iam.roles role,business_iam.permissions permission
         WHERE grant_row.role_id=role.id AND grant_row.permission_id=permission.id
           AND role.code=$1 AND permission.capability=$2",
    )
    .bind(code)
    .bind(capability.as_str())
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if changed != 1 {
        return Err("role permission not found".into());
    }
    audit(
        &mut tx,
        actor,
        "role_permission_revoke",
        json!({"role":code,"capability":capability.as_str()}),
    )
    .await?;
    tx.commit().await?;
    println!(
        "{}",
        json!({"role":code,"capability":capability.as_str(),"granted":false})
    );
    Ok(())
}

async fn role_assignment(
    pool: &PgPool,
    actor: &str,
    external_id: &str,
    code: &str,
    grant: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    validate_text(external_id, 1, 200, "external-id")?;
    validate_code(code, "role-code")?;
    let mut tx = pool.begin().await?;
    let changed = if grant {
        sqlx::query(
            "INSERT INTO business_iam.principal_roles(principal_id,role_id,reason)
             SELECT principal.id,role.id,'business-iam-admin'
             FROM business_iam.principals principal,business_iam.roles role
             WHERE principal.external_id=$1 AND principal.status='active'
               AND role.code=$2 AND role.status='active'
             ON CONFLICT(principal_id,role_id) DO UPDATE SET
               valid_from=now(),valid_until=NULL,reason=EXCLUDED.reason",
        )
        .bind(external_id)
        .bind(code)
        .execute(&mut *tx)
        .await?
        .rows_affected()
    } else {
        sqlx::query(
            "DELETE FROM business_iam.principal_roles assignment
             USING business_iam.principals principal,business_iam.roles role
             WHERE assignment.principal_id=principal.id AND assignment.role_id=role.id
               AND principal.external_id=$1 AND role.code=$2",
        )
        .bind(external_id)
        .bind(code)
        .execute(&mut *tx)
        .await?
        .rows_affected()
    };
    if changed != 1 {
        return Err("active principal/role or assignment not found".into());
    }
    audit(
        &mut tx,
        actor,
        if grant {
            "role_assign"
        } else {
            "role_unassign"
        },
        json!({"externalId":external_id,"role":code}),
    )
    .await?;
    tx.commit().await?;
    println!(
        "{}",
        json!({"externalId":external_id,"role":code,"assigned":grant})
    );
    Ok(())
}

async fn principals_list(pool: &PgPool) -> Result<(), Box<dyn std::error::Error>> {
    use sqlx::Row as _;
    let rows = sqlx::query(
        "SELECT id,kind,external_id,display_name,status,version
         FROM business_iam.principals ORDER BY kind,external_id",
    )
    .fetch_all(pool)
    .await?;
    let values = rows
        .into_iter()
        .map(|row| {
            json!({
                "id":row.get::<Uuid,_>("id"),
                "kind":row.get::<String,_>("kind"),
                "externalId":row.get::<String,_>("external_id"),
                "displayName":row.get::<String,_>("display_name"),
                "status":row.get::<String,_>("status"),
                "version":row.get::<i64,_>("version")
            })
        })
        .collect::<Vec<_>>();
    println!("{}", Value::Array(values));
    Ok(())
}

fn validate_code(value: &str, field: &str) -> Result<(), Box<dyn std::error::Error>> {
    let valid = (3..=128).contains(&value.len())
        && value.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || (index > 0 && b"_.:-".contains(&byte))
        });
    valid
        .then_some(())
        .ok_or_else(|| format!("{field} is invalid").into())
}

async fn audit(
    tx: &mut Transaction<'_, Postgres>,
    actor: &str,
    operation: &str,
    target: Value,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO security_audit_events(id,event_type,result,trace_id,metadata)
         VALUES($1,'BUSINESS_IAM_ADMIN_MUTATION','success',$2,
           jsonb_build_object(
             'actor',$3::text,'databasePrincipal',current_user,
             'operation',$4::text,'target',$5::jsonb
           ))",
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(actor)
    .bind(operation)
    .bind(target)
    .execute(&mut **tx)
    .await?;
    Ok(())
}
