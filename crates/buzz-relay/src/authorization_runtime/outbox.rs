//! Crash-durable adapter for the accepted connection-local status contract.

use std::fmt;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use buzz_auth::{BoundedAuthorizationLease, NipFiMode, ProofTransport, RouteCapability};
use buzz_core::client_binding_bootstrap::{
    ClientBindingBootstrapInputV1, CLIENT_BINDING_BOOTSTRAP_SUB_ID, CLIENT_BINDING_STATUS_SUB_ID,
};
use buzz_core::client_binding_status::{
    validate_client_binding_status_event, ClientBindingStatusDisposition,
    ClientBindingStatusInputV1,
};
use buzz_core::{CanonicalCurrentBindingEvidence, CommunityId};
use buzz_db::client_status_delivery::{
    ClaimedStatusDelivery, CompleteStatusDeliveryOutcome, NewStatusDelivery, StatusDeliveryFailure,
    StatusDeliveryKind,
};
use buzz_db::Db;
use chrono::{DateTime, Utc};
use nostr::{Event, Keys, PublicKey};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::sync::watch;
use tracing::warn;
use uuid::Uuid;

use super::status::{
    CurrentStatusContract, CurrentStatusEvidenceSource, CurrentStatusSink, StatusSessionError,
    UnchangedBootstrapDelivery,
};
use crate::connection::{AuthState, ConnectionState, StatusWriteIdentity, StatusWriter};
use crate::protocol::RelayMessage;

const CLAIM_LEASE: Duration = Duration::from_secs(30);
const BOOTSTRAP_WRITE_LIMIT: Duration = Duration::from_secs(5);
const MAX_STATUS_OPERATION_SECONDS: u64 = 7;

type StatusWriteResult = Result<(), StatusSessionError>;
type StatusWriteReceiver = watch::Receiver<Option<StatusWriteResult>>;

async fn bounded_status_operation<F>(limit: Duration, operation: F) -> StatusWriteResult
where
    F: Future<Output = StatusWriteResult>,
{
    tokio::time::timeout(limit, operation)
        .await
        .unwrap_or(Err(StatusSessionError::DeliveryFailed))
}

/// Signed current event paired with the private tuple needed after a crash.
pub struct DurableCurrentStatus {
    event: Event,
    evidence: CanonicalCurrentBindingEvidence,
}

impl fmt::Debug for DurableCurrentStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("DurableCurrentStatus([REDACTED])")
    }
}

/// Typed status contract whose current value retains private recovery evidence.
#[derive(Clone)]
pub struct DurableStatusContract {
    relay_keys: Keys,
    contract_fingerprint: [u8; 32],
}

impl DurableStatusContract {
    /// Bind signing to the exact relay key pinned by the connection scope.
    pub fn new(relay_keys: Keys, pinned_relay: PublicKey) -> Result<Self, StatusSessionError> {
        if relay_keys.public_key() != pinned_relay {
            return Err(StatusSessionError::ContractUnavailable);
        }
        Ok(Self {
            relay_keys,
            contract_fingerprint: Sha256::digest(b"buzz:client-binding-status:connection-local:v1")
                .into(),
        })
    }
}

impl CurrentStatusContract for DurableStatusContract {
    type Current = DurableCurrentStatus;
    type Withdrawal = Event;

    fn contract_fingerprint(&self) -> [u8; 32] {
        self.contract_fingerprint
    }

    fn current(
        &self,
        evidence: &CanonicalCurrentBindingEvidence,
        connection_revision: u64,
    ) -> Result<Self::Current, StatusSessionError> {
        // Nostr timestamps and the accepted kind-24244 contract are whole
        // seconds. Retain that exact representation in the private recovery
        // tuple too, otherwise PostgreSQL microseconds can never equal the
        // parsed wire freshness bound during enqueue validation.
        let observed_at = DateTime::<Utc>::from_timestamp(evidence.observed_at().timestamp(), 0)
            .ok_or(StatusSessionError::ContractUnavailable)?;
        let fresh_until = DateTime::<Utc>::from_timestamp(evidence.fresh_until().timestamp(), 0)
            .ok_or(StatusSessionError::ContractUnavailable)?;
        let normalized = CanonicalCurrentBindingEvidence::new(
            evidence.authorization_domain(),
            evidence.event_author_pubkey(),
            evidence.binding_id(),
            evidence.binding_version(),
            evidence.policy_revision(),
            evidence.invalidation_generation(),
            evidence.authority_epoch(),
            evidence.fence(),
            observed_at,
            fresh_until,
        )
        .map_err(|_| StatusSessionError::ContractUnavailable)?;
        let event =
            ClientBindingStatusInputV1::current_from_evidence(&normalized, connection_revision)
                .map_err(|_| StatusSessionError::ContractUnavailable)?
                .sign_with_relay_keys(&self.relay_keys)
                .map_err(|_| StatusSessionError::ContractUnavailable)?;
        Ok(DurableCurrentStatus {
            event,
            evidence: normalized,
        })
    }

    fn withdrawal(
        &self,
        domain: CommunityId,
        author: PublicKey,
        connection_revision: u64,
        issued_at: DateTime<Utc>,
        fresh_until: DateTime<Utc>,
    ) -> Result<Self::Withdrawal, StatusSessionError> {
        let issued_at = u64::try_from(issued_at.timestamp())
            .map_err(|_| StatusSessionError::ContractUnavailable)?;
        let fresh_until = u64::try_from(fresh_until.timestamp())
            .map_err(|_| StatusSessionError::ContractUnavailable)?;
        ClientBindingStatusInputV1::withdrawn(
            domain,
            author,
            connection_revision,
            issued_at,
            fresh_until,
        )
        .map_err(|_| StatusSessionError::ContractUnavailable)?
        .sign_with_relay_keys(&self.relay_keys)
        .map_err(|_| StatusSessionError::ContractUnavailable)
    }
}

impl fmt::Debug for DurableStatusContract {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("DurableStatusContract([REDACTED])")
    }
}

/// Activation or recovery failure that never exposes status material.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum DurableStatusError {
    /// The connection does not own a complete Enforce-mode status scope.
    #[error("status connection is not authorized")]
    Unauthorized,
    /// The bootstrap could not be physically acknowledged.
    #[error("status bootstrap delivery failed")]
    BootstrapDelivery,
    /// PostgreSQL delivery state was unavailable or inconsistent.
    #[error("status journal unavailable")]
    Journal,
}

/// Exact-connection crash-durable sink used by [`super::status::ConnectionStatusSession`].
pub struct DurableStatusSink<E>
where
    E: CurrentStatusEvidenceSource + ?Sized,
{
    db: Db,
    evidence: Arc<E>,
    writer: StatusWriter,
    cancel: tokio_util::sync::CancellationToken,
    community_id: CommunityId,
    author: PublicKey,
    relay_signer: PublicKey,
    connection_fingerprint: [u8; 32],
    owner: tokio::sync::Mutex<()>,
    in_flight: tokio::sync::Mutex<Option<StatusWriteReceiver>>,
}

impl<E> DurableStatusSink<E>
where
    E: CurrentStatusEvidenceSource + ?Sized + 'static,
{
    /// Validate exact connection ownership and physically flush bootstrap.
    ///
    /// The caller must pass the finalized direct binding-status lease that it
    /// will convert into `CurrentStatusAuthorization`; legacy AUTH state alone
    /// is intentionally insufficient.
    pub async fn activate(
        mode: NipFiMode,
        db: Db,
        evidence: Arc<E>,
        connection: Arc<ConnectionState>,
        lease: &BoundedAuthorizationLease,
        relay_keys: &Keys,
        authoritative_now: DateTime<Utc>,
    ) -> Result<(Self, UnchangedBootstrapDelivery), DurableStatusError> {
        if mode != NipFiMode::Enforce
            || lease.capability() != RouteCapability::BindingStatus
            || lease.owner_pubkey().is_some()
            || lease.transport() != ProofTransport::Nip42
            || lease.authorization_domain() != connection.tenant.community()
            || !lease.is_valid_at(authoritative_now)
        {
            return Err(DurableStatusError::Unauthorized);
        }
        let scope = connection
            .status_scope
            .read()
            .await
            .clone()
            .ok_or(DurableStatusError::Unauthorized)?;
        if scope.relay_signer() != relay_keys.public_key() {
            return Err(DurableStatusError::Unauthorized);
        }
        let author = lease.actor_pubkey();
        let directly_authenticated = match &*connection.auth_state.read().await {
            AuthState::Authenticated(context) => {
                context.pubkey == author && context.agent_owner_pubkey.is_none()
            }
            AuthState::Pending { .. } | AuthState::Failed => false,
        };
        if !directly_authenticated || connection.cancel.is_cancelled() {
            return Err(DurableStatusError::Unauthorized);
        }
        let connection_fingerprint = status_fingerprint(
            b"buzz:client-status-connection:v1",
            &[
                connection.tenant.community().as_uuid().as_bytes(),
                connection.conn_id.as_bytes(),
                author.as_bytes(),
                relay_keys.public_key().as_bytes(),
                scope.connection_epoch().as_str().as_bytes(),
            ],
        );
        let (_, authorized_target, _) = lease.request_binding();
        if authorized_target != &connection_fingerprint {
            return Err(DurableStatusError::Unauthorized);
        }
        db.install_status_delivery_capacity(connection.tenant.community())
            .await
            .map_err(|_| DurableStatusError::Journal)?;
        db.reconcile_status_deliveries(connection.tenant.community(), 1024)
            .await
            .map_err(|_| DurableStatusError::Journal)?;
        db.reap_status_deliveries(connection.tenant.community(), 1024)
            .await
            .map_err(|_| DurableStatusError::Journal)?;

        let issued_at = u64::try_from(authoritative_now.timestamp())
            .map_err(|_| DurableStatusError::Unauthorized)?;
        let bootstrap = ClientBindingBootstrapInputV1::new(
            connection.tenant.community(),
            author,
            scope.connection_epoch().clone(),
            issued_at,
        )
        .map_err(|_| DurableStatusError::Unauthorized)?
        .sign_with_relay_keys(relay_keys)
        .map_err(|_| DurableStatusError::Unauthorized)?;
        let deadline = tokio::time::Instant::now() + BOOTSTRAP_WRITE_LIMIT;
        let acknowledgement = connection
            .status_writer
            .write(
                RelayMessage::event(CLIENT_BINDING_BOOTSTRAP_SUB_ID, &bootstrap),
                None,
                true,
                deadline,
            )
            .await
            .map_err(|_| DurableStatusError::BootstrapDelivery)?;
        if acknowledgement.identity.is_some() {
            return Err(DurableStatusError::BootstrapDelivery);
        }

        Ok((
            Self {
                db,
                evidence,
                writer: connection.status_writer.clone(),
                cancel: connection.cancel.clone(),
                community_id: connection.tenant.community(),
                author,
                relay_signer: relay_keys.public_key(),
                connection_fingerprint,
                owner: tokio::sync::Mutex::new(()),
                in_flight: tokio::sync::Mutex::new(None),
            },
            UnchangedBootstrapDelivery::delivered(),
        ))
    }

    /// Recover at most one claimed job for this exact still-live connection.
    pub async fn recover_one(&self) -> Result<bool, DurableStatusError> {
        let _owner = self.owner.lock().await;
        self.wait_in_flight()
            .await
            .map_err(|_| DurableStatusError::Journal)?;
        let Some(claimed) = self
            .db
            .claim_status_delivery(self.community_id, self.connection_fingerprint, CLAIM_LEASE)
            .await
            .map_err(|_| DurableStatusError::Journal)?
        else {
            return Ok(false);
        };
        self.deliver_claim(&claimed)
            .await
            .map_err(|_| DurableStatusError::Journal)?;
        Ok(true)
    }

    /// Stop this exact owner after activation and terminalize its pending work.
    pub async fn shutdown(&self) {
        self.close_connection().await;
    }

    async fn enqueue_and_deliver(
        &self,
        event: &Event,
        evidence: Option<&CanonicalCurrentBindingEvidence>,
        kind: StatusDeliveryKind,
    ) -> Result<(), StatusSessionError> {
        let _owner = self.owner.lock().await;
        self.wait_in_flight().await?;
        self.db
            .reconcile_status_deliveries(self.community_id, 1024)
            .await
            .map_err(|_| StatusSessionError::DeliveryFailed)?;
        self.db
            .reap_status_deliveries(self.community_id, 1024)
            .await
            .map_err(|_| StatusSessionError::DeliveryFailed)?;
        let now = self
            .db
            .status_delivery_authoritative_now()
            .await
            .map_err(|_| StatusSessionError::DeliveryFailed)?;
        let validated = validate_client_binding_status_event(
            event,
            &self.relay_signer,
            self.community_id,
            &self.author,
            unix_seconds(now)?,
        )
        .map_err(|_| StatusSessionError::ContractUnavailable)?;
        if !kind_matches(kind, validated.disposition()) {
            return Err(StatusSessionError::ContractUnavailable);
        }
        let payload =
            serde_json::to_vec(event).map_err(|_| StatusSessionError::ContractUnavailable)?;
        let fresh_until = DateTime::<Utc>::from_timestamp(
            i64::try_from(validated.fresh_until())
                .map_err(|_| StatusSessionError::ContractUnavailable)?,
            0,
        )
        .ok_or(StatusSessionError::ContractUnavailable)?;
        let event_id = event.id.as_bytes();
        let transition_id = stable_uuid(b"transition", event_id, &self.connection_fingerprint);
        let operation_id = stable_uuid(b"operation", event_id, &self.connection_fingerprint);
        let delivery_id = stable_uuid(b"delivery", event_id, &self.connection_fingerprint);
        let request_fingerprint = status_fingerprint(
            b"buzz:client-status-request:v1",
            &[event_id, self.connection_fingerprint.as_slice()],
        );
        let delivery = NewStatusDelivery {
            community_id: self.community_id,
            delivery_id,
            transition_id,
            operation_id,
            request_fingerprint,
            kind,
            subject_fingerprint: status_fingerprint(
                b"buzz:client-status-subject:v1",
                &[
                    self.community_id.as_uuid().as_bytes(),
                    self.author.as_bytes(),
                ],
            ),
            signer_fingerprint: status_fingerprint(
                b"buzz:client-status-signer:v1",
                &[
                    self.community_id.as_uuid().as_bytes(),
                    self.relay_signer.as_bytes(),
                ],
            ),
            connection_fingerprint: self.connection_fingerprint,
            status_revision: validated.status_revision(),
            supersedes_revision: (kind == StatusDeliveryKind::Withdrawal)
                .then(|| validated.status_revision().saturating_sub(1)),
            fresh_until,
            signed_payload: &payload,
            current_evidence: evidence,
        };
        self.db
            .enqueue_status_delivery(&delivery)
            .await
            .map_err(|_| StatusSessionError::DeliveryFailed)?;
        let claimed = self
            .db
            .claim_status_delivery(self.community_id, self.connection_fingerprint, CLAIM_LEASE)
            .await
            .map_err(|_| StatusSessionError::DeliveryFailed)?
            .ok_or(StatusSessionError::DeliveryFailed)?;
        if claimed.delivery_id() != delivery_id {
            return Err(StatusSessionError::DeliveryFailed);
        }
        self.deliver_claim(&claimed).await
    }

    async fn deliver_claim(
        &self,
        claimed: &ClaimedStatusDelivery,
    ) -> Result<(), StatusSessionError> {
        if self.cancel.is_cancelled()
            || claimed.community_id() != self.community_id
            || claimed.connection_fingerprint() != self.connection_fingerprint
        {
            self.fail_terminal(claimed, StatusDeliveryFailure::ConnectionGone)
                .await;
            return Err(StatusSessionError::DeliveryFailed);
        }
        let event: Event = match serde_json::from_slice(claimed.signed_payload()) {
            Ok(event) => event,
            Err(_) => {
                self.fail_terminal(claimed, StatusDeliveryFailure::InvalidPayload)
                    .await;
                return Err(StatusSessionError::ContractUnavailable);
            }
        };
        let mut authoritative_now = self
            .db
            .status_delivery_authoritative_now()
            .await
            .map_err(|_| StatusSessionError::DeliveryFailed)?;
        if let Some(original) = claimed.current_evidence() {
            let (rechecked, recheck_now) = self.evidence.recheck(original).await?;
            if !original.accepts_exact_recheck(&rechecked, recheck_now) {
                self.fail_terminal(claimed, StatusDeliveryFailure::StaleFence)
                    .await;
                return Err(StatusSessionError::EvidenceChanged);
            }
            authoritative_now = recheck_now;
        } else if claimed.kind() != StatusDeliveryKind::Withdrawal {
            self.fail_terminal(claimed, StatusDeliveryFailure::InvalidPayload)
                .await;
            return Err(StatusSessionError::ContractUnavailable);
        }
        let validated = match validate_client_binding_status_event(
            &event,
            &self.relay_signer,
            self.community_id,
            &self.author,
            unix_seconds(authoritative_now)?,
        ) {
            Ok(validated) => validated,
            Err(_) => {
                self.fail_terminal(claimed, StatusDeliveryFailure::InvalidPayload)
                    .await;
                return Err(StatusSessionError::ContractUnavailable);
            }
        };
        if validated.status_revision() != claimed.status_revision()
            || !kind_matches(claimed.kind(), validated.disposition())
            || (claimed.kind() == StatusDeliveryKind::Current
                && claimed.current_evidence().is_none_or(|evidence| {
                    validated.binding_version() != Some(evidence.binding_version())
                        || validated
                            .policy_version()
                            .and_then(|revision| revision.parse::<u64>().ok())
                            != Some(evidence.policy_revision())
                }))
        {
            self.fail_terminal(claimed, StatusDeliveryFailure::InvalidPayload)
                .await;
            return Err(StatusSessionError::ContractUnavailable);
        }
        let mut authorization = self
            .db
            .authorize_status_delivery(claimed)
            .await
            .map_err(|_| StatusSessionError::DeliveryFailed)?
            .ok_or(StatusSessionError::EvidenceChanged)?;
        if self.cancel.is_cancelled() {
            self.db
                .abort_status_delivery_authorization(authorization)
                .await
                .map_err(|_| StatusSessionError::DeliveryFailed)?;
            self.fail_terminal(claimed, StatusDeliveryFailure::ConnectionGone)
                .await;
            return Err(StatusSessionError::DeliveryFailed);
        }
        let deadline = tokio::time::Instant::now()
            .checked_add(authorization.write_budget())
            .ok_or(StatusSessionError::Expired)?;
        let canonical_payload =
            serde_json::to_vec(&event).map_err(|_| StatusSessionError::ContractUnavailable)?;
        let canonical_digest: [u8; 32] = Sha256::digest(&canonical_payload).into();
        if canonical_payload != claimed.signed_payload()
            || canonical_digest != claimed.payload_digest()
            || canonical_digest != authorization.payload_digest()
        {
            self.db
                .abort_status_delivery_authorization(authorization)
                .await
                .map_err(|_| StatusSessionError::DeliveryFailed)?;
            self.fail_terminal(claimed, StatusDeliveryFailure::InvalidPayload)
                .await;
            return Err(StatusSessionError::ContractUnavailable);
        }
        let wire = RelayMessage::event(CLIENT_BINDING_STATUS_SUB_ID, &event);
        let identity = StatusWriteIdentity {
            delivery_id: authorization.delivery_id(),
            claim_id: authorization.claim_id(),
            payload_digest: authorization.payload_digest(),
            wire_digest: Sha256::digest(wire.as_bytes()).into(),
        };
        let db = self.db.clone();
        let writer = self.writer.clone();
        let (result_tx, result_rx) = watch::channel(None);
        *self.in_flight.lock().await = Some(result_rx.clone());
        // The task owns both the PostgreSQL fence and the queued writer
        // command. Dropping this caller future detaches the task but cannot
        // release the fence while the writer is still capable of I/O.
        tokio::spawn(async move {
            let operation = async {
                let acknowledgement =
                    match writer.write(wire, Some(identity), false, deadline).await {
                        Ok(acknowledgement) => acknowledgement,
                        Err(_) => {
                            db.abort_status_delivery_authorization(authorization)
                                .await
                                .map_err(|_| StatusSessionError::DeliveryFailed)?;
                            return Err(StatusSessionError::DeliveryFailed);
                        }
                    };
                if acknowledgement.identity != Some(identity) {
                    db.abort_status_delivery_authorization(authorization)
                        .await
                        .map_err(|_| StatusSessionError::DeliveryFailed)?;
                    return Err(StatusSessionError::DeliveryFailed);
                }
                let completion = db
                    .complete_status_delivery(&mut authorization)
                    .await
                    .map_err(|_| StatusSessionError::DeliveryFailed)?;
                if matches!(
                    completion,
                    CompleteStatusDeliveryOutcome::Delivered
                        | CompleteStatusDeliveryOutcome::ExactReplay
                ) {
                    Ok(())
                } else {
                    Err(StatusSessionError::DeliveryFailed)
                }
            };
            let result = bounded_status_operation(
                Duration::from_secs(MAX_STATUS_OPERATION_SECONDS),
                operation,
            )
            .await;
            let _ = result_tx.send(Some(result));
        });
        let result = Self::await_in_flight(result_rx).await;
        self.in_flight.lock().await.take();
        result
    }

    async fn wait_in_flight(&self) -> Result<(), StatusSessionError> {
        let receiver = self.in_flight.lock().await.as_ref().cloned();
        let Some(receiver) = receiver else {
            return Ok(());
        };
        let result = Self::await_in_flight(receiver).await;
        self.in_flight.lock().await.take();
        result
    }

    async fn await_in_flight(mut receiver: StatusWriteReceiver) -> Result<(), StatusSessionError> {
        loop {
            if let Some(result) = *receiver.borrow() {
                return result;
            }
            receiver
                .changed()
                .await
                .map_err(|_| StatusSessionError::DeliveryFailed)?;
        }
    }

    async fn fail_terminal(&self, claimed: &ClaimedStatusDelivery, failure: StatusDeliveryFailure) {
        let _ = self
            .db
            .fail_status_delivery(
                claimed.community_id(),
                claimed.delivery_id(),
                claimed.claim_id(),
                failure,
                Duration::from_secs(1),
            )
            .await;
    }

    async fn close_connection(&self) {
        self.cancel.cancel();
        // Serialize teardown with producer and recovery paths. Once the owner
        // is acquired, any detached write has either published its result or
        // remains represented by `in_flight`, so terminalization cannot race
        // a writer that is still capable of I/O.
        let _owner = self.owner.lock().await;
        if self.wait_in_flight().await.is_err() {
            // A failed physical write or completion still settles ownership.
            // Continue below so the dead exact target cannot retain capacity
            // until a future activation happens to reconcile it.
            warn!("status writer settled before connection cleanup");
        }
        for attempt in 0..3 {
            if self
                .db
                .terminalize_status_connection(self.community_id, self.connection_fingerprint)
                .await
                .is_ok()
            {
                return;
            }
            if attempt < 2 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
        warn!("status connection cleanup deferred");
    }
}

#[async_trait]
impl<E> CurrentStatusSink<DurableStatusContract> for DurableStatusSink<E>
where
    E: CurrentStatusEvidenceSource + ?Sized + 'static,
{
    async fn send_current(&self, current: &DurableCurrentStatus) -> Result<(), StatusSessionError> {
        self.enqueue_and_deliver(
            &current.event,
            Some(&current.evidence),
            StatusDeliveryKind::Current,
        )
        .await
    }

    async fn send_withdrawal(&self, withdrawal: &Event) -> Result<(), StatusSessionError> {
        self.enqueue_and_deliver(withdrawal, None, StatusDeliveryKind::Withdrawal)
            .await
    }

    async fn close(&self) {
        self.close_connection().await;
    }
}

impl<E> fmt::Debug for DurableStatusSink<E>
where
    E: CurrentStatusEvidenceSource + ?Sized,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("DurableStatusSink([REDACTED])")
    }
}

fn unix_seconds(value: DateTime<Utc>) -> Result<u64, StatusSessionError> {
    u64::try_from(value.timestamp()).map_err(|_| StatusSessionError::Expired)
}

fn kind_matches(kind: StatusDeliveryKind, disposition: ClientBindingStatusDisposition) -> bool {
    matches!(
        (kind, disposition),
        (
            StatusDeliveryKind::Current,
            ClientBindingStatusDisposition::DisplayCurrent
        ) | (
            StatusDeliveryKind::Withdrawal,
            ClientBindingStatusDisposition::Withdrawn
        )
    )
}

fn status_fingerprint(label: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update((label.len() as u64).to_be_bytes());
    digest.update(label);
    for part in parts {
        digest.update((part.len() as u64).to_be_bytes());
        digest.update(part);
    }
    digest.finalize().into()
}

fn stable_uuid(label: &[u8], event_id: &[u8], connection: &[u8; 32]) -> Uuid {
    let mut bytes = status_fingerprint(label, &[event_id, connection]);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let mut uuid_bytes = [0; 16];
    uuid_bytes.copy_from_slice(&bytes[..16]);
    Uuid::from_bytes(uuid_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_core::AuthorizationLeaseFence;

    #[tokio::test]
    async fn bounded_status_operation_terminates_fence_owner() {
        let result = tokio::time::timeout(
            Duration::from_millis(100),
            bounded_status_operation(
                Duration::from_millis(10),
                std::future::pending::<StatusWriteResult>(),
            ),
        )
        .await
        .expect("operation hard timeout");
        assert!(matches!(result, Err(StatusSessionError::DeliveryFailed)));
    }

    #[test]
    fn exact_connection_fingerprint_changes_with_server_generation() {
        let community = Uuid::new_v4();
        let author = [2; 32];
        let signer = [3; 32];
        let epoch = Uuid::new_v4();
        let first = status_fingerprint(
            b"buzz:client-status-connection:v1",
            &[
                community.as_bytes(),
                Uuid::new_v4().as_bytes(),
                &author,
                &signer,
                epoch.as_bytes(),
            ],
        );
        let second = status_fingerprint(
            b"buzz:client-status-connection:v1",
            &[
                community.as_bytes(),
                Uuid::new_v4().as_bytes(),
                &author,
                &signer,
                epoch.as_bytes(),
            ],
        );
        assert_ne!(first, second);
    }

    #[test]
    fn stable_ids_are_domain_separated_and_replay_stable() {
        let event = [4; 32];
        let connection = [5; 32];
        assert_eq!(
            stable_uuid(b"delivery", &event, &connection),
            stable_uuid(b"delivery", &event, &connection)
        );
        assert_ne!(
            stable_uuid(b"delivery", &event, &connection),
            stable_uuid(b"transition", &event, &connection)
        );
    }

    #[test]
    fn durable_contract_preserves_typed_privacy_boundary() {
        let relay = Keys::generate();
        let author = Keys::generate().public_key();
        let domain = CommunityId::from_uuid(Uuid::new_v4());
        let observed_at = DateTime::from_timestamp(1_800_000_000, 123_456_000).expect("timestamp");
        let fresh_until = observed_at + chrono::Duration::minutes(4);
        let evidence = CanonicalCurrentBindingEvidence::new(
            domain,
            author,
            Uuid::new_v4(),
            7,
            11,
            13,
            17,
            AuthorizationLeaseFence::from_bytes([19; 32]).expect("fence"),
            observed_at,
            fresh_until,
        )
        .expect("evidence");
        let contract = DurableStatusContract::new(relay.clone(), relay.public_key())
            .expect("durable contract");
        let current = contract.current(&evidence, 1).expect("current status");
        assert_eq!(current.evidence.observed_at().timestamp_subsec_nanos(), 0);
        assert_eq!(current.evidence.fresh_until().timestamp_subsec_nanos(), 0);
        let validated = validate_client_binding_status_event(
            &current.event,
            &relay.public_key(),
            domain,
            &author,
            u64::try_from(observed_at.timestamp()).expect("positive time"),
        )
        .expect("typed current validation");
        assert_eq!(validated.binding_version(), Some(7));
        assert_eq!(validated.policy_version(), Some("11"));
        assert!(current.event.tags.is_empty());
        assert!(validate_client_binding_status_event(
            &current.event,
            &Keys::generate().public_key(),
            domain,
            &author,
            u64::try_from(observed_at.timestamp()).expect("positive time"),
        )
        .is_err());

        let withdrawal = contract
            .withdrawal(domain, author, 2, observed_at, fresh_until)
            .expect("withdrawal");
        let validated_withdrawal = validate_client_binding_status_event(
            &withdrawal,
            &relay.public_key(),
            domain,
            &author,
            u64::try_from(observed_at.timestamp()).expect("positive time"),
        )
        .expect("typed withdrawal validation");
        assert_eq!(
            validated_withdrawal.disposition(),
            ClientBindingStatusDisposition::Withdrawn
        );
        assert_eq!(validated_withdrawal.binding_version(), None);
        assert_eq!(validated_withdrawal.policy_version(), None);
    }
}
