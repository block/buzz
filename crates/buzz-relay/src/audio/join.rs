//! Cross-pod huddle join coordination over the mesh `HuddleControl` profile.
//!
//! This is the control plane that decides, at join time, *which pod owns a
//! huddle* and wires a client into that owner's room — the counterpart to the
//! media datagram fan-out in [`super::mesh`]. It is the join path
//! [`super::handler`] calls once a client has authed and passed membership.
//!
//! ## Ownership decision (Redis is the arbiter, mesh is only a hint)
//!
//! A huddle's `session_id` is its `channel_id`. Exactly one pod owns it: the
//! holder of the Redis fenced CAS lease. On join we resolve ownership through
//! the [`HuddleDirectory`]:
//!
//! - **No live lease** → this pod acquires it and becomes owner
//!   ([`JoinOutcome::LocalOwner`]). The client admits to a local [`Room`] as in
//!   a single-pod huddle.
//! - **Lease held by us** → [`JoinOutcome::LocalOwner`] at the live generation.
//! - **Lease held by another pod** → [`JoinOutcome::RemoteOwner`]. The client
//!   admits to a *local* room too, but the pod also opens a `HuddleControl`
//!   stream to the owner and registers the client as a remote peer there so the
//!   owner fans media back (see [`super::mesh`]).
//!
//! Membership never grants ownership: it may say "route to that pod," never
//! "take over." The owner side re-validates every registration's fence against
//! Redis on receipt — fencing at every hop, not just at the origin — so a lease
//! that changes between our lookup and the owner's receipt is caught there.
//!
//! ## `HuddleControl` payload schema (owned here)
//!
//! The mesh wire layer carries huddle-control bytes opaquely in
//! [`MeshStreamFrame::Data`](buzz_relay_mesh::MeshStreamFrame). Their layout is
//! [`HuddleControlMsg`], postcard-encoded. Non-owner → owner:
//! [`HuddleControlMsg::RegisterPeer`] / [`HuddleControlMsg::UnregisterPeer`].
//! Owner → non-owner: [`HuddleControlMsg::PeerRegistered`] (assigned index) or
//! [`HuddleControlMsg::RegisterRejected`] (fence/admission failure surfaced to
//! the client as a join error, never a silent media drop).

use std::sync::Arc;
use std::time::Duration;

use buzz_core::CommunityId;
use buzz_relay_mesh::{
    FencedHeader, GoodbyeReason, MeshDatagram, MeshError, MeshStream, MeshStreamFrame, Profile,
    RelayPeerTransport, RuntimeId, StreamHello, StreamRole,
};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::debug;
use uuid::Uuid;

use super::mesh::spawn_remote_peer_sink;
use super::room::{
    AdmissionError, AudioRoomManager, Room, RosterDelta as RoomRosterDelta, RosterPeer,
};
use crate::tunnel::directory::{ReleaseResult, RenewResult, SessionDirectory, SessionLease};

/// The slice of the Redis fenced session directory the huddle join path needs.
///
/// Implemented by the session-directory lane's `SessionDirectory` over the
/// Redis CAS lease (see [`crate::tunnel::directory`]). Kept as a trait so the
/// coordinator is unit-testable without Redis, and so the handler depends on a
/// capability rather than a concrete type. Every method is a fenced,
/// linearizable Redis operation — the arbiter of ownership.
#[async_trait::async_trait]
pub trait HuddleDirectory: Send + Sync {
    /// Look up the live owner + generation for a huddle, or `None` if no lease
    /// exists yet.
    async fn owner_of(
        &self,
        community_id: CommunityId,
        session_id: Uuid,
    ) -> Result<Option<Ownership>, MeshError>;

    /// Acquire ownership of a huddle if it is currently unowned. `owner` is the
    /// runtime that would own the lease (this pod's mesh identity). Returns the
    /// resulting ownership either way: `Acquired` when this pod took the lease
    /// (carrying the full [`HuddleLease`] material so the owner side can renew /
    /// release it), `Held` when another pod won the race (CAS lost) — a routing
    /// hint only, so it carries the lighter [`Ownership`] snapshot.
    async fn acquire(
        &self,
        community_id: CommunityId,
        session_id: Uuid,
        owner: RuntimeId,
    ) -> Result<AcquireOutcome, MeshError>;

    /// Renew an owned huddle lease. The renewer calls this on an interval to
    /// hold the fenced lease; a [`HuddleRenewOutcome::Lost`] or an error is
    /// owner-loss (Redis, the arbiter, no longer names this pod at this
    /// generation). Mesh membership never enters here.
    async fn renew(&self, lease: &HuddleLease) -> Result<HuddleRenewOutcome, MeshError>;

    /// Release an owned huddle lease on clean teardown. A
    /// [`HuddleReleaseOutcome::NotOwner`] means the lease already moved — the
    /// caller lost ownership before it could release, which is owner-loss.
    async fn release(&self, lease: &HuddleLease) -> Result<HuddleReleaseOutcome, MeshError>;

    /// Validate a fenced header against the live lease. Returns a typed
    /// [`MeshError`] fence rejection when the frame is stale / unowned /
    /// owner-mismatched — the caller surfaces this to the client as a join
    /// rejection, never a media drop.
    async fn validate(
        &self,
        community_id: CommunityId,
        fenced: &FencedHeader,
    ) -> Result<(), MeshError>;
}

/// Bridges the concrete Redis-backed [`SessionDirectory`] to the huddle join
/// path's capability trait. Huddle sessions always use the
/// [`Profile::HuddleControl`] profile when acquiring a lease, so the profile is
/// fixed here rather than threaded through the join API.
#[async_trait::async_trait]
impl HuddleDirectory for crate::tunnel::directory::SessionDirectory {
    async fn owner_of(
        &self,
        community_id: CommunityId,
        session_id: Uuid,
    ) -> Result<Option<Ownership>, MeshError> {
        let lease = self
            .lookup(community_id, session_id)
            .await
            .map_err(|e| MeshError::Transport(e.to_string()))?;
        Ok(lease.map(|l| Ownership {
            owner_runtime_id: l.owner_runtime_id,
            generation: l.generation,
        }))
    }

    async fn acquire(
        &self,
        community_id: CommunityId,
        session_id: Uuid,
        owner: RuntimeId,
    ) -> Result<AcquireOutcome, MeshError> {
        use crate::tunnel::directory::AcquireResult;
        let result = self
            .acquire(community_id, session_id, owner, HUDDLE_CONTROL_PROFILE)
            .await
            .map_err(|e| MeshError::Transport(e.to_string()))?;
        Ok(match result {
            // Acquired: keep the full lease so the owner side can renew/release
            // it. Redis is the arbiter — we retain its material, never rebuild
            // authority from `{owner, generation}`.
            AcquireResult::Acquired(l) => AcquireOutcome::Acquired(HuddleLease(l)),
            AcquireResult::Exists(l) => AcquireOutcome::Held(Ownership {
                owner_runtime_id: l.owner_runtime_id,
                generation: l.generation,
            }),
        })
    }

    async fn renew(&self, lease: &HuddleLease) -> Result<HuddleRenewOutcome, MeshError> {
        let result = SessionDirectory::renew(self, &lease.0)
            .await
            .map_err(|e| MeshError::Transport(e.to_string()))?;
        Ok(match result {
            RenewResult::Renewed(l) => HuddleRenewOutcome::Renewed(HuddleLease(l)),
            RenewResult::Lost { .. } => HuddleRenewOutcome::Lost,
        })
    }

    async fn release(&self, lease: &HuddleLease) -> Result<HuddleReleaseOutcome, MeshError> {
        let result = SessionDirectory::release(self, &lease.0)
            .await
            .map_err(|e| MeshError::Transport(e.to_string()))?;
        Ok(match result {
            ReleaseResult::Released(_) => HuddleReleaseOutcome::Released,
            ReleaseResult::NotOwner { .. } => HuddleReleaseOutcome::NotOwner,
        })
    }

    async fn validate(
        &self,
        community_id: CommunityId,
        fenced: &FencedHeader,
    ) -> Result<(), MeshError> {
        self.validate_fenced_header(community_id, fenced).await
    }
}

/// A resolved ownership snapshot: which pod owns a huddle, at what generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ownership {
    /// Runtime holding the Redis lease for this huddle.
    pub owner_runtime_id: RuntimeId,
    /// Fenced generation of this ownership epoch; monotonic per session.
    pub generation: u64,
}

/// Real, fenced lease material for a huddle this pod owns.
///
/// Retained from [`HuddleDirectory::acquire`] so the owner side can renew and
/// release the exact `(owner_runtime_id, generation)` epoch Redis granted —
/// authority is never reconstructed from an `Ownership` snapshot or a mesh
/// membership hint. The inner [`SessionLease`] is the session-directory lane's
/// Redis-arbitrated lease; huddle code treats it as an opaque handle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HuddleLease(pub(crate) SessionLease);

impl HuddleLease {
    /// The fenced generation this lease owns.
    pub fn generation(&self) -> u64 {
        self.0.generation
    }

    /// The runtime that owns this lease (this pod).
    pub fn owner_runtime_id(&self) -> RuntimeId {
        self.0.owner_runtime_id
    }
}

/// Result of a huddle-lease renewal. Loss is owner-loss: Redis no longer names
/// this pod at this generation, so the owner must tear down and let clients
/// rejoin against the new owner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HuddleRenewOutcome {
    /// Lease TTL extended; this pod still owns the generation (carries the
    /// refreshed lease).
    Renewed(HuddleLease),
    /// Lease absent or moved to a different owner/generation — owner-loss.
    Lost,
}

/// Result of releasing a huddle lease on teardown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HuddleReleaseOutcome {
    /// Lease deleted; clean release by the owner.
    Released,
    /// Lease absent or already moved — the owner lost it before releasing.
    NotOwner,
}

/// Result of an ownership acquire attempt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AcquireOutcome {
    /// This pod created the lease and owns the returned generation. Carries the
    /// full [`HuddleLease`] so the owner side can renew/release it.
    Acquired(HuddleLease),
    /// Another pod already holds the lease (CAS lost); route to it instead.
    Held(Ownership),
}

/// What the handler should do with a join, decided by ownership.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JoinOutcome {
    /// This pod owns the huddle: admit the client to a local room directly, as
    /// in a single-pod huddle. Cross-pod peers (if any) reach us over the mesh.
    LocalOwner {
        /// Fenced generation this pod owns — stamped on media it fans out.
        generation: u64,
    },
    /// Another pod owns the huddle: admit the client locally *and* register it
    /// with the owner over `HuddleControl` so the owner fans media back. The
    /// fenced header is pre-validated against the live lease; the owner
    /// re-validates on receipt.
    RemoteOwner {
        /// Owner to open the `HuddleControl` stream to.
        owner_runtime_id: RuntimeId,
        /// Fenced generation of the owner's epoch.
        generation: u64,
    },
}

impl JoinOutcome {
    /// The fenced header for frames this join produces, given the huddle's
    /// session id (its channel id) and resolved owner. For a local-owner join
    /// the owner is this pod (`local_runtime_id`); for a remote-owner join it
    /// is the resolved owner.
    pub fn fenced_header(&self, session_id: Uuid, local_runtime_id: RuntimeId) -> FencedHeader {
        match *self {
            JoinOutcome::LocalOwner { generation } => FencedHeader {
                session_id,
                generation,
                owner_runtime_id: local_runtime_id,
            },
            JoinOutcome::RemoteOwner {
                owner_runtime_id,
                generation,
            } => FencedHeader {
                session_id,
                generation,
                owner_runtime_id,
            },
        }
    }
}

/// Outcome of [`resolve_join`]: the routing verdict plus, on the arm that
/// freshly acquired the lease, the real [`HuddleLease`] to install in the
/// [`HuddleOwnerRegistry`].
///
/// `acquired` is `Some` **only** when this call won the Redis CAS and minted a
/// new lease (`JoinOutcome::LocalOwner` via acquire). The steady-state owner
/// arm and every remote-owner arm carry `None`: a steady-state owner reuses the
/// registry's existing renewer rather than rebuilding authority from the
/// generation snapshot, honoring the authority-not-snapshot rule.
#[derive(Debug)]
pub struct ResolvedJoin {
    /// What the handler should do with the join.
    pub outcome: JoinOutcome,
    /// The freshly-acquired owner lease, present only on the acquire arm.
    pub acquired: Option<HuddleLease>,
}

/// Resolve who owns a huddle and how this pod should join it.
///
/// The ownership plane is Redis-arbitrated: we look up the live lease and, only
/// if the huddle is unowned, attempt to acquire it (losing the CAS gracefully
/// hands us a `RemoteOwner` outcome pointing at the winner). A remote-owner
/// outcome is fence-validated against the live lease before we route to it, so
/// a caller never opens a control stream on a header Redis would reject.
///
/// On the arm that freshly acquires the lease, the real [`HuddleLease`] is
/// returned in [`ResolvedJoin::acquired`] so the caller installs one per-room
/// renewer in the [`HuddleOwnerRegistry`] — the authority is the lease Redis
/// minted, never rebuilt from the generation snapshot.
///
/// `local_runtime_id` is this pod's mesh identity, used to tell "I own it" from
/// "someone else owns it."
pub async fn resolve_join<D: HuddleDirectory + ?Sized>(
    directory: &D,
    community_id: CommunityId,
    session_id: Uuid,
    local_runtime_id: RuntimeId,
) -> Result<ResolvedJoin, MeshError> {
    // Look up the live lease first: the common steady-state case is an already
    // owned huddle, and we avoid an acquire attempt (and its generation INCR
    // race window) when a live owner already exists.
    let ownership = match directory.owner_of(community_id, session_id).await? {
        Some(o) => o,
        None => {
            // Unowned: try to take it. A lost CAS means a peer beat us to it
            // between our lookup and acquire — treat the winner as the owner.
            match directory
                .acquire(community_id, session_id, local_runtime_id)
                .await?
            {
                AcquireOutcome::Acquired(o) => {
                    let generation = o.generation();
                    return Ok(ResolvedJoin {
                        outcome: JoinOutcome::LocalOwner { generation },
                        acquired: Some(o),
                    });
                }
                AcquireOutcome::Held(o) => o,
            }
        }
    };

    if ownership.owner_runtime_id == local_runtime_id {
        return Ok(ResolvedJoin {
            outcome: JoinOutcome::LocalOwner {
                generation: ownership.generation,
            },
            acquired: None,
        });
    }

    // Remote owner: validate the fence against the live lease before we commit
    // to routing there. This is the origin-side hop of the fencing law; the
    // owner re-validates on receipt.
    let fenced = FencedHeader {
        session_id,
        generation: ownership.generation,
        owner_runtime_id: ownership.owner_runtime_id,
    };
    directory.validate(community_id, &fenced).await?;

    Ok(ResolvedJoin {
        outcome: JoinOutcome::RemoteOwner {
            owner_runtime_id: ownership.owner_runtime_id,
            generation: ownership.generation,
        },
        acquired: None,
    })
}

/// How long to wait between re-resolves while a `LocalOwner` snapshot has no
/// live registry entry yet, and how many times to try before failing closed.
/// The CAS winner installs its registry entry immediately after the (fast,
/// local) `add_peer`, so the ambiguous window is tiny; ~500 ms of bounded
/// polling covers it with wide margin without ever proceeding ownerless.
const OWNER_READY_RETRY_INTERVAL: Duration = Duration::from_millis(20);
const OWNER_READY_MAX_ATTEMPTS: u32 = 25;

/// Resolve a join and, on the steady-state `LocalOwner` reuse arm, ensure a
/// live owner renewer actually exists before the caller admits a local owner
/// peer.
///
/// [`resolve_join`] returns `LocalOwner { generation }` with `acquired = None`
/// whenever Redis names this pod the owner but this call did not mint the lease
/// — the steady-state reuse arm. Reuse is only correct when a live entry exists
/// in the [`HuddleOwnerRegistry`]: that entry carries the `lost` signal the
/// admitted peer's owner-loss watcher selects on. If the CAS winner has
/// resolved but not yet installed its entry (the window between its
/// `resolve_join` and its post-`add_peer` `attach`), a naive reuse would admit
/// a local owner peer with **no** loss watcher — it would fan media at a
/// generation it cannot observe losing, the exact consumer-#2 split-brain the
/// fence exists to prevent.
///
/// So the registry lookup — never the `resolve_join` snapshot — gates reuse:
/// when the snapshot says `LocalOwner`/`acquired = None` but no live entry
/// exists, re-resolve in a bounded loop. Each retry lands on one of:
/// - the winner installed in the meantime → live entry → **reuse**;
/// - the room emptied and released underneath us → `owner_of` is now unowned →
///   our re-acquire wins CAS → `acquired = Some` → **fresh lease**;
/// - still the ambiguous window → sleep and retry.
///
/// If the loop exhausts we fail the join closed (a transient contention error,
/// surfaced to the client exactly like a lost CAS) rather than ever admit an
/// ownerless owner peer. The CAS-winning and remote-owner arms return
/// immediately — only the reuse arm can loop.
pub async fn resolve_join_owner_ready<D: HuddleDirectory + ?Sized>(
    directory: &D,
    community_id: CommunityId,
    session_id: Uuid,
    local_runtime_id: RuntimeId,
    owners: &HuddleOwnerRegistry,
) -> Result<ResolvedJoin, MeshError> {
    for _ in 0..OWNER_READY_MAX_ATTEMPTS {
        let resolved = resolve_join(directory, community_id, session_id, local_runtime_id).await?;

        // Only the steady-state reuse arm (LocalOwner minted by another
        // connection) can be ambiguous. The CAS winner (acquired = Some) and
        // every remote-owner arm are authoritative — return them as-is.
        match (&resolved.outcome, &resolved.acquired) {
            (JoinOutcome::LocalOwner { .. }, None) => {
                if owners.lost_for(session_id).is_some() {
                    return Ok(resolved); // live entry → reuse is safe
                }
                // Ambiguous window: winner not yet attached (or room released
                // underneath us). Wait and re-resolve — the retry either finds
                // the installed entry or wins a fresh CAS.
                tokio::time::sleep(OWNER_READY_RETRY_INTERVAL).await;
            }
            _ => return Ok(resolved),
        }
    }

    // Exhausted: fail closed. Never admit a local owner peer with no live
    // renewer — that is the ownerless split-brain this loop exists to prevent.
    Err(MeshError::Transport(format!(
        "huddle owner not ready for session {session_id} after {OWNER_READY_MAX_ATTEMPTS} attempts"
    )))
}

/// Renewal cadence for an owned huddle lease. Mirrors the reliable lane
/// (`crate::tunnel::reliable::DEFAULT_RENEW_INTERVAL`): renew every 10s against
/// the directory's 30s TTL, giving three renew attempts per lease lifetime.
const DEFAULT_HUDDLE_RENEW_INTERVAL: Duration = Duration::from_secs(10);

/// Background huddle-lease renewer plus an observable ownership-loss signal.
///
/// The owner-side analog of the reliable lane's
/// `crate::tunnel::reliable::ReliableLeaseRenewer`, with the identical
/// contract: `lost` is cancelled **only** on fenced owner-loss —
/// [`HuddleRenewOutcome::Lost`], a renew error, a release
/// [`HuddleReleaseOutcome::NotOwner`], or a release error on a non-caller-loss
/// exit. Caller-initiated `cancel` (client drain, clean leave, shutdown) is
/// normal teardown and stays silent.
pub struct HuddleLeaseRenewer {
    /// Worker task. Await during teardown if the caller needs release
    /// completion.
    pub task: JoinHandle<()>,
    /// Cancelled when renewal observes loss / `NotOwner` or a renewal error.
    /// Owner-side `serve_control_loop`s select on this to emit a proactive
    /// `Goodbye(StaleGeneration)` so non-owner pods rejoin against the new
    /// owner.
    pub lost: CancellationToken,
}

/// Start background renewal for an owner-held huddle lease and return a loss
/// signal consumers can observe.
///
/// A mirror of `crate::tunnel::reliable::spawn_observable_renewer`: renew the
/// fenced lease on an interval; on owner-loss trip `lost` and stop; on
/// caller-`cancel` release cleanly and stay silent. `directory` is the fenced
/// arbiter — the sole authority for whether this pod still owns the generation.
pub fn spawn_observable_huddle_renewer<D: HuddleDirectory + ?Sized + 'static>(
    directory: Arc<D>,
    lease: HuddleLease,
    cancel: CancellationToken,
) -> HuddleLeaseRenewer {
    spawn_huddle_renewer_with_interval(directory, lease, cancel, DEFAULT_HUDDLE_RENEW_INTERVAL)
}

fn spawn_huddle_renewer_with_interval<D: HuddleDirectory + ?Sized + 'static>(
    directory: Arc<D>,
    lease: HuddleLease,
    cancel: CancellationToken,
    renew_interval: Duration,
) -> HuddleLeaseRenewer {
    let lost = CancellationToken::new();
    let lost_for_task = lost.clone();
    let task = tokio::spawn(async move {
        let owner_runtime_id = lease.owner_runtime_id();
        let generation = lease.generation();
        let mut interval = tokio::time::interval(renew_interval);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let caller_cancelled = loop {
            tokio::select! {
                _ = cancel.cancelled() => break true,
                _ = interval.tick() => {
                    match directory.renew(&lease).await {
                        Ok(HuddleRenewOutcome::Renewed(_)) => {}
                        Ok(HuddleRenewOutcome::Lost) => {
                            tracing::warn!(
                                %owner_runtime_id,
                                generation,
                                "huddle lease renewal lost"
                            );
                            lost_for_task.cancel();
                            break false;
                        }
                        Err(err) => {
                            tracing::warn!(
                                %owner_runtime_id,
                                generation,
                                error = %err,
                                "huddle lease renewal failed"
                            );
                            lost_for_task.cancel();
                            break false;
                        }
                    }
                }
            }
        };

        match directory.release(&lease).await {
            Ok(HuddleReleaseOutcome::Released) => {}
            Ok(HuddleReleaseOutcome::NotOwner) => {
                tracing::warn!(
                    %owner_runtime_id,
                    generation,
                    "huddle lease release found non-owner"
                );
                lost_for_task.cancel();
            }
            Err(err) => {
                tracing::warn!(
                    %owner_runtime_id,
                    generation,
                    error = %err,
                    "huddle lease release failed"
                );
                // A release error after a clean caller cancel is logged but not
                // owner-loss: we are shutting down anyway. On any other exit
                // (already lost) the signal has already tripped; trip it here
                // too so a loss that only surfaces at release is never silent.
                if !caller_cancelled {
                    lost_for_task.cancel();
                }
            }
        }
    });

    HuddleLeaseRenewer { task, lost }
}

/// Per-room owner-lease coordination shared by the WS-join owner path and the
/// [`HuddleControlAcceptor`].
///
/// The huddle-lease renewer is a per-*room* resource, not per-connection: the
/// pod that wins the Redis CAS holds one lease for the room, and N local owner
/// joiners must not each spawn a renewer racing to renew/release it. This
/// registry holds that single owner state, keyed by `session_id`, so:
/// - the acquiring WS connection installs one renewer ([`Self::attach`]);
/// - later owner joiners and the inbound control acceptor read its `lost`
///   signal ([`Self::lost_for`]) instead of rebuilding authority from a
///   generation snapshot;
/// - when the room empties the owner connection releases the lease
///   ([`Self::release`]), cancelling the renewer.
///
/// **The registry, not the `resolve_join` snapshot, gates reuse.** A late
/// joiner whose ownership lookup says "owned by us" but finds no live entry
/// (the room emptied and released underneath it) must re-acquire rather than
/// adopt a torn-down lease — `lost_for` returning `None` is that signal.
#[derive(Default)]
pub struct HuddleOwnerRegistry {
    entries: DashMap<Uuid, HuddleOwnerEntry>,
    draining: std::sync::atomic::AtomicBool,
}

struct HuddleOwnerEntry {
    /// Owner-loss signal fanned into every control loop and owner WS peer for
    /// the room. Cancelled by the renewer on fenced loss.
    lost: CancellationToken,
    /// Owner-drain signal fanned into every control loop and owner WS peer for
    /// the room. Cancelled by [`Self::drain`] so peers close with
    /// `Goodbye(Draining)` rather than the fenced-loss `StaleGeneration` path.
    draining: CancellationToken,
    /// Caller-cancel handed to the renewer; cancelled by [`Self::release`] on
    /// room-empty so the lease is released cleanly (silent, not owner-loss), and
    /// by [`Self::drain`] after the drain signal so Redis releases the fenced
    /// lease promptly.
    cancel: CancellationToken,
    /// Generation this entry's lease owns. `release`/`drain` are fenced on it so
    /// a stale room-empty/drain cannot tear down a newer epoch a re-acquire
    /// installed.
    generation: u64,
}

/// Owner-side signals for one huddle epoch. Returned atomically from attach so
/// the CAS winner cannot miss a concurrent drain between installing the owner
/// entry and looking the drain token back up.
pub struct HuddleOwnerSignals {
    /// Fenced-loss signal.
    pub lost: CancellationToken,
    /// Intentional-drain signal.
    pub draining: CancellationToken,
}

impl HuddleOwnerRegistry {
    /// Empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether this runtime has begun shutdown drain. Once true, new huddle
    /// owner admissions fail closed even if they raced an earlier Redis lookup.
    pub fn is_draining(&self) -> bool {
        self.draining.load(std::sync::atomic::Ordering::Acquire)
    }

    /// The room's owner-loss signal, or `None` when this pod holds no live
    /// owner lease for the session. The acceptor passes this into
    /// `serve_control_loop`; a steady-state owner joiner uses it to reuse the
    /// existing renewer instead of acquiring again.
    pub fn lost_for(&self, session_id: Uuid) -> Option<CancellationToken> {
        self.entries.get(&session_id).map(|e| e.lost.clone())
    }

    /// The room's owner-drain signal, or `None` when this pod holds no live
    /// owner lease for the session. Cancelled by [`Self::drain`] / [`Self::drain_all`]
    /// to close local owner WS peers and emit `Goodbye(Draining)` to remote pods.
    pub fn drain_for(&self, session_id: Uuid) -> Option<CancellationToken> {
        self.entries.get(&session_id).map(|e| e.draining.clone())
    }

    /// Install the single per-room renewer for a freshly-acquired lease and
    /// return its `lost` signal.
    ///
    /// Called only on the acquire path — Redis CAS admits exactly one
    /// `Acquired` per generation, so at most one caller installs. If a live
    /// entry already exists (a re-acquire racing a not-yet-released prior
    /// epoch) the existing renewer wins and the just-acquired lease is released
    /// by cancelling a throwaway renewer, so no lease leaks.
    pub fn attach<D: HuddleDirectory + ?Sized + 'static>(
        &self,
        session_id: Uuid,
        directory: Arc<D>,
        lease: HuddleLease,
    ) -> CancellationToken {
        self.attach_signals(session_id, directory, lease).lost
    }

    /// Install the single per-room renewer and return both owner-side signals.
    /// The returned pair is captured from the entry while it is live, so the CAS
    /// winner cannot miss a concurrent drain between attach and a separate
    /// `drain_for` lookup.
    pub fn attach_signals<D: HuddleDirectory + ?Sized + 'static>(
        &self,
        session_id: Uuid,
        directory: Arc<D>,
        lease: HuddleLease,
    ) -> HuddleOwnerSignals {
        if self.is_draining() {
            let cancel = CancellationToken::new();
            cancel.cancel();
            spawn_observable_huddle_renewer(directory, lease, cancel);
            let draining = CancellationToken::new();
            draining.cancel();
            return HuddleOwnerSignals {
                lost: CancellationToken::new(),
                draining,
            };
        }
        let generation = lease.generation();
        if let Some(existing) = self.entries.get(&session_id) {
            // A live entry already owns this room; release our extra lease
            // cleanly rather than leaving two renewers on one session.
            let cancel = CancellationToken::new();
            cancel.cancel();
            spawn_observable_huddle_renewer(directory, lease, cancel);
            return HuddleOwnerSignals {
                lost: existing.lost.clone(),
                draining: existing.draining.clone(),
            };
        }
        let cancel = CancellationToken::new();
        let renewer = spawn_observable_huddle_renewer(directory, lease, cancel.clone());
        let draining = CancellationToken::new();
        let lost = renewer.lost.clone();
        self.entries.insert(
            session_id,
            HuddleOwnerEntry {
                lost: lost.clone(),
                draining: draining.clone(),
                cancel: cancel.clone(),
                generation,
            },
        );
        // Close the check→insert race with drain_all(): if shutdown began after
        // the first check but before publication, retract this exact epoch and
        // release its lease. A newer epoch, if any, is generation-fenced.
        if self.is_draining() {
            self.entries.remove_if(&session_id, |_, entry| {
                if entry.generation == generation {
                    entry.draining.cancel();
                    entry.cancel.cancel();
                    true
                } else {
                    false
                }
            });
            draining.cancel();
            cancel.cancel();
        }
        HuddleOwnerSignals { lost, draining }
    }

    /// Release the room's owner lease on room-empty: cancel the renewer's
    /// caller-token so it releases cleanly (silent) and drop the entry.
    ///
    /// Fenced on `generation`: a stale caller cannot tear down a newer epoch a
    /// re-acquire installed between this caller's room-empty and its release.
    pub fn release(&self, session_id: Uuid, generation: u64) {
        self.entries.remove_if(&session_id, |_, entry| {
            if entry.generation == generation {
                entry.cancel.cancel();
                true
            } else {
                false
            }
        });
    }

    /// Drain a room this pod owns: first fan out an explicit draining signal to
    /// local owner WS peers and remote control streams, then cancel the renewer
    /// so it CAS-releases the fenced lease. Fenced on generation exactly like
    /// [`Self::release`]; drain never transfers ownership, it only clears this
    /// pod's lease so rejoiners acquire through Redis.
    pub fn drain(&self, session_id: Uuid, generation: u64) -> bool {
        let mut drained = false;
        self.entries.remove_if(&session_id, |_, entry| {
            if entry.generation == generation {
                entry.draining.cancel();
                entry.cancel.cancel();
                drained = true;
                true
            } else {
                false
            }
        });
        drained
    }

    /// Drain every room currently owned by this runtime. Used by SIGTERM
    /// choreography after readiness has flipped and mesh membership is marked
    /// draining. Each individual room remains generation-fenced by [`Self::drain`].
    pub fn drain_all(&self) -> usize {
        self.draining
            .store(true, std::sync::atomic::Ordering::Release);
        let rooms: Vec<(Uuid, u64)> = self
            .entries
            .iter()
            .map(|entry| (*entry.key(), entry.generation))
            .collect();
        rooms
            .into_iter()
            .filter(|(session_id, generation)| self.drain(*session_id, *generation))
            .count()
    }

    /// Install an entry with a caller-supplied `lost` token and no renewer, for
    /// tests that exercise the fan-out to the control loop / WS peers in
    /// isolation from the (separately tested) renewer timing.
    #[cfg(test)]
    fn install_for_test(&self, session_id: Uuid, generation: u64) -> CancellationToken {
        let lost = CancellationToken::new();
        self.entries.insert(
            session_id,
            HuddleOwnerEntry {
                lost: lost.clone(),
                draining: CancellationToken::new(),
                cancel: CancellationToken::new(),
                generation,
            },
        );
        lost
    }
}

/// `HuddleControl` stream payload, carried in
/// [`MeshStreamFrame::Data`](buzz_relay_mesh::MeshStreamFrame)`.payload`,
/// postcard-encoded. This schema is owned by the huddle lane; the mesh wire
/// layer treats it as opaque bytes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HuddleControlMsg {
    /// Non-owner → owner: register a local client as a remote peer in the
    /// owner's room. The owner allocates the `peer_index`.
    RegisterPeer {
        /// The community the huddle belongs to, as its raw UUID. Rides the
        /// control frame because the mesh dispatch layer is community-agnostic —
        /// `Hello` and the fenced header carry no community — yet the owner's
        /// Redis fence key is `(community_id, session_id)`. Carried as a `Uuid`,
        /// not a [`CommunityId`], on purpose: `CommunityId` is deliberately
        /// non-deserializable so it can never be minted from client input. This
        /// mesh frame is server-to-server, and the owner reconstitutes the
        /// `CommunityId` explicitly via `from_uuid` before fencing. The
        /// assertion is self-verifying, not trusted: `validate` looks up the key
        /// the owner's own `acquire` created, so a wrong community finds no lease
        /// and is rejected (`no_active_lease`) *before* any room mutation. The
        /// owner latches the community from the first frame and rejects any
        /// later frame on the same stream that names a different one.
        community_id: Uuid,
        /// Nostr pubkey hex of the joining client.
        pubkey: String,
        /// Huddle audio protocol version the client negotiated; the owner's
        /// room is pinned to one version and rejects mismatches.
        protocol_version: u8,
    },
    /// Owner → non-owner: the client is registered; here is its assigned index.
    PeerRegistered {
        /// Pubkey the registration was for (echoed for correlation).
        pubkey: String,
        /// Owner-allocated 0..=254 index; the sole allocator is the owner, so
        /// indices never collide across pods.
        peer_index: u8,
        /// Complete authoritative roster after this admission. This is in the
        /// registration reply so no media/client identity can precede it.
        roster: RosterSnapshot,
    },
    /// Owner → non-owner: complete authoritative state at `revision`.
    RosterSnapshot {
        /// Owner-monotonic roster revision.
        revision: u64,
        /// Complete authoritative participants.
        peers: Vec<RosterEntry>,
    },
    /// Owner → non-owner: one ordered roster mutation. A revision gap is
    /// recovered by `RosterResync`, never applied speculatively.
    RosterDelta {
        /// Owner-monotonic roster revision.
        revision: u64,
        /// Newly admitted peer, when this is a join.
        joined: Option<RosterEntry>,
        /// Removed peer, when this is a leave.
        left: Option<RosterEntry>,
    },
    /// Non-owner → owner: request a complete snapshot after detecting a gap.
    RosterResync,
    /// Owner → non-owner: registration refused. Surfaced to the client as a
    /// join error (e.g. `room_full`, `upgrade_required`, or a fence rejection),
    /// never a silent media drop.
    RegisterRejected {
        /// Pubkey the registration was for (echoed for correlation).
        pubkey: String,
        /// Machine-readable reason, matching the single-pod WS error `code`s
        /// where applicable (`room_full`, `room_ended`, `upgrade_required`) or
        /// a fence reason (`stale_generation`, `no_active_lease`,
        /// `owner_mismatch`, `future_generation`).
        reason: RegisterRejection,
    },
    /// Non-owner → owner: the local client left; drop its remote peer.
    UnregisterPeer {
        /// Pubkey of the departing client.
        pubkey: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// One participant in the authoritative owner roster.
pub struct RosterEntry {
    /// Nostr pubkey hex.
    pub pubkey: String,
    /// Owner-assigned media routing index.
    pub peer_index: u8,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Complete authoritative roster at one owner revision.
pub struct RosterSnapshot {
    /// Owner-monotonic roster revision.
    pub revision: u64,
    /// Complete participants at this revision.
    pub peers: Vec<RosterEntry>,
}

impl From<RosterPeer> for RosterEntry {
    fn from(peer: RosterPeer) -> Self {
        Self {
            pubkey: peer.pubkey,
            peer_index: peer.peer_index,
        }
    }
}

/// Why an owner refused a remote-peer registration. Mirrors the single-pod
/// admission failures plus the fence-rejection taxonomy, so a cross-pod join
/// surfaces the same client-facing error a same-pod join would.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RegisterRejection {
    /// Owner's room hit the peer cap / exhausted the index space.
    RoomFull,
    /// Owner's room has ended (auto-ended or archived).
    RoomEnded,
    /// Owner's room is pinned to a different protocol version.
    VersionMismatch {
        /// Version the owner's room is pinned to.
        pinned: u8,
        /// Version the joining client requested.
        requested: u8,
    },
    /// The registration's fence was rejected by Redis on the owner. Carries the
    /// fence reason so `/_mesh` and the client see the same taxonomy the media
    /// path uses.
    Fenced(FenceRejection),
}

/// The Redis fence-rejection reasons, as a serializable enum for the
/// `HuddleControl` wire (the crate's [`MeshError`] fence variants are not
/// `Serialize`). Kept 1:1 with those variants so nothing is lost across the
/// wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FenceRejection {
    /// Frame generation is below the known floor.
    StaleGeneration,
    /// No live lease exists for the session.
    NoActiveLease,
    /// Frame owner does not match the live lease owner.
    OwnerMismatch,
    /// Frame generation does not match the live lease generation.
    FutureGeneration,
}

impl FenceRejection {
    /// Classify a [`MeshError`] fence variant. Returns `None` for non-fence
    /// errors (transport, encode/decode) — those are not registration
    /// rejections and are handled as stream failures by the caller.
    pub fn from_mesh_error(err: &MeshError) -> Option<Self> {
        match err {
            MeshError::StaleGeneration { .. } => Some(Self::StaleGeneration),
            MeshError::NoActiveLease { .. } => Some(Self::NoActiveLease),
            MeshError::OwnerMismatch { .. } => Some(Self::OwnerMismatch),
            MeshError::FutureGeneration { .. } => Some(Self::FutureGeneration),
            _ => None,
        }
    }

    /// Stable machine-readable code, matching the media path / `/_mesh`
    /// taxonomy (`stale_generation` | `no_active_lease` | `owner_mismatch` |
    /// `future_generation`).
    pub fn code(&self) -> &'static str {
        match self {
            Self::StaleGeneration => "stale_generation",
            Self::NoActiveLease => "no_active_lease",
            Self::OwnerMismatch => "owner_mismatch",
            Self::FutureGeneration => "future_generation",
        }
    }
}

/// Encode a [`HuddleControlMsg`] for a `MeshStreamFrame::Data` payload.
pub fn encode_control(msg: &HuddleControlMsg) -> Result<Vec<u8>, MeshError> {
    postcard::to_allocvec(msg).map_err(MeshError::Encode)
}

/// Decode a `HuddleControl` `Data` payload back into a [`HuddleControlMsg`].
pub fn decode_control(bytes: &[u8]) -> Result<HuddleControlMsg, MeshError> {
    postcard::from_bytes(bytes).map_err(MeshError::Decode)
}

/// The tunnel profile these control messages ride. `HuddleControl` is a
/// reliable stream — a dropped roster delta is an unrecoverable peer-index
/// desync, so it never rides datagrams.
pub const HUDDLE_CONTROL_PROFILE: Profile = Profile::HuddleControl;

// ── Owner-side HuddleControl accept path ─────────────────────────────────────
//
// The owner pod hosts the real [`Room`]. When a *non-owner* pod opens a
// `HuddleControl` stream and registers a client, the owner admits that client
// as an ordinary [`AudioPeer`] whose `audio_tx` is drained by
// [`super::mesh::spawn_remote_peer_sink`] back to the non-owner pod as
// datagrams. `Room` never learns about the mesh — a remote participant looks
// exactly like a local one to fan-out.

/// Owner-side handler for inbound `HuddleControl` streams.
///
/// One instance per relay; the boot-seam dispatcher routes every
/// `Profile::HuddleControl` session stream to [`Self::accept_inbound`]. It is
/// the counterpart to Perci's reliable-stream acceptor — same
/// `accept_inbound(from, hello, stream)` shape, different profile and body.
/// The community is not a handshake parameter: it rides the first stateful
/// frame and is self-verified by the fence (see [`Self::serve_control_loop`]).
pub struct HuddleControlAcceptor<D: HuddleDirectory + ?Sized> {
    rooms: Arc<AudioRoomManager>,
    transport: Arc<dyn RelayPeerTransport>,
    directory: Arc<D>,
    local_runtime_id: RuntimeId,
    /// Per-room owner state. The acceptor reads the room's owner-loss signal
    /// from here so every inbound control loop for a room this pod owns fans
    /// the *same* renewer's `lost` — the loss surfaces as a proactive
    /// `Goodbye(StaleGeneration)` to each non-owner pod.
    owners: Arc<HuddleOwnerRegistry>,
}

impl<D: HuddleDirectory + ?Sized> HuddleControlAcceptor<D> {
    /// Build the acceptor. `directory` is the fenced arbiter (re-validated on
    /// every registration); `transport` is used to open the media datagram
    /// sink back to each registering pod; `owners` supplies each room's
    /// owner-loss signal for the control-loop fan-out.
    pub fn new(
        rooms: Arc<AudioRoomManager>,
        transport: Arc<dyn RelayPeerTransport>,
        directory: Arc<D>,
        local_runtime_id: RuntimeId,
        owners: Arc<HuddleOwnerRegistry>,
    ) -> Self {
        Self {
            rooms,
            transport,
            directory,
            local_runtime_id,
            owners,
        }
    }

    /// Accept and validate an inbound `HuddleControl` stream, then serve its
    /// register/unregister control loop until the stream closes.
    ///
    /// The `Hello` is validated **structurally only** — it admits no peer and
    /// touches no room, so it is deliberately not Redis-fenced: the fence key is
    /// `(community_id, session_id)` and the community is not known until the
    /// first `RegisterPeer` frame carries it. Structural checks are: the claimed
    /// sender is the authenticated peer, the role is a `HuddleControl` session,
    /// and this pod is the header's named owner. The first *stateful* operation
    /// (`RegisterPeer`) is where the Redis fence runs, before `room.add_peer` —
    /// see [`Self::serve_control_loop`]. Matches the dispatcher callback shape
    /// `(from, hello, stream)`; no `community_id` param — community rides the
    /// wire and is self-verified by the fence.
    ///
    /// The owner-loss fan-out is read from the [`HuddleOwnerRegistry`] keyed by
    /// the stream's `session_id`: if this pod holds a live owner lease for the
    /// room the loop selects on that renewer's `lost` and emits a proactive
    /// [`GoodbyeReason::StaleGeneration`] before teardown when the lease is
    /// lost, so non-owner pods rejoin against the new owner. If no live entry
    /// exists (the room emptied and released, or the entry has not landed yet)
    /// the loop degenerates to recv-only, exactly the prior behavior.
    pub async fn accept_inbound(
        &self,
        from: RuntimeId,
        hello: StreamHello,
        stream: MeshStream,
    ) -> Result<(), MeshError> {
        if hello.sender != from {
            return Err(MeshError::Transport(format!(
                "huddle-control hello.sender {} != authenticated peer {from}",
                hello.sender
            )));
        }
        let StreamRole::Session { fenced, profile } = hello.role else {
            return Err(MeshError::Transport(
                "huddle-control stream Hello was not a session role".into(),
            ));
        };
        if profile != Profile::HuddleControl {
            return Err(MeshError::Transport(format!(
                "huddle-control acceptor got profile {profile:?}"
            )));
        }
        // Structural owner check: reject an obviously-misrouted stream cheaply,
        // before serving any frame. This is not the fence — the authoritative
        // Redis re-validation happens per control frame in the loop, keyed by
        // the community the frame carries.
        if fenced.owner_runtime_id != self.local_runtime_id {
            return Err(MeshError::OwnerMismatch {
                session_id: fenced.session_id,
                generation: fenced.generation,
                frame_owner_runtime_id: fenced.owner_runtime_id,
                current_owner_runtime_id: self.local_runtime_id,
            });
        }

        let lost = self.owners.lost_for(fenced.session_id);
        let draining = self.owners.drain_for(fenced.session_id);
        self.serve_control_loop(from, fenced, stream, lost, draining)
            .await
    }

    /// Serve register/unregister frames for one non-owner pod's stream.
    ///
    /// The community is learned from the first `RegisterPeer` frame and latched
    /// for the life of the stream — a stream is one non-owner pod's view of one
    /// huddle, so exactly one community applies. Later frames naming a different
    /// community are rejected. The Redis fence runs on `RegisterPeer` *before*
    /// `room.add_peer` (validate-before-admit): a frame asserting the wrong
    /// community keys a lease that does not exist and is rejected before any
    /// state mutation. `UnregisterPeer` only removes entries from this stream's
    /// own `registered` map, so it needs no fence — it cannot mutate another
    /// community's room.
    ///
    /// Peers this stream registers are tracked so a stream close (the non-owner
    /// pod went away) tears them all down — no leaked remote peers holding
    /// index slots in the owner's room.
    /// `lost`, when `Some`, is an owner-loss signal (from the huddle-lease
    /// renewer): if it fires the loop stops and sends a proactive
    /// [`GoodbyeReason::StaleGeneration`] before teardown, so the non-owner pod
    /// rejoins against the new owner instead of waiting for the stream to fault.
    /// `None` disables the arm and preserves the recv-only behavior.
    async fn serve_control_loop(
        &self,
        from: RuntimeId,
        fenced: FencedHeader,
        mut stream: MeshStream,
        lost: Option<CancellationToken>,
        draining: Option<CancellationToken>,
    ) -> Result<(), MeshError> {
        let session_id = fenced.session_id;
        // pubkey -> peer_id, for UnregisterPeer and teardown on stream close.
        let mut registered: std::collections::HashMap<String, Uuid> =
            std::collections::HashMap::new();
        // Community (raw UUID) latched from the first RegisterPeer; every later
        // frame must agree. `None` until the first register arrives.
        let mut stream_community: Option<Uuid> = None;
        let mut roster_rx: Option<tokio::sync::broadcast::Receiver<RoomRosterDelta>> = None;

        // Owner teardown latch: set when `lost`/`draining` fires so teardown
        // sends the matching proactive Goodbye. A stream faulting on its own
        // leaves this empty and the close stays silent, as before.
        let mut teardown_reason: Option<GoodbyeReason> = None;

        let result = loop {
            // A future that never resolves when there is no loss signal, so the
            // `select!` degenerates to a plain recv for the `None` case.
            let lost_fired = async {
                match &lost {
                    Some(token) => token.cancelled().await,
                    None => std::future::pending().await,
                }
            };
            let drain_fired = async {
                match &draining {
                    Some(token) => token.cancelled().await,
                    None => std::future::pending().await,
                }
            };
            let roster_event = async {
                match &mut roster_rx {
                    Some(rx) => Some(rx.recv().await),
                    None => std::future::pending().await,
                }
            };
            let frame = tokio::select! {
                _ = drain_fired => {
                    teardown_reason = Some(GoodbyeReason::Draining);
                    break Ok(());
                }
                _ = lost_fired => {
                    teardown_reason = Some(GoodbyeReason::StaleGeneration);
                    break Ok(());
                }
                event = roster_event => {
                    let Some(event) = event else {
                        continue;
                    };
                    let msg = match event {
                        Ok(delta) => roster_delta_msg(delta),
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            let Some(community_id) = stream_community else {
                                break Ok(());
                            };
                            let Some(room) = self.rooms.get(
                                CommunityId::from_uuid(community_id),
                                session_id,
                            ) else {
                                break Ok(());
                            };
                            roster_snapshot_msg(&room)
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break Ok(()),
                    };
                    stream.send_frame(MeshStreamFrame::Data {
                        fenced,
                        payload: encode_control(&msg)?,
                    }).await?;
                    continue;
                }
                frame = stream.recv_frame() => frame,
            };
            let msg = match frame {
                Ok(Some(MeshStreamFrame::Data { fenced: f, payload })) => {
                    // Every control frame must carry the same fenced header the
                    // Hello did: a lease that moves mid-stream (owner or
                    // generation change) rejects subsequent frames.
                    if f != fenced {
                        break Err(MeshError::OwnerMismatch {
                            session_id,
                            generation: f.generation,
                            frame_owner_runtime_id: f.owner_runtime_id,
                            current_owner_runtime_id: self.local_runtime_id,
                        });
                    }
                    match decode_control(&payload) {
                        Ok(m) => m,
                        Err(e) => break Err(e),
                    }
                }
                Ok(Some(MeshStreamFrame::Goodbye { .. })) | Ok(None) => break Ok(()),
                Ok(Some(other)) => {
                    break Err(MeshError::Transport(format!(
                        "huddle-control stream got unexpected frame {other:?}"
                    )));
                }
                Err(e) => break Err(e),
            };

            match msg {
                HuddleControlMsg::RegisterPeer {
                    community_id,
                    pubkey,
                    protocol_version,
                } => {
                    // Latch the community on first receipt; reject any later
                    // frame that names a different one (tenant-boundary guard).
                    match stream_community {
                        None => stream_community = Some(community_id),
                        Some(latched) if latched != community_id => {
                            break Err(MeshError::Transport(format!(
                                "huddle-control stream community changed {latched} -> {community_id}"
                            )));
                        }
                        Some(_) => {}
                    }
                    // Reconstitute the server-trusted `CommunityId` from the wire
                    // UUID — explicit and localized, honoring `CommunityId`'s
                    // no-client-input invariant — then fence.
                    let community = CommunityId::from_uuid(community_id);
                    // Validate-before-admit: the Redis fence keyed by the
                    // asserted community must pass before any room mutation. A
                    // wrong community keys a lease the owner never wrote → a
                    // typed fence rejection, so no peer is admitted and the
                    // client sees the same taxonomy a same-pod join would. A
                    // *non-fence* validate error (Redis unreachable, decode) is
                    // not a clean rejection — it tears the stream down.
                    if self.owners.is_draining() {
                        teardown_reason = Some(GoodbyeReason::Draining);
                        break Ok(());
                    }
                    let room = self.rooms.get_or_create(community, session_id);
                    // Subscribe before admission; PeerRegistered carries a
                    // snapshot after admission, and queued deltas at or below
                    // that revision are ignored by the receiver.
                    let new_roster_rx = room.subscribe_roster();
                    let reply = match self.directory.validate(community, &fenced).await {
                        Ok(()) => self.register_remote_peer(
                            Arc::clone(&room),
                            fenced,
                            from,
                            &pubkey,
                            protocol_version,
                            &mut registered,
                        ),
                        Err(e) => match FenceRejection::from_mesh_error(&e) {
                            Some(reason) => HuddleControlMsg::RegisterRejected {
                                pubkey: pubkey.clone(),
                                reason: RegisterRejection::Fenced(reason),
                            },
                            None => break Err(e),
                        },
                    };
                    if let Err(e) = stream
                        .send_frame(MeshStreamFrame::Data {
                            fenced,
                            payload: encode_control(&reply)?,
                        })
                        .await
                    {
                        break Err(e);
                    }
                    if matches!(reply, HuddleControlMsg::PeerRegistered { .. }) {
                        roster_rx = Some(new_roster_rx);
                    }
                }
                HuddleControlMsg::UnregisterPeer { pubkey } => {
                    if let Some(peer_id) = registered.remove(&pubkey) {
                        if let Some(room) = stream_community.and_then(|community_id| {
                            self.rooms
                                .get(CommunityId::from_uuid(community_id), session_id)
                        }) {
                            let peer_index = room.peers.get(&peer_id).map(|peer| peer.peer_index);
                            room.remove_peer(peer_id);
                            if let Some(peer_index) = peer_index {
                                room.broadcast_control(
                                    serde_json::json!({
                                        "type": "left",
                                        "pubkey": pubkey,
                                        "peer_index": peer_index,
                                    })
                                    .to_string(),
                                );
                            }
                        }
                    }
                }
                HuddleControlMsg::RosterResync => {
                    let Some(room) = stream_community.and_then(|community_id| {
                        self.rooms
                            .get(CommunityId::from_uuid(community_id), session_id)
                    }) else {
                        break Ok(());
                    };
                    stream
                        .send_frame(MeshStreamFrame::Data {
                            fenced,
                            payload: encode_control(&roster_snapshot_msg(&room))?,
                        })
                        .await?;
                }
                // Owner→non-owner replies never arrive on the owner's accept
                // side; a peer sending one is a protocol violation.
                HuddleControlMsg::PeerRegistered { .. }
                | HuddleControlMsg::RosterSnapshot { .. }
                | HuddleControlMsg::RosterDelta { .. }
                | HuddleControlMsg::RegisterRejected { .. } => {
                    break Err(MeshError::Transport(
                        "huddle-control owner received an owner→non-owner reply".into(),
                    ));
                }
            }
        };

        // Owner-initiated teardown: tell the non-owner pod why this owner is
        // closing so it can rejoin against Redis. Best-effort — teardown
        // proceeds even if the stream is already gone. Normal stream/client
        // closes stay silent.
        if let Some(reason) = teardown_reason {
            let _ = stream
                .send_frame(MeshStreamFrame::Goodbye { fenced, reason })
                .await;
        }

        // Teardown: drop every peer this stream registered, regardless of how
        // the loop ended. Dropping the peer drops its `audio_tx`, which ends the
        // matching `spawn_remote_peer_sink` task.
        if let Some(room) = stream_community.and_then(|community_id| {
            self.rooms
                .get(CommunityId::from_uuid(community_id), session_id)
        }) {
            for (pubkey, peer_id) in registered {
                let peer_index = room.peers.get(&peer_id).map(|peer| peer.peer_index);
                room.remove_peer(peer_id);
                if let Some(peer_index) = peer_index {
                    room.broadcast_control(
                        serde_json::json!({
                            "type": "left",
                            "pubkey": pubkey,
                            "peer_index": peer_index,
                        })
                        .to_string(),
                    );
                }
            }
        }
        result
    }

    /// Admit one remote client into the owner's room and wire its fan-out back
    /// to the registering pod as datagrams. Returns the reply to send.
    fn register_remote_peer(
        &self,
        room: Arc<Room>,
        fenced: FencedHeader,
        from: RuntimeId,
        pubkey: &str,
        protocol_version: u8,
        registered: &mut std::collections::HashMap<String, Uuid>,
    ) -> HuddleControlMsg {
        match room.add_peer(pubkey.to_string(), protocol_version) {
            Ok((peer_id, peer_index, audio_rx, _peer_ctrl_rx)) => {
                registered.insert(pubkey.to_string(), peer_id);
                // The owner's Room fans out to this remote peer's `audio_tx`;
                // the sink drains `audio_rx` and ships each frame as a datagram
                // to the pod that hosts the client.
                spawn_remote_peer_sink(Arc::clone(&self.transport), from, fenced, audio_rx);
                let joined = serde_json::json!({
                    "type": "joined",
                    "pubkey": pubkey,
                    "peer_index": peer_index,
                    "peers": [{"pubkey": pubkey, "peer_index": peer_index}],
                })
                .to_string();
                room.broadcast_control(joined);
                HuddleControlMsg::PeerRegistered {
                    pubkey: pubkey.to_string(),
                    peer_index,
                    roster: roster_snapshot(&room),
                }
            }
            Err(reason) => HuddleControlMsg::RegisterRejected {
                pubkey: pubkey.to_string(),
                reason: admission_to_rejection(reason),
            },
        }
    }
}

fn roster_snapshot(room: &Room) -> RosterSnapshot {
    let snapshot = room.roster_snapshot();
    RosterSnapshot {
        revision: snapshot.revision,
        peers: snapshot.peers.into_iter().map(Into::into).collect(),
    }
}

fn roster_snapshot_msg(room: &Room) -> HuddleControlMsg {
    let snapshot = roster_snapshot(room);
    HuddleControlMsg::RosterSnapshot {
        revision: snapshot.revision,
        peers: snapshot.peers,
    }
}

fn roster_delta_msg(delta: RoomRosterDelta) -> HuddleControlMsg {
    HuddleControlMsg::RosterDelta {
        revision: delta.revision,
        joined: delta.joined.map(Into::into),
        left: delta.left.map(Into::into),
    }
}

/// Map a room admission failure to the wire rejection taxonomy. Kept 1:1 with
/// the single-pod WS error codes so a cross-pod join surfaces the same
/// client-facing error a same-pod join would.
fn admission_to_rejection(err: AdmissionError) -> RegisterRejection {
    match err {
        AdmissionError::Full => RegisterRejection::RoomFull,
        AdmissionError::Ended => RegisterRejection::RoomEnded,
        AdmissionError::VersionMismatch { pinned, requested } => {
            RegisterRejection::VersionMismatch { pinned, requested }
        }
    }
}

/// The `Goodbye` reason a non-owner sends when its client leaves the huddle
/// cleanly. Re-exported so the handler's dial path uses one spelling.
pub const HUDDLE_SESSION_ENDED: GoodbyeReason = GoodbyeReason::SessionEnded;

// ── Non-owner-side HuddleControl dial path ───────────────────────────────────
//
// A client connected here whose huddle is owned by another pod. We keep the
// client as an ordinary local WS peer (heartbeats, `joined`/`left`, delivery of
// the owner's fan-out) but there is NO local fan-out: the owner is the sole
// fan-out authority. Each client Opus frame is shipped to the owner as a
// datagram tagged with the OWNER-assigned peer index; the owner fans out to
// everyone (including this pod's co-located clients, which hear each other via
// the owner round-trip — `deliver_prefixed` skips a client's own index so it
// never hears itself).

/// A registered cross-pod huddle session on the non-owner side.
///
/// Holds everything needed to forward the local client's media to the owner and
/// to unregister cleanly on disconnect. Media delivery *back* to the client goes
/// through the ordinary local room via `MeshAudioRouter::on_media_datagram`, so
/// this handle owns only the outbound (client→owner) half plus teardown.
pub struct RemoteHuddleSession {
    /// The owner-allocated peer index this client occupies in the owner's room.
    /// Stamped on every media datagram so the owner attributes frames correctly.
    peer_index: u8,
    /// Latest complete authoritative owner roster.
    roster: RosterSnapshot,
    /// Fenced header for this session's owner epoch; every datagram carries it.
    fenced: FencedHeader,
    /// The pod that owns the huddle.
    owner: RuntimeId,
    /// Pubkey of the local client, for the closing `UnregisterPeer`.
    pubkey: String,
    /// Transport for datagrams and the control-stream teardown.
    transport: Arc<dyn RelayPeerTransport>,
    /// Per-datagram monotonic sequence for loss/reorder observability.
    seq: u64,
}

/// Why a non-owner pod is tearing down a client's cross-pod huddle session.
///
/// Read off the owner's `HuddleControl` stream: the owner speaks its intent as
/// a [`MeshStreamFrame::Goodbye`] reason, or the stream simply ends. The
/// non-owner maps that cause to a client-facing outcome — every cause tears the
/// local client down and closes its WS so it can rejoin (against a fresh owner
/// or a fresh generation); the distinction is observability, not divergent
/// behaviour. There is no `Lost` wire variant: owner loss surfaces as
/// [`GoodbyeReason::StaleGeneration`] (the owner fenced itself out) or, if it
/// died mid-flight, as a bare stream close.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HuddleTeardownCause {
    /// Owner sent `Goodbye(StaleGeneration)` — it observed a newer generation
    /// and fenced itself out. From the client's view: the owner was lost; a
    /// rejoin will resolve the new owner via Redis.
    OwnerLost,
    /// Owner sent `Goodbye(Draining)` — it is shutting down (SIGTERM). A rejoin
    /// re-establishes against whichever pod takes the lease next.
    OwnerDraining,
    /// Owner sent `Goodbye(SessionEnded)` — the session ended normally
    /// (e.g. the room emptied on the owner). An ordinary disconnect.
    SessionEnded,
    /// The stream closed or reset with no `Goodbye` — the owner pod died or the
    /// link broke mid-flight. Treated like owner loss: rejoin to recover.
    StreamClosed,
}

impl HuddleTeardownCause {
    fn from_goodbye(reason: GoodbyeReason) -> Self {
        match reason {
            GoodbyeReason::StaleGeneration => Self::OwnerLost,
            GoodbyeReason::Draining => Self::OwnerDraining,
            GoodbyeReason::SessionEnded => Self::SessionEnded,
        }
    }
}

/// Forward authoritative roster control to one ingress client until teardown.
/// Revision gaps trigger a snapshot resync request before any newer delta is applied.
pub async fn read_owner_control(
    stream: &mut MeshStream,
    fenced: FencedHeader,
    mut revision: u64,
    ctrl_tx: &tokio::sync::mpsc::Sender<axum::extract::ws::Message>,
) -> HuddleTeardownCause {
    loop {
        match stream.recv_frame().await {
            Ok(Some(MeshStreamFrame::Goodbye { reason, .. })) => {
                return HuddleTeardownCause::from_goodbye(reason);
            }
            Ok(Some(MeshStreamFrame::Data { payload, .. })) => match decode_control(&payload) {
                Ok(HuddleControlMsg::RosterSnapshot {
                    revision: next,
                    peers,
                }) => {
                    revision = next;
                    let json = serde_json::json!({
                        "type": "roster", "revision": revision,
                        "peers": peers.into_iter().map(|p| serde_json::json!({
                            "pubkey": p.pubkey, "peer_index": p.peer_index,
                        })).collect::<Vec<_>>()
                    })
                    .to_string();
                    if ctrl_tx
                        .send(axum::extract::ws::Message::Text(json.into()))
                        .await
                        .is_err()
                    {
                        return HuddleTeardownCause::SessionEnded;
                    }
                }
                Ok(HuddleControlMsg::RosterDelta {
                    revision: next,
                    joined,
                    left,
                }) if next == revision.wrapping_add(1) => {
                    revision = next;
                    let json = if let Some(peer) = joined {
                        serde_json::json!({
                            "type": "joined", "revision": revision,
                            "pubkey": peer.pubkey, "peer_index": peer.peer_index,
                            "peers": [{"pubkey": peer.pubkey, "peer_index": peer.peer_index}],
                        })
                    } else if let Some(peer) = left {
                        serde_json::json!({
                            "type": "left", "revision": revision,
                            "pubkey": peer.pubkey, "peer_index": peer.peer_index,
                        })
                    } else {
                        continue;
                    };
                    if ctrl_tx
                        .send(axum::extract::ws::Message::Text(json.to_string().into()))
                        .await
                        .is_err()
                    {
                        return HuddleTeardownCause::SessionEnded;
                    }
                }
                Ok(HuddleControlMsg::RosterDelta { revision: next, .. }) if next <= revision => {}
                Ok(HuddleControlMsg::RosterDelta { .. }) => {
                    let payload = match encode_control(&HuddleControlMsg::RosterResync) {
                        Ok(payload) => payload,
                        Err(_) => return HuddleTeardownCause::StreamClosed,
                    };
                    if stream
                        .send_frame(MeshStreamFrame::Data { fenced, payload })
                        .await
                        .is_err()
                    {
                        return HuddleTeardownCause::StreamClosed;
                    }
                }
                Ok(_) => {}
                Err(e) => debug!(owner_stream_error = %e, "invalid huddle owner control"),
            },
            Ok(Some(_)) => continue,
            Ok(None) => return HuddleTeardownCause::StreamClosed,
            Err(e) => {
                debug!(owner_stream_error = %e, "huddle owner stream ended abnormally");
                return HuddleTeardownCause::StreamClosed;
            }
        }
    }
}

/// Read the owner's `HuddleControl` stream until it signals teardown.
///
/// The non-owner side's stream carries only owner→client control today
/// (`PeerRegistered`/`RegisterRejected` are consumed during the dial). Any
/// further `Data`/`Gossip` frame is non-terminal and skipped — this is the
/// forward-compatible seam for owner→client roster deltas, which must not be
/// mistaken for teardown. The loop returns exactly once, on the first terminal
/// signal: a [`MeshStreamFrame::Goodbye`], or a clean close / transport error
/// (both [`HuddleTeardownCause::StreamClosed`]).
pub async fn read_teardown_cause(stream: &mut MeshStream) -> HuddleTeardownCause {
    loop {
        match stream.recv_frame().await {
            Ok(Some(MeshStreamFrame::Goodbye { reason, .. })) => {
                return HuddleTeardownCause::from_goodbye(reason);
            }
            // Non-terminal owner→client traffic (future roster deltas): ignore
            // and keep reading. A stray Hello here would be a protocol error,
            // but the transport already validated the opening Hello, so treat
            // any non-Goodbye frame as non-terminal rather than tearing down.
            Ok(Some(_)) => continue,
            // Clean close (None) or transport error: the owner is gone.
            Ok(None) => return HuddleTeardownCause::StreamClosed,
            Err(e) => {
                debug!(owner_stream_error = %e, "huddle owner stream ended abnormally");
                return HuddleTeardownCause::StreamClosed;
            }
        }
    }
}

/// Why a cross-pod join could not complete on the non-owner side.
#[derive(Debug)]
pub enum DialError {
    /// The owner refused the registration; surfaced to the client as the same
    /// WS error a same-pod join would produce, never a silent media drop.
    Rejected(RegisterRejection),
    /// Transport / protocol failure opening or serving the control stream.
    Mesh(MeshError),
}

impl From<MeshError> for DialError {
    fn from(e: MeshError) -> Self {
        DialError::Mesh(e)
    }
}

/// Open a `HuddleControl` stream to the owner and register the local client.
///
/// On success the owner has admitted the client as a remote peer and returned
/// its owner-assigned index; the returned [`RemoteHuddleSession`] forwards media
/// and unregisters on drop. On [`DialError::Rejected`] the caller surfaces the
/// owner's admission failure to the client unchanged.
pub async fn dial_remote_owner(
    transport: Arc<dyn RelayPeerTransport>,
    local_runtime_id: RuntimeId,
    owner: RuntimeId,
    fenced: FencedHeader,
    community_id: CommunityId,
    pubkey: String,
    protocol_version: u8,
) -> Result<(RemoteHuddleSession, MeshStream), DialError> {
    let hello = StreamHello {
        sender: local_runtime_id,
        role: StreamRole::Session {
            fenced,
            profile: Profile::HuddleControl,
        },
    };
    // `open_session_stream` sends the Hello before returning.
    let mut stream = transport.open_session_stream(owner, hello).await?;

    stream
        .send_frame(MeshStreamFrame::Data {
            fenced,
            payload: encode_control(&HuddleControlMsg::RegisterPeer {
                community_id: *community_id.as_uuid(),
                pubkey: pubkey.clone(),
                protocol_version,
            })?,
        })
        .await?;

    match stream.recv_frame().await? {
        Some(MeshStreamFrame::Data { payload, .. }) => match decode_control(&payload)? {
            HuddleControlMsg::PeerRegistered {
                peer_index, roster, ..
            } => Ok((
                RemoteHuddleSession {
                    peer_index,
                    roster,
                    fenced,
                    owner,
                    pubkey,
                    transport,
                    seq: 0,
                },
                stream,
            )),
            HuddleControlMsg::RegisterRejected { reason, .. } => Err(DialError::Rejected(reason)),
            other => Err(DialError::Mesh(MeshError::Transport(format!(
                "expected PeerRegistered/RegisterRejected, got {other:?}"
            )))),
        },
        Some(MeshStreamFrame::Goodbye { .. }) | None => Err(DialError::Mesh(MeshError::Transport(
            "owner closed HuddleControl stream before replying".into(),
        ))),
        Some(other) => Err(DialError::Mesh(MeshError::Transport(format!(
            "unexpected HuddleControl frame from owner: {other:?}"
        )))),
    }
}

/// The `StreamHello.sender` for a dialed session: the fenced header carries the
/// owner's identity, but the *sender* is this pod. The owner validates
/// `hello.sender == authenticated peer`, so it must be our own runtime id — the
/// handler threads `local_runtime_id` in explicitly.
impl RemoteHuddleSession {
    /// The owner-assigned index this client occupies in the owner's room.
    pub fn peer_index(&self) -> u8 {
        self.peer_index
    }

    /// Complete authoritative roster returned atomically with registration.
    pub fn roster(&self) -> &RosterSnapshot {
        &self.roster
    }

    /// The session fence — used by the reader task to author the closing
    /// `UnregisterPeer` / `Goodbye` on the owner's control stream.
    pub fn fenced(&self) -> FencedHeader {
        self.fenced
    }

    /// The local client's pubkey — used by the reader task's `UnregisterPeer`.
    pub fn pubkey(&self) -> &str {
        &self.pubkey
    }

    /// Forward one client Opus frame to the owner as a media datagram, tagged
    /// with the owner-assigned index. Drop-on-error: realtime audio never blocks
    /// on a slow or gone link (the same discipline as local fan-out).
    pub fn forward_media(&mut self, client_frame: &[u8]) {
        let dgram = media_datagram(self.peer_index, self.fenced, self.seq, client_frame);
        self.seq = self.seq.wrapping_add(1);
        if let Err(e) = self.transport.send_datagram(self.owner, dgram) {
            debug!(owner = %self.owner, "huddle media datagram to owner failed: {e}");
        }
    }
}

/// Unregister the client from the owner and close the control stream cleanly.
///
/// Called on a *local-client-initiated* disconnect (the reader task's cancel
/// branch), so the owner drops the remote peer and stops fanning media back.
/// Best-effort: teardown never blocks connection cleanup, and a `Goodbye`
/// already received from the owner makes this a no-op the owner ignores.
pub async fn send_clean_close(stream: &mut MeshStream, fenced: FencedHeader, pubkey: &str) {
    if let Ok(payload) = encode_control(&HuddleControlMsg::UnregisterPeer {
        pubkey: pubkey.to_string(),
    }) {
        let _ = stream
            .send_frame(MeshStreamFrame::Data { fenced, payload })
            .await;
    }
    let _ = stream
        .send_frame(MeshStreamFrame::Goodbye {
            fenced,
            reason: HUDDLE_SESSION_ENDED,
        })
        .await;
    let _ = stream.finish();
}

/// Build the media datagram a non-owner ships to the owner for one client
/// frame: `[owner_peer_index][client frame]`, stamped with the session fence
/// and sequence. Pure so the framing is unit-testable without a live transport
/// or stream.
fn media_datagram(
    peer_index: u8,
    fenced: FencedHeader,
    seq: u64,
    client_frame: &[u8],
) -> MeshDatagram {
    let mut payload = Vec::with_capacity(1 + client_frame.len());
    payload.push(peer_index);
    payload.extend_from_slice(client_frame);
    MeshDatagram {
        fenced,
        seq,
        payload,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn rt(b: u8) -> RuntimeId {
        RuntimeId([b; 32])
    }

    fn community() -> CommunityId {
        CommunityId::from_uuid(Uuid::from_u128(0xC0FFEE))
    }

    /// Scripted directory: `owner_of` returns a queued lookup, `acquire`
    /// returns a queued outcome, `validate` returns a queued result. Records
    /// call counts so ordering can be asserted.
    #[derive(Default)]
    struct FakeDir {
        owner: Mutex<Option<Ownership>>,
        acquire: Mutex<Option<AcquireOutcome>>,
        validate_fails: Mutex<bool>,
        acquire_calls: Mutex<u32>,
        validate_calls: Mutex<u32>,
        // Renewer scripting: each `renew` pops the next outcome; an empty queue
        // yields `Renewed` (lease holds). `release` returns the scripted value.
        renew_outcomes: Mutex<std::collections::VecDeque<HuddleRenewOutcome>>,
        release_outcome: Mutex<Option<HuddleReleaseOutcome>>,
        renew_calls: Mutex<u32>,
        release_calls: Mutex<u32>,
    }

    impl FakeDir {
        fn owned_by(o: Ownership) -> Self {
            let d = Self::default();
            *d.owner.lock().unwrap() = Some(o);
            d
        }
        fn unowned_then_acquire(a: AcquireOutcome) -> Self {
            let d = Self::default();
            *d.acquire.lock().unwrap() = Some(a);
            d
        }
        /// Renewer-lane double: `renew` yields the scripted outcomes in order
        /// (then holds), `release` yields `release`.
        fn with_renew_script(
            renews: impl IntoIterator<Item = HuddleRenewOutcome>,
            release: HuddleReleaseOutcome,
        ) -> Self {
            let d = Self::default();
            *d.renew_outcomes.lock().unwrap() = renews.into_iter().collect();
            *d.release_outcome.lock().unwrap() = Some(release);
            d
        }
    }

    #[async_trait::async_trait]
    impl HuddleDirectory for FakeDir {
        async fn owner_of(
            &self,
            _c: CommunityId,
            _s: Uuid,
        ) -> Result<Option<Ownership>, MeshError> {
            Ok(*self.owner.lock().unwrap())
        }
        async fn acquire(
            &self,
            _c: CommunityId,
            _s: Uuid,
            _owner: RuntimeId,
        ) -> Result<AcquireOutcome, MeshError> {
            *self.acquire_calls.lock().unwrap() += 1;
            Ok(self
                .acquire
                .lock()
                .unwrap()
                .clone()
                .expect("acquire not scripted"))
        }
        async fn validate(&self, _c: CommunityId, _f: &FencedHeader) -> Result<(), MeshError> {
            *self.validate_calls.lock().unwrap() += 1;
            if *self.validate_fails.lock().unwrap() {
                Err(MeshError::OwnerMismatch {
                    session_id: Uuid::nil(),
                    generation: 0,
                    frame_owner_runtime_id: rt(0),
                    current_owner_runtime_id: rt(1),
                })
            } else {
                Ok(())
            }
        }
        async fn renew(&self, _lease: &HuddleLease) -> Result<HuddleRenewOutcome, MeshError> {
            *self.renew_calls.lock().unwrap() += 1;
            Ok(self
                .renew_outcomes
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(HuddleRenewOutcome::Renewed(test_lease())))
        }
        async fn release(&self, _lease: &HuddleLease) -> Result<HuddleReleaseOutcome, MeshError> {
            *self.release_calls.lock().unwrap() += 1;
            Ok(self
                .release_outcome
                .lock()
                .unwrap()
                .clone()
                .unwrap_or(HuddleReleaseOutcome::Released))
        }
    }

    /// A `HuddleLease` for renewer tests: the inner `SessionLease` is opaque to
    /// the huddle lane, so any well-formed fenced tuple works.
    fn test_lease() -> HuddleLease {
        lease_for(Uuid::from_u128(0xFEED), 7)
    }

    /// A `HuddleLease` for a specific session and generation — the registry
    /// tests key on `session_id` and fence `release` on `generation`, so they
    /// need to vary both.
    fn lease_for(session_id: Uuid, generation: u64) -> HuddleLease {
        HuddleLease(SessionLease {
            community_id: community(),
            session_id,
            owner_runtime_id: rt(1),
            generation,
            profile: HUDDLE_CONTROL_PROFILE,
        })
    }

    #[tokio::test]
    async fn unowned_huddle_is_acquired_as_local_owner() {
        let dir =
            FakeDir::unowned_then_acquire(AcquireOutcome::Acquired(HuddleLease(SessionLease {
                community_id: community(),
                session_id: Uuid::from_u128(0xFEED),
                owner_runtime_id: rt(1),
                generation: 7,
                profile: HUDDLE_CONTROL_PROFILE,
            })));
        let out = resolve_join(&dir, community(), Uuid::new_v4(), rt(1))
            .await
            .unwrap();
        assert_eq!(out.outcome, JoinOutcome::LocalOwner { generation: 7 });
        // Freshly acquired → the real lease is surfaced for the renewer.
        assert_eq!(out.acquired.as_ref().map(HuddleLease::generation), Some(7));
        assert_eq!(*dir.acquire_calls.lock().unwrap(), 1);
        // No fence validation on the local-owner path — we ARE the lease.
        assert_eq!(*dir.validate_calls.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn huddle_owned_by_us_is_local_owner_without_acquire() {
        let dir = FakeDir::owned_by(Ownership {
            owner_runtime_id: rt(1),
            generation: 3,
        });
        let out = resolve_join(&dir, community(), Uuid::new_v4(), rt(1))
            .await
            .unwrap();
        assert_eq!(out.outcome, JoinOutcome::LocalOwner { generation: 3 });
        // Steady-state owner reuses the registry renewer — no fresh lease.
        assert!(out.acquired.is_none());
        // Live lease found → no acquire attempt.
        assert_eq!(*dir.acquire_calls.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn huddle_owned_by_peer_is_remote_owner_and_fence_validated() {
        let dir = FakeDir::owned_by(Ownership {
            owner_runtime_id: rt(2),
            generation: 9,
        });
        let out = resolve_join(&dir, community(), Uuid::new_v4(), rt(1))
            .await
            .unwrap();
        assert_eq!(
            out.outcome,
            JoinOutcome::RemoteOwner {
                owner_runtime_id: rt(2),
                generation: 9,
            }
        );
        assert!(out.acquired.is_none());
        // The remote-owner path validates the fence before routing.
        assert_eq!(*dir.validate_calls.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn lost_acquire_race_routes_to_winner_as_remote_owner() {
        let dir = FakeDir::unowned_then_acquire(AcquireOutcome::Held(Ownership {
            owner_runtime_id: rt(2),
            generation: 4,
        }));
        let out = resolve_join(&dir, community(), Uuid::new_v4(), rt(1))
            .await
            .unwrap();
        assert_eq!(
            out.outcome,
            JoinOutcome::RemoteOwner {
                owner_runtime_id: rt(2),
                generation: 4,
            }
        );
        assert!(out.acquired.is_none());
        assert_eq!(*dir.acquire_calls.lock().unwrap(), 1);
        assert_eq!(*dir.validate_calls.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn remote_owner_fence_rejection_propagates() {
        let dir = FakeDir::owned_by(Ownership {
            owner_runtime_id: rt(2),
            generation: 9,
        });
        *dir.validate_fails.lock().unwrap() = true;
        let err = resolve_join(&dir, community(), Uuid::new_v4(), rt(1))
            .await
            .unwrap_err();
        assert!(matches!(err, MeshError::OwnerMismatch { .. }));
    }

    #[test]
    fn control_msg_roundtrips() {
        for msg in [
            HuddleControlMsg::RegisterPeer {
                community_id: *community().as_uuid(),
                pubkey: "abc123".into(),
                protocol_version: 2,
            },
            HuddleControlMsg::PeerRegistered {
                pubkey: "abc123".into(),
                peer_index: 42,
                roster: RosterSnapshot {
                    revision: 1,
                    peers: vec![RosterEntry {
                        pubkey: "abc123".into(),
                        peer_index: 42,
                    }],
                },
            },
            HuddleControlMsg::RosterDelta {
                revision: 2,
                joined: None,
                left: Some(RosterEntry {
                    pubkey: "abc123".into(),
                    peer_index: 42,
                }),
            },
            HuddleControlMsg::RosterResync,
            HuddleControlMsg::RegisterRejected {
                pubkey: "abc123".into(),
                reason: RegisterRejection::VersionMismatch {
                    pinned: 2,
                    requested: 1,
                },
            },
            HuddleControlMsg::RegisterRejected {
                pubkey: "abc123".into(),
                reason: RegisterRejection::Fenced(FenceRejection::StaleGeneration),
            },
            HuddleControlMsg::UnregisterPeer {
                pubkey: "abc123".into(),
            },
        ] {
            let bytes = encode_control(&msg).unwrap();
            assert_eq!(decode_control(&bytes).unwrap(), msg);
        }
    }

    // ── In-memory MeshStream pair for handshake round-trip tests ─────────────
    //
    // A channel-backed `StreamSendHalf`/`StreamRecvHalf` pair drives
    // `accept_inbound` end-to-end without iroh: what the owner side sends, the
    // client side receives, and vice versa. Uses only the public
    // `MeshStream::new` seam plus the public half traits.
    use buzz_relay_mesh::{BoxFuture, StreamRecvHalf, StreamSendHalf};
    use tokio::sync::mpsc as tmpsc;

    struct ChanSend(tmpsc::UnboundedSender<MeshStreamFrame>);
    struct ChanRecv(tmpsc::UnboundedReceiver<MeshStreamFrame>);

    impl StreamSendHalf for ChanSend {
        fn send_frame(&mut self, frame: MeshStreamFrame) -> BoxFuture<'_, Result<(), MeshError>> {
            let r = self
                .0
                .send(frame)
                .map_err(|_| MeshError::Transport("peer closed".into()));
            Box::pin(async move { r })
        }
        fn finish(&mut self) -> Result<(), MeshError> {
            Ok(())
        }
    }

    impl StreamRecvHalf for ChanRecv {
        fn recv_frame(&mut self) -> BoxFuture<'_, Result<Option<MeshStreamFrame>, MeshError>> {
            Box::pin(async move { Ok(self.0.recv().await) })
        }
    }

    /// A connected `(owner_side, client_side)` `MeshStream` pair.
    fn stream_pair() -> (MeshStream, MeshStream) {
        let (a_tx, a_rx) = tmpsc::unbounded_channel();
        let (b_tx, b_rx) = tmpsc::unbounded_channel();
        // owner sends on a_tx (client reads a_rx); client sends on b_tx (owner
        // reads b_rx).
        let owner = MeshStream::new(Box::new(ChanSend(a_tx)), Box::new(ChanRecv(b_rx)));
        let client = MeshStream::new(Box::new(ChanSend(b_tx)), Box::new(ChanRecv(a_rx)));
        (owner, client)
    }

    #[tokio::test]
    async fn roster_revision_gap_requests_resync_before_forwarding_new_state() {
        let session_id = Uuid::new_v4();
        let fenced = fenced_owned_by(rt(1), session_id);
        let (mut owner, mut client) = stream_pair();
        let (ctrl_tx, mut ctrl_rx) = tokio::sync::mpsc::channel(4);
        let reader =
            tokio::spawn(async move { read_owner_control(&mut client, fenced, 1, &ctrl_tx).await });

        owner
            .send_frame(MeshStreamFrame::Data {
                fenced,
                payload: encode_control(&HuddleControlMsg::RosterDelta {
                    revision: 3,
                    joined: Some(RosterEntry {
                        pubkey: "bob".into(),
                        peer_index: 7,
                    }),
                    left: None,
                })
                .unwrap(),
            })
            .await
            .unwrap();

        let request = owner.recv_frame().await.unwrap().unwrap();
        let MeshStreamFrame::Data { payload, .. } = request else {
            panic!("expected roster resync request");
        };
        assert_eq!(
            decode_control(&payload).unwrap(),
            HuddleControlMsg::RosterResync
        );
        assert!(
            ctrl_rx.try_recv().is_err(),
            "gapped delta was not forwarded"
        );

        owner
            .send_frame(MeshStreamFrame::Data {
                fenced,
                payload: encode_control(&HuddleControlMsg::RosterSnapshot {
                    revision: 3,
                    peers: vec![RosterEntry {
                        pubkey: "bob".into(),
                        peer_index: 7,
                    }],
                })
                .unwrap(),
            })
            .await
            .unwrap();
        let message = ctrl_rx.recv().await.expect("replacement roster forwarded");
        let axum::extract::ws::Message::Text(json) = message else {
            panic!("expected roster JSON");
        };
        let json: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(json["type"], "roster");
        assert_eq!(json["revision"], 3);

        drop(owner);
        assert_eq!(reader.await.unwrap(), HuddleTeardownCause::StreamClosed);
    }

    /// Transport whose only exercised method is a no-op `send_datagram` (the
    /// remote-peer sink fires into it). `open_session_stream`/`set_inbound` are
    /// not reached on the accept path.
    struct NullTransport;
    impl RelayPeerTransport for NullTransport {
        fn send_datagram(&self, _to: RuntimeId, _d: MeshDatagram) -> Result<(), MeshError> {
            Ok(())
        }
        fn open_session_stream(
            &self,
            _to: RuntimeId,
            _hello: StreamHello,
        ) -> BoxFuture<'_, Result<MeshStream, MeshError>> {
            Box::pin(async { Err(MeshError::Transport("unused".into())) })
        }
        fn set_inbound(&self, _handler: Box<dyn buzz_relay_mesh::InboundHandler>) {}
    }

    fn fenced_owned_by(owner: RuntimeId, session_id: Uuid) -> FencedHeader {
        FencedHeader {
            session_id,
            generation: 7,
            owner_runtime_id: owner,
        }
    }

    fn huddle_hello(sender: RuntimeId, fenced: FencedHeader) -> StreamHello {
        StreamHello {
            sender,
            role: StreamRole::Session {
                fenced,
                profile: Profile::HuddleControl,
            },
        }
    }

    /// Full accept-side handshake: a structural `Hello`, then a
    /// community-bearing `RegisterPeer` whose fence passes, yields
    /// `PeerRegistered`. Exercises the public `MeshStream::new` seam and the
    /// validate-before-admit path end-to-end.
    #[tokio::test]
    async fn register_peer_handshake_admits_on_valid_fence() {
        let owner_rt = rt(1);
        let from = rt(2);
        let session_id = Uuid::new_v4();
        let fenced = fenced_owned_by(owner_rt, session_id);

        let acceptor = HuddleControlAcceptor::new(
            Arc::new(AudioRoomManager::new()),
            Arc::new(NullTransport) as Arc<dyn RelayPeerTransport>,
            Arc::new(FakeDir::default()), // validate() succeeds by default
            owner_rt,
            Arc::new(HuddleOwnerRegistry::new()), // no owner lease → recv-only
        );

        let (owner_stream, mut client) = stream_pair();
        let hello = huddle_hello(from, fenced);
        let served =
            tokio::spawn(async move { acceptor.accept_inbound(from, hello, owner_stream).await });

        // Client registers, carrying its community as the wire UUID.
        client
            .send_frame(MeshStreamFrame::Data {
                fenced,
                payload: encode_control(&HuddleControlMsg::RegisterPeer {
                    community_id: *community().as_uuid(),
                    pubkey: "client-a".into(),
                    protocol_version: 2,
                })
                .unwrap(),
            })
            .await
            .unwrap();

        let reply = match client.recv_frame().await.unwrap().unwrap() {
            MeshStreamFrame::Data { payload, .. } => decode_control(&payload).unwrap(),
            other => panic!("expected Data reply, got {other:?}"),
        };
        assert!(
            matches!(reply, HuddleControlMsg::PeerRegistered { ref pubkey, .. } if pubkey == "client-a"),
            "expected PeerRegistered for client-a, got {reply:?}"
        );

        // Closing the client stream ends the serve loop cleanly.
        client.finish().unwrap();
        drop(client);
        served.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn abnormal_control_stream_close_fans_out_remote_leave() {
        let owner_rt = rt(1);
        let from = rt(2);
        let session_id = Uuid::new_v4();
        let fenced = fenced_owned_by(owner_rt, session_id);
        let rooms = Arc::new(AudioRoomManager::new());
        let room = rooms.get_or_create(community(), session_id);
        let (_local_id, _local_index, _audio_rx, mut local_ctrl_rx) =
            room.add_peer("owner-local".into(), 2).unwrap();
        // Discard the local peer's own roster delta; this assertion targets the
        // websocket-compatible control fanout below.

        let acceptor = HuddleControlAcceptor::new(
            Arc::clone(&rooms),
            Arc::new(NullTransport) as Arc<dyn RelayPeerTransport>,
            Arc::new(FakeDir::default()),
            owner_rt,
            Arc::new(HuddleOwnerRegistry::new()),
        );
        let (owner_stream, mut client) = stream_pair();
        let hello = huddle_hello(from, fenced);
        let served =
            tokio::spawn(async move { acceptor.accept_inbound(from, hello, owner_stream).await });

        client
            .send_frame(MeshStreamFrame::Data {
                fenced,
                payload: encode_control(&HuddleControlMsg::RegisterPeer {
                    community_id: *community().as_uuid(),
                    pubkey: "remote".into(),
                    protocol_version: 2,
                })
                .unwrap(),
            })
            .await
            .unwrap();
        let _registered = client.recv_frame().await.unwrap().unwrap();
        let joined = local_ctrl_rx.recv().await.expect("remote join fanout");
        let super::super::room::PeerCtrl::Json(joined) = joined else {
            panic!("expected joined JSON");
        };
        let joined: serde_json::Value = serde_json::from_str(&joined).unwrap();
        assert_eq!(joined["type"], "joined");
        assert_eq!(joined["pubkey"], "remote");
        let remote_index = joined["peer_index"].as_u64().unwrap();

        drop(client);
        served.await.unwrap().unwrap();
        let left = local_ctrl_rx
            .recv()
            .await
            .expect("abnormal-close leave fanout");
        let super::super::room::PeerCtrl::Json(left) = left else {
            panic!("expected left JSON");
        };
        let left: serde_json::Value = serde_json::from_str(&left).unwrap();
        assert_eq!(left["type"], "left");
        assert_eq!(left["pubkey"], "remote");
        assert_eq!(left["peer_index"], remote_index);
        assert_eq!(room.peer_pubkeys(), vec![("owner-local".into(), 0)]);
    }

    /// A `RegisterPeer` whose fence is rejected (wrong community keys a lease
    /// Redis never wrote) yields a `RegisterRejected(Fenced(..))` reply — no
    /// peer admitted — and the stream stays alive for the client to close.
    #[tokio::test]
    async fn register_peer_handshake_rejects_on_fence_failure() {
        let owner_rt = rt(1);
        let from = rt(2);
        let session_id = Uuid::new_v4();
        let fenced = fenced_owned_by(owner_rt, session_id);

        let dir = FakeDir::default();
        *dir.validate_fails.lock().unwrap() = true;
        let acceptor = HuddleControlAcceptor::new(
            Arc::new(AudioRoomManager::new()),
            Arc::new(NullTransport) as Arc<dyn RelayPeerTransport>,
            Arc::new(dir),
            owner_rt,
            Arc::new(HuddleOwnerRegistry::new()), // no owner lease → recv-only
        );

        let (owner_stream, mut client) = stream_pair();
        let hello = huddle_hello(from, fenced);
        let served =
            tokio::spawn(async move { acceptor.accept_inbound(from, hello, owner_stream).await });

        client
            .send_frame(MeshStreamFrame::Data {
                fenced,
                payload: encode_control(&HuddleControlMsg::RegisterPeer {
                    community_id: *community().as_uuid(),
                    pubkey: "client-a".into(),
                    protocol_version: 2,
                })
                .unwrap(),
            })
            .await
            .unwrap();

        let reply = match client.recv_frame().await.unwrap().unwrap() {
            MeshStreamFrame::Data { payload, .. } => decode_control(&payload).unwrap(),
            other => panic!("expected Data reply, got {other:?}"),
        };
        assert!(
            matches!(
                reply,
                HuddleControlMsg::RegisterRejected {
                    reason: RegisterRejection::Fenced(_),
                    ..
                }
            ),
            "expected fence rejection, got {reply:?}"
        );

        drop(client);
        served.await.unwrap().unwrap();
    }

    // --- Huddle-lease renewer: owner-loss observability ---------------------
    //
    // These mirror the reliable lane's renewer tests
    // (`crate::tunnel::reliable::observable_renewer_*`), driven off the
    // `HuddleDirectory` seam with a scripted `FakeDir` so no Redis is needed.

    #[tokio::test]
    async fn observable_renewer_signals_loss_when_lease_disappears() {
        // First renew observes the lease gone → owner-loss.
        let dir = Arc::new(FakeDir::with_renew_script(
            [HuddleRenewOutcome::Lost],
            HuddleReleaseOutcome::Released,
        ));
        let cancel = CancellationToken::new();
        let renewer = spawn_huddle_renewer_with_interval(
            Arc::clone(&dir) as Arc<dyn HuddleDirectory>,
            test_lease(),
            cancel,
            Duration::from_millis(5),
        );

        tokio::time::timeout(Duration::from_secs(2), renewer.lost.cancelled())
            .await
            .expect("lost token is cancelled after ownership loss");
        renewer.task.await.unwrap();
        assert_eq!(*dir.renew_calls.lock().unwrap(), 1);
        // Loss still runs the release attempt on the way out.
        assert_eq!(*dir.release_calls.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn observable_renewer_normal_cancel_does_not_signal_loss() {
        // Empty renew script → `Renewed` forever; caller cancels cleanly.
        let dir = Arc::new(FakeDir::with_renew_script(
            std::iter::empty(),
            HuddleReleaseOutcome::Released,
        ));
        let cancel = CancellationToken::new();
        let renewer = spawn_huddle_renewer_with_interval(
            Arc::clone(&dir) as Arc<dyn HuddleDirectory>,
            test_lease(),
            cancel.clone(),
            Duration::from_millis(5),
        );
        cancel.cancel();
        renewer.task.await.unwrap();
        assert!(
            !renewer.lost.is_cancelled(),
            "caller-initiated shutdown is not ownership loss"
        );
        // Clean release ran and did not trip the loss signal.
        assert_eq!(*dir.release_calls.lock().unwrap(), 1);
    }

    // --- Huddle owner registry: install / reuse / fenced release ------------

    /// Poll `dir.release_calls` until it reaches `want` or the deadline: the
    /// registry owns the renewer task (no JoinHandle exposed), so a released
    /// lease is observed through the directory double it releases against.
    async fn await_release_calls(dir: &FakeDir, want: u32) {
        tokio::time::timeout(Duration::from_secs(2), async {
            while *dir.release_calls.lock().unwrap() < want {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap_or_else(|_| {
            panic!(
                "expected {want} release(s), saw {}",
                *dir.release_calls.lock().unwrap()
            )
        });
    }

    /// A second `attach` on a live room reuses the installed renewer and hands
    /// back its `lost` — and releases the extra lease so no second renewer
    /// leaks. The reused signal is the same token the first attach returned.
    #[tokio::test]
    async fn registry_double_attach_reuses_renewer_and_releases_extra_lease() {
        let dir = Arc::new(FakeDir::default()); // renew holds, release => Released
        let registry = HuddleOwnerRegistry::new();
        let session = Uuid::new_v4();

        let first = registry.attach(
            session,
            Arc::clone(&dir) as Arc<dyn HuddleDirectory>,
            lease_for(session, 7),
        );
        let second = registry.attach(
            session,
            Arc::clone(&dir) as Arc<dyn HuddleDirectory>,
            lease_for(session, 7),
        );

        // Same room → same loss signal; the second attach did not replace it.
        assert!(!first.is_cancelled());
        first.cancel();
        assert!(
            second.is_cancelled(),
            "reused entry shares the first renewer's lost token"
        );

        // Exactly one extra lease was released (the throwaway renewer); the
        // installed renewer is still live (only its caller-cancel releases it).
        await_release_calls(&dir, 1).await;
        assert!(
            registry.lost_for(session).is_some(),
            "installed entry stays live"
        );
    }

    /// `release` fenced on generation: a stale room-empty (older generation
    /// than the live entry, e.g. a re-acquire landed in the gap) is a no-op —
    /// it neither drops the entry nor cancels the live renewer.
    #[tokio::test]
    async fn registry_release_is_generation_fenced() {
        let dir = Arc::new(FakeDir::default());
        let registry = HuddleOwnerRegistry::new();
        let session = Uuid::new_v4();

        // Live entry owns generation 9 (a re-acquire installed the new epoch).
        let lost = registry.attach(
            session,
            Arc::clone(&dir) as Arc<dyn HuddleDirectory>,
            lease_for(session, 9),
        );

        // A stale leaver releases generation 8 → no-op against the newer epoch.
        registry.release(session, 8);
        assert!(
            registry.lost_for(session).is_some(),
            "stale release must not drop a newer entry"
        );
        assert!(
            !lost.is_cancelled(),
            "stale release must not cancel the live renewer"
        );
        assert_eq!(
            *dir.release_calls.lock().unwrap(),
            0,
            "stale release runs no directory release"
        );

        // The matching generation releases cleanly: entry gone, renewer's
        // caller-cancel drives the directory release, and loss stays silent.
        registry.release(session, 9);
        assert!(
            registry.lost_for(session).is_none(),
            "matching release drops the entry"
        );
        await_release_calls(&dir, 1).await;
        assert!(!lost.is_cancelled(), "clean release is not owner-loss");
    }

    /// The room-empty release path: attach installs, `release` on the same
    /// generation drops the entry and releases the lease. Mirrors the
    /// `handler.rs` `room_emptied` call with a matching generation.
    #[tokio::test]
    async fn registry_room_empty_release_drops_entry_and_releases_lease() {
        let dir = Arc::new(FakeDir::default());
        let registry = HuddleOwnerRegistry::new();
        let session = Uuid::new_v4();

        let lost = registry.attach(
            session,
            Arc::clone(&dir) as Arc<dyn HuddleDirectory>,
            lease_for(session, 3),
        );
        assert!(registry.lost_for(session).is_some());

        registry.release(session, 3);
        assert!(
            registry.lost_for(session).is_none(),
            "room-empty release clears the entry"
        );
        await_release_calls(&dir, 1).await;
        assert!(
            !lost.is_cancelled(),
            "room-empty is a clean release, not owner-loss"
        );
    }

    /// `drain` is generation-fenced like `release`, but unlike room-empty it
    /// also cancels the drain signal so local owner peers and remote control
    /// streams can rejoin with an explicit draining cause before the renewer
    /// releases the lease.
    #[tokio::test]
    async fn registry_drain_signals_rejoin_and_releases_lease() {
        let dir = Arc::new(FakeDir::default());
        let registry = HuddleOwnerRegistry::new();
        let session = Uuid::new_v4();

        let lost = registry.attach(
            session,
            Arc::clone(&dir) as Arc<dyn HuddleDirectory>,
            lease_for(session, 4),
        );
        let draining = registry.drain_for(session).expect("drain signal installed");

        assert!(registry.drain(session, 4), "matching generation drains");
        assert!(
            registry.lost_for(session).is_none(),
            "drain drops the owner entry"
        );
        assert!(draining.is_cancelled(), "drain fans out to owners/streams");
        await_release_calls(&dir, 1).await;
        assert!(!lost.is_cancelled(), "drain is not fenced owner-loss");
    }

    #[test]
    fn drain_all_permanently_fences_new_owner_admission() {
        let registry = HuddleOwnerRegistry::new();
        assert!(!registry.is_draining());
        assert_eq!(registry.drain_all(), 0);
        assert!(registry.is_draining());
    }

    #[tokio::test]
    async fn attach_after_drain_releases_new_lease_and_returns_cancelled_signal() {
        let dir = Arc::new(FakeDir::default());
        let registry = HuddleOwnerRegistry::new();
        let session = Uuid::new_v4();
        registry.drain_all();

        let signals = registry.attach_signals(
            session,
            Arc::clone(&dir) as Arc<dyn HuddleDirectory>,
            lease_for(session, 11),
        );
        assert!(signals.draining.is_cancelled());
        assert!(registry.lost_for(session).is_none());
        await_release_calls(&dir, 1).await;
    }

    /// A stale drain racing a fresh re-acquire must not cancel the new epoch's
    /// drain signal or release its renewer.
    #[tokio::test]
    async fn registry_drain_is_generation_fenced() {
        let dir = Arc::new(FakeDir::default());
        let registry = HuddleOwnerRegistry::new();
        let session = Uuid::new_v4();

        registry.attach(
            session,
            Arc::clone(&dir) as Arc<dyn HuddleDirectory>,
            lease_for(session, 9),
        );
        let draining = registry.drain_for(session).expect("drain signal installed");

        assert!(!registry.drain(session, 8), "stale generation is no-op");
        assert!(registry.lost_for(session).is_some());
        assert!(!draining.is_cancelled());
        assert_eq!(*dir.release_calls.lock().unwrap(), 0);
    }

    // --- resolve_join_owner_ready: the LocalOwner reuse race gate ------------

    /// The exact interleaving the review flagged: the CAS winner has resolved
    /// (Redis names this pod owner) but has not yet installed its registry entry
    /// when a second joiner resolves `LocalOwner`. `resolve_join_owner_ready`
    /// must NOT return the bare snapshot — it re-resolves until the winner's
    /// entry appears, then returns reuse. Never admits an ownerless owner peer.
    #[tokio::test]
    async fn owner_ready_waits_for_winner_install_then_reuses() {
        // Redis says this pod (rt(1)) owns generation 5 — the steady-state
        // reuse arm (acquired = None), winner not yet attached.
        let dir = Arc::new(FakeDir::owned_by(Ownership {
            owner_runtime_id: rt(1),
            generation: 5,
        }));
        let registry = Arc::new(HuddleOwnerRegistry::new());
        let session = Uuid::new_v4();

        // The winner installs its entry after a couple of retry intervals.
        let installer = Arc::clone(&registry);
        let dir_for_install = Arc::clone(&dir);
        let install = tokio::spawn(async move {
            tokio::time::sleep(OWNER_READY_RETRY_INTERVAL * 3).await;
            installer.attach(
                session,
                dir_for_install as Arc<dyn HuddleDirectory>,
                lease_for(session, 5),
            );
        });

        let resolved = resolve_join_owner_ready(&*dir, community(), session, rt(1), &registry)
            .await
            .unwrap();

        install.await.unwrap();
        // Reuse: LocalOwner with no fresh lease (the winner holds it), and the
        // live entry now exists so the caller gets a real loss watcher.
        assert_eq!(resolved.outcome, JoinOutcome::LocalOwner { generation: 5 });
        assert!(resolved.acquired.is_none(), "reuse arm mints no new lease");
        assert!(
            registry.lost_for(session).is_some(),
            "returned only once a live entry gated reuse"
        );
    }

    /// If the room emptied and released underneath the racing joiner, the
    /// re-resolve wins a fresh CAS instead of adopting the torn-down lease:
    /// `owner_of` now reports unowned, `acquire` grants a new generation, and
    /// the result carries the real lease (acquired = Some) for the caller to
    /// install.
    #[tokio::test]
    async fn owner_ready_reacquires_when_room_released_underneath() {
        // Starts LocalOwner/no-entry (the ambiguous window)...
        let dir = Arc::new(FakeDir::owned_by(Ownership {
            owner_runtime_id: rt(1),
            generation: 5,
        }));
        // ...then the room empties + releases: owner_of goes unowned and our
        // acquire wins a fresh generation.
        let dir_flip = Arc::clone(&dir);
        let flip = tokio::spawn(async move {
            tokio::time::sleep(OWNER_READY_RETRY_INTERVAL * 2).await;
            *dir_flip.owner.lock().unwrap() = None;
            *dir_flip.acquire.lock().unwrap() =
                Some(AcquireOutcome::Acquired(lease_for(Uuid::nil(), 6)));
        });
        let registry = HuddleOwnerRegistry::new(); // stays empty → forces retry

        let resolved =
            resolve_join_owner_ready(&*dir, community(), Uuid::new_v4(), rt(1), &registry)
                .await
                .unwrap();

        flip.await.unwrap();
        assert_eq!(resolved.outcome, JoinOutcome::LocalOwner { generation: 6 });
        assert_eq!(
            resolved.acquired.as_ref().map(HuddleLease::generation),
            Some(6),
            "re-acquire mints a fresh lease for the caller to install"
        );
    }

    /// The window never resolves (winner wedged, entry never installed, room
    /// never releases): the loop exhausts and fails closed rather than admit an
    /// ownerless owner. The handler surfaces this to the client exactly like a
    /// lost CAS.
    #[tokio::test]
    async fn owner_ready_fails_closed_when_window_never_resolves() {
        // Perpetual LocalOwner/no-entry: owner_of always names us, registry
        // stays empty, acquire is never scripted (and never reached). The
        // bounded loop (~500ms real time) must terminate and fail closed.
        let dir = FakeDir::owned_by(Ownership {
            owner_runtime_id: rt(1),
            generation: 5,
        });
        let registry = HuddleOwnerRegistry::new();

        let err = resolve_join_owner_ready(&dir, community(), Uuid::new_v4(), rt(1), &registry)
            .await
            .unwrap_err();
        assert!(
            matches!(err, MeshError::Transport(_)),
            "exhaustion fails closed with a transient error, not an ownerless success"
        );
    }

    /// The owner-side control loop, given the room's `lost` from the registry,
    /// emits a proactive `Goodbye(StaleGeneration)` to the non-owner pod when
    /// the lease is lost — the fan-out the live-attach seam drives. The
    /// acceptor reads `lost` from the shared registry keyed by `session_id`,
    /// not from a per-call argument.
    #[tokio::test]
    async fn serve_control_loop_emits_stale_generation_goodbye_on_loss() {
        let owner_rt = rt(1);
        let from = rt(2);
        let session_id = Uuid::new_v4();
        let fenced = fenced_owned_by(owner_rt, session_id);

        // This pod owns the room: install its owner entry so the acceptor's
        // `lost_for(session_id)` returns the room's loss signal.
        let owners = Arc::new(HuddleOwnerRegistry::new());
        let lost = owners.install_for_test(session_id, fenced.generation);

        let acceptor = HuddleControlAcceptor::new(
            Arc::new(AudioRoomManager::new()),
            Arc::new(NullTransport) as Arc<dyn RelayPeerTransport>,
            Arc::new(FakeDir::default()),
            owner_rt,
            Arc::clone(&owners),
        );

        let (owner_stream, mut client) = stream_pair();
        let hello = huddle_hello(from, fenced);
        let served =
            tokio::spawn(async move { acceptor.accept_inbound(from, hello, owner_stream).await });

        // Owner observes lease loss → proactive Goodbye down the client stream.
        lost.cancel();

        let frame = tokio::time::timeout(Duration::from_secs(2), client.recv_frame())
            .await
            .expect("goodbye arrives")
            .unwrap()
            .unwrap();
        assert!(
            matches!(
                frame,
                MeshStreamFrame::Goodbye {
                    reason: GoodbyeReason::StaleGeneration,
                    ..
                }
            ),
            "expected Goodbye(StaleGeneration), got {frame:?}"
        );
        served.await.unwrap().unwrap();
    }

    /// Owner drain is distinct from fenced loss: control streams emit
    /// `Goodbye(Draining)` so non-owner clients rejoin for rollout rather than
    /// recording a stale-generation loss.
    #[tokio::test]
    async fn serve_control_loop_emits_draining_goodbye_on_drain() {
        let owner_rt = rt(1);
        let from = rt(2);
        let session_id = Uuid::new_v4();
        let fenced = fenced_owned_by(owner_rt, session_id);

        let draining = CancellationToken::new();
        let acceptor = HuddleControlAcceptor::new(
            Arc::new(AudioRoomManager::new()),
            Arc::new(NullTransport) as Arc<dyn RelayPeerTransport>,
            Arc::new(FakeDir::default()),
            owner_rt,
            Arc::new(HuddleOwnerRegistry::new()),
        );

        let (owner_stream, mut client) = stream_pair();
        let draining_for_loop = draining.clone();
        let served = tokio::spawn(async move {
            acceptor
                .serve_control_loop(from, fenced, owner_stream, None, Some(draining_for_loop))
                .await
        });

        draining.cancel();

        let frame = tokio::time::timeout(Duration::from_secs(2), client.recv_frame())
            .await
            .expect("goodbye arrives")
            .unwrap()
            .unwrap();
        assert!(
            matches!(
                frame,
                MeshStreamFrame::Goodbye {
                    reason: GoodbyeReason::Draining,
                    ..
                }
            ),
            "expected Goodbye(Draining), got {frame:?}"
        );
        served.await.unwrap().unwrap();
    }

    #[test]
    fn fence_rejection_classifies_only_fence_errors() {
        assert_eq!(
            FenceRejection::from_mesh_error(&MeshError::StaleGeneration {
                session_id: Uuid::nil(),
                frame_generation: 1,
                known_generation: 2,
            }),
            Some(FenceRejection::StaleGeneration)
        );
        assert_eq!(
            FenceRejection::from_mesh_error(&MeshError::Transport("x".into())),
            None
        );
    }

    #[test]
    fn fenced_header_uses_local_id_for_local_owner_and_owner_id_for_remote() {
        let s = Uuid::new_v4();
        let local = JoinOutcome::LocalOwner { generation: 5 };
        assert_eq!(
            local.fenced_header(s, rt(1)),
            FencedHeader {
                session_id: s,
                generation: 5,
                owner_runtime_id: rt(1),
            }
        );
        let remote = JoinOutcome::RemoteOwner {
            owner_runtime_id: rt(2),
            generation: 8,
        };
        assert_eq!(
            remote.fenced_header(s, rt(1)),
            FencedHeader {
                session_id: s,
                generation: 8,
                owner_runtime_id: rt(2),
            }
        );
    }

    #[test]
    fn admission_errors_map_to_wire_rejections() {
        assert_eq!(
            admission_to_rejection(AdmissionError::Full),
            RegisterRejection::RoomFull
        );
        assert_eq!(
            admission_to_rejection(AdmissionError::Ended),
            RegisterRejection::RoomEnded
        );
        assert_eq!(
            admission_to_rejection(AdmissionError::VersionMismatch {
                pinned: 2,
                requested: 1
            }),
            RegisterRejection::VersionMismatch {
                pinned: 2,
                requested: 1
            }
        );
    }

    #[test]
    fn media_datagram_tags_owner_index_and_stamps_fence() {
        let fenced = FencedHeader {
            session_id: Uuid::new_v4(),
            generation: 9,
            owner_runtime_id: rt(2),
        };
        // Owner-assigned index is the first payload byte; client bytes follow.
        let d0 = media_datagram(42, fenced, 0, &[0xDE, 0xAD]);
        assert_eq!(d0.payload, vec![42, 0xDE, 0xAD]);
        assert_eq!(d0.fenced, fenced);
        assert_eq!(d0.seq, 0);
        // Empty client frame still carries the index byte (owner tolerates it).
        let d1 = media_datagram(7, fenced, 3, &[]);
        assert_eq!(d1.payload, vec![7]);
        assert_eq!(d1.seq, 3);
    }

    // ── Non-owner teardown reader: wire signal → HuddleTeardownCause ──────────
    //
    // Drive `read_teardown_cause` over the in-memory stream pair: the "owner"
    // half writes a terminal signal, and the "client" half's reader must
    // classify it. Every arm of `GoodbyeReason` plus a bare stream close.

    /// Send a `Goodbye(reason)` from the owner half and assert the reader maps
    /// it to `expected`.
    async fn assert_goodbye_maps(reason: GoodbyeReason, expected: HuddleTeardownCause) {
        let fenced = fenced_owned_by(rt(2), Uuid::new_v4());
        let (mut owner, mut client) = stream_pair();
        owner
            .send_frame(MeshStreamFrame::Goodbye { fenced, reason })
            .await
            .unwrap();
        assert_eq!(read_teardown_cause(&mut client).await, expected);
    }

    #[tokio::test]
    async fn reader_maps_stale_generation_to_owner_lost() {
        assert_goodbye_maps(
            GoodbyeReason::StaleGeneration,
            HuddleTeardownCause::OwnerLost,
        )
        .await;
    }

    #[tokio::test]
    async fn reader_maps_draining_to_owner_draining() {
        assert_goodbye_maps(GoodbyeReason::Draining, HuddleTeardownCause::OwnerDraining).await;
    }

    #[tokio::test]
    async fn reader_maps_session_ended_to_ordinary_disconnect() {
        assert_goodbye_maps(
            GoodbyeReason::SessionEnded,
            HuddleTeardownCause::SessionEnded,
        )
        .await;
    }

    #[tokio::test]
    async fn reader_maps_bare_stream_close_to_stream_closed() {
        let (owner, mut client) = stream_pair();
        // Owner pod dies mid-flight: the send half drops with no Goodbye, so the
        // client's recv sees a clean close.
        drop(owner);
        assert_eq!(
            read_teardown_cause(&mut client).await,
            HuddleTeardownCause::StreamClosed
        );
    }

    #[tokio::test]
    async fn reader_skips_non_terminal_frames_then_tears_down() {
        // A future owner→client roster delta (a `Data` frame) must NOT be read
        // as teardown; the reader keeps going and returns on the later Goodbye.
        let fenced = fenced_owned_by(rt(2), Uuid::new_v4());
        let (mut owner, mut client) = stream_pair();
        owner
            .send_frame(MeshStreamFrame::Data {
                fenced,
                payload: vec![0xAB, 0xCD],
            })
            .await
            .unwrap();
        owner
            .send_frame(MeshStreamFrame::Goodbye {
                fenced,
                reason: GoodbyeReason::StaleGeneration,
            })
            .await
            .unwrap();
        assert_eq!(
            read_teardown_cause(&mut client).await,
            HuddleTeardownCause::OwnerLost
        );
    }

    /// The client-initiated clean close emits `UnregisterPeer` then
    /// `Goodbye(SessionEnded)` on the owner's control stream, in that order.
    #[tokio::test]
    async fn clean_close_sends_unregister_then_goodbye() {
        let fenced = fenced_owned_by(rt(2), Uuid::new_v4());
        let (mut owner, mut client) = stream_pair();
        send_clean_close(&mut client, fenced, "client-a").await;

        match owner.recv_frame().await.unwrap().unwrap() {
            MeshStreamFrame::Data { payload, .. } => assert_eq!(
                decode_control(&payload).unwrap(),
                HuddleControlMsg::UnregisterPeer {
                    pubkey: "client-a".into()
                }
            ),
            other => panic!("expected UnregisterPeer Data, got {other:?}"),
        }
        match owner.recv_frame().await.unwrap().unwrap() {
            MeshStreamFrame::Goodbye { reason, .. } => {
                assert_eq!(reason, GoodbyeReason::SessionEnded)
            }
            other => panic!("expected Goodbye(SessionEnded), got {other:?}"),
        }
    }
}
