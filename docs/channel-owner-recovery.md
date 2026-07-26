# Channel-owner recovery

Buzz has a dedicated recovery command for the narrow case where every current
human channel owner recorded durable self-consent for the same replacement
before becoming unavailable. Recovery preserves the channel UUID, messages,
threads, canvas, memberships, and workflow references. It promotes one existing
member; it does not delete, demote, or archive anyone.

This policy does not recover a lost or deleted owner key unless the required
prior self-consent already exists. Presence, last-seen data, an administrator
assertion, or archive state applied by someone else are not eligibility proof.

## User flow

The actor must be an active, known-human community owner. The target must be an
active, known-human member of both the same community and the channel, must not
already be an owner or administrator, and must not be archived. Recovery is
also denied while any active channel administrator or owner/administrator agent
can use an ordinary governance path.

Each current human channel owner must previously have submitted a self-signed
NIP-IA archive request (`kind:9035`) with `replaced-by` naming the exact target.
When that evidence exists, use:

```bash
buzz channels recover-owner \
  --channel <channel-uuid> \
  --pubkey <replacement-pubkey-hex> \
  --reason "Why exceptional recovery is required"
```

The reason is required, becomes part of the immutable audit, and is limited to
500 bytes without control characters. A successful command adds the target as
an owner. Removing a retired identity remains a separate ordinary membership
action after recovery.

Desktop exposes the same dedicated action in channel management with an
explicit confirmation. Generic role management continues to reject owner
grants.

## Event contract

Recovery is a protected, user-signed `kind:9038` event:

```jsonc
{
  "kind": 9038,
  "content": "",
  "tags": [
    ["-"],
    ["h", "<channel-uuid>"],
    ["p", "<replacement-pubkey-hex>"],
    ["reason", "<human-readable audit reason>"]
  ]
}
```

The event requires exactly one tag of each shape above and rejects additional
or duplicate tags. The relay resolves the community from the connection or
HTTP host; no client-supplied community identifier is accepted. Requests must
be within the relay's 120-second freshness window. Exact cryptographic replays
remain idempotent after a successful commit so pending audit delivery can
converge.

Clients submit the event through the normal WebSocket `EVENT` path or
`POST /events`. The command is deliberately executed before generic event
storage because the request event, owner promotion, immutable database audit,
and delivery outbox must commit in one transaction. The dedicated handler
performs the request insert inside that transaction. A denial stores no request,
does not promote the target, and does not create audit or outbox state. This
pre-storage exception requires maintainer approval with the recovery predicate.

After commit, the relay publishes a deterministic, relay-signed `kind:40099`
channel system event. It contains the actor, target, prior elevated membership,
predicate identifier, reason code, reason, request ID, and transaction
timestamp, and carries:

```jsonc
["audit", "channel-owner-recovery-v1"]
```

The matching database audit row cannot be updated or deleted. Audit delivery
atomically stores the relay event and records its exact event ID in the outbox;
only that community-scoped durable linkage makes the visible event immutable.
This keeps audits signed before relay-key rotation protected without trusting
marker shape alone. Public ingest reserves the marker, while unlinked lookalikes
accepted by an older relay remain deletable during a rolling upgrade. Linked
audits are rejected as targets of both NIP-29 `kind:9005` and standard NIP-09
`kind:5` deletion, including by the newly promoted owner.

## Relay configuration and operations

The delivery worker retries committed audits independently of promotion:

| Variable | Default | Meaning |
|---|---:|---|
| `BUZZ_RECOVERY_AUDIT_INTERVAL_SECS` | `10` | Seconds between outbox scans; values below 1 become 1. |
| `BUZZ_RECOVERY_AUDIT_BATCH_LIMIT` | `100` | Pending audits requested per scan; values are clamped to `1..=1000`. |

Operators should monitor relay warnings containing
`pending owner recovery audit delivery failed` and inspect
`channel_owner_recovery_outbox` rows whose `delivered_at` is null. The
`attempts` and `last_error` columns retain retry state. Reverting application
code leaves the additive audit/outbox tables and immutability trigger in place;
do not drop them while any audit or pending delivery must be retained.
