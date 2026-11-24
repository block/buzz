# Operator identity archive and unarchive

A deployment Operator can hide an identity and its confirmed direct agents in
one community using [NIP-IA](nips/NIP-IA.md). An Operator can restore visibility
for one selected key at a time. Archive state controls discovery and pickers.
Relay access, agent ownership, and workflow execution are independent.

## Operator API routes

```http
GET  /operator/communities/members/search?communityHost=<host>&q=<text>&limit=<n>
GET  /operator/communities/members/<hex>/ownership?communityHost=<host>
POST /operator/communities/members/<hex>/archive?communityHost=<host>
POST /operator/communities/members/<hex>/unarchive?communityHost=<host>
```

Use a key from `RELAY_OPERATOR_PUBKEYS` and sign the request with NIP-98. The
`u` tag covers `RELAY_OPERATOR_API_ORIGIN` plus the exact path and raw query,
including `communityHost`. For each POST the `payload` tag covers the exact
JSON bytes. Bodies are limited to 4096 bytes. Use a fresh NIP-98 event for each
HTTP attempt. A unique `nonce` tag distinguishes attempts signed within the
same second.

The admin-host API also provides `GET /api/admin/v1/members/<hex>/ownership`
and `POST /api/admin/v1/members/<hex>/archive` with the same community query and
archive body. These routes require a signed Operator and use `BUZZ_ADMIN_HOST`
for the signed origin.

Operator mutations are off by default. Set
`BUZZ_ADMIN_IDENTITY_ARCHIVE=enabled` to enable them. Signed NIP-IA requests
have their own authorization rules.

## Archive and unarchive

First read the [ownership preview](operator-identity-inventory.md). Use its
`inventoryVersion` in the archive body:

```json
{
  "requestId": "d6f6beda-dc88-482b-b899-3222facda63e",
  "inventoryVersion": "<version from the preview>",
  "reason": "duplicate",
  "replacedBy": "<optional 64-character lowercase hex key>"
}
```

The relay re-reads the target and its direct agents in one admitted
transaction. A successful archive inserts missing rows with `consent=relay`
and preserves existing rows. It records `replacedBy` on the target only and
refuses to archive a relay staff target. Use an exact lowercase hex pubkey in
the route.

For unarchive, use the selected key in the route and send a JSON body with
`requestId` and an optional `reason`. Unarchive removes that key's archive
entry. Operators can address an archived agent directly to restore its
visibility. Where supplied, `reason` must contain 1–64 characters drawn from
lowercase ASCII letters, digits, and hyphens.

## Responses and retries

A successful write returns `requestId`, `operation`, `communityHost`, the
target `pubkey`, and `identities`. Each identity reports `pubkey`, `role`
(`root` or `agent`), `archived`, and whether this attempt `changed` it.

The relay tries to emit a `kind:8002` archive delta or `kind:8003` unarchive
delta for each changed key. These deltas use `consent=relay`. It then publishes
the canonical `kind:13535` snapshot, including after a no-op mutation. A
successful snapshot publish returns HTTP 200 with `stages.visibility` set to
`confirmed`. HTTP 202 with `pending` means the write committed and the
snapshot publish failed. Retry the same request to publish again.

`requestId` is a UUID used for correlation. A retry applies the operation
to the current state, so a delayed retry can reverse a more recent change.
A retry can republish the current snapshot after an earlier failure.

- `409 inventory_changed` means the direct identities differ from the
  confirmed preview. Read the preview and confirm again.
- `422 inventory_too_large` means the archive exceeds 200 identities.
- `503 identity_archive_disabled` means the feature is off.
- `503 retry_later` means the store reported a lock or statement timeout.
  The write was rolled back.

## Schema

Migration 0107 widens the `archived_identities` consent check to accept
`relay`, and 0108 validates it. Operator archive rows store an empty
`request_event_id` because they originate from an HTTP action. Both migrations
use bounded locks.
