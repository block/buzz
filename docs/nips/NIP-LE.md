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

The lock file contains a JSON object with the following shape
(provisional — finalized against Phase 2 implementation):

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
- `pid`: the operating-system process id of the leading window.
- `claimed_at`: when the current leader claimed the lock.

An instance is the leader for an agent identity iff a lock file exists and its
`instance_id` equals that instance's own election identity. A malformed or
unparseable lock file MUST fail safe to leader, preserving solo-dev behavior.

Acquire and steal MUST be `flock`-guarded to close the read-check-write TOCTOU
window between two instances racing to claim or steal the same lock.

## Claim/Steal Semantics

Ownership is explicit and sticky to a window: a window claims an agent (via the
agent sidebar menu) and remains its leader until the lock is released or stolen.

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
