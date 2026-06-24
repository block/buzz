NIP-LE
======

Leader Election (Shared-Identity Multi-Instance)
------------------------------------------------

`draft` `optional`

This NIP defines a client-side, local-filesystem convention by which multiple
instances running under a single shared agent identity elect exactly one
*prompter* (leader), so that a mention fanned out to every instance produces a
single agent response.

## Motivation

When several Buzz client instances run with the same agent keypair, the relay
correctly fans every event addressed to that key out to all connected instances
(per NIP-01). Without coordination, every instance promotes the event to an
agent prompt and every instance responds — duplicating work and producing
duplicate replies under one identity.

This commonly arises in development: a packaged build (DMG) and one or more
`just staging` builds from in-progress worktrees may run concurrently, all
sharing the developer's agent identities.

This NIP defines a minimal coordination layer that elects a single prompter per
agent identity without any relay-side logic and without defining a wire format.

## Non-Goals

This NIP does not define any Nostr event kind or tag.
This NIP does not define relay-side coordination.
This NIP does not define a cancel mechanism — hard-steal reuses NIP-AO's
`cancel_turn` control frame.
This NIP does not define the durable election identity source; it requires only
that the identity be per-window-unique (see The Lock Contract).

## The Invariant

A single agent identity MAY have N subscribed instances but MUST have exactly
one *prompter* (the leader).

Non-leader instances MUST subscribe to and render the full conversation — the
message queue stays ungated so the UI displays all messages identically to the
leader. Non-leader instances MUST suppress:

- (a) the prompt/dispatch path — they do not promote queued events to agent
  prompts; and
- (b) pre-dispatch side-effects — specifically the `👀` acknowledgement reaction,
  which fires at queue-acceptance time, before dispatch; and
- (c) the autonomous heartbeat prompt path — the periodic self-prompt that, when
  enabled, has the agent act on the wire (send messages, approve workflows);
  non-leaders MUST NOT fire it, for the same duplicate-actor reason as (a).

Fail-safe: if no lock exists for an agent identity, every instance is a leader.
This is the single-instance / solo-dev default and is byte-unchanged from
behavior prior to this NIP — a solo developer has exactly one instance, which
leads unconditionally.

## The Lock Contract

The lock lives on the LOCAL filesystem at
`~/.buzz/leader-locks/<agent-pubkey-hex>.lock`. It is NOT a Nostr event and
defines no wire format. This is why this NIP touches no relay protocol: leader
election is entirely local to the machine running the instances.

The lock file contains a JSON object with the following shape:

```json
{
  "instance_id": "<per-window-unique election id>",
  "pid": 12345,
  "claimed_at": "<iso8601 or unix timestamp>"
}
```

- `instance_id`: the identity of the leading window. It MUST be a
  per-window-unique election identity. The Tauri bundle identifier is
  insufficient — it collides across same-class windows (for example a DMG build
  and a `just staging` dev build, or two worktree builds whose unique-icon
  generation fell back to the shared identifier), causing two windows to both
  match the lock and both lead. A correct election identity is unique per
  running window (for example a process-unique value derived at launch).
- `pid`: the operating-system process id of the leading window. Read by the
  acquire path to detect a dead leader (dead-pid takeover); informational to the
  read side.
- `claimed_at`: when the current leader claimed the lock. Rewritten by the
  leader on every refresh tick, and read by the acquire path to detect a stale
  (abandoned) claim whose pid has been recycled by an unrelated process (see
  Claim/Steal Semantics); informational to the read side.

Only `instance_id` is significant to the read side; `pid` and `claimed_at` are
informational to it, written by the acquire path and ignored when deciding
leadership.

An instance is the leader for an agent identity iff a lock file exists and its
`instance_id` equals that instance's own election identity. A malformed or
unparseable lock file MUST fail safe to leader, preserving solo-dev behavior.
An absent or unreadable lock file (permission error, or a partial read while
another instance is mid-rewrite) likewise fails safe to leader. The read side is
deliberately unguarded (it takes no `flock`), so a concurrent rewrite read as
malformed can briefly produce a duplicate responder; this self-heals on the next
refresh (≤5s, see below) and is the intended trade — a transient duplicate beats
silencing the only responder.

Leadership changes (claim, release, failover) take effect within a bounded refresh
interval of 5 seconds: each instance re-reads its leader lock(s) every 5s, so a
stale-cache window after a leadership change is capped at that interval. The
refresh also re-attempts the claim, so a survivor takes over a dead leader's lock
within the same bound.

Acquire and steal MUST be `flock`-guarded to close the read-check-write TOCTOU
window between two instances racing to claim or steal the same lock: the writer
holds an exclusive `flock` across the read-decide-write sequence so at most one
racer wins.

## Claim/Steal Semantics

Claim is **auto-on-launch plus explicit re-claim**. An instance self-elects at
startup: it acquires the lock if it is unowned (absent, released, held by a
dead pid, or held by a stale claim whose pid was recycled — see below), and
otherwise runs as an observer. The first instance up for an agent key leads; a
later instance under the same key observes. On graceful shutdown an instance
releases its lock so a co-located sibling can take over without waiting out the
dead-pid failover window. An explicit re-claim gesture (the agent sidebar menu)
layers on top, letting a chosen window take ownership; both paths share one writer.

Failover MUST tolerate pid recycling. A leader that crashes without releasing
leaves a lock whose `pid` the OS may later reuse for an unrelated, long-lived
process; a bare pid-liveness probe would then read the recycled pid as the
original leader and never take over, wedging the agent leaderless. The acquire
path therefore treats a claim as takeable when its pid is dead **or** its
`claimed_at` is older than a staleness bound that comfortably exceeds the 5s
refresh interval (e.g. 10s). A live leader rewrites `claimed_at` on every refresh
tick, so only an abandoned claim ages past the bound — distinguishing a recycled
pid from a genuinely active leader without evicting the latter.

### Cooperative Steal (Manual Leadership Transfer)

A deliberate steal — initiated from the desktop sidebar UI — uses a
**cooperative** mechanism: the current leader voluntarily stands down rather than
being forcibly evicted. This preserves the split-brain guard (`lock_is_takeable`)
that prevents two leaders from coexisting.

**Stand-down primitive.** When the current leader receives a `claim_leadership`
control frame targeting a different instance, it enters *stand-down*: a state
that suppresses `acquire()` (returns false without touching the lock file) until
either `resume()` is called or a timeout expires. The leader then releases its
lock, making it free for the target.

**Timeout.** Stand-down auto-expires after 10 seconds (2× the 5s refresh tick).
This bounds the zero-leader window: if the target instance crashes or fails to
acquire, the old leader resumes and re-claims on its next tick. The worst case
is a brief gap with no leader (bounded, self-healing) — never a split-brain.

**Handoff sequence:**

1. Desktop sends a `claim_leadership` control frame (NIP-AO kind 24200) with
   `{ "type": "claim_leadership", "targetInstanceId": "<target>" }`.
2. All co-located instances receive the broadcast.
3. The **current leader** (non-target): calls `stand_down()` then `release()`.
   Its next refresh tick's `acquire()` returns false (standing down), preventing
   re-grab.
4. The **target instance** (match): calls `acquire()`. If the lock is now free
   (old leader released), it succeeds and emits a `control_result` with
   `status: "claimed"`. If the lock is not yet free (race — old leader hasn't
   processed the frame yet), it does nothing; its next 5s refresh tick will
   succeed once the old leader stands down.
5. **Other observers** (non-target, non-leader): ignore the frame.

**Failure direction.** Cooperative steal never bypasses the live-leader guard.
The worst case is zero leaders briefly (stand-down entered but target didn't
acquire), which self-heals via the 10s timeout. This is preferable to force-steal
which would risk two leaders briefly.

### Scope Boundary

Leader election is **single-host only**. The lock file lives at
`~/.buzz/leader-locks/<pubkey>.lock` — a local filesystem artifact. Instances on
different machines have different `~/.buzz/` directories and cannot see each
other's locks.

The `claim_leadership` control frame (sent via the relay) reaches all subscribed
instances regardless of host, but the `acquire()` that follows only contends with
co-located processes sharing the same lock file. Cross-machine coordination
(relay-mediated lock or consensus) is a separate future change, explicitly out of
scope.

Hard-steal: claiming an agent in window B immediately aborts window A's
in-flight turn. The abort reuses NIP-AO's `cancel_turn` control frame
(kind 24200); this NIP does not define a separate cancel mechanism. See NIP-AO
for the control-frame structure and authorization rules.

## Relationship to other NIPs

NIP-LE references the following NIPs; it does not amend any of them.

- **NIP-OA (Owner Attestation)** — owns the owner↔agent identity model (one owner
  authorizes one agent key) but is silent on what happens when that same agent
  key runs in N instances at once. NIP-LE fills that gap.
- **NIP-RS (Cross-Device Read State Sync)** — precedent for same-user,
  multi-instance coordination. NIP-RS synchronizes *data* (read position) across
  instances; NIP-LE coordinates *behavior* (which instance prompts).
- **NIP-AO (Agent Observability)** — provides the `cancel_turn` control frame
  (kind 24200) that NIP-LE's hard-steal reuses to abort a displaced window's
  in-flight turn.
