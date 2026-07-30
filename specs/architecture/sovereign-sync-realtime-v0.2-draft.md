# Sovereign sync realtime v0.2 (draft)

Status: design recorded, implementation deferred. Realtime replication is
relationship-layer work and stays gated behind the Buzz Solo
(sticky-attention) milestone. Proposed by the downstream consumer node in
journal record `6e0f4231…` after the first closed release loop; the pull
half has a working precedent in `a36cb7b79` (drain rendezvous on beacon
transitions).

## Purpose

Reduce replication latency between sovereign nodes from the polling
interval to near-realtime, without introducing any new authority, custody
path, or replication code path. Realtime is a latency optimization over
the existing scheduled recovery — never a correctness dependency.

## Design

Three triggers, one procedure:

1. **Commit-triggered push.** A durable local journal commit enqueues an
   immediate, debounced push using the same declared source, stream
   filters, and profile-owned cursor as scheduled recovery. The debounce
   window coalesces bursts; the push that runs is byte-for-byte the push
   the schedule would have run.

2. **Pulse-triggered pull (peer wake).** When the rendezvous journal head
   advances, it wakes authenticated peer sessions over their existing
   standing sockets — the same sessions the pulse layer already witnesses
   under `coherence.sessions`. A woken peer runs the same drain as
   scheduled recovery. This extends the transition-driven drain precedent
   (`a36cb7b79`) from beacon transitions to head-advance wake.

3. **Poll as backstop.** Scheduled timers remain unchanged and are the
   missed-signal safety net. A lost wake degrades to poll latency, never
   to data loss.

## Invariants

- **Wake signals carry no authority.** A pulse or commit trigger conveys
  "look now," nothing else. Every triggered drain re-evaluates the
  declared export/admit/read agreements exactly as a scheduled drain
  does; no trigger can widen a selection or bypass a grant.
- **One code path.** Realtime and scheduled replication are the same
  procedure with different triggers. There is no separate fast path to
  drift from the recovery path.
- **Cursor integrity.** All triggers share the profile-owned cursor and
  stream selection. No trigger may fall back to a default cursor or an
  unselected stream set — the `context sync` cursor/selection
  passthrough defect reported in `6e0f4231…` is a hard precondition for
  this work, not a parallel concern.
- **Fail silent, converge on schedule.** Missed wakes are not errors and
  are not retried at the wake layer; the backstop poll converges them.
- **Pulse-layer discipline.** Wake signals are ephemeral,
  conversation-not-record, per beacon pulse v0.2. Durable state changes
  belong to the journal; the wake layer never becomes a second source of
  truth.

## Out of scope

- New event kinds or transport mechanisms; the existing pulse semantics
  and authenticated sockets suffice.
- Any change to agreement evaluation, artifact custody, or checkpoint
  semantics.
- Unattended execution concerns; claim exclusivity is a separate track.

## Sequencing

1. Precondition: profile cursor and stream-selection passthrough in
   `context sync` (defect fix, lands with the managed context CLI).
2. Commit-triggered push on the development node.
3. Head-advance peer wake at the rendezvous.
4. Only then, tune backstop intervals down if the realtime path proves
   reliable — never remove them.
