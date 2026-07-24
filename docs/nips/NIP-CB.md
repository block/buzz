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
- `finalBrief` for `completed` or `degraded`, otherwise null;
- redacted `failure.code` for `cancelled` or `failed`, otherwise null; and
- `previousLifecycleEventId`, exactly matching the optional public tag.

The signed event ID is deliberately absent from the plaintext used to derive
that ID. A client adds it only to its post-signing `PublishedCommandBrief`
view.

Payloads and every nested string, array, object, identifier, and retry queue
are bounded by the implementation contract. Payloads MUST NOT contain raw
prompts, hidden reasoning, credentials, API keys, bearer tokens, arbitrary
provider error bodies, or unvalidated retrieved passages.

## Lifecycle and storage

Events are regular, stored, and append-only. A new lifecycle record for a run
MUST reference its current head through `previous`. A conflicting predecessor
is rejected locally. Neither relay nor client may replace or overwrite an
earlier record.

The client signs and commits the exact event and event ID to a protected
SQLite WAL spool before reporting local completion. Relay unavailability
leaves the record queued. Reconnect republishes the exact signed bytes
idempotently by event ID with bounded retry metadata.

## Access and privacy

Relays require NIP-42 authentication. `REQ`, `COUNT`, NIP-50 search,
kindless-ID queries, live fan-out, and archive reads MUST reveal neither
existence nor content unless the authenticated reader equals the sole `p`
owner.

Kind `44210` has a NULL full-text-search vector. Relays do not decrypt it.
Clients decrypt only after the current unlocked identity proves that it is
both the event author and the `p` recipient. Consumers receive a validated
Command Brief view model, never raw arbitrary JSON.

## Retention and backup

The encrypted signed event is suitable for normal relay retention. The local
spool is owner-scoped, permission-protected, backup-compatible SQLite. Backup
and restore preserve exact signed event bytes, publish state, predecessor
links, and bounded retry state.

## Forward compatibility

Unknown versions, lifecycle states, classifications, tags, or fields fail
closed. A future incompatible shape requires a new payload version and must
retain the owner-only read and result-level access gates.
