use super::{claim_owner, ClaimLog, InviteClaimResult, MembershipClaimResult};
use crate::relay_members::MembershipWrite;
use crate::{AdmittedTx, DbError, Result};

/// Maximum number of claims sharing one atomic roster update.
pub const MAX_CLAIM_BATCH_SIZE: usize = 128;

/// Caller-verified credential for an invitation claim.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub enum ClaimCredential {
    /// Hash of a durable v2 invitation code.
    V2 {
        /// SHA-256 of the presented code.
        token_hash: [u8; 32],
    },
    /// Role from a verified legacy invitation.
    Legacy {
        /// Membership role authenticated by the legacy invitation.
        role: String,
    },
}

/// One verified claim; all requests in a batch belong to its admitted community.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct ClaimRequest {
    /// Claiming public key in canonical hexadecimal form.
    pub pubkey: String,
    /// Accepted policy version, if the community requires one.
    pub policy_version: Option<String>,
    /// Owner whose attestation the caller has verified.
    pub verified_owner: Option<String>,
    /// Verified legacy role or durable invitation hash.
    pub credential: ClaimCredential,
}

/// A committed claim's outcome, preserving each protocol's existing result.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub enum ClaimResult {
    /// Durable invitation outcome and committed owner changes.
    V2(InviteClaimResult),
    /// Legacy invitation outcome and committed owner changes.
    Legacy(MembershipClaimResult),
}

/// Claims applied to the caller's transaction, with logs deferred until commit.
pub(super) struct AppliedClaims {
    pub(super) write: MembershipWrite<Vec<Result<ClaimResult>>>,
    logs: Vec<ClaimLog>,
}

impl AppliedClaims {
    /// Call only after the enclosing transaction committed successfully.
    pub(super) fn into_committed(self) -> MembershipWrite<Vec<Result<ClaimResult>>> {
        for log in self.logs {
            log.emit();
        }
        self.write
    }
}

/// Apply a bounded batch and one full roster snapshot inside an admitted transaction.
///
/// The caller holds the community's invitation admission lock and commits the
/// returned outcomes with its durable work records before calling `into_committed`.
/// Expected owner/restriction denials write nothing for that request. Any other
/// error requires rolling back the entire transaction; no claim has succeeded yet.
pub(super) async fn apply_claim_batch(
    tx: &mut AdmittedTx,
    requests: &[ClaimRequest],
    relay_keypair: &nostr::Keys,
) -> Result<AppliedClaims> {
    if requests.is_empty() || requests.len() > MAX_CLAIM_BATCH_SIZE {
        return Err(DbError::InvalidData(format!(
            "claim batch must contain 1 to {MAX_CLAIM_BATCH_SIZE} requests"
        )));
    }
    let community = tx.community();
    let mut outcomes = Vec::with_capacity(requests.len());
    let mut logs: Vec<ClaimLog> = Vec::new();
    let mut changed = false;
    for request in requests {
        let result = match &request.credential {
            ClaimCredential::V2 { token_hash } => {
                let prepared = if super::invite_known(tx, community, token_hash).await? {
                    super::claim_relay_invite_in_tx(
                        tx,
                        community,
                        token_hash,
                        &request.pubkey,
                        request.policy_version.as_deref(),
                        request.verified_owner.as_deref(),
                    )
                    .await
                } else {
                    Ok(super::PreparedInviteClaim::new(
                        super::ClaimOutcome::Invalid.into(),
                        ClaimLog::new(community, None, "invalid", None, None),
                    ))
                };
                prepared.map(|prepared| {
                    changed |=
                        matches!(prepared.result.outcome, super::ClaimOutcome::Joined { .. });
                    logs.push(prepared.log);
                    ClaimResult::V2(prepared.result)
                })
            }
            ClaimCredential::Legacy { role } => claim_owner::claim_relay_membership_in_tx(
                tx,
                community,
                &request.pubkey,
                role,
                request.policy_version.as_deref(),
                request.verified_owner.as_deref(),
            )
            .await
            .map(|result| {
                changed |= result.inserted;
                ClaimResult::Legacy(result)
            }),
        };
        match result {
            Ok(outcome) => outcomes.push(Ok(outcome)),
            Err(error @ (DbError::InviteOwnerMismatch | DbError::InviteRestricted)) => {
                outcomes.push(Err(error));
            }
            Err(error) => return Err(error),
        }
    }
    let snapshot = if changed {
        Some(super::write_nip43_membership_snapshot(tx, relay_keypair).await?)
    } else {
        None
    };
    Ok(AppliedClaims {
        write: MembershipWrite {
            outcome: outcomes,
            snapshot,
        },
        logs,
    })
}
