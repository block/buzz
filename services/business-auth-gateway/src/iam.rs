use crate::store::{Rejection, Store};
use business_iam::{
    evaluate, Authority, AuthorizationRequest, Capability, DataScope, Entitlement, Obligation,
    PrincipalKind, PrincipalStatus,
};
use serde_json::Value;
use sqlx::{Postgres, Row, Transaction};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

pub(crate) struct GrantedTurn {
    pub decision_id: Uuid,
    pub agent_principal_id: Uuid,
    pub scopes: Vec<String>,
    pub effective_grants: Value,
}

pub(crate) enum TurnDecision {
    Granted(GrantedTurn),
    Denied(&'static str),
}

struct ResolvedAuthority {
    id: Uuid,
    authority: Authority,
}

impl Store {
    pub(crate) async fn authorize_agent_turn_tx(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        enterprise_user_id: Uuid,
        agent_id: &str,
        agent_turn_id: &str,
        requested_scopes: &[String],
        trace_id: Uuid,
    ) -> Result<TurnDecision, Rejection> {
        let Some(agent) = load_agent_authority(tx, agent_id).await? else {
            return Ok(TurnDecision::Denied("iam_agent_not_registered"));
        };
        let human = load_authority(
            tx,
            "human",
            &enterprise_user_id.to_string(),
            PrincipalKind::Human,
        )
        .await?;
        let requested = requested_scopes
            .iter()
            .map(|scope| {
                Capability::parse(scope.clone())
                    .map(|capability| (capability, DataScope::Unrestricted))
                    .map_err(|_| Rejection::Invalid("invalid_iam_capability"))
            })
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        let request = AuthorizationRequest { requested };
        let decision = evaluate(
            human.as_ref().map(|resolved| &resolved.authority),
            &agent.authority,
            &request,
        );
        let decision_id = Uuid::new_v4();
        let allowed_scopes = decision
            .grants
            .iter()
            .map(|grant| grant.capability.as_str().to_string())
            .collect::<Vec<_>>();
        let denied_scopes = decision
            .denied_capabilities
            .iter()
            .map(|capability| capability.as_str().to_string())
            .collect::<Vec<_>>();
        let effective_grants =
            serde_json::to_value(&decision.grants).map_err(|_| Rejection::Database)?;
        let result = if !decision.allowed {
            "deny"
        } else if denied_scopes.is_empty() {
            "allow"
        } else {
            "partial"
        };
        sqlx::query(
            "INSERT INTO business_iam.authorization_decisions(
               id,human_principal_id,agent_principal_id,agent_kind,task_id,
               requested_capabilities,allowed_capabilities,denied_capabilities,
               effective_grants,result,reason_code,trace_id)
             VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)",
        )
        .bind(decision_id)
        .bind(human.as_ref().map(|resolved| resolved.id))
        .bind(agent.id)
        .bind(kind_name(agent.authority.kind))
        .bind(agent_turn_id)
        .bind(requested_scopes)
        .bind(&allowed_scopes)
        .bind(&denied_scopes)
        .bind(&effective_grants)
        .bind(result)
        .bind(decision.reason)
        .bind(trace_id)
        .execute(&mut **tx)
        .await
        .map_err(|_| Rejection::Database)?;

        if decision.allowed {
            Ok(TurnDecision::Granted(GrantedTurn {
                decision_id,
                agent_principal_id: agent.id,
                scopes: allowed_scopes,
                effective_grants,
            }))
        } else {
            Ok(TurnDecision::Denied(decision.reason))
        }
    }
}

async fn load_agent_authority(
    tx: &mut Transaction<'_, Postgres>,
    external_id: &str,
) -> Result<Option<ResolvedAuthority>, Rejection> {
    let row = sqlx::query(
        "SELECT id,kind,status FROM business_iam.principals
         WHERE external_id=$1 AND kind IN ('independent_agent','proxy_agent')
         ORDER BY (status='active') DESC,updated_at DESC LIMIT 1",
    )
    .bind(external_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|_| Rejection::Database)?;
    let Some(row) = row else {
        return Ok(None);
    };
    let kind = match row.get::<String, _>("kind").as_str() {
        "independent_agent" => PrincipalKind::IndependentAgent,
        "proxy_agent" => PrincipalKind::ProxyAgent,
        _ => return Err(Rejection::Database),
    };
    load_authority_from_row(tx, kind, row).await.map(Some)
}

async fn load_authority(
    tx: &mut Transaction<'_, Postgres>,
    kind_name: &str,
    external_id: &str,
    kind: PrincipalKind,
) -> Result<Option<ResolvedAuthority>, Rejection> {
    let row = sqlx::query(
        "SELECT id,kind,status FROM business_iam.principals
         WHERE external_id=$1 AND kind=$2 ORDER BY updated_at DESC LIMIT 1",
    )
    .bind(external_id)
    .bind(kind_name)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|_| Rejection::Database)?;
    let Some(row) = row else {
        return Ok(None);
    };
    load_authority_from_row(tx, kind, row).await.map(Some)
}

async fn load_authority_from_row(
    tx: &mut Transaction<'_, Postgres>,
    kind: PrincipalKind,
    row: sqlx::postgres::PgRow,
) -> Result<ResolvedAuthority, Rejection> {
    let principal_id: Uuid = row.get("id");
    let status = match row.get::<String, _>("status").as_str() {
        "active" => PrincipalStatus::Active,
        "disabled" => PrincipalStatus::Disabled,
        _ => return Err(Rejection::Database),
    };
    let rows = sqlx::query(
        "SELECT permission.capability,grant_row.data_scope,
                permission.obligations AS permission_obligations,
                grant_row.obligations AS grant_obligations
         FROM (
           SELECT permission_id,data_scope,obligations
           FROM business_iam.principal_permissions
           WHERE principal_id=$1 AND valid_from<=now()
             AND (valid_until IS NULL OR valid_until>now())
           UNION ALL
           SELECT rp.permission_id,rp.data_scope,rp.obligations
           FROM business_iam.principal_roles pr
           JOIN business_iam.roles role ON role.id=pr.role_id AND role.status='active'
           JOIN business_iam.role_permissions rp ON rp.role_id=pr.role_id
           WHERE pr.principal_id=$1 AND pr.valid_from<=now()
             AND (pr.valid_until IS NULL OR pr.valid_until>now())
         ) grant_row
         JOIN business_iam.permissions permission
           ON permission.id=grant_row.permission_id AND permission.status='active'
         ORDER BY permission.capability",
    )
    .bind(principal_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(|_| Rejection::Database)?;
    let mut entitlements = Vec::with_capacity(rows.len());
    for grant in rows {
        let capability = Capability::parse(grant.get::<String, _>("capability"))
            .map_err(|_| Rejection::Database)?;
        let data_scope = serde_json::from_value::<DataScope>(grant.get("data_scope"))
            .map_err(|_| Rejection::Database)?;
        let permission_obligations = serde_json::from_value::<BTreeSet<Obligation>>(
            grant.get::<Value, _>("permission_obligations"),
        )
        .map_err(|_| Rejection::Database)?;
        let grant_obligations = serde_json::from_value::<BTreeSet<Obligation>>(
            grant.get::<Value, _>("grant_obligations"),
        )
        .map_err(|_| Rejection::Database)?;
        entitlements.push(Entitlement {
            capability,
            data_scope,
            obligations: permission_obligations
                .union(&grant_obligations)
                .cloned()
                .collect(),
        });
    }
    Ok(ResolvedAuthority {
        id: principal_id,
        authority: Authority {
            principal_id: principal_id.to_string(),
            kind,
            status,
            entitlements,
        },
    })
}

fn kind_name(kind: PrincipalKind) -> &'static str {
    match kind {
        PrincipalKind::Human => "human",
        PrincipalKind::IndependentAgent => "independent_agent",
        PrincipalKind::ProxyAgent => "proxy_agent",
    }
}
