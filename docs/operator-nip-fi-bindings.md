# NIP-FI Community Issuer Bindings

A deployment that provisions communities under one host suffix, for example
`<name>.communities.buzz.xyz`, stores the NIP-FI issuers allowed to vouch for
each community in `community_nip_fi_issuers`. The relay writes these rows when
it creates a community; `buzz-admin nip-fi` backfills and edits them.

To make an existing community require federated sign-in, use the separate
[community policy management command](operator-nip-fi-upgrade.md).
An issuer binding alone does not change that community's required policy.

A host may hold a binding only when it is exactly one DNS label under the
suffix, has no port, and is not *reserved*. A host is reserved when
`https://<host>` is a `command_audiences` entry of any issuer in
`BUZZ_NIP_FI_ISSUERS`, so a command audience can never become a community
assertion audience. The relay refuses to start while a reserved host holds a
binding, and the operator lookup route reports no issuers for a host that is not
bindable or for an issuer that is not configured.

A community with durable `requires_nip_fi=true` always retains its last issuer.
The internal `allow_last` exception for unbindable hosts cannot override this
policy. Bind a replacement first, or archive/delete the community for containment.
Bulk backfill skips required communities whose bindings are missing; its closing
report still counts them as unbound. Repair those bindings with an explicit bind.

## Configuration

`BUZZ_NIP_FI_PROVISION_BINDING` is read in every NIP-FI mode. Unset or empty
binds nothing.

```json
{
  "host_suffix": "communities.buzz.xyz",
  "authorized_issuers": ["https://app.builderlab.xyz/api/goose"]
}
```

- `host_suffix` is a lowercase domain name without a port or trailing dot.
- `authorized_issuers` is non-empty, and every entry must be an `issuer` in
  `BUZZ_NIP_FI_ISSUERS`. Each issuer is 1 to 2048 bytes.
- The suffix must not cover any `BUZZ_NIP_FI_COMMUNITIES` host, and every
  `canonical_uri` there must be readable. The relay and `buzz-admin` both
  refuse to start otherwise.

`buzz-admin` parses the same variables with the relay's parser. Run it with the
relay's environment: `BUZZ_NIP_FI_PROVISION_BINDING`, `BUZZ_NIP_FI_ISSUERS`,
`BUZZ_NIP_FI_COMMUNITIES`, and `DATABASE_URL`.

### Database community source

`BUZZ_NIP_FI_COMMUNITY_SOURCE=database` resolves communities from these
bindings instead of `BUZZ_NIP_FI_COMMUNITIES`. The relay refuses to start if
`BUZZ_NIP_FI_PROVISION_BINDING` is unset or `BUZZ_NIP_FI_COMMUNITIES` is set.
In `off` mode no snapshot is loaded.

| Variable | Default | Bound |
|----------|---------|-------|
| `BUZZ_NIP_FI_COMMUNITY_SOURCE` | `env` | `env` or `database` |
| `BUZZ_NIP_FI_SNAPSHOT_MAX_ROWS` | 1,000,000 | 1 to 10,000,000 community-issuer rows; a larger load fails |
| `BUZZ_NIP_FI_SNAPSHOT_REFRESH_TIMEOUT_MS` | 10,000 | under the 30s refresh interval |
| `BUZZ_NIP_FI_RESOLVER_NEGATIVE_CACHE_MAX` | 100,000 | 1 to 1,000,000 hosts |
| `BUZZ_NIP_FI_RESOLVER_MAX_INFLIGHT` | 8 | at least 1, clamped below the database pool size |

Each pod loads a snapshot before it binds any listener, and refreshes it
every 30s. A removed binding stops admitting new connections within 30s plus
the configured refresh timeout (40s by default) while refreshes succeed, and within two minutes of the last good load while they
fail. Open WebSocket sessions are not rechecked when a binding is removed;
they end at their own session deadline. Alert on
`time() - buzz_nip_fi_binding_snapshot_loaded_timestamp_seconds > 120` and on
`absent(buzz_nip_fi_binding_snapshot_loaded_timestamp_seconds)`.

## Initial policy on operator creation

A trusted NIP-98 operator may send `initial_requires_nip_fi: true` with
`create_only: true` to `POST /operator/communities`. The community, owner,
issuer bindings and required policy (generation 1) commit in one transaction.
Omitted, null or false creates an optional community at generation 0. A non-null
policy field without `create_only: true` returns 400
`initial_policy_requires_create_only`.

Required insertion needs an effective provision binding for the host; otherwise
400 `nip_fi_binding_required` leaves no row. Reserved hosts retain the existing
`reserved_host` refusal. The successful response includes the stored
`requires_nip_fi` and `status: "created"` for insertion or `"existed"` for a
same-owner replay. A replay changes neither policy nor bindings, including after
an operator disable or a change in issuer configuration. A different owner
retains the existing conflict. Legacy ensure and deployment bootstrap remain
optional and do not accept an explicit initial policy.

This policy is independent of `BUZZ_NIP_FI_CHANGES`. Mode `off` stores
it but bypasses enforcement. The production creation paths are operator
create-only, legacy operator ensure, and deployment bootstrap; only create-only
accepts this choice.

Upgrade the **whole serving relay fleet** before callers request true. Older
writers may ignore the unknown field and insert an optional row. Checking the
created response for stored true detects that incompatible writer, but cannot
undo its insertion; a later existing-row lookup remains read-only. Turn caller
targeting off before rolling back the relay. Turning it off affects future
insertions only; it does not clear existing policy.

## Commands

```sh
buzz-admin nip-fi backfill
buzz-admin nip-fi bind --host acme.communities.buzz.xyz --issuer https://issuer.example
buzz-admin nip-fi unbind --host acme.communities.buzz.xyz --issuer https://issuer.example
```

`backfill` binds the authorized issuers to every active, unbound, bindable
community, a page of 1000 hosts at a time. Archived communities are bound on
purpose, so unarchiving leaves a community that can log in. A community whose
deletion has started is skipped. It always prints its counts and exits 1 while
any bindable community remains unbound:

```json
{"rows_bound": 12, "unbound": 0, "unbound_hosts": [], "truncated": false}
```

`unbound_hosts` lists at most 100 hosts; `truncated` is true when there are
more.

`bind` refuses a reserved host, a host that is not one label under the suffix,
and an issuer not in `BUZZ_NIP_FI_ISSUERS`.

`unbind` works without `BUZZ_NIP_FI_PROVISION_BINDING`. It refuses to remove a
community's last issuer, because an empty community looks never-provisioned and
the next `backfill` would bind it again. To swap issuers, bind the new one and
then unbind the old one. To stop a community logging in, archive or delete it.
When the provisioning config is present and shows the host is not bindable (a
reserved host or one outside the suffix), `unbind` removes the last issuer too;
this is how to clear a reserved-host binding that blocks relay startup.

`backfill` never adds the default issuers beside an operator `bind`: both lock
the community row, and the backfill only inserts while the community still has
no binding.

## Lookup route

`GET /operator/communities/nip-fi-binding?host=<host>` is operator-signed and
returns `{"host": ..., "active": ..., "issuers": [...]}`. kgoose relies on this
contract:

- `host` is the host stored on the community row.
- Every lifecycle state answers 200. `active` is `false` once the community is
  archived or its deletion has started quiescing, including a tombstoned
  community; it stays `true` while a deletion is only requested.
- `issuers` is `[]` when `BUZZ_NIP_FI_PROVISION_BINDING` is unset, the host is
  not bindable, or nothing is bound. Only issuers in `BUZZ_NIP_FI_ISSUERS` are
  reported, in byte order.
- 404 means no community has this host; 400 means the host is malformed.

## Rollout

1. Deploy the relay with `BUZZ_NIP_FI_PROVISION_BINDING` set. New communities
   are bound at creation from then on.
2. Run `buzz-admin nip-fi backfill` with the relay's environment. Repeat it until
   it exits 0.
3. Do not turn on the database binding source in enforce mode while any
   bindable community is unbound: an unbound community fails closed.

## Database source runbook

`BUZZ_NIP_FI_COMMUNITY_SOURCE=database` reads communities from these bindings
instead of `BUZZ_NIP_FI_COMMUNITIES`. Move one mode at a time, and only move
forward when the step's go signals hold.

### Off

Set `BUZZ_NIP_FI_COMMUNITY_SOURCE=database` with `BUZZ_NIP_FI_MODE=off`. No
snapshot loads in `off`, but every `BUZZ_NIP_FI_RESOLVER_*` and
`BUZZ_NIP_FI_SNAPSHOT_*` value is validated, so a bad one fails this deploy.

- Go: the rollout completes, and `buzz-admin nip-fi backfill` exits 0.
- No-go: a pod fails config validation. Fix the named variable.

### Shadow

Set `BUZZ_NIP_FI_MODE=shadow`. Each pod now loads the snapshot before any
listener binds, refreshes it every 30 s, and records what enforce would decide
without refusing anything. Shadow labels database-source records
`community="bound"` or `"unmapped"`, never a host.

- Go, held for at least a day of normal traffic:
  - `time() - buzz_nip_fi_binding_snapshot_loaded_timestamp_seconds` stays
    under 60 on every pod.
  - `buzz_nip_fi_binding_snapshot_load_failures_total` is flat.
  - `buzz_nip_fi_binding_snapshot_rows / buzz_nip_fi_binding_snapshot_max_rows`
    is under 0.5.
  - `buzz_nip_fi_resolver_miss_rejections_total` is flat.
  - The non-`admit` share of `buzz_nip_fi_shadow_total` is only what you expect
    to deny; `community="unmapped"` comes only from hosts you do not serve.
  - `stage="owner"` counts as would-deny: each is an agent enforce will refuse
    for its token owner.
- No-go: any of these fails. Return to `off` and investigate.

### Enforce

Set `BUZZ_NIP_FI_MODE=enforce`. Watch the same signals, plus the rate of 503
responses on login.

- Go: the shadow signals keep holding, and 503s stay at their shadow baseline.
- No-go: logins return 503 or are refused unexpectedly. Set `mode=off` (below).

### Alerts

- Stale snapshot: `time() - buzz_nip_fi_binding_snapshot_loaded_timestamp_seconds > 120`,
  together with `absent(buzz_nip_fi_binding_snapshot_loaded_timestamp_seconds)`
  for a database-source pod.
- Near the row cap: `buzz_nip_fi_binding_snapshot_rows / buzz_nip_fi_binding_snapshot_max_rows > 0.8`.
  `buzz_nip_fi_binding_snapshot_rows` is the size of the last good load.
- Over the cap or too slow: any increase of
  `buzz_nip_fi_binding_snapshot_load_failures_total{reason=~"over_cap|timeout"}`.
- Live lookups shed: any increase of `buzz_nip_fi_resolver_miss_rejections_total`.

### When the snapshot is too big or too slow

A load fails when the snapshot has more rows than
`BUZZ_NIP_FI_SNAPSHOT_MAX_ROWS` or takes longer than
`BUZZ_NIP_FI_SNAPSHOT_REFRESH_TIMEOUT_MS`. Both get worse as communities are
added, so they usually arrive during normal growth, not at a deploy.

- New pods: the startup load fails and the process exits, so new pods
  crash-loop and a rollout stalls. Boot already fails when the database is
  unreachable; what this source adds is that snapshot size and load time can
  also fail it.
- Running pods: their refreshes fail but the last good snapshot keeps
  answering. Once it is 120 s old it is stale, and every login's host goes to
  a live lookup. Live lookups share `BUZZ_NIP_FI_RESOLVER_MAX_INFLIGHT` slots
  (8 by default). Concurrent logins for the same host share one lookup, and a
  login waits up to 50 ms for a free slot; how many are shed depends on login
  load across distinct hosts and on lookup latency. In enforce, a shed login
  gets 503 (`authorization_unavailable`); in shadow it
  is only recorded as a would-deny.

Ways out, in order:

1. Raise `BUZZ_NIP_FI_SNAPSHOT_MAX_ROWS` (up to 10,000,000) or
   `BUZZ_NIP_FI_SNAPSHOT_REFRESH_TIMEOUT_MS` (under 30,000), whichever the
   failure `reason` names, and roll out.
2. If that cannot land quickly, set `BUZZ_NIP_FI_MODE=off`. `off` loads no
   snapshot, so pods boot and admit as they did before NIP-FI.

## Revocation during a deploy

An accepted `POST /api/nip-fi/disconnect` is written to Redis before the relay
answers 200, and each entry expires at its `until`. A pod that starts, or that
loses its Redis subscription, restores every active entry before it admits a
NIP-FI assertion again. Until then it refuses every assertion-bearing
connection, so a key revoked just before a deploy can't come back in on a
fresh pod.

What this covers, and what it does not:

- **Covered:** pod restarts, and a pod that loses its Redis subscription or
  falls behind on it.
- **Not covered:** a pod that stays healthy still learns of a disconnect only
  through live pub/sub, which is best effort. If a publication never reaches
  it, that pod neither closes the target's open sessions nor refuses the
  target at its own admission check until its next restore.
- **Not covered:** losing Redis data. Run Redis with a non-evicting
  `maxmemory-policy` of `noeviction`, with persistence if you need
  revocations to survive a Redis restart. Every deny key carries an expiry,
  so any `volatile-*` policy can evict it.
- **Redis down at startup:** if Redis is completely unreachable at boot, the
  relay exits before it binds, and the process supervisor restarts it. If
  Redis is reachable but the restore fails, the pod runs and refuses
  assertion-bearing connections with `503` (authorization unavailable) until
  it has restored. Watch `buzz_nip_fi_deny_restore_failures_total{reason}`.
- **Unreadable deny record:** `reason="unreadable"` means Redis answered but
  a stored deny record could not be decoded: its field is not a hex pubkey,
  or its value is not exactly `secs:nanos`. The pod still applies the
  issuer's other records and closes their sessions, but stays closed until the
  record is removed. The warning log names the record. Find it in the
  issuer's deny hash, `buzz:nip-fi:deny-until:<sha256(issuer) hex>`
  (`buzz:nip-fi:shadow-deny-until:…` in shadow mode), with `HGETALL`, then
  `HDEL` that field. The pod retries on its own. Removing the record drops
  that revocation, so send the disconnect again if it is still needed.
  A disconnect with `revoke_through` stores its cutoff in a second hash,
  `buzz:nip-fi:deny-revoke-through:<sha256(issuer) hex>`, under the same
  field (`buzz:nip-fi:shadow-deny-revoke-through:…` in shadow mode). A
  cutoff is also unreadable when it is not an integer, or when its field has
  no entry in the deny hash. Recover with `HDEL` on the cutoff hash, but the
  two cases differ:
  - The cutoff is malformed and its field is also in the deny hash: after
    the `HDEL`, that entry still refuses every token for the key until its
    `until`.
  - The cutoff is orphaned (its field is not in the deny hash): there is no
    deny underneath it, so after the `HDEL` that key is not revoked at all.
    Send the disconnect again if the revocation is still needed.
  A new disconnect for a key whose stored cutoff is unreadable also fails
  with `503` and records nothing until that `HDEL`; then send it again.
- **Deny set over capacity on a pod:** if a revocation this pod receives does
  not fit its `deny_set_capacity`, the pod closes admission the same way and
  restores until its active entries fit.

### Disconnect responses

- `403` is a replay: this command's `jti` was already used.
- `503 deny set full` means the issuer's shared deny set is full. Nothing was
  recorded, so the same command can be retried.
- `503 authorization unavailable` after a Redis error means the outcome is
  unknown. If Redis applied the command before the reply was lost, the deny is
  already stored and its `jti` is used up, so a retry returns `403`. Send a
  freshly signed command to get a `200`.

### Capacity

`deny_set_capacity` applies to each issuer across all pods, not per pod. If a
restore doesn't fit a pod's map, the pod stays closed and
`buzz_nip_fi_deny_restore_failures_total{reason="capacity"}` increases. This
can happen during the shadow → enforce switch below, when an enforce pod
restores both stores. To recover, either:

- raise `deny_set_capacity` to at least the sum of the issuer's live entries
  and roll out; or
- wait for entries to expire (at most one assertion lifetime). The pod retries
  the restore on its own.

Count live entries for an issuer with
`ZCOUNT buzz:nip-fi:deny:<sha256(iss)> (<now_ms> +inf` and
`ZCOUNT buzz:nip-fi:shadow-deny:<sha256(iss)> (<now_ms> +inf`, where
`<sha256(iss)>` is the lowercase hex SHA-256 of the exact issuer URI.

### Switching shadow → enforce

Shadow pods write their accepts under `buzz:nip-fi:shadow-deny:*`, and enforce
pods normally never read them. For the deploy that switches modes, the bridge
setting `BUZZ_NIP_FI_ACCEPT_SHADOW_DISCONNECTS=true` makes enforce pods also
subscribe to and restore shadow accepts. It gives disconnects accepted by
shadow pods real effect on enforce pods, so keep the issuer configuration
identical across the switch.

Let `L` be the largest `maximum_assertion_age_seconds + skew_seconds` across
configured issuers.

1. Deploy this relay version while still in `shadow`.
2. Wait until the last pod of the previous version has been gone for at least
   `L`. Older versions keep accepts only in memory, so this wait lets those
   expire before anything relies on the store.
3. Deploy `BUZZ_NIP_FI_MODE=enforce` with
   `BUZZ_NIP_FI_ACCEPT_SHADOW_DISCONNECTS=true`.
4. Keep the setting until both hold:
   - the last shadow pod has been gone for at least `L`;
   - `ZCOUNT buzz:nip-fi:shadow-deny:<sha256(iss)> (<now_ms> +inf` is `0` for
     every configured issuer.
5. Deploy without the setting.

## Assertion policy ID change

This release changes every verified assertion's policy ID once, on purpose.
The base ID (`derive_assertion_policy_id` in `crates/buzz-auth/src/nip_fi/config.rs`)
no longer hashes the configured community audiences. The ID carried on a
verified assertion is now `IssuerPolicy::id_for_community`: a hash of that base
and the one community audience the request resolved to. The issuer is still
committed through the base.

Policy IDs from earlier releases therefore do not match. Inside the relay the ID
is only carried on the verified assertion and never compared with a stored ID, so
admission is unaffected. Anything outside the relay that recorded old policy IDs,
such as dashboards, log queries or allow-lists, should expect new values after
the upgrade.
