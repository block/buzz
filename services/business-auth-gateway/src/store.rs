use crate::{
    auth::Claims,
    config::Config,
    model::{
        Audit, Binding, ChallengeRequest, ChallengeResponse, EnterpriseUserSummary,
        IssueEmbedRequest, IssueEmbedResponse, MeResponse, Principal, RequestFacts,
    },
    security,
};
use chrono::{Duration, Utc};
use nostr::Kind;
use sqlx::{AssertSqlSafe, PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

#[derive(Clone)]
pub struct Store {
    pool: PgPool,
    config: Config,
}

#[derive(Debug)]
pub enum Rejection {
    Unauthorized(&'static str),
    Forbidden(&'static str),
    Invalid(&'static str),
    Conflict(&'static str),
    RateLimited,
    NotFound,
    Database,
}
type Result<T> = std::result::Result<T, Rejection>;
fn db(_: sqlx::Error) -> Rejection {
    Rejection::Database
}

#[derive(Debug)]
pub struct Bootstrap {
    pub target_path: String,
    pub session_id: Uuid,
    pub session_token: String,
    pub csrf_token: String,
    pub trace_id: Uuid,
}

pub struct BusinessState {
    pub session_id: Uuid,
    pub user_id: Uuid,
    pub subject: String,
    pub display_name: String,
    pub csrf_token_hash: Vec<u8>,
    pub trace_id: Uuid,
    pub workbench_session_id: Uuid,
}

impl Store {
    pub fn new(pool: PgPool, config: Config) -> Self {
        Self { pool, config }
    }
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
    pub(crate) fn config(&self) -> &Config {
        &self.config
    }
    pub async fn migrate(pool: &PgPool) -> std::result::Result<(), sqlx::migrate::MigrateError> {
        // This directory is the shared Business platform migration history through S1.5:
        // identity first, then authorization/master-data modules, operating-report
        // indexes, incident events, immutable cadence snapshots, and workflow
        // role read grants, inventory counts, inventory replenishment, and
        // versioned supplier delivery commitments with operational scorecards,
        // controlled core-master maintenance with disable-impact checks,
        // governed product/SKU catalogs with product-specific unit conversions,
        // configurable numbering rules with scoped, period-aware counter pools,
        // an append-only committed-number issuance ledger, and the logically
        // independent Business IAM schema used by human and Agent principals.
        sqlx::migrate!().run(pool).await
    }
    pub async fn grant_runtime(pool: &PgPool, role: &str) -> std::result::Result<(), sqlx::Error> {
        if role.is_empty()
            || role.len() > 63
            || !role
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            return Err(sqlx::Error::Protocol(
                "BUSINESS_AUTH_RUNTIME_DATABASE_ROLE is not a safe PostgreSQL identifier".into(),
            ));
        }
        let sql = format!(
            "GRANT USAGE ON SCHEMA public, business_iam TO {role};
             GRANT SELECT, INSERT, UPDATE, DELETE ON
               enterprise_users, buzz_identity_bindings, identity_binding_challenges,
               embed_sessions, business_sessions, workbench_sessions,
               agent_read_delegations TO {role};
             GRANT SELECT, INSERT ON security_audit_events TO {role};
             GRANT SELECT ON
               business_iam.principals, business_iam.roles,
               business_iam.permissions, business_iam.role_permissions,
               business_iam.principal_roles, business_iam.principal_permissions
               TO {role};
             GRANT SELECT, INSERT ON business_iam.authorization_decisions TO {role};"
        );
        // The identifier is restricted above to an ASCII PostgreSQL identifier;
        // no untrusted SQL fragments can reach this migration-only statement.
        sqlx::raw_sql(AssertSqlSafe(sql)).execute(pool).await?;
        Ok(())
    }
    pub async fn ready(&self) -> Result<()> {
        sqlx::query_scalar::<_, i32>("SELECT 1")
            .fetch_one(&self.pool)
            .await
            .map_err(db)
            .map(|_| ())
    }

    pub async fn principal(&self, claims: &Claims, facts: &RequestFacts) -> Result<Principal> {
        let provisional = claims
            .provisional_principal()
            .map_err(|_| Rejection::Unauthorized("invalid_claims"))?;
        let mut tx = self.pool.begin().await.map_err(db)?;
        let row = sqlx::query(
            "INSERT INTO enterprise_users(id,oidc_issuer,oidc_subject,email,display_name,oidc_sid)
             VALUES($1,$2,$3,$4,$5,$6)
             ON CONFLICT(oidc_issuer,oidc_subject) DO UPDATE SET
               email=EXCLUDED.email, display_name=EXCLUDED.display_name,
               oidc_sid=EXCLUDED.oidc_sid, updated_at=now(), last_login_at=now(),
               version=enterprise_users.version+1
             RETURNING id,status",
        )
        .bind(Uuid::new_v4())
        .bind(&provisional.issuer)
        .bind(&provisional.subject)
        .bind(&provisional.email)
        .bind(&provisional.display_name)
        .bind(&provisional.sid)
        .fetch_one(&mut *tx)
        .await
        .map_err(db)?;
        let user_id: Uuid = row.get("id");
        let status: String = row.get("status");
        if status != "active" {
            let mut audit = Audit::event("AUTHORIZATION_REJECTED", "failure", facts.clone());
            audit.reason = Some("user_disabled");
            audit.user_id = Some(user_id);
            Self::audit_tx(&mut tx, audit).await?;
            tx.commit().await.map_err(db)?;
            return Err(Rejection::Forbidden("user_disabled"));
        }
        let existing = if let Some(sid) = &provisional.sid {
            sqlx::query_scalar::<_, Uuid>("SELECT id FROM workbench_sessions WHERE enterprise_user_id=$1 AND oidc_sid=$2 AND status='active' AND expires_at>now() ORDER BY created_at DESC LIMIT 1 FOR UPDATE")
                .bind(user_id).bind(sid).fetch_optional(&mut *tx).await.map_err(db)?
        } else {
            sqlx::query_scalar::<_, Uuid>("SELECT id FROM workbench_sessions WHERE enterprise_user_id=$1 AND oidc_sid IS NULL AND status='active' AND expires_at>now() ORDER BY created_at DESC LIMIT 1 FOR UPDATE")
                .bind(user_id).fetch_optional(&mut *tx).await.map_err(db)?
        };
        let session_id = match existing {
            Some(id) => {
                sqlx::query("UPDATE workbench_sessions SET last_seen_at=now(),expires_at=GREATEST(expires_at,$2) WHERE id=$1").bind(id).bind(provisional.token_expires_at).execute(&mut *tx).await.map_err(db)?;
                id
            }
            None => {
                let id = Uuid::new_v4();
                sqlx::query("INSERT INTO workbench_sessions(id,enterprise_user_id,oidc_sid,expires_at,trace_id) VALUES($1,$2,$3,$4,$5)").bind(id).bind(user_id).bind(&provisional.sid).bind(provisional.token_expires_at).bind(facts.trace_id).execute(&mut *tx).await.map_err(db)?;
                id
            }
        };
        let mut audit = Audit::event("OIDC_LOGIN_SUCCEEDED", "success", facts.clone());
        audit.user_id = Some(user_id);
        audit.issuer = Some(provisional.issuer.clone());
        audit.subject = Some(provisional.subject.clone());
        audit.workbench_session_id = Some(session_id);
        Self::audit_tx(&mut tx, audit).await?;
        tx.commit().await.map_err(db)?;
        Ok(Principal {
            user_id,
            workbench_session_id: session_id,
            ..provisional
        })
    }

    pub async fn me(&self, principal: &Principal) -> Result<MeResponse> {
        let row = sqlx::query("SELECT email,display_name,status FROM enterprise_users WHERE id=$1")
            .bind(principal.user_id)
            .fetch_one(&self.pool)
            .await
            .map_err(db)?;
        Ok(MeResponse {
            user: EnterpriseUserSummary {
                id: principal.user_id,
                email: row.get("email"),
                display_name: row.get("display_name"),
                status: row.get("status"),
            },
            workbench_session_id: principal.workbench_session_id,
            bindings: self.bindings(principal).await?,
        })
    }

    pub async fn bindings(&self, principal: &Principal) -> Result<Vec<Binding>> {
        sqlx::query_as::<_, Binding>("SELECT id,buzz_pubkey,device_id,device_name,device_platform,status,bound_at,last_seen_at,revoked_at,version FROM buzz_identity_bindings WHERE enterprise_user_id=$1 ORDER BY created_at DESC")
            .bind(principal.user_id).fetch_all(&self.pool).await.map_err(db)
    }

    pub async fn challenge(
        &self,
        principal: &Principal,
        request: ChallengeRequest,
        facts: RequestFacts,
    ) -> Result<ChallengeResponse> {
        if !security::valid_pubkey(&request.pubkey)
            || !security::safe_text(&request.device_id, 8, 200)
            || !security::safe_text(&request.device_name, 1, 200)
            || !matches!(
                request.device_platform.as_str(),
                "macos" | "windows" | "linux" | "web"
            )
        {
            return Err(Rejection::Invalid("invalid_binding_request"));
        }
        let id = Uuid::new_v4();
        let nonce = security::random_token();
        let now = Utc::now();
        let expires =
            now + Duration::from_std(self.config.challenge_ttl).map_err(|_| Rejection::Database)?;
        let payload = security::canonical_binding_payload(
            id,
            &nonce,
            &principal.issuer,
            &principal.subject,
            &request.pubkey,
            &request.device_id,
            now.timestamp(),
            expires.timestamp(),
        );
        let mut tx = self.pool.begin().await.map_err(db)?;
        sqlx::query("INSERT INTO identity_binding_challenges(id,enterprise_user_id,requested_pubkey,device_id,device_name,device_platform,challenge_hash,canonical_payload,audience,expires_at,created_ip,trace_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8,'bizfin-workbench-device-binding',$9,$10::inet,$11)")
            .bind(id).bind(principal.user_id).bind(&request.pubkey).bind(&request.device_id).bind(&request.device_name).bind(&request.device_platform).bind(security::hash(&nonce)).bind(&payload).bind(expires).bind(&facts.ip).bind(facts.trace_id).execute(&mut *tx).await.map_err(db)?;
        let mut audit = Audit::event(
            "IDENTITY_BINDING_CHALLENGE_CREATED",
            "success",
            facts.clone(),
        );
        audit.user_id = Some(principal.user_id);
        audit.pubkey_short = Some(security::short_pubkey(&request.pubkey));
        audit.device_id = Some(request.device_id);
        audit.workbench_session_id = Some(principal.workbench_session_id);
        Self::audit_tx(&mut tx, audit).await?;
        tx.commit().await.map_err(db)?;
        Ok(ChallengeResponse {
            id,
            audience: "bizfin-workbench-device-binding",
            payload,
            expires_at: expires,
            trace_id: facts.trace_id,
        })
    }

    pub async fn verify_binding(
        &self,
        principal: &Principal,
        challenge_id: Uuid,
        event: nostr::Event,
        facts: RequestFacts,
    ) -> Result<Binding> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        let row=sqlx::query("SELECT requested_pubkey,device_id,device_name,device_platform,canonical_payload,status,expires_at FROM identity_binding_challenges WHERE id=$1 AND enterprise_user_id=$2 FOR UPDATE")
            .bind(challenge_id).bind(principal.user_id).fetch_optional(&mut *tx).await.map_err(db)?.ok_or(Rejection::NotFound)?;
        let pubkey: String = row.get("requested_pubkey");
        let device_id: String = row.get("device_id");
        let status: String = row.get("status");
        let expires: chrono::DateTime<Utc> = row.get("expires_at");
        let payload: String = row.get("canonical_payload");
        let valid = status == "active"
            && expires > Utc::now()
            && event.kind == Kind::Custom(24243)
            && event.pubkey.to_hex() == pubkey
            && event.content == payload
            && event.verify_id()
            && event.verify_signature();
        if !valid {
            sqlx::query("UPDATE identity_binding_challenges SET failed_attempts=failed_attempts+1,status=CASE WHEN expires_at<=now() THEN 'expired' ELSE status END WHERE id=$1").bind(challenge_id).execute(&mut *tx).await.map_err(db)?;
            let mut audit = Audit::event("IDENTITY_BINDING_FAILED", "failure", facts);
            audit.reason = Some(if status != "active" {
                "challenge_replay"
            } else if expires <= Utc::now() {
                "challenge_expired"
            } else {
                "signature_invalid"
            });
            audit.user_id = Some(principal.user_id);
            audit.pubkey_short = Some(security::short_pubkey(&pubkey));
            audit.device_id = Some(device_id);
            audit.workbench_session_id = Some(principal.workbench_session_id);
            Self::audit_tx(&mut tx, audit).await?;
            tx.commit().await.map_err(db)?;
            return Err(Rejection::Invalid("binding_verification_failed"));
        }
        let consumed=sqlx::query("UPDATE identity_binding_challenges SET status='consumed',consumed_at=now() WHERE id=$1 AND status='active' AND expires_at>now()").bind(challenge_id).execute(&mut *tx).await.map_err(db)?;
        if consumed.rows_affected() != 1 {
            tx.rollback().await.map_err(db)?;
            return Err(Rejection::Conflict("challenge_already_consumed"));
        }
        if let Some(owner)=sqlx::query_scalar::<_,Uuid>("SELECT enterprise_user_id FROM buzz_identity_bindings WHERE buzz_pubkey=$1 AND status='active' FOR UPDATE").bind(&pubkey).fetch_optional(&mut *tx).await.map_err(db)? { if owner!=principal.user_id { tx.rollback().await.map_err(db)?; return Err(Rejection::Conflict("pubkey_already_bound")); } }
        let replaced_bindings = sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM buzz_identity_bindings WHERE enterprise_user_id=$1 AND status='active' AND (buzz_pubkey=$2 OR device_id=$3) FOR UPDATE",
        )
        .bind(principal.user_id)
        .bind(&pubkey)
        .bind(&device_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(db)?;
        for replaced_id in replaced_bindings {
            Self::revoke_binding_tx(&mut tx, principal, replaced_id, facts.clone()).await?;
        }
        let id = Uuid::new_v4();
        let binding=sqlx::query_as::<_,Binding>("INSERT INTO buzz_identity_bindings(id,enterprise_user_id,buzz_pubkey,device_id,device_name,device_platform) VALUES($1,$2,$3,$4,$5,$6) RETURNING id,buzz_pubkey,device_id,device_name,device_platform,status,bound_at,last_seen_at,revoked_at,version")
            .bind(id).bind(principal.user_id).bind(&pubkey).bind(&device_id).bind(row.get::<String,_>("device_name")).bind(row.get::<String,_>("device_platform")).fetch_one(&mut *tx).await.map_err(db)?;
        for event_type in ["IDENTITY_BINDING_VERIFIED", "IDENTITY_BINDING_CREATED"] {
            let mut audit = Audit::event(event_type, "success", facts.clone());
            audit.user_id = Some(principal.user_id);
            audit.binding_id = Some(id);
            audit.pubkey_short = Some(security::short_pubkey(&pubkey));
            audit.device_id = Some(device_id.clone());
            audit.workbench_session_id = Some(principal.workbench_session_id);
            Self::audit_tx(&mut tx, audit).await?;
        }
        tx.commit().await.map_err(db)?;
        Ok(binding)
    }

    pub async fn revoke_binding(
        &self,
        principal: &Principal,
        id: Uuid,
        facts: RequestFacts,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        Self::revoke_binding_tx(&mut tx, principal, id, facts).await?;
        tx.commit().await.map_err(db)
    }

    async fn revoke_binding_tx(
        tx: &mut Transaction<'_, Postgres>,
        principal: &Principal,
        id: Uuid,
        facts: RequestFacts,
    ) -> Result<()> {
        let row=sqlx::query("UPDATE buzz_identity_bindings SET status='revoked',revoked_at=now(),updated_at=now(),version=version+1 WHERE id=$1 AND enterprise_user_id=$2 AND status='active' RETURNING buzz_pubkey,device_id").bind(id).bind(principal.user_id).fetch_optional(&mut **tx).await.map_err(db)?.ok_or(Rejection::NotFound)?;
        sqlx::query("INSERT INTO security_audit_events(id,event_type,result,enterprise_user_id,identity_binding_id,workbench_session_id,embed_session_id,trace_id) SELECT gen_random_uuid(),'EMBED_SESSION_REVOKED','success',enterprise_user_id,identity_binding_id,workbench_session_id,id,trace_id FROM embed_sessions WHERE identity_binding_id=$1 AND status='active'").bind(id).execute(&mut **tx).await.map_err(db)?;
        sqlx::query("INSERT INTO security_audit_events(id,event_type,result,enterprise_user_id,identity_binding_id,workbench_session_id,embed_session_id,business_session_id,trace_id) SELECT gen_random_uuid(),'BUSINESS_SESSION_REVOKED','success',enterprise_user_id,identity_binding_id,workbench_session_id,embed_session_id,id,trace_id FROM business_sessions WHERE identity_binding_id=$1 AND status='active'").bind(id).execute(&mut **tx).await.map_err(db)?;
        sqlx::query("UPDATE embed_sessions SET status='revoked',revoked_at=now(),version=version+1 WHERE identity_binding_id=$1 AND status='active'").bind(id).execute(&mut **tx).await.map_err(db)?;
        sqlx::query("UPDATE business_sessions SET status='revoked',revoked_at=now() WHERE identity_binding_id=$1 AND status='active'").bind(id).execute(&mut **tx).await.map_err(db)?;
        sqlx::query("INSERT INTO security_audit_events(id,event_type,result,enterprise_user_id,identity_binding_id,delegation_id,agent_id,agent_turn_id,source_buzz_event_id,source_channel_id,trace_id) SELECT gen_random_uuid(),'AGENT_DELEGATION_REVOKED','success',enterprise_user_id,identity_binding_id,id,agent_id,agent_turn_id,source_buzz_event_id,source_channel_id,trace_id FROM agent_read_delegations WHERE identity_binding_id=$1 AND status IN ('active','exhausted')").bind(id).execute(&mut **tx).await.map_err(db)?;
        sqlx::query("UPDATE agent_read_delegations SET status='revoked',revoked_at=now(),version=version+1 WHERE identity_binding_id=$1 AND status IN ('active','exhausted')").bind(id).execute(&mut **tx).await.map_err(db)?;
        let mut audit = Audit::event("IDENTITY_BINDING_REVOKED", "success", facts);
        audit.user_id = Some(principal.user_id);
        audit.binding_id = Some(id);
        audit.pubkey_short = Some(security::short_pubkey(row.get("buzz_pubkey")));
        audit.device_id = Some(row.get("device_id"));
        audit.workbench_session_id = Some(principal.workbench_session_id);
        Self::audit_tx(tx, audit).await?;
        Ok(())
    }

    pub async fn issue_embed(
        &self,
        principal: &Principal,
        request: IssueEmbedRequest,
        facts: RequestFacts,
    ) -> Result<IssueEmbedResponse> {
        if !security::valid_pubkey(&request.pubkey)
            || !security::safe_text(&request.device_id, 8, 200)
            || !security::safe_text(&request.target.r#type, 1, 80)
            || !security::safe_text(&request.target.id, 1, 200)
            || !security::safe_target(&request.target.path)
        {
            let mut a = Audit::event("EMBED_SESSION_TARGET_REJECTED", "failure", facts);
            a.user_id = Some(principal.user_id);
            self.audit(a).await?;
            return Err(Rejection::Invalid("target_rejected"));
        }
        let mut tx = self.pool.begin().await.map_err(db)?;
        let binding=sqlx::query_scalar::<_,Uuid>("SELECT id FROM buzz_identity_bindings WHERE enterprise_user_id=$1 AND buzz_pubkey=$2 AND device_id=$3 AND status='active' FOR UPDATE")
            .bind(principal.user_id).bind(&request.pubkey).bind(&request.device_id).fetch_optional(&mut *tx).await.map_err(db)?;
        let Some(binding_id) = binding else {
            let mut a = Audit::event("DEVICE_ACCESS_REJECTED", "failure", facts);
            a.reason = Some("binding_required_or_revoked");
            a.user_id = Some(principal.user_id);
            a.pubkey_short = Some(security::short_pubkey(&request.pubkey));
            a.device_id = Some(request.device_id);
            a.workbench_session_id = Some(principal.workbench_session_id);
            Self::audit_tx(&mut tx, a).await?;
            tx.commit().await.map_err(db)?;
            return Err(Rejection::Forbidden("binding_required_or_revoked"));
        };
        let count:i64=sqlx::query_scalar("SELECT count(*) FROM embed_sessions WHERE workbench_session_id=$1 AND created_at>now()-interval '1 minute'").bind(principal.workbench_session_id).fetch_one(&mut *tx).await.map_err(db)?;
        if count >= self.config.rate_limit {
            let mut audit = Audit::event("EMBED_SESSION_RATE_LIMITED", "failure", facts);
            audit.reason = Some("per_workbench_session_limit");
            audit.user_id = Some(principal.user_id);
            audit.workbench_session_id = Some(principal.workbench_session_id);
            Self::audit_tx(&mut tx, audit).await?;
            tx.commit().await.map_err(db)?;
            return Err(Rejection::RateLimited);
        }
        let code = security::random_token();
        let id = Uuid::new_v4();
        let expires = Utc::now()
            + Duration::from_std(self.config.embed_ttl).map_err(|_| Rejection::Database)?;
        sqlx::query("INSERT INTO embed_sessions(id,code_hash,enterprise_user_id,identity_binding_id,workbench_session_id,oidc_sid,audience,deployment_id,target_path,target_resource_type,target_resource_id,expires_at,created_ip,user_agent_hash,trace_id) VALUES($1,$2,$3,$4,$5,$6,'business-dock',$7,$8,$9,$10,$11,$12::inet,$13,$14)")
            .bind(id).bind(security::hash(&code)).bind(principal.user_id).bind(binding_id).bind(principal.workbench_session_id).bind(&principal.sid).bind(&self.config.deployment_id).bind(&request.target.path).bind(&request.target.r#type).bind(&request.target.id).bind(expires).bind(&facts.ip).bind(&facts.user_agent_hash).bind(facts.trace_id).execute(&mut *tx).await.map_err(db)?;
        let mut a = Audit::event("EMBED_SESSION_ISSUED", "success", facts.clone());
        a.user_id = Some(principal.user_id);
        a.binding_id = Some(binding_id);
        a.pubkey_short = Some(security::short_pubkey(&request.pubkey));
        a.device_id = Some(request.device_id);
        a.workbench_session_id = Some(principal.workbench_session_id);
        a.embed_session_id = Some(id);
        a.target_type = Some(request.target.r#type);
        a.target_id = Some(request.target.id);
        if let Some(s) = request.source {
            a.metadata = serde_json::json!({"buzzChannelId":s.buzz_channel_id,"buzzEventId":s.buzz_event_id});
        }
        Self::audit_tx(&mut tx, a).await?;
        tx.commit().await.map_err(db)?;
        Ok(IssueEmbedResponse {
            id,
            embed_url: format!(
                "{}/embed/bootstrap?code={}",
                self.config.business_origin, code
            ),
            expires_at: expires,
            trace_id: facts.trace_id,
        })
    }

    pub async fn revoke_embed(
        &self,
        principal: &Principal,
        id: Uuid,
        facts: RequestFacts,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        let changed=sqlx::query("UPDATE embed_sessions SET status='revoked',revoked_at=now(),version=version+1 WHERE id=$1 AND enterprise_user_id=$2 AND workbench_session_id=$3 AND status='active'").bind(id).bind(principal.user_id).bind(principal.workbench_session_id).execute(&mut *tx).await.map_err(db)?;
        if changed.rows_affected() != 1 {
            return Err(Rejection::NotFound);
        }
        let mut a = Audit::event("EMBED_SESSION_REVOKED", "success", facts);
        a.user_id = Some(principal.user_id);
        a.workbench_session_id = Some(principal.workbench_session_id);
        a.embed_session_id = Some(id);
        Self::audit_tx(&mut tx, a).await?;
        tx.commit().await.map_err(db)
    }

    pub async fn bootstrap(&self, code: &str, facts: RequestFacts) -> Result<Bootstrap> {
        if code.len() != 43
            || !code
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
        {
            return Err(Rejection::Invalid("invalid_embed_code"));
        }
        let mut tx = self.pool.begin().await.map_err(db)?;
        let row=sqlx::query("UPDATE embed_sessions e SET status='consumed',consumed_at=now(),consumed_ip=$2::inet,version=e.version+1 FROM enterprise_users u,buzz_identity_bindings b,workbench_sessions w WHERE e.code_hash=$1 AND e.enterprise_user_id=u.id AND e.identity_binding_id=b.id AND e.workbench_session_id=w.id AND e.status='active' AND e.expires_at>now() AND e.audience='business-dock' AND e.deployment_id=$3 AND u.status='active' AND b.status='active' AND w.status='active' AND w.expires_at>now() RETURNING e.id,e.enterprise_user_id,e.identity_binding_id,e.workbench_session_id,e.oidc_sid,e.target_path,e.target_resource_type,e.target_resource_id,e.trace_id")
            .bind(security::hash(code)).bind(&facts.ip).bind(&self.config.deployment_id).fetch_optional(&mut *tx).await.map_err(db)?;
        let Some(row) = row else {
            let exists = sqlx::query(
                "SELECT id,status,audience,deployment_id,expires_at,trace_id FROM embed_sessions WHERE code_hash=$1",
            )
            .bind(security::hash(code))
            .fetch_optional(&mut *tx)
            .await
            .map_err(db)?;
            let event_type = exists.as_ref().map_or("AUTHORIZATION_REJECTED", |row| {
                if row.get::<String, _>("audience") != "business-dock"
                    || row.get::<String, _>("deployment_id") != self.config.deployment_id
                {
                    "EMBED_SESSION_AUDIENCE_REJECTED"
                } else if row.get::<String, _>("status") != "active" {
                    "EMBED_SESSION_REPLAY_REJECTED"
                } else {
                    "AUTHORIZATION_REJECTED"
                }
            });
            let mut a = Audit::event(event_type, "failure", facts);
            a.reason = Some(match event_type {
                "EMBED_SESSION_AUDIENCE_REJECTED" => "embed_audience_or_deployment_mismatch",
                "EMBED_SESSION_REPLAY_REJECTED" => "embed_code_expired_revoked_or_replayed",
                _ => "embed_code_invalid_or_principal_revoked",
            });
            if let Some(r) = exists {
                a.embed_session_id = Some(r.get("id"));
                a.facts.trace_id = r.get("trace_id");
            }
            Self::audit_tx(&mut tx, a).await?;
            tx.commit().await.map_err(db)?;
            return Err(Rejection::Unauthorized("embed_code_rejected"));
        };
        let session_token = security::random_token();
        let csrf_token = security::random_token();
        let session_id = Uuid::new_v4();
        let expires = Utc::now()
            + Duration::from_std(self.config.business_ttl).map_err(|_| Rejection::Database)?;
        let trace_id: Uuid = row.get("trace_id");
        sqlx::query("INSERT INTO business_sessions(id,session_token_hash,csrf_token_hash,enterprise_user_id,identity_binding_id,workbench_session_id,embed_session_id,oidc_sid,expires_at,created_ip,user_agent_hash,trace_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10::inet,$11,$12)").bind(session_id).bind(security::hash(&session_token)).bind(security::hash(&csrf_token)).bind(row.get::<Uuid,_>("enterprise_user_id")).bind(row.get::<Uuid,_>("identity_binding_id")).bind(row.get::<Uuid,_>("workbench_session_id")).bind(row.get::<Uuid,_>("id")).bind(row.get::<Option<String>,_>("oidc_sid")).bind(expires).bind(&facts.ip).bind(&facts.user_agent_hash).bind(trace_id).execute(&mut *tx).await.map_err(db)?;
        for event_type in ["EMBED_SESSION_CONSUMED", "BUSINESS_SESSION_CREATED"] {
            let mut a = Audit::event(
                event_type,
                "success",
                RequestFacts {
                    trace_id,
                    ..facts.clone()
                },
            );
            a.user_id = Some(row.get("enterprise_user_id"));
            a.binding_id = Some(row.get("identity_binding_id"));
            a.workbench_session_id = Some(row.get("workbench_session_id"));
            a.embed_session_id = Some(row.get("id"));
            a.business_session_id = Some(session_id);
            a.target_type = Some(row.get("target_resource_type"));
            a.target_id = Some(row.get("target_resource_id"));
            Self::audit_tx(&mut tx, a).await?;
        }
        let target_path = row.get("target_path");
        tx.commit().await.map_err(db)?;
        Ok(Bootstrap {
            target_path,
            session_id,
            session_token,
            csrf_token,
            trace_id,
        })
    }

    pub async fn business_state(&self, token: &str) -> Result<BusinessState> {
        let row=sqlx::query("SELECT s.id,s.enterprise_user_id,s.workbench_session_id,u.oidc_subject,u.display_name,s.csrf_token_hash,s.trace_id FROM business_sessions s JOIN enterprise_users u ON u.id=s.enterprise_user_id JOIN buzz_identity_bindings b ON b.id=s.identity_binding_id JOIN workbench_sessions w ON w.id=s.workbench_session_id WHERE s.session_token_hash=$1 AND s.status='active' AND s.expires_at>now() AND u.status='active' AND b.status='active' AND w.status='active' AND w.expires_at>now()")
            .bind(security::hash(token)).fetch_optional(&self.pool).await.map_err(db)?.ok_or(Rejection::Unauthorized("business_session_invalid"))?;
        let id: Uuid = row.get("id");
        sqlx::query("UPDATE business_sessions SET last_seen_at=now() WHERE id=$1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(db)?;
        Ok(BusinessState {
            session_id: id,
            user_id: row.get("enterprise_user_id"),
            workbench_session_id: row.get("workbench_session_id"),
            subject: row.get("oidc_subject"),
            display_name: row.get("display_name"),
            csrf_token_hash: row.get("csrf_token_hash"),
            trace_id: row.get("trace_id"),
        })
    }

    pub async fn refresh_business_csrf(&self, state: &BusinessState) -> Result<String> {
        let csrf_token = security::random_token();
        let updated = sqlx::query(
            "UPDATE business_sessions SET csrf_token_hash=$2,last_seen_at=now() WHERE id=$1 AND status='active' AND expires_at>now()",
        )
        .bind(state.session_id)
        .bind(security::hash(&csrf_token))
        .execute(&self.pool)
        .await
        .map_err(db)?;
        if updated.rows_affected() != 1 {
            return Err(Rejection::Unauthorized("business_session_invalid"));
        }
        Ok(csrf_token)
    }

    pub async fn business_logout(&self, state: &BusinessState, facts: RequestFacts) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        let mut revoked = Audit::event("BUSINESS_SESSION_REVOKED", "success", facts.clone());
        revoked.user_id = Some(state.user_id);
        revoked.workbench_session_id = Some(state.workbench_session_id);
        revoked.business_session_id = Some(state.session_id);
        Self::audit_tx(&mut tx, revoked).await?;
        sqlx::query("UPDATE business_sessions SET status='revoked',revoked_at=now() WHERE id=$1 AND status='active'").bind(state.session_id).execute(&mut *tx).await.map_err(db)?;
        let mut a = Audit::event("BUSINESS_LOGOUT", "success", facts);
        a.user_id = Some(state.user_id);
        a.workbench_session_id = Some(state.workbench_session_id);
        a.business_session_id = Some(state.session_id);
        Self::audit_tx(&mut tx, a).await?;
        tx.commit().await.map_err(db)
    }

    pub async fn workbench_logout(
        &self,
        principal: &Principal,
        global: bool,
        facts: RequestFacts,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        sqlx::query("INSERT INTO security_audit_events(id,event_type,result,enterprise_user_id,identity_binding_id,workbench_session_id,embed_session_id,trace_id) SELECT gen_random_uuid(),'EMBED_SESSION_REVOKED','success',enterprise_user_id,identity_binding_id,workbench_session_id,id,trace_id FROM embed_sessions WHERE workbench_session_id=$1 AND status='active'").bind(principal.workbench_session_id).execute(&mut *tx).await.map_err(db)?;
        sqlx::query("INSERT INTO security_audit_events(id,event_type,result,enterprise_user_id,identity_binding_id,workbench_session_id,embed_session_id,business_session_id,trace_id) SELECT gen_random_uuid(),'BUSINESS_SESSION_REVOKED','success',enterprise_user_id,identity_binding_id,workbench_session_id,embed_session_id,id,trace_id FROM business_sessions WHERE workbench_session_id=$1 AND status='active'").bind(principal.workbench_session_id).execute(&mut *tx).await.map_err(db)?;
        sqlx::query("UPDATE workbench_sessions SET status='revoked',revoked_at=now() WHERE id=$1 AND status='active'").bind(principal.workbench_session_id).execute(&mut *tx).await.map_err(db)?;
        sqlx::query("UPDATE embed_sessions SET status='revoked',revoked_at=now(),version=version+1 WHERE workbench_session_id=$1 AND status='active'").bind(principal.workbench_session_id).execute(&mut *tx).await.map_err(db)?;
        sqlx::query("UPDATE business_sessions SET status='revoked',revoked_at=now() WHERE workbench_session_id=$1 AND status='active'").bind(principal.workbench_session_id).execute(&mut *tx).await.map_err(db)?;
        let mut a = Audit::event(
            if global {
                "GLOBAL_LOGOUT"
            } else {
                "WORKBENCH_LOGOUT"
            },
            "success",
            facts,
        );
        a.user_id = Some(principal.user_id);
        a.workbench_session_id = Some(principal.workbench_session_id);
        Self::audit_tx(&mut tx, a).await?;
        tx.commit().await.map_err(db)
    }

    pub async fn backchannel_logout(
        &self,
        sid: Option<&str>,
        issuer: &str,
        subject: &str,
        facts: RequestFacts,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        let user_id: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM enterprise_users WHERE oidc_issuer=$1 AND oidc_subject=$2",
        )
        .bind(issuer)
        .bind(subject)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db)?;
        sqlx::query("INSERT INTO security_audit_events(id,event_type,result,enterprise_user_id,identity_binding_id,workbench_session_id,embed_session_id,trace_id) SELECT gen_random_uuid(),'EMBED_SESSION_REVOKED','success',enterprise_user_id,identity_binding_id,workbench_session_id,id,trace_id FROM embed_sessions WHERE (($1::text IS NOT NULL AND oidc_sid=$1) OR ($1::text IS NULL AND enterprise_user_id=$2)) AND status='active'").bind(sid).bind(user_id).execute(&mut *tx).await.map_err(db)?;
        sqlx::query("INSERT INTO security_audit_events(id,event_type,result,enterprise_user_id,identity_binding_id,workbench_session_id,embed_session_id,business_session_id,trace_id) SELECT gen_random_uuid(),'BUSINESS_SESSION_REVOKED','success',enterprise_user_id,identity_binding_id,workbench_session_id,embed_session_id,id,trace_id FROM business_sessions WHERE (($1::text IS NOT NULL AND oidc_sid=$1) OR ($1::text IS NULL AND enterprise_user_id=$2)) AND status='active'").bind(sid).bind(user_id).execute(&mut *tx).await.map_err(db)?;
        sqlx::query("UPDATE workbench_sessions SET status='revoked',revoked_at=now() WHERE (($1::text IS NOT NULL AND oidc_sid=$1) OR ($1::text IS NULL AND enterprise_user_id=$2)) AND status='active'").bind(sid).bind(user_id).execute(&mut *tx).await.map_err(db)?;
        sqlx::query("UPDATE embed_sessions SET status='revoked',revoked_at=now(),version=version+1 WHERE (($1::text IS NOT NULL AND oidc_sid=$1) OR ($1::text IS NULL AND enterprise_user_id=$2)) AND status='active'").bind(sid).bind(user_id).execute(&mut *tx).await.map_err(db)?;
        sqlx::query("UPDATE business_sessions SET status='revoked',revoked_at=now() WHERE (($1::text IS NOT NULL AND oidc_sid=$1) OR ($1::text IS NULL AND enterprise_user_id=$2)) AND status='active'").bind(sid).bind(user_id).execute(&mut *tx).await.map_err(db)?;
        let mut a = Audit::event("BACKCHANNEL_LOGOUT", "success", facts);
        a.user_id = user_id;
        a.issuer = Some(issuer.into());
        a.subject = Some(subject.into());
        Self::audit_tx(&mut tx, a).await?;
        tx.commit().await.map_err(db)
    }

    pub async fn cleanup(&self) -> Result<(u64, u64, u64)> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        let c=sqlx::query("UPDATE identity_binding_challenges SET status='expired' WHERE status='active' AND expires_at<=now()").execute(&mut *tx).await.map_err(db)?.rows_affected();
        sqlx::query("INSERT INTO security_audit_events(id,event_type,result,embed_session_id,enterprise_user_id,identity_binding_id,workbench_session_id,target_resource_type,target_resource_id,trace_id) SELECT gen_random_uuid(),'EMBED_SESSION_EXPIRED','success',id,enterprise_user_id,identity_binding_id,workbench_session_id,target_resource_type,target_resource_id,trace_id FROM embed_sessions WHERE status='active' AND expires_at<=now()").execute(&mut *tx).await.map_err(db)?;
        let e=sqlx::query("UPDATE embed_sessions SET status='expired',version=version+1 WHERE status='active' AND expires_at<=now()").execute(&mut *tx).await.map_err(db)?.rows_affected();
        sqlx::query("INSERT INTO security_audit_events(id,event_type,result,business_session_id,enterprise_user_id,identity_binding_id,workbench_session_id,embed_session_id,trace_id) SELECT gen_random_uuid(),'BUSINESS_SESSION_EXPIRED','success',id,enterprise_user_id,identity_binding_id,workbench_session_id,embed_session_id,trace_id FROM business_sessions WHERE status='active' AND expires_at<=now()").execute(&mut *tx).await.map_err(db)?;
        let b=sqlx::query("UPDATE business_sessions SET status='expired' WHERE status='active' AND expires_at<=now()").execute(&mut *tx).await.map_err(db)?.rows_affected();
        sqlx::query("INSERT INTO security_audit_events(id,event_type,result,enterprise_user_id,identity_binding_id,delegation_id,agent_id,agent_turn_id,source_buzz_event_id,source_channel_id,trace_id) SELECT gen_random_uuid(),'AGENT_DELEGATION_EXPIRED','success',enterprise_user_id,identity_binding_id,id,agent_id,agent_turn_id,source_buzz_event_id,source_channel_id,trace_id FROM agent_read_delegations WHERE status='active' AND expires_at<=now()").execute(&mut *tx).await.map_err(db)?;
        sqlx::query("UPDATE agent_read_delegations SET status='expired',version=version+1 WHERE status='active' AND expires_at<=now()").execute(&mut *tx).await.map_err(db)?;
        tx.commit().await.map_err(db)?;
        Ok((c, e, b))
    }

    pub async fn audit(&self, audit: Audit) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        Self::audit_tx(&mut tx, audit).await?;
        tx.commit().await.map_err(db)
    }
    pub(crate) async fn audit_tx(tx: &mut Transaction<'_, Postgres>, a: Audit) -> Result<()> {
        sqlx::query("INSERT INTO security_audit_events(id,event_type,result,reason_code,enterprise_user_id,oidc_issuer,oidc_subject,identity_binding_id,buzz_pubkey_short,device_id,workbench_session_id,embed_session_id,business_session_id,delegation_id,agent_id,agent_turn_id,source_buzz_event_id,response_buzz_event_id,source_channel_id,tool_name,result_count,finding_count,resource_ref_count,rule_set_version,anomaly_run_id,duration_ms,target_resource_type,target_resource_id,source_ip,user_agent_hash,trace_id,metadata) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21,$22,$23,$24,$25,$26,$27,$28,$29::inet,$30,$31,$32)").bind(Uuid::new_v4()).bind(a.event_type).bind(a.result).bind(a.reason).bind(a.user_id).bind(a.issuer).bind(a.subject).bind(a.binding_id).bind(a.pubkey_short).bind(a.device_id).bind(a.workbench_session_id).bind(a.embed_session_id).bind(a.business_session_id).bind(a.delegation_id).bind(a.agent_id).bind(a.agent_turn_id).bind(a.source_buzz_event_id).bind(a.response_buzz_event_id).bind(a.source_channel_id).bind(a.tool_name).bind(a.result_count).bind(a.finding_count).bind(a.resource_ref_count).bind(a.rule_set_version).bind(a.anomaly_run_id).bind(a.duration_ms).bind(a.target_type).bind(a.target_id).bind(a.facts.ip).bind(a.facts.user_agent_hash).bind(a.facts.trace_id).bind(a.metadata).execute(&mut **tx).await.map_err(db)?;
        Ok(())
    }
}
