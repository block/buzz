//! Immutable provider-free enrollment-policy reconciliation.
//!
//! Production supplies complete typed semantics, never a caller-selected
//! digest. This module computes the canonical digest and atomically installs
//! both policy revisions and their immutable event-capacity policies before
//! an enabled runtime can become observable.

use std::{collections::HashSet, fmt, time::Duration};

use buzz_auth::AuthorizationEventCapacityPolicy;
use buzz_core::CommunityId;
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;

use crate::{
    authorization_events::{
        reconcile_authorization_event_capacities_tx, validate_authorization_event_capacities,
    },
    Db, DbError, Result,
};

/// Exact acknowledgement required by the risk-labelled TOFU posture.
pub const RISK_LABELLED_TOFU_ACKNOWLEDGEMENT_V1: &str = "I_ACKNOWLEDGE_RISK_LABELLED_TOFU_V1";

/// Closed provider-free enrollment modes persisted by migration 0029.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i16)]
pub enum AuthorizationEnrollmentMode {
    /// The accepted assertion attests the exact event-author key.
    AttestedKey = 1,
    /// A separately verified provisioning intent selects the key.
    Provisioned = 2,
    /// A local risk-labelled first-use decision selects the key.
    RiskLabelledTofu = 3,
}

/// Complete verifier, provenance, and bound semantics for one policy.
#[derive(Clone, PartialEq, Eq)]
pub struct AuthorizationEnrollmentPolicySemantics {
    issuer: String,
    audience: String,
    subject_claim: String,
    assertion_key_set_digest: [u8; 32],
    nostr_key_claim: Option<String>,
    assertion_header: String,
    provenance_header: String,
    provenance_key_digest: [u8; 32],
    maximum_token_age_seconds: u64,
    clock_skew_seconds: u64,
    maximum_lease_seconds: u64,
    maximum_events: u64,
    maximum_event_bytes: u64,
    maximum_envelope_bytes: u32,
    risk_acknowledgement: Option<String>,
    restore_bootstrap_id: Uuid,
}

impl AuthorizationEnrollmentPolicySemantics {
    /// Validate all bytes that contribute to the immutable policy digest.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        issuer: impl Into<String>,
        audience: impl Into<String>,
        subject_claim: impl Into<String>,
        assertion_key_set_digest: [u8; 32],
        nostr_key_claim: Option<String>,
        assertion_header: impl Into<String>,
        provenance_header: impl Into<String>,
        provenance_key_digest: [u8; 32],
        maximum_token_age: Duration,
        clock_skew: Duration,
        maximum_lease: Duration,
        event_capacity: AuthorizationEventCapacityPolicy,
        risk_acknowledgement: Option<String>,
        restore_bootstrap_id: Uuid,
    ) -> Result<Self> {
        let issuer = issuer.into();
        let audience = audience.into();
        let subject_claim = subject_claim.into();
        let assertion_header = assertion_header.into();
        let provenance_header = provenance_header.into();
        if !valid_exact_part(&issuer, 2_048)
            || !valid_exact_part(&audience, 2_048)
            || !valid_exact_part(&subject_claim, 256)
            || nostr_key_claim
                .as_deref()
                .is_some_and(|claim| !valid_exact_part(claim, 256))
            || !valid_exact_part(&assertion_header, 256)
            || !valid_exact_part(&provenance_header, 256)
            || assertion_header == provenance_header
            || assertion_key_set_digest == [0; 32]
            || provenance_key_digest == [0; 32]
            || maximum_token_age.is_zero()
            || maximum_token_age > Duration::from_secs(86_400)
            || maximum_token_age.subsec_nanos() != 0
            || clock_skew > Duration::from_secs(300)
            || clock_skew.subsec_nanos() != 0
            || maximum_lease.is_zero()
            || maximum_lease > Duration::from_secs(300)
            || maximum_lease.subsec_nanos() != 0
            || restore_bootstrap_id.is_nil()
        {
            return Err(invalid_policy());
        }
        Ok(Self {
            issuer,
            audience,
            subject_claim,
            assertion_key_set_digest,
            nostr_key_claim,
            assertion_header,
            provenance_header,
            provenance_key_digest,
            maximum_token_age_seconds: maximum_token_age.as_secs(),
            clock_skew_seconds: clock_skew.as_secs(),
            maximum_lease_seconds: maximum_lease.as_secs(),
            maximum_events: event_capacity.max_events_per_domain(),
            maximum_event_bytes: event_capacity.max_bytes_per_domain(),
            maximum_envelope_bytes: event_capacity.max_envelope_bytes(),
            risk_acknowledgement,
            restore_bootstrap_id,
        })
    }
}

impl fmt::Debug for AuthorizationEnrollmentPolicySemantics {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AuthorizationEnrollmentPolicySemantics([REDACTED])")
    }
}

/// One complete immutable enrollment-policy revision.
#[derive(Clone, PartialEq, Eq)]
pub struct AuthorizationEnrollmentPolicy {
    authorization_domain: CommunityId,
    policy_revision: u64,
    enrollment_mode: AuthorizationEnrollmentMode,
    policy_digest: [u8; 32],
    event_capacity: AuthorizationEventCapacityPolicy,
    effective_at: DateTime<Utc>,
}

impl AuthorizationEnrollmentPolicy {
    /// Construct a stable, immediately effective policy revision.
    pub fn new(
        authorization_domain: CommunityId,
        policy_revision: u64,
        enrollment_mode: AuthorizationEnrollmentMode,
        semantics: AuthorizationEnrollmentPolicySemantics,
    ) -> Result<Self> {
        if authorization_domain.as_uuid().is_nil()
            || policy_revision == 0
            || policy_revision > i64::MAX as u64
            || !valid_mode_semantics(enrollment_mode, &semantics)
        {
            return Err(invalid_policy());
        }
        let effective_at = DateTime::from_timestamp(0, 0).ok_or_else(invalid_policy)?;
        let policy_digest = policy_digest(
            authorization_domain,
            policy_revision,
            enrollment_mode,
            effective_at,
            &semantics,
        );
        let event_capacity = AuthorizationEventCapacityPolicy::new(
            semantics.maximum_events,
            semantics.maximum_event_bytes,
            semantics.maximum_envelope_bytes,
        )
        .map_err(|_| invalid_policy())?;
        Ok(Self {
            authorization_domain,
            policy_revision,
            enrollment_mode,
            policy_digest,
            event_capacity,
            effective_at,
        })
    }

    /// Server-resolved authorization domain.
    pub const fn authorization_domain(&self) -> CommunityId {
        self.authorization_domain
    }

    /// Positive immutable policy revision.
    pub const fn policy_revision(&self) -> u64 {
        self.policy_revision
    }

    /// Closed enrollment mode.
    pub const fn enrollment_mode(&self) -> AuthorizationEnrollmentMode {
        self.enrollment_mode
    }

    /// Internally computed canonical policy digest.
    pub const fn policy_digest(&self) -> [u8; 32] {
        self.policy_digest
    }
}

impl fmt::Debug for AuthorizationEnrollmentPolicy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthorizationEnrollmentPolicy")
            .field("policy_revision", &self.policy_revision)
            .field("enrollment_mode", &self.enrollment_mode)
            .field("coordinates", &"[REDACTED]")
            .finish()
    }
}

/// Atomic reconciliation result for one complete startup set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AuthorizationPolicyReconciliation {
    inserted: usize,
    replayed: usize,
}

impl AuthorizationPolicyReconciliation {
    /// Newly inserted immutable policy rows.
    pub const fn inserted(self) -> usize {
        self.inserted
    }

    /// Byte-identical current policy rows.
    pub const fn replayed(self) -> usize {
        self.replayed
    }
}

impl Db {
    /// Atomically reconcile complete policy and event-capacity configuration.
    ///
    /// Validation of the full set precedes I/O. Per-domain parent-row locks
    /// serialize revision selection. Only an exact current replay or a
    /// strictly greater revision is accepted; policy and capacity writes
    /// commit together or leave zero residue.
    pub async fn reconcile_authorization_enrollment_startup(
        &self,
        configurations: &[(
            AuthorizationEnrollmentPolicy,
            AuthorizationEventCapacityPolicy,
        )],
    ) -> Result<AuthorizationPolicyReconciliation> {
        let mut domains = HashSet::with_capacity(configurations.len());
        let capacities = configurations
            .iter()
            .map(|(policy, capacity)| {
                if policy.event_capacity != *capacity
                    || !domains.insert(policy.authorization_domain)
                {
                    return Err(DbError::InvalidData(
                        "authorization enrollment startup domain set is invalid".to_owned(),
                    ));
                }
                Ok((policy.authorization_domain, *capacity))
            })
            .collect::<Result<Vec<_>>>()?;
        validate_authorization_event_capacities(&capacities)?;
        if configurations.is_empty() {
            return Ok(AuthorizationPolicyReconciliation::default());
        }

        let mut ordered = configurations.to_vec();
        ordered.sort_by_key(|(policy, _)| *policy.authorization_domain.as_uuid());
        let mut transaction = self.pool.begin().await?;
        let mut reconciliation = AuthorizationPolicyReconciliation::default();
        for (policy, _) in &ordered {
            if let Err(error) =
                reconcile_policy_tx(&mut transaction, policy, &mut reconciliation).await
            {
                transaction.rollback().await?;
                return Err(error);
            }
        }
        let ordered_capacities = ordered
            .iter()
            .map(|(policy, capacity)| (policy.authorization_domain, *capacity))
            .collect::<Vec<_>>();
        if let Err(error) =
            reconcile_authorization_event_capacities_tx(&mut transaction, &ordered_capacities).await
        {
            transaction.rollback().await?;
            return Err(error);
        }
        transaction.commit().await?;
        Ok(reconciliation)
    }
}

async fn reconcile_policy_tx(
    transaction: &mut Transaction<'_, Postgres>,
    policy: &AuthorizationEnrollmentPolicy,
    reconciliation: &mut AuthorizationPolicyReconciliation,
) -> Result<()> {
    let community_exists =
        sqlx::query_scalar::<_, Uuid>("SELECT id FROM communities WHERE id=$1 FOR UPDATE")
            .bind(policy.authorization_domain.as_uuid())
            .fetch_optional(&mut **transaction)
            .await?
            .is_some();
    if !community_exists {
        return Err(invalid_policy());
    }
    let maximum_revision: Option<i64> = sqlx::query_scalar(
        "SELECT MAX(policy_revision) FROM identity_enrollment_policies WHERE community_id=$1",
    )
    .bind(policy.authorization_domain.as_uuid())
    .fetch_one(&mut **transaction)
    .await?;
    let revision = i64::try_from(policy.policy_revision).map_err(|_| {
        DbError::InvalidData("authorization policy revision is out of range".into())
    })?;
    match maximum_revision {
        Some(current) if revision < current => {
            return Err(DbError::InvalidData(
                "authorization enrollment policy revision rolled back".to_owned(),
            ));
        }
        Some(current) if revision == current => {
            require_exact_policy_row(transaction, policy, revision).await?;
            reconciliation.replayed += 1;
            return Ok(());
        }
        _ => {}
    }

    sqlx::query(
        "INSERT INTO identity_enrollment_policies \
         (community_id,policy_revision,enrollment_mode,policy_digest,effective_at) \
         VALUES ($1,$2,$3,$4,$5)",
    )
    .bind(policy.authorization_domain.as_uuid())
    .bind(revision)
    .bind(policy.enrollment_mode as i16)
    .bind(policy.policy_digest.as_slice())
    .bind(policy.effective_at)
    .execute(&mut **transaction)
    .await?;
    require_exact_policy_row(transaction, policy, revision).await?;
    reconciliation.inserted += 1;
    Ok(())
}

async fn require_exact_policy_row(
    transaction: &mut Transaction<'_, Postgres>,
    policy: &AuthorizationEnrollmentPolicy,
    revision: i64,
) -> Result<()> {
    let row = sqlx::query(
        "SELECT enrollment_mode,policy_digest,effective_at,expires_at \
         FROM identity_enrollment_policies \
         WHERE community_id=$1 AND policy_revision=$2 FOR UPDATE",
    )
    .bind(policy.authorization_domain.as_uuid())
    .bind(revision)
    .fetch_one(&mut **transaction)
    .await?;
    let stored_digest: Vec<u8> = row.try_get("policy_digest")?;
    let stored_digest: [u8; 32] = stored_digest
        .try_into()
        .map_err(|_| DbError::InvalidData("authorization policy digest is malformed".to_owned()))?;
    if row.try_get::<i16, _>("enrollment_mode")? != policy.enrollment_mode as i16
        || stored_digest != policy.policy_digest
        || row.try_get::<DateTime<Utc>, _>("effective_at")? != policy.effective_at
        || row
            .try_get::<Option<DateTime<Utc>>, _>("expires_at")?
            .is_some()
    {
        return Err(DbError::InvalidData(
            "authorization enrollment policy revision conflicts".to_owned(),
        ));
    }
    Ok(())
}

fn valid_mode_semantics(
    mode: AuthorizationEnrollmentMode,
    semantics: &AuthorizationEnrollmentPolicySemantics,
) -> bool {
    match mode {
        AuthorizationEnrollmentMode::AttestedKey => {
            semantics.nostr_key_claim.is_some() && semantics.risk_acknowledgement.is_none()
        }
        AuthorizationEnrollmentMode::Provisioned => semantics.risk_acknowledgement.is_none(),
        AuthorizationEnrollmentMode::RiskLabelledTofu => {
            semantics.risk_acknowledgement.as_deref() == Some(RISK_LABELLED_TOFU_ACKNOWLEDGEMENT_V1)
        }
    }
}

fn policy_digest(
    authorization_domain: CommunityId,
    policy_revision: u64,
    enrollment_mode: AuthorizationEnrollmentMode,
    effective_at: DateTime<Utc>,
    semantics: &AuthorizationEnrollmentPolicySemantics,
) -> [u8; 32] {
    let mut digest = Sha256::new();
    framed(&mut digest, b"buzz:authorization-enrollment-policy:v2");
    framed(&mut digest, authorization_domain.as_uuid().as_bytes());
    framed(&mut digest, &policy_revision.to_be_bytes());
    framed(&mut digest, &(enrollment_mode as i16).to_be_bytes());
    framed(&mut digest, semantics.issuer.as_bytes());
    framed(&mut digest, semantics.audience.as_bytes());
    framed(&mut digest, semantics.subject_claim.as_bytes());
    framed(&mut digest, &semantics.assertion_key_set_digest);
    match semantics.nostr_key_claim.as_deref() {
        Some(claim) => {
            framed(&mut digest, &[1]);
            framed(&mut digest, b"nostr-key-claim/lower-hex-v1");
            framed(&mut digest, claim.as_bytes());
        }
        None => framed(&mut digest, &[0]),
    }
    framed(&mut digest, b"trusted-proxy-assertion-v1");
    framed(&mut digest, semantics.assertion_header.as_bytes());
    framed(&mut digest, semantics.provenance_header.as_bytes());
    framed(&mut digest, &semantics.provenance_key_digest);
    framed(
        &mut digest,
        &semantics.maximum_token_age_seconds.to_be_bytes(),
    );
    framed(&mut digest, &semantics.clock_skew_seconds.to_be_bytes());
    framed(&mut digest, &semantics.maximum_lease_seconds.to_be_bytes());
    framed(&mut digest, &semantics.maximum_events.to_be_bytes());
    framed(&mut digest, &semantics.maximum_event_bytes.to_be_bytes());
    framed(&mut digest, &semantics.maximum_envelope_bytes.to_be_bytes());
    framed_optional(&mut digest, semantics.risk_acknowledgement.as_deref());
    framed(&mut digest, semantics.restore_bootstrap_id.as_bytes());
    framed(&mut digest, &effective_at.timestamp().to_be_bytes());
    framed(
        &mut digest,
        &effective_at.timestamp_subsec_nanos().to_be_bytes(),
    );
    framed(&mut digest, &[0]); // Base V1 policies have no expiry.
    digest.finalize().into()
}

fn framed_optional(digest: &mut Sha256, value: Option<&str>) {
    match value {
        Some(value) => {
            framed(digest, &[1]);
            framed(digest, value.as_bytes());
        }
        None => framed(digest, &[0]),
    }
}

fn framed(digest: &mut Sha256, value: &[u8]) {
    digest.update((value.len() as u64).to_be_bytes());
    digest.update(value);
}

fn valid_exact_part(value: &str, maximum_bytes: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum_bytes
        && !value.chars().any(|character| character.is_control())
}

fn invalid_policy() -> DbError {
    DbError::InvalidData("authorization enrollment policy is invalid".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::postgres::PgPoolOptions;

    fn loopback_test_database_url() -> String {
        format!(
            "{}://{}:{}@{}:{}/{}",
            "postgres", "buzz", "buzz_dev", "localhost", 5432, "buzz"
        )
    }

    fn domain(value: u128) -> CommunityId {
        CommunityId::from_uuid(Uuid::from_u128(value))
    }

    fn capacity(events: u64) -> AuthorizationEventCapacityPolicy {
        AuthorizationEventCapacityPolicy::new(events, 1 << 20, 16 << 10).expect("capacity")
    }

    fn semantics(
        key_claim: Option<&str>,
        acknowledgement: Option<&str>,
        capacity: AuthorizationEventCapacityPolicy,
    ) -> AuthorizationEnrollmentPolicySemantics {
        AuthorizationEnrollmentPolicySemantics::new(
            "https://issuer.example",
            "buzz-relay",
            "employee_id",
            [7; 32],
            key_claim.map(str::to_owned),
            "x-buzz-identity",
            "x-buzz-identity-proof",
            [9; 32],
            Duration::from_secs(300),
            Duration::from_secs(30),
            Duration::from_secs(60),
            capacity,
            acknowledgement.map(str::to_owned),
            Uuid::from_u128(100),
        )
        .expect("valid semantics")
    }

    fn policy(
        domain: CommunityId,
        revision: u64,
        mode: AuthorizationEnrollmentMode,
        capacity: AuthorizationEventCapacityPolicy,
    ) -> AuthorizationEnrollmentPolicy {
        let (claim, acknowledgement) = match mode {
            AuthorizationEnrollmentMode::AttestedKey => (Some("nostr_pubkey"), None),
            AuthorizationEnrollmentMode::Provisioned => (None, None),
            AuthorizationEnrollmentMode::RiskLabelledTofu => {
                (None, Some(RISK_LABELLED_TOFU_ACKNOWLEDGEMENT_V1))
            }
        };
        AuthorizationEnrollmentPolicy::new(
            domain,
            revision,
            mode,
            semantics(claim, acknowledgement, capacity),
        )
        .expect("valid policy")
    }

    #[test]
    fn canonical_policy_binds_complete_semantics_and_mode_specific_proof() {
        let event_capacity = capacity(100);
        let original = semantics(Some("nostr_pubkey"), None, event_capacity);
        let digest_for = |semantics| {
            AuthorizationEnrollmentPolicy::new(
                domain(1),
                2,
                AuthorizationEnrollmentMode::AttestedKey,
                semantics,
            )
            .expect("valid policy")
            .policy_digest()
        };
        let first = AuthorizationEnrollmentPolicy::new(
            domain(1),
            2,
            AuthorizationEnrollmentMode::AttestedKey,
            original.clone(),
        )
        .expect("valid policy");
        let replay = AuthorizationEnrollmentPolicy::new(
            domain(1),
            2,
            AuthorizationEnrollmentMode::AttestedKey,
            original,
        )
        .expect("valid replay");
        assert_eq!(first, replay);
        let original_digest = first.policy_digest();
        let mut changes = Vec::new();
        let mut changed = semantics(Some("nostr_pubkey"), None, event_capacity);
        changed.issuer = "https://issuer.changed.example".to_owned();
        changes.push(changed);
        let mut changed = semantics(Some("nostr_pubkey"), None, event_capacity);
        changed.audience = "buzz-relay-changed".to_owned();
        changes.push(changed);
        let mut changed = semantics(Some("nostr_pubkey"), None, event_capacity);
        changed.subject_claim = "opaque_subject".to_owned();
        changes.push(changed);
        let mut changed = semantics(Some("nostr_pubkey"), None, event_capacity);
        changed.assertion_key_set_digest = [8; 32];
        changes.push(changed);
        let mut changed = semantics(Some("nostr_pubkey"), None, event_capacity);
        changed.nostr_key_claim = Some("event_author".to_owned());
        changes.push(changed);
        let mut changed = semantics(Some("nostr_pubkey"), None, event_capacity);
        changed.assertion_header = "x-buzz-other-identity".to_owned();
        changes.push(changed);
        let mut changed = semantics(Some("nostr_pubkey"), None, event_capacity);
        changed.provenance_header = "x-buzz-other-proof".to_owned();
        changes.push(changed);
        let mut changed = semantics(Some("nostr_pubkey"), None, event_capacity);
        changed.provenance_key_digest = [10; 32];
        changes.push(changed);
        let mut changed = semantics(Some("nostr_pubkey"), None, event_capacity);
        changed.maximum_token_age_seconds = 301;
        changes.push(changed);
        let mut changed = semantics(Some("nostr_pubkey"), None, event_capacity);
        changed.clock_skew_seconds = 31;
        changes.push(changed);
        let mut changed = semantics(Some("nostr_pubkey"), None, event_capacity);
        changed.maximum_lease_seconds = 61;
        changes.push(changed);
        let mut changed = semantics(Some("nostr_pubkey"), None, event_capacity);
        changed.maximum_events = 101;
        changes.push(changed);
        let mut changed = semantics(Some("nostr_pubkey"), None, event_capacity);
        changed.maximum_event_bytes += 1;
        changes.push(changed);
        let mut changed = semantics(Some("nostr_pubkey"), None, event_capacity);
        changed.maximum_envelope_bytes = 8 << 10;
        changes.push(changed);
        let mut changed = semantics(Some("nostr_pubkey"), None, event_capacity);
        changed.restore_bootstrap_id = Uuid::from_u128(101);
        changes.push(changed);
        for changed in changes {
            assert_ne!(original_digest, digest_for(changed));
        }

        let tofu = semantics(
            None,
            Some(RISK_LABELLED_TOFU_ACKNOWLEDGEMENT_V1),
            event_capacity,
        );
        let acknowledged_digest = policy_digest(
            domain(1),
            2,
            AuthorizationEnrollmentMode::RiskLabelledTofu,
            DateTime::from_timestamp(0, 0).expect("epoch"),
            &tofu,
        );
        let mut changed_acknowledgement = tofu;
        changed_acknowledgement.risk_acknowledgement = Some("wrong".to_owned());
        assert_ne!(
            acknowledged_digest,
            policy_digest(
                domain(1),
                2,
                AuthorizationEnrollmentMode::RiskLabelledTofu,
                DateTime::from_timestamp(0, 0).expect("epoch"),
                &changed_acknowledgement,
            )
        );

        assert!(AuthorizationEnrollmentPolicy::new(
            domain(1),
            3,
            AuthorizationEnrollmentMode::AttestedKey,
            semantics(None, None, event_capacity),
        )
        .is_err());
        assert!(AuthorizationEnrollmentPolicy::new(
            domain(1),
            3,
            AuthorizationEnrollmentMode::RiskLabelledTofu,
            semantics(None, None, event_capacity),
        )
        .is_err());
        assert!(AuthorizationEnrollmentPolicy::new(
            domain(1),
            3,
            AuthorizationEnrollmentMode::Provisioned,
            semantics(
                None,
                Some(RISK_LABELLED_TOFU_ACKNOWLEDGEMENT_V1),
                event_capacity
            ),
        )
        .is_err());
        assert!(!format!("{first:?}").contains(&domain(1).as_uuid().to_string()));
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn postgres_startup_reconciliation_is_monotonic_and_zero_residue() {
        let admin_url =
            std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| loopback_test_database_url());
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect(&admin_url)
            .await
            .expect("connect admin");
        let database_name = format!("s5_policy_{}", Uuid::new_v4().simple());
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "CREATE DATABASE {database_name}"
        )))
        .execute(&admin)
        .await
        .expect("create scratch database");
        let split = admin_url.rfind('/').expect("database URL path");
        let scratch_url = format!("{}/{database_name}", &admin_url[..split]);
        let pool = PgPoolOptions::new()
            .max_connections(2)
            .connect(&scratch_url)
            .await
            .expect("connect scratch database");
        crate::migration::run_migrations(&pool)
            .await
            .expect("migrate scratch database");

        let first_domain = domain(1);
        let conflict_domain = domain(2);
        for (community, host) in [
            (first_domain, "policy-first.example"),
            (conflict_domain, "policy-conflict.example"),
        ] {
            sqlx::query("INSERT INTO communities (id,host) VALUES ($1,$2)")
                .bind(community.as_uuid())
                .bind(host)
                .execute(&pool)
                .await
                .expect("insert community");
        }
        let db = Db::from_pool(pool.clone());
        let installed_capacity = capacity(100);
        let mismatched_capacity_policy = policy(
            first_domain,
            1,
            AuthorizationEnrollmentMode::Provisioned,
            installed_capacity,
        );
        assert!(db
            .reconcile_authorization_enrollment_startup(&[(
                mismatched_capacity_policy,
                capacity(101),
            )])
            .await
            .is_err());
        let preflight_residue: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM identity_enrollment_policies WHERE community_id=$1",
        )
        .bind(first_domain.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("count capacity-policy mismatch residue");
        assert_eq!(preflight_residue, 0);
        let original = policy(
            conflict_domain,
            7,
            AuthorizationEnrollmentMode::AttestedKey,
            installed_capacity,
        );
        let inserted = db
            .reconcile_authorization_enrollment_startup(&[(original.clone(), installed_capacity)])
            .await
            .expect("insert original policy and capacity");
        assert_eq!(inserted.inserted(), 1);
        let replayed = db
            .reconcile_authorization_enrollment_startup(&[(original.clone(), installed_capacity)])
            .await
            .expect("exact current replay");
        assert_eq!(replayed.replayed(), 1);

        let lower = policy(
            conflict_domain,
            6,
            AuthorizationEnrollmentMode::AttestedKey,
            installed_capacity,
        );
        assert!(db
            .reconcile_authorization_enrollment_startup(&[(lower, installed_capacity)])
            .await
            .is_err());
        let drift = policy(
            conflict_domain,
            7,
            AuthorizationEnrollmentMode::RiskLabelledTofu,
            installed_capacity,
        );
        let candidate_before_drift = policy(
            first_domain,
            2,
            AuthorizationEnrollmentMode::Provisioned,
            installed_capacity,
        );
        assert!(db
            .reconcile_authorization_enrollment_startup(&[
                (candidate_before_drift, installed_capacity),
                (drift, installed_capacity),
            ])
            .await
            .is_err());
        let rows_after_drift: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM identity_enrollment_policies WHERE community_id=$1",
        )
        .bind(first_domain.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("count same-revision rollback residue");
        assert_eq!(rows_after_drift, 0);

        let candidate = policy(
            first_domain,
            3,
            AuthorizationEnrollmentMode::Provisioned,
            capacity(101),
        );
        let greater = policy(
            conflict_domain,
            8,
            AuthorizationEnrollmentMode::RiskLabelledTofu,
            capacity(101),
        );
        assert!(db
            .reconcile_authorization_enrollment_startup(&[
                (candidate, capacity(101)),
                (greater, capacity(101)),
            ])
            .await
            .is_err());
        let first_policy_rows: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM identity_enrollment_policies WHERE community_id=$1",
        )
        .bind(first_domain.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("count rolled-back policies");
        let first_capacity_rows: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM authorization_event_capacity WHERE community_id=$1",
        )
        .bind(first_domain.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("count rolled-back capacities");
        let conflict_max: i64 = sqlx::query_scalar(
            "SELECT MAX(policy_revision) FROM identity_enrollment_policies WHERE community_id=$1",
        )
        .bind(conflict_domain.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("read unchanged maximum revision");
        assert_eq!(first_policy_rows, 0);
        assert_eq!(first_capacity_rows, 0);
        assert_eq!(conflict_max, 7);

        let greater = policy(
            conflict_domain,
            8,
            AuthorizationEnrollmentMode::RiskLabelledTofu,
            installed_capacity,
        );
        let advanced = db
            .reconcile_authorization_enrollment_startup(&[(greater, installed_capacity)])
            .await
            .expect("strictly greater revision");
        assert_eq!(advanced.inserted(), 1);

        pool.close().await;
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "DROP DATABASE {database_name} WITH (FORCE)"
        )))
        .execute(&admin)
        .await
        .expect("drop scratch database");
    }
}
