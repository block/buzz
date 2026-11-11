# Operator identity retirement

A deployment Operator can retire an exact key in one community: archive it and
its verified agents (NIP-IA), and hold their workflow automation. Visibility and
automation are separate states with separate commands:

| State | Set by | Cleared by |
|-------|--------|------------|
| NIP-IA archive (`archived_identities`) | archive | a signed NIP-IA unarchive only (see [Clearing an archive](#clearing-an-archive)) |
| Execution hold (`identity_retirement_holds`) | archive | release only |

There is no operator unarchive. Release never clears an archive, and an
unarchive never resumes automation: a key can be visible again and still held,
or released and still archived.

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
DELETE /api/admin/v1/members/<hex>/execution-hold?communityHost=<host>
       {"requestId": "<uuid>"}
```

`<hex>` is the 64-character lowercase key.

### Operator API routes

A deployment operator that reaches the relay at `RELAY_OPERATOR_API_ORIGIN`,
for example over an internal service mesh, uses the same operations under
`/operator/communities/members`, with the same query, bodies and responses:

```http
GET    /operator/communities/members/search?communityHost=<host>&q=<text>&limit=<n>
GET    /operator/communities/members/<hex>/ownership?communityHost=<host>
POST   /operator/communities/members/<hex>/archive?communityHost=<host>
DELETE /operator/communities/members/<hex>/execution-hold?communityHost=<host>
```

These routes do not use `BUZZ_ADMIN_HOST` or the inbound `Host`. Only keys in
`RELAY_OPERATOR_PUBKEYS` are accepted (Operators added through the admin
roster are not). The NIP-98 `u` tag must be `RELAY_OPERATOR_API_ORIGIN`
followed by the path and query exactly as sent, so the signature covers
`communityHost`; every mutation needs a `payload` tag for its exact body,
`DELETE` included. Bodies are limited to 4096 bytes.

An authorization refusal uses the operator API's envelope, `{"error":
"<text>", "code": "unauthorized" | "not_operator"}` (401 or 403) and applies
nothing. Every other error uses the lifecycle envelope described under
*Responses*. A proxy error without either envelope (such as a gateway 502,
503 or 504) or a lost response is an unknown outcome: repeat the same request
to get the stored result. Only the lifecycle `503 retry_later` means the
transaction rolled back.

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

### Release

Removes the target key's hold and restores the workflows this key's retirement
archived, but only those still `archived` and not updated since. Others are
reported with `"skipped": "modified_since_retirement"` (or `status: "deleted"`).
Release is per key: releasing a root does not release its agents, and an agent
stays blocked while its owner is held (`newWorkAdmission: "blocked"`) until the
root is released too.

Release never changes NIP-IA visibility and publishes nothing, so it always
answers `200` with `stages.visibility: "unchanged"`. Only release uses that
value; archive answers `confirmed` or `pending`. The response lists
exactly one identity, the target, with `role: "root"`.

## Clearing an archive

Only a signed NIP-IA `kind:9036` unarchive request ([NIP-IA](nips/NIP-IA.md))
removes a key's archive; the relay then publishes `kind:8003` and the
snapshot. The relay accepts the request when the key itself, its NIP-OA owner
or a community `owner`/`admin` signs it, whatever archived the key, including
this feature's `consent=relay` archive. It removes the archive row only; the
execution hold stays until release. Nothing else clears it: release, a new
profile, a new agent-owner link and re-joining the community do not.
Community deletion removes it with the community.

The archive and the hold are keyed by community and pubkey, not by account,
and the relay knows nothing of account bindings. Moving a person to a new key
gives them a key that is neither archived nor held, while the retired key stays
archived and held.

**Known gap: a restored key stays retired.** If a deployment later binds an
account back to a key retired here (for example, account recovery that
restores an earlier key), nothing tells the relay: the key stays hidden and
held in every community where it was retired. Returning it takes two separate
steps: a signed NIP-IA unarchive (by the key, its owner or a community admin)
for visibility, and release for automation. This feature adds neither step to
recovery.

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

A held key also cannot gain agents. Recording a new agent-owner link to a held
owner is refused: NIP-OA authentication fails with the owner-link error, and
an invitation claim that presents the held owner is refused as restricted. A
link recorded before retirement is unchanged and keeps working. The hold is
checked under the same admission lock retirement takes, so a link racing a
retirement is either recorded first (and retired with it) or refused.

The relay cannot stop agent processes or external hosts; `agentRuntime` is
always `unsupported`.

## Responses

`200` when the NIP-IA snapshot is published (or the action does not affect
visibility), `202` when publication is pending. Repeat the same request to
retry it. `visibility: "confirmed"` means the `kind:13535` snapshot, the state
clients converge on, reflects the action. The per-key `kind:8002` deltas are
best-effort history: a failed delta is logged and never re-sent,
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

## Routing

The relay sees only the caller's NIP-98 request. An admin portal in front of it
has two separate hops, and each needs its own routing check:

1. browser → portal: the portal's own pages and form posts;
2. portal → relay: `GET` ownership, `POST` archive and `DELETE`
   execution-hold, each mutation with a JSON body.

On the portal → relay hop, every proxy and ingress must forward the method,
the exact body bytes (including on `DELETE`) and the path and query unchanged.
The relay checks the signed URL against configuration, never the inbound
`Host` header: `BUZZ_ADMIN_HOST` for the admin routes (which also require that
`Host`), `RELAY_OPERATOR_API_ORIGIN` for the operator routes. A dropped
`DELETE` body answers `400 invalid_body`; a changed body or URL fails NIP-98
(`401`). Both fail closed, but leave keys held that cannot be released. Before
enabling mutations, run release through the real ingress, not only against
the relay directly.

A proxy or client deadline can expire while the relay is still working: the
transaction waits at most 5 s for each lock and 15 s for each statement, and
publication follows it, so there is no fixed bound on the whole request. Treat
a timed-out request as an unknown outcome and repeat it.

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
`BUZZ_ADMIN_IDENTITY_RETIREMENT` off refuses every mutation, release included,
and changes no stored state. Existing holds stay until the flag is on again and
release runs; existing archives stay until a signed NIP-IA unarchive, which does
not depend on the flag.
