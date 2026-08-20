//! Durable owner-identity capabilities (C2 / P30-C1).
//!
//! Split from the parent module (which owns the bounded per-send lease, the
//! registry, and the coordinator drain/latch) so each file stays within the
//! desktop file-size discipline. This half is the DURABLE capability substrate:
//! authority that outlives the bounded lease that derived it (long-lived
//! sessions, pre-minted bearers). It reads the parent's registry generation
//! and admission state through `super::` — the two halves share ONE registry.

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::{LazyLock, Mutex};

use serde::Serialize;
use tokio_util::sync::CancellationToken;

use super::{lock_inner, IdentityPersistenceState, OwnerIdentityEgressLease, REGISTRY};

/// A generation-stamped, registry-tracked owner-identity capability whose
/// authority OUTLIVES the bounded lease that derived it.
///
/// The bounded [`OwnerIdentityEgressLease`] above covers authority derived and
/// consumed inside one sign → auth → transmit window. But two owner-key
/// operations mint authority that a LATER, separate operation exercises:
///
/// - **Sessions** ([`SessionPolicy`]) — the huddle audio socket authenticates
///   ONCE with a NIP-42 event, then a long-lived task emits unsigned frames
///   over the established peer indefinitely; the frontend relay WS is the same
///   shape (`create_auth_event` signs the handshake, later frames ride the
///   connection). Cloning the owner keys before the transition cannot express
///   that a later unsigned frame inherits pre-transition authority.
/// - **Bearers** ([`BearerPolicy`]) — `mint_media_get_auth` /
///   `sign_blossom_upload_auth` sign a server-scoped Blossom header that
///   callers attach at LATER HTTP transmissions, up to ten minutes after
///   issuance. The bearer TTL is wider than a transition, so TTL is not a
///   substitute for invalidation.
///
/// Every durable capability is REGISTERED in this same egress registry and
/// STAMPED with the identity-persistence generation current at issuance
/// (issuance itself runs under a bounded lease — the signing that derives the
/// capability is an ordinary leased operation). Every transmission over it
/// validates, immediately before the irreversible boundary (frame send /
/// header attach), BOTH current egress admission AND
/// `capability_generation == current identity-persistence generation`
/// ([`OwnerIdentityCapability::admit_exercise`]) — a stale capability is
/// refused with zero bytes sent. The type is the only carrier of that stamped
/// generation, so no exercise site can skip the check (there is no raw handle
/// to exercise).
///
/// The registry additionally holds each capability's REVOCATION HANDLE (the
/// session cancellation token; the bearer's registry id for invalidation), so
/// the C5 coordinator barrier only *invokes* what C2 already registered — it
/// never retrofits the registry schema. Registering the teardown authority is
/// substrate (C2); invoking it at the transition barrier is C5.
#[derive(Debug)]
#[must_use = "a durable owner-identity capability must be validated \
              (admit_exercise) immediately before every transmission over it"]
pub struct OwnerIdentityCapability<P: CapabilityPolicy> {
    /// Registry id — the key under which this capability's revocation handle
    /// lives, so the C5 barrier can invalidate it by generation.
    id: u64,
    /// The identity-persistence generation current when this capability was
    /// issued. Exercise is refused once the registry generation advances past
    /// it.
    generation: u64,
    _policy: std::marker::PhantomData<P>,
}

/// A policy distinguishing the durable capability kinds. Zero-sized markers —
/// the shared behavior (registration, generation-stamp, exercise validation,
/// revocation-handle storage) lives on [`OwnerIdentityCapability`]; the policy
/// only names the kind so the constructor inventory and the registry's
/// per-kind revocation are type-directed.
pub trait CapabilityPolicy: std::fmt::Debug + private::Sealed {
    /// A human label for diagnostics and the closed-world inventory.
    const KIND: &'static str;
}

/// Authenticated-connection authority (huddle audio socket, frontend relay
/// WS). The registered revocation handle is a [`CancellationToken`] whose
/// cancellation tears the connection down.
#[derive(Debug)]
pub struct SessionPolicy;
/// Pre-minted bearer authority (Blossom `t=get`/`t=upload` headers). The
/// registered revocation handle is the registry id; invalidation removes the
/// entry so a later attach fails admission.
#[derive(Debug)]
pub struct BearerPolicy;

/// A generation-stamped owner-key-derived VALUE (owner-signed relay event JSON,
/// identity-binding response, encrypt-to-self ciphertext, or the plaintext of
/// an owner-key decryption) that leaves its producing lease and is exercised —
/// exported, published, signed around, or applied to identity-indexed state —
/// LATER. Unlike a session or bearer, an artifact owns NO side-effecting
/// teardown: a stamped `String`/JSON value has nothing to cancel or remove. Its
/// only revocation mechanism is the exercise-time generation compare, which the
/// generation bump at [`begin_egress_drain`] triggers by construction — so the
/// registry keeps NO artifact entry for the barrier to invalidate (shape (ii),
/// acked by Paul 2026-08-18). The registered revocation handle is therefore
/// [`RevocationHandle::Artifact`], a marker carrying no teardown authority; the
/// capability's `id` is correlation/diagnostic only, never a revocation target.
///
/// Rust-consumed / collapsed-transaction artifacts (project git-workflow
/// sign+submit, archive-ingest decrypts, snapshot-import, engram-listing
/// decrypt) exercise synchronously inside their producing lease and validate
/// via [`OwnerIdentityCapability::admit_exercise`] before their local
/// application — identical to a bounded lease. Boundary-crossing artifacts (the
/// frontend-consumed producers) serialize their stamp across the Tauri boundary
/// as [`ArtifactStamp`]; the frontend threads it opaque until C6/C7 adds the
/// generation-compare at each application site.
#[derive(Debug)]
pub struct ArtifactPolicy;

impl CapabilityPolicy for SessionPolicy {
    const KIND: &'static str = "session";
}
impl CapabilityPolicy for BearerPolicy {
    const KIND: &'static str = "bearer";
}
impl CapabilityPolicy for ArtifactPolicy {
    const KIND: &'static str = "artifact";
}

mod private {
    pub trait Sealed {}
    impl Sealed for super::SessionPolicy {}
    impl Sealed for super::BearerPolicy {}
    impl Sealed for super::ArtifactPolicy {}
}

/// The revocation authority for one registered durable capability, invoked by
/// the C5 coordinator barrier to tear down old-generation authority before the
/// journal write + durable dispatch.
enum RevocationHandle {
    /// Cancel the connection (huddle socket / frontend WS teardown path).
    Session(CancellationToken),
    /// Bearer invalidation is by-entry: removing the registry entry is the
    /// invalidation, so no side-effecting handle is needed. The variant exists
    /// so the registry records the capability's kind for the per-kind barrier.
    Bearer,
    /// Artifact invalidation is BY GENERATION BUMP, not by entry: a stamped
    /// value owns no side-effecting teardown, and its only revocation is the
    /// exercise-time / application-site generation compare failing once the
    /// bump advances the current generation (shape (ii)). The registry keeps NO
    /// artifact entry for the barrier to invalidate — this marker exists only
    /// so `register_owner_artifact` shares the one registration path; the entry
    /// is removed immediately on the handle's Drop (serialization time for a
    /// boundary-crossing artifact, scope end for a collapsed one), so the id is
    /// correlation/diagnostic only, never a revocation target.
    Artifact,
}

/// Registry of live durable capabilities, keyed by capability id, under its
/// own [`Mutex`] (`DURABLE`) — separate from the bounded-lease state
/// (`REGISTRY.inner`). Registration is therefore NOT lock-linearized against
/// generation bumps: a bump can slip between a lease's admission and its
/// capability's registration. The safety guarantee is not lock ordering but
/// EXERCISE-TIME validation — each capability stamps its issuing lease's
/// generation and [`OwnerIdentityCapability::admit_exercise`] refuses once the
/// current generation advances past it, so a capability registered across a
/// bump fails closed on first use.
#[derive(Default)]
struct DurableRegistry {
    /// Next capability id. Monotonic; ids are never reused.
    next_id: u64,
    /// Live capabilities: id → (issued generation, revocation handle).
    entries: HashMap<u64, (u64, RevocationHandle)>,
}

static DURABLE: LazyLock<Mutex<DurableRegistry>> =
    LazyLock::new(|| Mutex::new(DurableRegistry::default()));

fn lock_durable() -> std::sync::MutexGuard<'static, DurableRegistry> {
    DURABLE.lock().unwrap_or_else(|p| p.into_inner())
}

/// Clear all registered durable capabilities. Called by the parent's
/// `reset_registry_for_test` so a test starts from an empty registry. `next_id`
/// is monotonic and never resets, mirroring the generation.
#[cfg(test)]
pub(super) fn reset_for_test() {
    lock_durable().entries.clear();
}

impl<P: CapabilityPolicy> OwnerIdentityCapability<P> {
    /// The identity-persistence generation this capability was issued under.
    #[cfg(test)]
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Validate this capability immediately before an irreversible
    /// transmission over it (frame send / header attach). Succeeds only when
    /// egress admission is `Live` AND the stamped generation still equals the
    /// current identity-persistence generation. A stale or drained capability
    /// is refused so the caller sends zero bytes.
    ///
    /// This is a cheap compare, not a re-authentication: the durable
    /// capability already proved identity at issuance; exercise only confirms
    /// that proof has not been superseded by a transition.
    pub fn admit_exercise(&self) -> Result<(), String> {
        let inner = lock_inner();
        if inner.state != IdentityPersistenceState::Live {
            return Err(format!(
                "owner-identity {} capability cannot transmit: egress is {:?}",
                P::KIND,
                inner.state
            ));
        }
        let current = REGISTRY.generation.load(Ordering::Acquire);
        if self.generation != current {
            return Err(format!(
                "owner-identity {} capability is stale (issued under generation \
                 {}, current {}); the identity transitioned and this authority \
                 was revoked",
                P::KIND,
                self.generation,
                current
            ));
        }
        Ok(())
    }
}

impl<P: CapabilityPolicy> Drop for OwnerIdentityCapability<P> {
    fn drop(&mut self) {
        // Dropping the capability handle deregisters it — a session whose task
        // has ended or a bearer no longer attachable must not linger as a
        // revocation target. Barrier invalidation (C5) also removes entries;
        // the id is unique and never reused, so a double-remove is a no-op.
        lock_durable().entries.remove(&self.id);
    }
}

/// Issue a durable owner-identity session capability, registering its
/// cancellation token so the C5 barrier can tear the connection down.
///
/// Takes the issuing [`OwnerIdentityEgressLease`] as a witness and stamps the
/// generation THAT LEASE was admitted under — never a freshly re-read one. A
/// bump can slip between admission and registration (`begin_egress_drain`
/// advances the generation immediately, before in-flight leases drain), so
/// re-reading here would stamp the winning generation onto authority derived
/// from the losing identity — a barrier bypass. Stamping the lease generation
/// fails closed instead: if a bump slipped in, the stamp is already stale and
/// the first `admit_exercise` refuses.
///
/// Call this AFTER the authenticating sign/auth has completed under the lease
/// (issuance is an ordinary leased operation, spec L4569–4570) and the
/// connection is established. The lease may already be dropped — its generation
/// is copied here — but taking it by reference makes leased issuance
/// compile-enforced, not conventional.
pub fn register_owner_session(
    lease: &OwnerIdentityEgressLease,
    cancel: CancellationToken,
) -> OwnerIdentityCapability<SessionPolicy> {
    register_durable(lease.generation(), RevocationHandle::Session(cancel))
}

/// Issue a durable owner-identity bearer capability, registering it so the C5
/// barrier can invalidate it by generation.
///
/// Stamps the issuing [`OwnerIdentityEgressLease`]'s generation, not a re-read
/// one — see [`register_owner_session`] for why re-reading is a barrier bypass.
///
/// Call this immediately after minting the bearer header under the lease. Every
/// attach site validates the returned capability via
/// [`OwnerIdentityCapability::admit_exercise`] before the HTTP dispatch.
pub fn register_owner_bearer(
    lease: &OwnerIdentityEgressLease,
) -> OwnerIdentityCapability<BearerPolicy> {
    register_durable(lease.generation(), RevocationHandle::Bearer)
}

/// The wire form of an artifact's generation stamp, serialized across the Tauri
/// boundary alongside the owner-key-derived value as `{ value, artifact }`.
///
/// The frontend threads this pair OPAQUE (never unwrapping and discarding it at
/// the adapter — the witness must survive to the application site); C6/C7 adds
/// the generation-compare at each application boundary using `generation`. The
/// `id` is correlation/diagnostic only (log correlation, completeness count),
/// never a revocation target — see [`ArtifactPolicy`] for the shape (ii)
/// rationale.
#[derive(Debug, Clone, Serialize)]
pub struct ArtifactStamp {
    /// Correlation id — monotonic, diagnostic only, never a revocation target.
    pub id: u64,
    /// The identity-persistence generation this artifact was issued under; the
    /// application-site compare (C6/C7) refuses once the current generation
    /// advances past it.
    pub generation: u64,
}

/// The serialized `{ value, artifact }` pair a boundary-crossing producer
/// returns across the Tauri boundary. The frontend threads this OPAQUE (the
/// witness must survive to the application site — never unwrap-and-discard at
/// the adapter); C6/C7 adds the generation-compare against `artifact.generation`
/// immediately before each irreversible application effect.
#[derive(Debug, Serialize)]
pub struct StampedArtifact<T: Serialize> {
    /// The owner-key-derived value (signed event JSON, ciphertext, decrypted
    /// payload). Consumers destructure `.value` at their current use point.
    pub value: T,
    /// The generation stamp carried alongside the value.
    pub artifact: ArtifactStamp,
}

impl OwnerIdentityCapability<ArtifactPolicy> {
    /// The wire stamp for a boundary-crossing artifact, serialized alongside
    /// the value as `{ value, artifact: ArtifactStamp }`. Producing the stamp
    /// does not consume the capability; the caller drops the capability once
    /// the pair is serialized (the entry is correlation-only — see
    /// [`ArtifactPolicy`]).
    pub fn stamp(&self) -> ArtifactStamp {
        ArtifactStamp {
            id: self.id,
            generation: self.generation,
        }
    }

    /// Wrap a boundary-crossing `value` with this capability's stamp, consuming
    /// the capability. The returned [`StampedArtifact`] serializes as
    /// `{ value, artifact }`; the capability's registry entry deregisters as it
    /// drops at the end of this call (the entry is correlation-only under shape
    /// (ii), so early deregistration loses no revocation authority).
    pub fn stamp_value<T: Serialize>(self, value: T) -> StampedArtifact<T> {
        StampedArtifact {
            value,
            artifact: self.stamp(),
        }
    }
}

/// Issue a durable owner-identity artifact capability over an owner-key-derived
/// value, stamped with the issuing lease's generation.
///
/// Stamps the issuing [`OwnerIdentityEgressLease`]'s generation, not a re-read
/// one — see [`register_owner_session`] for why re-reading is a barrier bypass.
/// The F1 lesson applies identically: the signing/decryption that derives the
/// value runs under the lease, and the stamp must be that lease's generation so
/// a value derived across a bump fails closed at its application site.
///
/// Call this AFTER the owner-key operation completes under the lease (admit
/// before sign/decrypt, stamp after). A Rust-consumed / collapsed artifact
/// validates via [`OwnerIdentityCapability::admit_exercise`] before its local
/// application; a boundary-crossing artifact serializes [`ArtifactStamp`] via
/// [`OwnerIdentityCapability::stamp`] and the frontend validates in C6/C7.
pub fn register_owner_artifact(
    lease: &OwnerIdentityEgressLease,
) -> OwnerIdentityCapability<ArtifactPolicy> {
    register_durable(lease.generation(), RevocationHandle::Artifact)
}

/// Register a durable capability stamped with `generation` and carrying
/// `handle` as its revocation authority. Shared by both constructors so the
/// id allocation and registry insert live in one place.
fn register_durable<P: CapabilityPolicy>(
    generation: u64,
    handle: RevocationHandle,
) -> OwnerIdentityCapability<P> {
    let mut durable = lock_durable();
    let id = durable.next_id;
    durable.next_id += 1;
    durable.entries.insert(id, (generation, handle));
    OwnerIdentityCapability {
        id,
        generation,
        _policy: std::marker::PhantomData,
    }
}

/// Revoke every durable capability issued under a generation older than
/// `winning_generation`: cancel old sessions (invoking each registered
/// [`CancellationToken`]) and drop old bearers (removing their entries so a
/// later attach fails [`OwnerIdentityCapability::admit_exercise`]). Returns the
/// number of capabilities revoked.
///
/// ARTIFACTS ARE NOT REVOKED HERE, and this is correct, not a gap (shape (ii),
/// acked by Paul 2026-08-18). The spec text reads "invalidates every
/// old-generation bearer AND artifact"; a reviewer diffing against it will look
/// for artifact handling in this function and find none. An artifact owns no
/// side-effecting teardown — it is a stamped value, not a socket or a bearer
/// entry — so its ONLY revocation mechanism is the exercise-time /
/// application-site generation compare, which the generation bump at
/// [`begin_egress_drain`] triggers by construction. The bump IS the artifact
/// invalidation, and it precedes this call (and the journal write) in the
/// coordinator sequence, so every old-generation artifact is already
/// invalidated before the durable boundary without a per-entry action. Per-entry
/// work here is reserved for authorities with side-effecting teardown (session
/// cancel-tokens, bearer entries). Any artifact entry that happens to be live
/// at barrier time (a producer between lease-drop and value-serialization)
/// self-deregisters on its imminent Drop and is skipped either way.
///
/// The C5 coordinator barrier calls this after
/// [`begin_egress_drain`]/`await_egress_drain` and BEFORE the journal write +
/// durable B dispatch, so no old-generation session or bearer can transmit
/// across the durable boundary. It only *invokes* the handles C2 registered.
pub fn revoke_durable_capabilities_before(winning_generation: u64) -> usize {
    let mut durable = lock_durable();
    let stale: Vec<u64> = durable
        .entries
        .iter()
        .filter(|(_, (gen, handle))| {
            // Artifacts are generation-invalidated (shape (ii)); the barrier
            // acts only on side-effecting authorities.
            *gen < winning_generation && !matches!(handle, RevocationHandle::Artifact)
        })
        .map(|(id, _)| *id)
        .collect();
    for id in &stale {
        if let Some((_, RevocationHandle::Session(cancel))) = durable.entries.remove(id) {
            cancel.cancel();
        }
    }
    stale.len()
}

/// The number of live durable capabilities registered. Used by the
/// registration-completeness assertion (C2) to prove every issued
/// session/bearer is present with a revocation handle the C5 barrier can
/// invoke.
#[cfg(test)]
pub fn live_durable_capability_count() -> usize {
    lock_durable().entries.len()
}

#[cfg(test)]
mod tests {
    use super::super::{
        begin_egress_drain, current_identity_persistence_generation, latch_identity_indeterminate,
        resume_egress_live,
    };
    use super::*;

    fn guard() -> std::sync::MutexGuard<'static, ()> {
        let g = super::super::EGRESS_REGISTRY_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        super::super::reset_registry_for_test();
        g
    }

    /// A bare owner-identity lease admitted under the current generation, for
    /// registering durable capabilities in tests.
    fn lease() -> OwnerIdentityEgressLease {
        super::super::admit_owner_identity_after_wait().expect("test lease admits when live")
    }

    #[test]
    fn durable_capabilities_stamp_the_current_generation() {
        let _g = guard();
        let session = register_owner_session(&lease(), CancellationToken::new());
        let bearer = register_owner_bearer(&lease());
        let current = current_identity_persistence_generation();
        assert_eq!(session.generation(), current);
        assert_eq!(bearer.generation(), current);
    }

    #[test]
    fn durable_capability_exercise_admits_when_live_and_current() {
        let _g = guard();
        let session = register_owner_session(&lease(), CancellationToken::new());
        let bearer = register_owner_bearer(&lease());
        assert!(session.admit_exercise().is_ok());
        assert!(bearer.admit_exercise().is_ok());
    }

    // §7 revocation schedule (session): a session issued under A cannot
    // transmit after the generation advances to B — the frame send is refused
    // with zero bytes. Drives the registry generation directly (C1 drain-test
    // pattern); no production transition driver exists until C5.
    #[test]
    fn stale_session_exercise_refuses_after_a_generation_bump() {
        let _g = guard();
        let session = register_owner_session(&lease(), CancellationToken::new());
        // A transition drains then resumes at the winning generation.
        begin_egress_drain().unwrap();
        resume_egress_live();
        assert!(
            session.admit_exercise().is_err(),
            "a session stamped under A must refuse to transmit after B wins"
        );
    }

    // §7 revocation schedule (bearer): same stale-generation control for the
    // pre-minted bearer attach path.
    #[test]
    fn stale_bearer_exercise_refuses_after_a_generation_bump() {
        let _g = guard();
        let bearer = register_owner_bearer(&lease());
        begin_egress_drain().unwrap();
        resume_egress_live();
        assert!(
            bearer.admit_exercise().is_err(),
            "a bearer minted under A must refuse to attach after B wins"
        );
    }

    // F1 regression: a capability REGISTERED after a bump but derived from a
    // lease admitted BEFORE it stamps the lease's (losing) generation, not the
    // winning one — so it fails closed. This is the barrier-bypass control:
    // before the stamp-from-lease fix, registration re-read the winning
    // generation and the capability survived the barrier and admitted under B.
    #[test]
    fn capability_registered_across_a_bump_stamps_the_losing_generation() {
        let _g = guard();
        // Lease admitted under the losing generation A.
        let lease = lease();
        // A transition begins and bumps the generation to B before the lease
        // drains (begin_egress_drain advances immediately).
        let winning = begin_egress_drain().unwrap();
        // Registration happens now, under the pre-bump lease.
        let bearer = register_owner_bearer(&lease);
        assert_eq!(
            bearer.generation(),
            winning - 1,
            "the capability stamps the lease's losing generation, not the winning one"
        );
        // The barrier revokes everything older than the winner — this bearer
        // is caught, not bypassed.
        assert_eq!(
            revoke_durable_capabilities_before(winning),
            1,
            "the across-a-bump capability is revoked by the barrier"
        );
        // And even had it survived, exercise refuses once the state is Live
        // again: its stamp is stale.
        resume_egress_live();
        drop(lease);
        assert!(
            bearer.admit_exercise().is_err(),
            "a capability derived from the losing identity must refuse to transmit"
        );
    }

    // Stale-capability ZERO-BYTES control (Paul's condition a): exercise
    // validation fails BEFORE any transmission, for both durable kinds, under
    // both a generation mismatch and the drain/latch state — so no site can
    // send a byte on stale authority.
    #[test]
    fn durable_exercise_refuses_while_draining_and_while_latched() {
        let _g = guard();
        let session = register_owner_session(&lease(), CancellationToken::new());
        let bearer = register_owner_bearer(&lease());
        begin_egress_drain().unwrap();
        assert!(
            session.admit_exercise().is_err(),
            "draining refuses session"
        );
        assert!(bearer.admit_exercise().is_err(), "draining refuses bearer");
        latch_identity_indeterminate();
        assert!(session.admit_exercise().is_err(), "latch refuses session");
        assert!(bearer.admit_exercise().is_err(), "latch refuses bearer");
    }

    // The C5 barrier cancels old-generation sessions and drops old bearers,
    // and does NOT touch capabilities issued at the winning generation.
    #[test]
    fn barrier_revokes_old_generation_capabilities_only() {
        let _g = guard();
        let old_session_cancel = CancellationToken::new();
        let old_session = register_owner_session(&lease(), old_session_cancel.clone());
        let _old_bearer = register_owner_bearer(&lease());
        assert_eq!(live_durable_capability_count(), 2);

        // Transition to B.
        let winning = begin_egress_drain().unwrap();
        resume_egress_live();
        // A capability issued at the winning generation survives the barrier.
        let new_session = register_owner_session(&lease(), CancellationToken::new());

        let revoked = revoke_durable_capabilities_before(winning);
        assert_eq!(revoked, 2, "both old-generation capabilities revoked");
        assert!(
            old_session_cancel.is_cancelled(),
            "the old session's registered token was invoked"
        );
        assert_eq!(
            live_durable_capability_count(),
            1,
            "only the winning-generation capability remains"
        );
        assert!(
            new_session.admit_exercise().is_ok(),
            "the winning-generation session still transmits"
        );
        // old_session is now deregistered by the barrier; dropping it is a
        // no-op remove.
        drop(old_session);
    }

    // Registration-completeness assertion (Paul's condition b): every issued
    // durable capability is present in the registry with a revocation handle
    // the C5 barrier can invoke, and dropping the handle deregisters it so a
    // dead session/bearer is never a stale revocation target.
    #[test]
    fn issued_capabilities_are_registered_and_deregister_on_drop() {
        let _g = guard();
        assert_eq!(live_durable_capability_count(), 0);
        let session = register_owner_session(&lease(), CancellationToken::new());
        assert_eq!(live_durable_capability_count(), 1);
        let bearer = register_owner_bearer(&lease());
        assert_eq!(live_durable_capability_count(), 2);
        drop(session);
        assert_eq!(live_durable_capability_count(), 1, "session deregistered");
        drop(bearer);
        assert_eq!(live_durable_capability_count(), 0, "bearer deregistered");
    }

    // A no-transition control: with no generation bump, a registered
    // capability keeps transmitting and the barrier revokes nothing.
    #[test]
    fn no_transition_leaves_durable_capabilities_intact() {
        let _g = guard();
        let session = register_owner_session(&lease(), CancellationToken::new());
        let current = current_identity_persistence_generation();
        assert_eq!(
            revoke_durable_capabilities_before(current),
            0,
            "nothing older than the current generation to revoke"
        );
        assert!(
            session.admit_exercise().is_ok(),
            "with no transition the session still transmits"
        );
    }

    // §7 artifact schedules (P31-C1) — three schedules proving shape (ii): the
    // generation bump reaches artifacts WITHOUT a barrier registry entry, so a
    // stamped value's application is refused post-bump by the generation
    // compare alone. `admit_exercise` on an ArtifactPolicy capability stands in
    // for both the collapsed-artifact local check and the boundary-crossing
    // artifact's simulated application-site compare (C6/C7).

    // P31-C1 (a): a Rust-consumed / collapsed artifact stamped under A refuses
    // its local application after the generation advances to B.
    #[test]
    fn stale_artifact_application_refuses_after_a_generation_bump() {
        let _g = guard();
        let artifact = register_owner_artifact(&lease());
        begin_egress_drain().unwrap();
        resume_egress_live();
        assert!(
            artifact.admit_exercise().is_err(),
            "an artifact stamped under A must refuse application after B wins"
        );
    }

    // P31-C1 (b): the barrier does NOT revoke artifacts by entry — the bump is
    // the invalidation. `revoke_durable_capabilities_before` reports zero
    // side-effecting revocations for a registry holding only artifacts, and the
    // stamped artifact still refuses application by generation compare. This is
    // the test-level proof that the bump reaches artifacts without registry
    // entries (Paul's condition 2).
    #[test]
    fn barrier_leaves_artifacts_to_generation_compare_not_entry_revocation() {
        let _g = guard();
        let artifact = register_owner_artifact(&lease());
        assert_eq!(live_durable_capability_count(), 1);
        let winning = begin_egress_drain().unwrap();
        resume_egress_live();
        assert_eq!(
            revoke_durable_capabilities_before(winning),
            0,
            "the barrier performs no per-entry revocation for artifacts (shape (ii))"
        );
        assert!(
            artifact.admit_exercise().is_err(),
            "the bump alone invalidates the artifact — the generation compare refuses"
        );
    }

    // P31-C1 (c): a boundary-crossing artifact's wire stamp carries the issuing
    // generation, and once the generation advances the stamp is stale relative
    // to the current generation — the invariant C6/C7's application-site
    // compare enforces against `ArtifactStamp::generation`.
    #[test]
    fn boundary_crossing_artifact_stamp_is_stale_after_a_bump() {
        let _g = guard();
        let artifact = register_owner_artifact(&lease());
        let stamp = artifact.stamp();
        assert_eq!(stamp.generation, current_identity_persistence_generation());
        let winning = begin_egress_drain().unwrap();
        resume_egress_live();
        assert_ne!(
            stamp.generation, winning,
            "the wire stamp is stale relative to the winning generation; the C6/C7 \
             application-site compare refuses it"
        );
        assert!(
            artifact.admit_exercise().is_err(),
            "and the capability itself refuses local application"
        );
    }

    // §7 decryption-artifact schedules (P32-C1) — two schedules for the
    // decryption outputs (nip44_decrypt_from_self / decrypt_observer_event):
    // the decrypted plaintext stamped under A refuses application after B, and
    // a no-transition control keeps it applicable.

    // P32-C1 (a): a decryption-output artifact stamped under A refuses
    // application after the generation advances to B.
    #[test]
    fn stale_decryption_artifact_application_refuses_after_a_bump() {
        let _g = guard();
        let decrypted = register_owner_artifact(&lease());
        begin_egress_drain().unwrap();
        resume_egress_live();
        assert!(
            decrypted.admit_exercise().is_err(),
            "a decryption output stamped under A must refuse application after B wins"
        );
    }

    // P32-C1 (b): no-transition control — a decryption-output artifact with no
    // intervening bump stays applicable and the barrier revokes nothing.
    #[test]
    fn no_transition_leaves_decryption_artifact_applicable() {
        let _g = guard();
        let decrypted = register_owner_artifact(&lease());
        let current = current_identity_persistence_generation();
        assert_eq!(
            revoke_durable_capabilities_before(current),
            0,
            "no side-effecting revocation with no transition"
        );
        assert!(
            decrypted.admit_exercise().is_ok(),
            "with no transition the decryption output stays applicable"
        );
    }

    // Wire-shape LOCK (Paul's condition 2): the boundary payload serializes as
    // exactly `{ value, artifact: { id, generation } }`. C6/C7 inherits this
    // contract — the frontend `StampedArtifact<T>` mirror and every
    // application-site compare read `artifact.generation`, so a silent drift
    // here (renamed/reshaped field) would break C6/C7 undetectably. Pinning it
    // now makes any drift a test failure, not a C6/C7 rediscovery.
    #[test]
    fn stamped_artifact_serializes_to_the_locked_wire_shape() {
        let stamped = StampedArtifact {
            value: "signed-event-json",
            artifact: ArtifactStamp {
                id: 7,
                generation: 3,
            },
        };
        let json = serde_json::to_value(&stamped).expect("serializes");
        assert_eq!(
            json,
            serde_json::json!({
                "value": "signed-event-json",
                "artifact": { "id": 7, "generation": 3 },
            }),
            "the boundary payload must be {{value, artifact:{{id, generation}}}} \
             — the contract C6/C7 threads to application sites"
        );
    }
}
