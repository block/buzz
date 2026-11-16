# Offline account cleanup

`buzz-admin user-deletions account-batches` previews or deletes supported account
records. Preview is the default. These are operator commands against the selected
`DATABASE_URL`; stop relay instances and external writers before capturing scope
and keep them stopped through deletion. The command does not stop them for you.

## Capture the human and agents before deleting ownership evidence

```sh
buzz-admin user-deletions account-scope --pubkey "$USER_PUBKEY" > account-scope.json
buzz-admin user-deletions account-batches --scope-file account-scope.json \
  --actor "$OPERATOR_PUBKEY" --read-only
buzz-admin user-deletions account-batches --scope-file account-scope.json \
  --actor "$OPERATOR_PUBKEY" --delete
```

Review and retain the same scope file for every retry. It is an explicit offline
operator input, not a signed workflow approval. Scope capture reads one database
snapshot without writing progress, source data or caches. It includes the human
in **every stored community**, so former membership and a lack of membership do
not hide residue. It also includes agents recorded as owned by that human in
each community. Agent ownership in one community does not prove ownership in
another community or exclusive ownership of deployment-global records.

The human's global pass runs even when there are no communities. Global agent
records remain retained until their ownership can be established; they are not
silently selected from a community-scoped owner link. If explicit ownership has
been established separately, the single-key global command below is available.

A saved scope containing an owner membership is refused before any mutation.
Resolve the owned-community handoff first. Scope generation is bounded to 1,000
communities and 10,000 total human/agent entries; exceeding either limit fails
without returning a partial scope. The scope file is limited to 4 MiB.

## Explicit smaller scopes

```sh
# One human or agent in one community; never touches global roles/listeners.
buzz-admin user-deletions account-batches --pubkey "$SUBJECT_PUBKEY" \
  --community "$COMMUNITY_ID" --read-only

# Explicit deployment-wide authority; --actor identifies the audit actor.
buzz-admin user-deletions account-batches --pubkey "$SUBJECT_PUBKEY" \
  --global --actor "$OPERATOR_PUBKEY" --read-only
```

Replace `--read-only` with `--delete` to execute. `--print-only` is another preview
spelling. The modes conflict rather than letting one silently override another.
Global mode and community mode are mutually exclusive.

| Scope | Supported operations |
| --- | --- |
| Community | Target-owned tokens, created invitations, all channel/community memberships (including removed rows), policy acceptances, allowlist entries, read frontiers and read-account rows. Existing nullable attribution clears preserve other owners' records and revocation/removal state. |
| Community feedback | Delete the target's submitted feedback in that community, including its body and tags. |
| Personal events | Delete all stored versions, including soft-deleted revisions, of directly authored kinds **3, 10000, 10001, 10002, 10003, 10030, 30000, 30003, 30315, 30078**. These cover contacts, mute/pin/bookmark/follow lists, relay preferences, personal emoji lists, status and application/read state. |
| Personal-event copies | Before deleting source events, delete matching mention-index rows, push-match jobs, push-wake outbox rows, listener outbox rows and delivery logs in the same community. Other people's signed references and reactions are preserved. |
| Global | Delete the selected key's database operator/moderator role, listener registrations in either listener or target position, and submitted feedback across communities and with NULL community. |

Role revocation and its audit row commit together using the existing roster
lock and last-operator guard. This offline command conservatively requires a
surviving database operator; it cannot prove another relay's configuration.
Known local `RELAY_OPERATOR_PUBKEYS` / owner-fallback authority blocks deletion.
Absence from the CLI's environment is **not** proof that deployed configuration
has been cleared. Configuration-backed privileges must be removed separately.
Other principals' `added_by` attribution and prior audit history remain intact.

## Progress and retry

`--batch-size` bounds each transaction to 1–100 selected row operations.
`--max-items` bounds an invocation to 1–1,000,000 operations, across all keys and
communities when using a scope file. A role revocation also writes one audit row
atomically. Each proposed batch is printed before mutation; `batch_committed`
appears only after commit. Output failure before mutation rolls that batch back.

Exit 0 means the supported selectors were exhausted; exit 2 means the row budget
was reached; other failures return an error. A scope ending exactly on its budget
may conservatively return 2 if later scopes have not been inspected. Rerun the
same command and scope file: it selects remaining rows, does not rediscover
ownership, and does not append another audit entry for an already removed role.
Preview uses read-only transactions; a multi-scope preview uses one snapshot per
scope. The offline writer-stop requirement makes those observations stable.

## Deliberate limits

See [the retention decisions](operator-user-account-retention.md) for records we
keep. The start output distinguishes retained replay protection from deferred
profile work. Replay watermark deletion is not a missing feature.

This expansion does not reconcile shared signed membership snapshots, remove
relay-signed membership notices, infer device ownership, or prove repository
owner/delegated access has been revoked. It does not delete shared emoji sets,
profiles, messages, repository events or unknown personal event kinds. Those
require their named handler or an attribution/reconciliation implementation.
Target notification subscriptions and push leases belong to content cleanup;
capture any device attribution before that handler removes the source links.

The existing `archived_identities.replaced_by` clear is still present; moving it
belongs to the profile handoff. The saved workflow adapter still consumes its
approved single-key/community inputs. It does not silently broaden an existing
approval to this new scope file or run deployment-global cleanup as a community
stage. Final coordinator integration and independent verification remain separate.
Every result therefore keeps **`account_erasure_complete: false`**.

This is application code over existing tables: no schema migration, backfill,
trigger, function or foreign key is added. Normal serving behavior and existing
operator API calls retain their existing database schema contract.
