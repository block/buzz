# Operator identity retirement

A deployment Operator can retire an exact key in one community: archive it and
its verified agents (NIP-IA), and hold their workflow automation. Visibility and
automation are separate states with separate commands:

| State | Set by | Cleared by |
|-------|--------|------------|
| NIP-IA archive (`archived_identities`) | archive | unarchive, or a signed NIP-IA self/owner/admin unarchive |
| Execution hold (`identity_retirement_holds`) | archive | release only |

Unarchiving never resumes automation: a key can be visible again and still held.

Read the ownership preview first ([operator-identity-inventory.md](operator-identity-inventory.md)).

## Enabling

Mutations answer `503 identity_retirement_disabled` unless the relay runs with
`BUZZ_ADMIN_IDENTITY_RETIREMENT=enabled` (unset or `disabled` keeps them off).
Enable it only after every serving relay runs a release that contains this
feature: an older relay does not enforce the execution hold, so it could start
runs for a held key.

## Requests

Every route requires a NIP-98 signed Operator (Moderators get 403), the community
query `?communityHost=<host>`, and a JSON body. Unknown body fields are rejected.
`requestId` is the caller's idempotency key, unique per community. `reason` is
optional, 1–64 characters of `a-z`, `0-9` and `-`.

```http
POST   /api/admin/v1/members/<hex>/archive?communityHost=<host>
       {"requestId": "<uuid>", "inventoryVersion": "<from the preview>",
        "reason": "left-company", "replacedBy": "<hex>"}
DELETE /api/admin/v1/members/<hex>/archive?communityHost=<host>
       {"requestId": "<uuid>", "reason": "returned"}
DELETE /api/admin/v1/members/<hex>/execution-hold?communityHost=<host>
       {"requestId": "<uuid>"}
```

`<hex>` is the 64-character lowercase key.

### Archive

In one transaction, serialized against agent-owner link writers and every
workflow and run writer, the relay:

1. re-reads the inventory (the key plus agents whose recorded owner is that key,
   and their workflows). More than 200 identities or workflows: `422
   inventory_too_large`. A different version than `inventoryVersion`: `409
   inventory_changed`; re-read the preview and confirm again.
2. archives each key with `consent=relay` (rows store `''` as the request event),
   and holds each key;
3. moves `active` workflows to `archived`, recording which ones it changed;
4. cancels `pending`, `waiting_approval` and `running` runs and expires pending
   approvals.

Then it publishes one `kind:8002` delta per newly archived key (no `e` tag, no
consent actor, per NIP-IA) and the `kind:13535` snapshot. `replacedBy` is
recorded on the root only. The target must not be relay staff (`409
target_is_staff`).

### Unarchive

Removes the target key's NIP-IA archive only, emits `kind:8003` if it changed,
and republishes the snapshot. Holds, workflows and agents are unchanged.

### Release

Removes the target key's hold and restores the workflows this key's retirement
archived, but only those still `archived` and not updated since. Others are
reported with `"skipped": "modified_since_retirement"` (or `status: "deleted"`).
Release is per key: releasing a root does not release its agents, and an agent
stays blocked while its owner is held (`newWorkAdmission: "blocked"`).

## Holds

While a key or its recorded owner is held, the relay refuses to create or update
its workflows and to start runs for them, on every trigger path (events,
schedules, webhooks, manual triggers). A cancelled run is never moved out of
`cancelled`; the executor checks before each step and stops. A run that was
executing when cancelled counts as draining (`drainingRuns` in the preview)
until its executor's next write records `completed_at`. The check is a read
before each step, so a step that started before (or races with) the
cancellation can still finish. If the executing relay process stops, nothing
records `completed_at` and the run stays draining; treat a long-draining run as
unconfirmed and check the process it ran on.

The relay cannot stop agent processes or external hosts; `agentRuntime` is
always `unsupported`.

## Responses

`200` when the NIP-IA snapshot is published (or the action does not affect
visibility), `202` when publication is pending. Repeat the same request to
retry it. `visibility: "confirmed"` means the `kind:13535` snapshot, the state
clients converge on, reflects the action. The per-key `kind:8002`/`kind:8003`
deltas are best-effort history: a failed delta is logged and never re-sent,
because a late delta could contradict a later action. Body:

```json
{
  "actionId": "<uuid>", "requestId": "<uuid>", "operation": "archive",
  "replayed": false, "communityHost": "community.example", "pubkey": "<hex>",
  "identities": [{"pubkey": "<hex>", "role": "root", "archived": true,
                  "executionHeld": true, "changed": true}],
  "workflows": [{"id": "<uuid>", "ownerPubkey": "<hex>", "status": "archived",
                 "changed": true}],
  "runs": {"cancelled": 2, "approvalsExpired": 1, "inFlight": 1},
  "stages": {"visibility": "confirmed", "newWorkAdmission": "blocked",
             "workflows": "confirmed", "inFlightRuns": "draining",
             "agentRuntime": "unsupported"}
}
```

| Stage | Values |
|-------|--------|
| `visibility` | `confirmed`, `pending`, `unchanged` (release) |
| `newWorkAdmission` | `blocked`, `released`, `unchanged` |
| `workflows` | `confirmed`, `partial`, `unchanged` |
| `inFlightRuns` | `none`, `draining` |
| `agentRuntime` | `unsupported` |

The result is stored with the action. Repeating the same `requestId` with the
same body, operation, target and signer returns it with `"replayed": true` and
re-applies nothing; it only republishes the current snapshot if the first
attempt did not. Any difference is `409 request_id_conflict`.

Errors use the admin envelope `{"error": {"code", "message", "requestId"}}`:
`400 invalid_body | invalid_pubkey | invalid_reason | invalid_replaced_by |
unknown_community_host`, `403`, `409 target_is_staff | request_id_conflict |
inventory_changed`, `422 inventory_too_large`, `503 identity_retirement_disabled |
retry_later`. `retry_later` means the transaction timed out waiting for a lock
and nothing was applied; repeat the same request.

## Rollout

Migration 0087 adds three community-scoped tables and widens the consent check
without validating it; 0088 validates it. A relay that predates the precursor
release ([#9304](https://github.com/block/buzz/pull/9304)) fails community
deletion closed for every community once 0087 exists, because its deletion
catalog check finds tables it does not know. The precursor knows these tables before they exist: it deletes
communities with them absent or present, and a deletion it or an older release
approved before 0087 still executes afterwards and purges their rows. Roll out
in this order:

1. Deploy the precursor everywhere that runs community deletion or reads the
   schema: every relay, the deletion executor (including any CronJob image),
   and the `buzz-admin` used by operators. Once 0087 is applied, the precursor
   is the compatibility floor: never run an older release against the
   database again.
2. Run relays with auto-migration off (`migrate.autoMigrate=false` in the Helm
   chart, `BUZZ_AUTO_MIGRATE` unset or false) and keep it off through every
   restart and rollback. With it on, a precursor relay that restarts after
   0087 fails at startup on the migrations it does not know (sqlx
   `VersionMissing`, Rule 5 in [database.md](database.md)).
3. Apply 0087 out of band with `buzz-admin migrate` from this release, and
   restart one precursor relay to confirm it still starts and serves.
4. Deploy this release. It requires 0087 before it serves, so never start it
   against a database without it. Apply 0088 once every relay runs it.
5. Keep `BUZZ_ADMIN_IDENTITY_RETIREMENT` off until every serving relay runs
   this release (see *Enabling*).

Rolling back from this release to the precursor keeps community deletion
working, but the precursor does not enforce execution holds. It is an
acceptable feature rollback only before the first retirement. After a
retirement, roll back only to a release that enforces holds, or to separately
validated containment that fails closed. Turning
`BUZZ_ADMIN_IDENTITY_RETIREMENT` off stops new retirements; it does not enforce
existing holds.
