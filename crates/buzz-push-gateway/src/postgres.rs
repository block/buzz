//! PostgreSQL authority store. Mutations use row locks and compare-and-swap
//! predicates so counters, epochs, and generation tombstones only move forward.
use crate::authority::*;
use crate::model::AppProfile;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};
use uuid::Uuid;

#[derive(Clone)]
pub struct PostgresAuthorityStore {
    pool: PgPool,
}
impl PostgresAuthorityStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}
fn at(ts: i64) -> Result<DateTime<Utc>, AuthorityError> {
    DateTime::from_timestamp(ts, 0).ok_or(AuthorityError::Rejected)
}
fn ts(v: DateTime<Utc>) -> i64 {
    v.timestamp()
}
fn profile(v: &str) -> Result<AppProfile, AuthorityError> {
    match v {
        "buzz-ios-production" => Ok(AppProfile::BuzzIosProduction),
        "buzz-ios-sandbox" => Ok(AppProfile::BuzzIosSandbox),
        _ => Err(AuthorityError::Unavailable),
    }
}
fn db(_: sqlx::Error) -> AuthorityError {
    AuthorityError::Unavailable
}
fn bytes32(v: Vec<u8>) -> Result<[u8; 32], AuthorityError> {
    v.try_into().map_err(|_| AuthorityError::Unavailable)
}

#[async_trait]
impl AuthorityStore for PostgresAuthorityStore {
    async fn ready(&self) -> Result<(), AuthorityError> {
        sqlx::query_scalar::<_, i32>("SELECT 1")
            .fetch_one(&self.pool)
            .await
            .map(|_| ())
            .map_err(db)
    }

    async fn put_challenge(&self, c: Challenge) -> Result<(), AuthorityError> {
        use sha2::{Digest, Sha256};
        sqlx::query(
            "INSERT INTO push_gateway_challenges(id,challenge_hash,expires_at) VALUES($1,$2,$3)",
        )
        .bind(c.id)
        .bind(Sha256::digest(c.value).to_vec())
        .bind(at(c.expires_at)?)
        .execute(&self.pool)
        .await
        .map_err(db)?;
        Ok(())
    }
    async fn consume_challenge(
        &self,
        id: Uuid,
        value: [u8; 32],
        now: i64,
    ) -> Result<(), AuthorityError> {
        use sha2::{Digest, Sha256};
        let result = sqlx::query("DELETE FROM push_gateway_challenges WHERE id=$1 AND challenge_hash=$2 AND expires_at >= $3")
            .bind(id).bind(Sha256::digest(value).to_vec()).bind(at(now)?).execute(&self.pool).await.map_err(db)?;
        if result.rows_affected() != 1 {
            return Err(AuthorityError::Rejected);
        }
        Ok(())
    }
    async fn create_installation(&self, n: NewInstallation) -> Result<(), AuthorityError> {
        let result = sqlx::query("INSERT INTO push_gateway_installations(id,app_attest_key_id,app_attest_public_key,assertion_counter,app_profile,token_ciphertext,token_fingerprint,endpoint_epoch,expires_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9) ON CONFLICT DO NOTHING")
            .bind(n.id).bind(n.app_attest_key_id).bind(n.app_attest_public_key).bind(i64::from(n.assertion_counter)).bind(n.profile.as_str()).bind(n.token_ciphertext).bind(n.token_fingerprint.to_vec()).bind(n.endpoint_epoch).bind(at(n.expires_at)?).execute(&self.pool).await.map_err(db)?;
        if result.rows_affected() != 1 {
            return Err(AuthorityError::Rejected);
        }
        Ok(())
    }
    async fn installation(&self, id: Uuid, now: i64) -> Result<Installation, AuthorityError> {
        let r = sqlx::query("SELECT * FROM push_gateway_installations WHERE id=$1 AND revoked_at IS NULL AND expires_at >= $2")
            .bind(id).bind(at(now)?).fetch_optional(&self.pool).await.map_err(db)?.ok_or(AuthorityError::Rejected)?;
        Ok(Installation {
            id,
            app_attest_key_id: r.try_get("app_attest_key_id").map_err(db)?,
            app_attest_public_key: r.try_get("app_attest_public_key").map_err(db)?,
            assertion_counter: u32::try_from(r.try_get::<i64, _>("assertion_counter").map_err(db)?)
                .map_err(|_| AuthorityError::Unavailable)?,
            profile: profile(r.try_get("app_profile").map_err(db)?)?,
            token_ciphertext: r.try_get("token_ciphertext").map_err(db)?,
            token_fingerprint: bytes32(r.try_get("token_fingerprint").map_err(db)?)?,
            endpoint_epoch: r.try_get("endpoint_epoch").map_err(db)?,
            expires_at: ts(r.try_get("expires_at").map_err(db)?),
            revoked: false,
        })
    }
    async fn advance_assertion_counter(
        &self,
        id: Uuid,
        previous: u32,
        next: u32,
    ) -> Result<(), AuthorityError> {
        if next <= previous {
            return Err(AuthorityError::Rejected);
        }
        let result=sqlx::query("UPDATE push_gateway_installations SET assertion_counter=$3,updated_at=now() WHERE id=$1 AND assertion_counter=$2 AND revoked_at IS NULL")
            .bind(id).bind(i64::from(previous)).bind(i64::from(next)).execute(&self.pool).await.map_err(db)?;
        if result.rows_affected() != 1 {
            return Err(AuthorityError::Rejected);
        }
        Ok(())
    }
    async fn upsert_delegation(&self, d: Delegation) -> Result<(), AuthorityError> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        let i=sqlx::query("SELECT endpoint_epoch,expires_at,revoked_at FROM push_gateway_installations WHERE id=$1 FOR UPDATE").bind(d.installation_id).fetch_optional(&mut *tx).await.map_err(db)?.ok_or(AuthorityError::Rejected)?;
        if i.try_get::<Option<DateTime<Utc>>, _>("revoked_at")
            .map_err(db)?
            .is_some()
            || i.try_get::<i64, _>("endpoint_epoch").map_err(db)? != d.endpoint_epoch
            || at(d.expires_at)? > i.try_get::<DateTime<Utc>, _>("expires_at").map_err(db)?
        {
            return Err(AuthorityError::Rejected);
        }
        let relay = hex::decode(&d.relay_pubkey).map_err(|_| AuthorityError::Rejected)?;
        let result=sqlx::query("INSERT INTO push_gateway_delegations(id,installation_id,relay_pubkey,endpoint_epoch,generation,not_before,expires_at,revoked_at) VALUES($1,$2,$3,$4,$5,$6,$7,NULL) ON CONFLICT(installation_id,relay_pubkey) DO UPDATE SET id=EXCLUDED.id,endpoint_epoch=EXCLUDED.endpoint_epoch,generation=EXCLUDED.generation,not_before=EXCLUDED.not_before,expires_at=EXCLUDED.expires_at,revoked_at=NULL,updated_at=now() WHERE EXCLUDED.generation > push_gateway_delegations.generation")
            .bind(d.id).bind(d.installation_id).bind(relay).bind(d.endpoint_epoch).bind(d.generation).bind(at(d.not_before)?).bind(at(d.expires_at)?).execute(&mut *tx).await.map_err(db)?;
        if result.rows_affected() != 1 {
            return Err(AuthorityError::Rejected);
        }
        tx.commit().await.map_err(db)?;
        Ok(())
    }
    async fn rotate_endpoint(
        &self,
        id: Uuid,
        expected: i64,
        new: i64,
        ciphertext: Vec<u8>,
        fingerprint: [u8; 32],
    ) -> Result<(), AuthorityError> {
        if new != expected.checked_add(1).ok_or(AuthorityError::Rejected)? {
            return Err(AuthorityError::Rejected);
        }
        let result=sqlx::query("UPDATE push_gateway_installations SET endpoint_epoch=$3,token_ciphertext=$4,token_fingerprint=$5,updated_at=now() WHERE id=$1 AND endpoint_epoch=$2 AND revoked_at IS NULL").bind(id).bind(expected).bind(new).bind(ciphertext).bind(fingerprint.to_vec()).execute(&self.pool).await.map_err(db)?;
        if result.rows_affected() != 1 {
            return Err(AuthorityError::Rejected);
        }
        Ok(())
    }
    async fn revoke_delegation(
        &self,
        id: Uuid,
        relay: &str,
        generation: i64,
    ) -> Result<(), AuthorityError> {
        let relay = hex::decode(relay).map_err(|_| AuthorityError::Rejected)?;
        let result=sqlx::query("UPDATE push_gateway_delegations SET generation=$3,revoked_at=now(),updated_at=now() WHERE installation_id=$1 AND relay_pubkey=$2 AND generation<$3").bind(id).bind(relay).bind(generation).execute(&self.pool).await.map_err(db)?;
        if result.rows_affected() != 1 {
            return Err(AuthorityError::Rejected);
        }
        Ok(())
    }
    async fn revoke_installation(
        &self,
        id: Uuid,
        expected: i64,
        new: i64,
    ) -> Result<(), AuthorityError> {
        if new != expected.checked_add(1).ok_or(AuthorityError::Rejected)? {
            return Err(AuthorityError::Rejected);
        }
        let result=sqlx::query("UPDATE push_gateway_installations SET endpoint_epoch=$3,revoked_at=now(),updated_at=now() WHERE id=$1 AND endpoint_epoch=$2 AND revoked_at IS NULL").bind(id).bind(expected).bind(new).execute(&self.pool).await.map_err(db)?;
        if result.rows_affected() != 1 {
            return Err(AuthorityError::Rejected);
        }
        Ok(())
    }
    async fn authorize_delivery(
        &self,
        did: Uuid,
        relay: &str,
        epoch: i64,
        generation: i64,
        event_id: &str,
        request_id: Uuid,
        request_expires_at: i64,
        quota_window_seconds: i64,
        quota_max_deliveries: i64,
        now: i64,
    ) -> Result<DeliveryPermit, AuthorityError> {
        let relay_bytes = hex::decode(relay).map_err(|_| AuthorityError::Rejected)?;
        let event_bytes = hex::decode(event_id).map_err(|_| AuthorityError::Rejected)?;
        if event_bytes.len() != 32 {
            return Err(AuthorityError::Rejected);
        }
        let mut tx = self.pool.begin().await.map_err(db)?;
        // Every authority mutation locks installation before delegation. Keep
        // this order here to avoid delivery-vs-refresh deadlocks.
        let i = sqlx::query(
            "SELECT app_profile,token_ciphertext,token_fingerprint,endpoint_epoch,expires_at,revoked_at
             FROM push_gateway_installations
             WHERE id=(SELECT installation_id FROM push_gateway_delegations WHERE id=$1)
             FOR UPDATE",
        )
        .bind(did)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db)?
        .ok_or(AuthorityError::Rejected)?;
        if i.try_get::<Option<DateTime<Utc>>, _>("revoked_at")
            .map_err(db)?
            .is_some()
            || i.try_get::<i64, _>("endpoint_epoch").map_err(db)? != epoch
            || i.try_get::<DateTime<Utc>, _>("expires_at").map_err(db)? < at(now)?
        {
            return Err(AuthorityError::Rejected);
        }
        let d = sqlx::query(
            "SELECT installation_id,expires_at FROM push_gateway_delegations
             WHERE id=$1 AND relay_pubkey=$2 AND endpoint_epoch=$3 AND generation=$4
               AND revoked_at IS NULL AND not_before<=$5 AND expires_at>=$5
             FOR UPDATE",
        )
        .bind(did)
        .bind(&relay_bytes)
        .bind(epoch)
        .bind(generation)
        .bind(at(now)?)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db)?
        .ok_or(AuthorityError::Rejected)?;
        let installation_id: Uuid = d.try_get("installation_id").map_err(db)?;
        let authority = DeliveryAuthority {
            delegation_id: did,
            installation_id,
            relay_pubkey: relay.to_owned(),
            profile: profile(i.try_get("app_profile").map_err(db)?)?,
            token_ciphertext: i.try_get("token_ciphertext").map_err(db)?,
            endpoint_epoch: epoch,
            generation,
            expires_at: ts(d.try_get("expires_at").map_err(db)?),
        };
        if request_expires_at < now || request_expires_at > authority.expires_at {
            return Err(AuthorityError::Rejected);
        }
        if quota_window_seconds < 1 || quota_max_deliveries < 1 {
            return Err(AuthorityError::Unavailable);
        }
        let fingerprint: Vec<u8> = i.try_get("token_fingerprint").map_err(db)?;
        let quota = sqlx::query("INSERT INTO push_gateway_endpoint_quotas(token_fingerprint,window_started_at,admitted) VALUES($1,$2,1) ON CONFLICT(token_fingerprint) DO UPDATE SET window_started_at=CASE WHEN push_gateway_endpoint_quotas.window_started_at <= $2 - make_interval(secs => $3::double precision) THEN $2 ELSE push_gateway_endpoint_quotas.window_started_at END, admitted=CASE WHEN push_gateway_endpoint_quotas.window_started_at <= $2 - make_interval(secs => $3::double precision) THEN 1 ELSE push_gateway_endpoint_quotas.admitted + 1 END, updated_at=now() WHERE push_gateway_endpoint_quotas.window_started_at <= $2 - make_interval(secs => $3::double precision) OR push_gateway_endpoint_quotas.admitted < $4")
            .bind(fingerprint).bind(at(now)?).bind(quota_window_seconds).bind(quota_max_deliveries).execute(&mut *tx).await.map_err(db)?;
        if quota.rows_affected() != 1 {
            return Err(AuthorityError::Rejected);
        }
        let auth_inserted = sqlx::query("INSERT INTO push_gateway_delivery_auth_replays(relay_pubkey,auth_event_id,expires_at) VALUES($1,$2,$3) ON CONFLICT DO NOTHING")
            .bind(&relay_bytes).bind(event_bytes).bind(at(request_expires_at)?).execute(&mut *tx).await.map_err(db)?;
        let request_inserted = sqlx::query("INSERT INTO push_gateway_delivery_request_replays(relay_pubkey,request_id,expires_at) VALUES($1,$2,$3) ON CONFLICT DO NOTHING")
            .bind(&relay_bytes).bind(request_id).bind(at(request_expires_at)?).execute(&mut *tx).await.map_err(db)?;
        if auth_inserted.rows_affected() != 1 || request_inserted.rows_affected() != 1 {
            return Err(AuthorityError::Rejected);
        }
        tx.commit().await.map_err(db)?;
        Ok(DeliveryPermit::new(authority, relay.to_owned(), request_id))
    }

    async fn finish_delivery(
        &self,
        permit: DeliveryPermit,
        disposition: DeliveryDisposition,
    ) -> Result<(), AuthorityError> {
        if disposition == DeliveryDisposition::Retryable {
            sqlx::query("DELETE FROM push_gateway_delivery_request_replays WHERE relay_pubkey=$1 AND request_id=$2")
                .bind(hex::decode(permit.relay_pubkey).map_err(|_| AuthorityError::Unavailable)?)
                .bind(permit.request_id)
                .execute(&self.pool)
                .await
                .map_err(db)?;
        }
        Ok(())
    }

    async fn reap_expired(&self, now: i64) -> Result<(), AuthorityError> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        sqlx::query("DELETE FROM push_gateway_challenges WHERE expires_at < $1")
            .bind(at(now)?)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        sqlx::query("DELETE FROM push_gateway_delivery_auth_replays WHERE expires_at < $1")
            .bind(at(now)?)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        sqlx::query("DELETE FROM push_gateway_delivery_request_replays WHERE expires_at < $1")
            .bind(at(now)?)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        sqlx::query(
            "DELETE FROM push_gateway_endpoint_quotas WHERE updated_at < $1 - interval '1 day'",
        )
        .bind(at(now)?)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
        // A parent may become retention-eligible before an otherwise-active
        // child. Parent eligibility must therefore reap every child first;
        // otherwise the installation delete violates the delegation FK and
        // rolls back all cleanup in this transaction.
        sqlx::query(
            "DELETE FROM push_gateway_delegations d
             WHERE d.expires_at < $1
                OR d.revoked_at < $1 - interval '1 day'
                OR EXISTS (
                    SELECT 1 FROM push_gateway_installations i
                    WHERE i.id = d.installation_id
                      AND (i.expires_at < $1 OR i.revoked_at < $1 - interval '1 day')
                )",
        )
        .bind(at(now)?)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
        sqlx::query("DELETE FROM push_gateway_installations WHERE expires_at < $1 OR revoked_at < $1 - interval '1 day'")
            .bind(at(now)?).execute(&mut *tx).await.map_err(db)?;
        tx.commit().await.map_err(db)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::{postgres::PgPoolOptions, AssertSqlSafe};

    const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz";

    #[tokio::test]
    #[ignore = "requires PostgreSQL"]
    async fn reaper_deletes_active_child_of_retention_eligible_revoked_installation() {
        let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap_or_else(|_| TEST_DB_URL.to_owned());
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .connect(&database_url)
            .await
            .expect("connect to PostgreSQL test database");
        let schema = format!("push_reaper_{}", Uuid::new_v4().simple());
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&pool)
            .await
            .expect("create isolated test schema");
        sqlx::query(AssertSqlSafe(format!("SET search_path TO {schema}")))
            .execute(&pool)
            .await
            .expect("select isolated test schema");
        sqlx::raw_sql(
            "CREATE TABLE push_gateway_challenges (expires_at TIMESTAMPTZ NOT NULL);
             CREATE TABLE push_gateway_delivery_auth_replays (expires_at TIMESTAMPTZ NOT NULL);
             CREATE TABLE push_gateway_delivery_request_replays (expires_at TIMESTAMPTZ NOT NULL);
             CREATE TABLE push_gateway_endpoint_quotas (updated_at TIMESTAMPTZ NOT NULL);
             CREATE TABLE push_gateway_installations (
                 id UUID PRIMARY KEY,
                 expires_at TIMESTAMPTZ NOT NULL,
                 revoked_at TIMESTAMPTZ
             );
             CREATE TABLE push_gateway_delegations (
                 id UUID PRIMARY KEY,
                 installation_id UUID NOT NULL REFERENCES push_gateway_installations(id),
                 expires_at TIMESTAMPTZ NOT NULL,
                 revoked_at TIMESTAMPTZ
             );",
        )
        .execute(&pool)
        .await
        .expect("create authority retention tables");

        let now = Utc::now();
        let installation_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO push_gateway_installations(id, expires_at, revoked_at)
             VALUES ($1, $2, $3)",
        )
        .bind(installation_id)
        .bind(now + chrono::Duration::days(30))
        .bind(now - chrono::Duration::days(2))
        .execute(&pool)
        .await
        .expect("insert retention-eligible revoked installation");
        sqlx::query(
            "INSERT INTO push_gateway_delegations(id, installation_id, expires_at, revoked_at)
             VALUES ($1, $2, $3, NULL)",
        )
        .bind(Uuid::new_v4())
        .bind(installation_id)
        .bind(now + chrono::Duration::days(7))
        .execute(&pool)
        .await
        .expect("insert active future-expiring child delegation");

        PostgresAuthorityStore::new(pool.clone())
            .reap_expired(now.timestamp())
            .await
            .expect("reaper must delete the child before its revoked parent");
        let delegations: i64 = sqlx::query_scalar("SELECT count(*) FROM push_gateway_delegations")
            .fetch_one(&pool)
            .await
            .expect("count delegations");
        let installations: i64 =
            sqlx::query_scalar("SELECT count(*) FROM push_gateway_installations")
                .fetch_one(&pool)
                .await
                .expect("count installations");
        assert_eq!(delegations, 0);
        assert_eq!(installations, 0);

        sqlx::query("SET search_path TO public")
            .execute(&pool)
            .await
            .expect("restore public schema");
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop isolated test schema");
    }

    // Full authority schema in a private search_path so a multi-connection pool
    // exercises the real PK/UNIQUE replay fences that the memory store's single
    // mutex cannot. Returns (pool, schema) for teardown.
    async fn full_schema(max_connections: u32) -> (PgPool, String) {
        let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap_or_else(|_| TEST_DB_URL.to_owned());
        let schema = format!("push_admit_{}", Uuid::new_v4().simple());
        let bootstrap = PgPoolOptions::new()
            .max_connections(1)
            .connect(&database_url)
            .await
            .expect("connect to PostgreSQL test database");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&bootstrap)
            .await
            .expect("create isolated test schema");
        bootstrap.close().await;
        // search_path is per-session, so pin it on every pooled connection.
        let set_path = format!("SET search_path TO {schema}");
        let pool = PgPoolOptions::new()
            .max_connections(max_connections)
            .after_connect(move |conn, _| {
                let set_path = set_path.clone();
                Box::pin(async move {
                    sqlx::query(AssertSqlSafe(set_path)).execute(conn).await?;
                    Ok(())
                })
            })
            .connect(&database_url)
            .await
            .expect("connect isolated-schema pool");
        // Real DDL from migration 0010 (minus the _operator_global_tables audit
        // insert, which lives outside the isolated schema).
        sqlx::raw_sql(
            "CREATE TABLE push_gateway_installations (
                 id UUID PRIMARY KEY,
                 app_attest_key_id BYTEA NOT NULL UNIQUE,
                 app_attest_public_key BYTEA NOT NULL,
                 assertion_counter BIGINT NOT NULL,
                 app_profile TEXT NOT NULL,
                 token_ciphertext BYTEA NOT NULL,
                 token_fingerprint BYTEA NOT NULL CHECK (length(token_fingerprint) = 32),
                 endpoint_epoch BIGINT NOT NULL,
                 expires_at TIMESTAMPTZ NOT NULL,
                 revoked_at TIMESTAMPTZ,
                 updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                 UNIQUE (app_profile, token_fingerprint)
             );
             CREATE TABLE push_gateway_delegations (
                 id UUID PRIMARY KEY,
                 installation_id UUID NOT NULL REFERENCES push_gateway_installations(id),
                 relay_pubkey BYTEA NOT NULL CHECK (length(relay_pubkey) = 32),
                 endpoint_epoch BIGINT NOT NULL,
                 generation BIGINT NOT NULL,
                 not_before TIMESTAMPTZ NOT NULL,
                 expires_at TIMESTAMPTZ NOT NULL,
                 revoked_at TIMESTAMPTZ,
                 updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                 UNIQUE (installation_id, relay_pubkey)
             );
             CREATE TABLE push_gateway_endpoint_quotas (
                 token_fingerprint BYTEA PRIMARY KEY CHECK (length(token_fingerprint) = 32),
                 window_started_at TIMESTAMPTZ NOT NULL,
                 admitted BIGINT NOT NULL,
                 updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
             );
             CREATE TABLE push_gateway_delivery_auth_replays (
                 relay_pubkey BYTEA NOT NULL CHECK (length(relay_pubkey) = 32),
                 auth_event_id BYTEA NOT NULL CHECK (length(auth_event_id) = 32),
                 expires_at TIMESTAMPTZ NOT NULL,
                 PRIMARY KEY (relay_pubkey, auth_event_id)
             );
             CREATE TABLE push_gateway_delivery_request_replays (
                 relay_pubkey BYTEA NOT NULL CHECK (length(relay_pubkey) = 32),
                 request_id UUID NOT NULL,
                 expires_at TIMESTAMPTZ NOT NULL,
                 PRIMARY KEY (relay_pubkey, request_id)
             );",
        )
        .execute(&pool)
        .await
        .expect("create authority admission tables");
        (pool, schema)
    }

    const RELAY_HEX: &str = "11111111111111111111111111111111111111111111111111111111111111aa";
    const DELEGATION_ID: u128 = 2;

    // One installation + one live delegation that admits at now=1_000.
    async fn install_authority(pool: &PgPool) {
        let now = Utc::now();
        sqlx::query(
            "INSERT INTO push_gateway_installations(id,app_attest_key_id,app_attest_public_key,assertion_counter,app_profile,token_ciphertext,token_fingerprint,endpoint_epoch,expires_at)
             VALUES ($1,$2,$3,0,'buzz-ios-production',$4,$5,1,$6)",
        )
        .bind(Uuid::from_u128(1))
        .bind(vec![1u8])
        .bind(vec![2u8; 33])
        .bind(vec![3u8])
        .bind(vec![4u8; 32])
        .bind(now + chrono::Duration::days(30))
        .execute(pool)
        .await
        .expect("insert installation");
        sqlx::query(
            "INSERT INTO push_gateway_delegations(id,installation_id,relay_pubkey,endpoint_epoch,generation,not_before,expires_at,revoked_at)
             VALUES ($1,$2,$3,1,1,$4,$5,NULL)",
        )
        .bind(Uuid::from_u128(DELEGATION_ID))
        .bind(Uuid::from_u128(1))
        .bind(hex::decode(RELAY_HEX).unwrap())
        .bind(now - chrono::Duration::days(1))
        .bind(now + chrono::Duration::days(7))
        .execute(pool)
        .await
        .expect("insert delegation");
    }

    fn admit<'a>(
        store: &'a PostgresAuthorityStore,
        event_hex: &'a str,
        request_id: Uuid,
    ) -> impl std::future::Future<Output = Result<DeliveryPermit, AuthorityError>> + 'a {
        let now = Utc::now().timestamp();
        store.authorize_delivery(
            Uuid::from_u128(DELEGATION_ID),
            RELAY_HEX,
            1,
            1,
            event_hex,
            request_id,
            now + 300,
            60,
            10,
            now,
        )
    }

    // Two concurrent admissions colliding on the same (relay,request_id) PK must
    // admit exactly once; the loser rejects with its whole tx rolled back, so
    // quota is charged once and the auth-event fence is not consumed by the loser.
    #[tokio::test]
    #[ignore = "requires PostgreSQL"]
    async fn concurrent_same_request_id_admits_exactly_once() {
        let (pool, schema) = full_schema(4).await;
        install_authority(&pool).await;
        let store = PostgresAuthorityStore::new(pool.clone());
        let request_id = Uuid::new_v4();
        let event_a = "22".repeat(32);
        let event_b = "33".repeat(32);

        let (a, b) = tokio::join!(
            admit(&store, &event_a, request_id),
            admit(&store, &event_b, request_id),
        );
        assert_eq!(
            [a.is_ok(), b.is_ok()].iter().filter(|ok| **ok).count(),
            1,
            "exactly one concurrent same-request_id admission may win"
        );

        let requests: i64 =
            sqlx::query_scalar("SELECT count(*) FROM push_gateway_delivery_request_replays")
                .fetch_one(&pool)
                .await
                .expect("count request replays");
        assert_eq!(requests, 1, "winner leaves exactly one request-id fence");
        let auth_events: i64 =
            sqlx::query_scalar("SELECT count(*) FROM push_gateway_delivery_auth_replays")
                .fetch_one(&pool)
                .await
                .expect("count auth replays");
        assert_eq!(auth_events, 1, "loser's auth-event insert rolled back");
        let admitted: i64 = sqlx::query_scalar("SELECT admitted FROM push_gateway_endpoint_quotas")
            .fetch_one(&pool)
            .await
            .expect("read quota");
        assert_eq!(admitted, 1, "loser's quota reservation rolled back");

        pool.close().await;
        drop_schema(&schema).await;
    }

    // Retryable release must free the real request-id PK: after finish_delivery,
    // the same request_id re-admits with a fresh auth event; a Terminal finish
    // leaves it burned.
    #[tokio::test]
    #[ignore = "requires PostgreSQL"]
    async fn retryable_release_frees_request_id_on_real_postgres() {
        let (pool, schema) = full_schema(2).await;
        install_authority(&pool).await;
        let store = PostgresAuthorityStore::new(pool.clone());
        let request_id = Uuid::new_v4();

        let permit = admit(&store, &"22".repeat(32), request_id)
            .await
            .expect("first admission");
        store
            .finish_delivery(permit, DeliveryDisposition::Retryable)
            .await
            .expect("retryable release");
        // Same request_id, fresh auth event: released PK admits again.
        let permit = admit(&store, &"33".repeat(32), request_id)
            .await
            .expect("retryable release frees the request-id PK");
        // Terminal now burns it: a further re-admit with the same request_id fails.
        store
            .finish_delivery(permit, DeliveryDisposition::Terminal)
            .await
            .expect("terminal finish");
        assert!(
            admit(&store, &"44".repeat(32), request_id).await.is_err(),
            "terminal outcome keeps the request-id fence burned"
        );

        pool.close().await;
        drop_schema(&schema).await;
    }

    async fn drop_schema(schema: &str) {
        let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap_or_else(|_| TEST_DB_URL.to_owned());
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .connect(&database_url)
            .await
            .expect("connect for teardown");
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop isolated test schema");
    }
}
