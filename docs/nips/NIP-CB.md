# NIP-CB: Owner-Private Command Brief Lifecycle Events

`draft` `optional`

NIP-CB defines an append-only audit record for an advisory Daily Command
Brief. It uses regular stored kind `44210`. It is not a task, order, or
operational control message.

## Public envelope

The owner authors the event and encrypts its JSON content to the same owner
public key with NIP-44 version 2. The event MUST have exactly these tags:

```json
[
  ["p", "<owner-pubkey-equal-to-event-pubkey>"],
  ["d", "<run-id>"],
  ["status", "completed|degraded|cancelled|failed"],
  ["previous", "<optional-previous-lifecycle-event-id>"]
]
```

There is exactly one `p`, `d`, and `status` tag and zero or one `previous`
tag. No other tag is permitted. `d` does not make kind `44210`
parameterized-replaceable; it is a correlation value only.

## Encrypted payload

The camel-case plaintext object contains:

- `version`, currently `1`;
- `classification`, always `OFFICIAL`;
- `runId` and `scheduleId`;
- `lifecycleState`, matching the public `status`;
- RFC3339 `occurredAt`;
- `frozenSnapshotId`;
- `finalBrief` for `completed` or `degraded`, otherwise null. It MUST match the
  authoritative strict `CommandBrief` contract: unknown fields and enum values,
  missing adviser/section/source provenance, broken citations, and unbounded
  values are rejected;
- redacted `failure.code` for `cancelled` or `failed`, otherwise null. Codes use
  the closed implementation vocabulary (`cancellation_requested`,
  `brief_generation_failed`, snapshot/RAG/source failures,
  `chief_of_staff_failed`, `chief_of_staff_output_rejected`, or
  `brief_assembly_rejected`); and
- `previousLifecycleEventId`, exactly matching the optional public tag.

The signed event ID is deliberately absent from the plaintext used to derive
that ID. A client adds it only to its post-signing `PublishedCommandBrief`
view.

Payloads and every nested string, array, object, identifier, and retry queue
are bounded by the implementation contract. The final signed event content is
limited to 256 KiB of NIP-44 ciphertext at both client admission and relay
ingest. Payloads MUST NOT contain raw prompts, hidden reasoning, credentials,
API keys, bearer tokens, arbitrary provider error bodies, or unvalidated
retrieved passages.

## Lifecycle and storage

Events are regular, stored, and append-only. A new lifecycle record for a run
MUST reference its current head through `previous`. A conflicting predecessor
is rejected locally. Neither relay nor client may replace or overwrite an
earlier record.

The client signs and commits the exact event and event ID to a protected
SQLite WAL spool before reporting local completion. That commit is the
cancellation linearization point: accepted cancellation before it produces one
`cancelled` event; cancellation after it cannot replace the committed terminal.
Every terminal path (`completed`, `degraded`, `cancelled`, and `failed`) uses
this same durable path.

Relay unavailability leaves the record queued. Startup and a
degraded-to-connected relay transition re-arm a bounded owner batch, including
rows that previously reached retry 8, and republish the exact signed bytes
idempotently by event ID. Rows whose stored envelope, signature, timestamp,
ciphertext, or event ID no longer validate are permanently quarantined and are
not re-armed.

## Access and privacy

Relays require NIP-42 authentication. `REQ`, `COUNT`, NIP-50 search,
kindless-ID queries, live fan-out, and archive reads MUST reveal neither
existence nor content unless the authenticated reader equals the sole `p`
owner.

Kind `44210` has a NULL full-text-search vector. Relays do not decrypt it.
Clients and the archive pipeline decrypt only after the current unlocked
identity proves that it is both the event author and the `p` recipient.
Consumers receive a validated Command Brief view model, never raw arbitrary
JSON or ciphertext masquerading as a brief.

## Retention and backup

The encrypted signed event is suitable for normal relay retention. The local
spool is owner-scoped, permission-protected, backup-compatible SQLite. Backup
and restore preserve exact signed event bytes, publish state, predecessor
links, bounded retry state, the protected schedule, and daily idempotency
claims.

## Local schedule

The built-in schedule defaults to `06:00` in the current macOS IANA timezone.
Before any generation side effect it atomically stores both the exact key
`<schedule_id>:<YYYY-MM-DD>` and the deterministic run ID derived from that
key. Restart reconciliation reuses that run ID and checks both the live
orchestrator and durable terminal spool, so crashes before start, after start
returns, or after the started marker cannot create a second logical run.
Startup and wake may perform at most one current-day catch-up and never replay
earlier dates. If `catch_up_same_day` is false, startup, native wake, and timer
checks all remain disabled.

macOS may delay application execution while the Mac is asleep. The signed
Apple helper listens to `NSWorkspace.didWakeNotification` and emits a bounded
local wake signal; Tauri foreground/resume events are not treated as system
wake. A periodic timer remains a resilience check. The product therefore
promises same-day catch-up after a verified wake when all local authorization
and readiness gates pass; it does not promise exact execution while asleep.

Every due production attempt re-attests the unlocked signing identity, selected
LM Studio model, exact RAG snapshot, Apple allowlist and helper identity, local
SQLite schema, and configured scheduler capacity before the claim may start.
The readiness transition token binds the resulting configuration, runtime
generation, and current capacity availability. Locked identity, unavailable
LM Studio/RAG, or missing mandatory local state stays visibly deferred and
retries only after a distinct bounded readiness transition.

## Forward compatibility

Unknown versions, lifecycle states, classifications, tags, or fields fail
closed. A future incompatible shape requires a new payload version and must
retain the owner-only read and result-level access gates.
