# Account-table batch command

`buzz-admin user-deletions account-batches` previews or executes the supported
account-record cleanup for **one human in one surviving community**. It is an
offline debugging command. Community selection belongs to its caller.

```sh
export DATABASE_URL=postgresql://localhost/disposable_deletion_test
buzz-admin user-deletions account-batches --pubkey "$HUMAN_PUBKEY" --community "$COMMUNITY_ID"
# Equivalent explicit preview modes:
buzz-admin user-deletions account-batches --pubkey "$HUMAN_PUBKEY" --community "$COMMUNITY_ID" --print-only
buzz-admin user-deletions account-batches --pubkey "$HUMAN_PUBKEY" --community "$COMMUNITY_ID" --read-only
# Execute against a disposable fixture after inspecting the preview:
buzz-admin user-deletions account-batches --pubkey "$HUMAN_PUBKEY" --community "$COMMUNITY_ID" --delete
```

Omitting the mode means print-only. `--delete` conflicts with either preview
flag. Public keys accept hex or npub. `--batch-size` defaults to 100 and accepts
1–100. `--max-items` defaults to 100,000 and accepts 1–1,000,000.

The command passes `ExecutionMode::ReadOnly | Delete` to one entry point:
`run_account_batches(options, mode, writer)`. Both modes share the operation
catalog, selectors, keyset ordering and proposal output. The handler owns
mutation, transaction and deletion-safety decisions. The older preview/delete
functions remain as delegating compatibility wrappers.

Both modes emit newline-delimited JSON. Each `proposed_batch` identifies the
community, table, action and complete row keys within that community. It prints
no credential hashes, token names, or stored payloads. Delete mode flushes the
proposal before mutation, commits, then emits `batch_committed`. A proposal is
never an execution receipt.

## Operations and order

| Order | Operation |
| --- | --- |
| 1 | Delete `api_tokens` owned by the human, including expired and revoked tokens. |
| 2 | Delete `relay_invites` created by the human; redeemed memberships survive. |
| 3 | Delete the human's `channel_members`, including removed memberships. |
| 4 | Delete `join_policy_acceptances`, then the human's `relay_members`. |
| 5 | Delete the human's `pubkey_allowlist` rows. |
| 6 | Delete `personal_read_frontiers`, then `personal_read_accounts`. |
| 7 | Clear nullable provenance on surviving rows: token `revoked_by`; channel member `invited_by` and `removed_by`; relay member and allowlist `added_by`; archive `replaced_by`. |

Every selection binds both community and human identity. Provenance updates
preserve the other person's row, role, revocation/removal timestamps, and archive
visibility. Membership updates advance their normal synchronization timestamps.
Rows deleted in an earlier operation are excluded from later preview updates.

Each table operation selects at most one batch in a transaction, locks its exact
rows and deletes or updates those rows. Parents are locked before children, and
the parent cannot be deleted while any child remains. Database cascades never
stand in for printed child batches. Preview uses keyset pagination in a
repeatable read-only transaction; execution repeatedly selects remaining rows.
Neither mode loads the whole account into memory or uses `OFFSET`.

## Execution and recovery

Stop the relay and all external workers before using `--delete`. The command
holds the community deletion lock across batches and validates the deletion
catalog under the schema lock. It refuses inactive communities, current
community owners, known agent profiles, and live serving-write leases.
These checks do not stop external workers or revoke configured access for you.

Each SQL statement has a 30-second timeout and lock waits have a 3-second
timeout. Each output record is capped at 256 KiB. A failing current batch rolls
back; earlier commits remain. If receipt output fails after commit, the batch
remains committed. Rerun the command to select remaining rows. A lost receipt
does not authorize replaying an imported list of keys.

Exit 0 means the supported operations were exhausted; exit 2 means the item
limit stopped the invocation. Errors exit nonzero. The summary reports
`supported_scope_exhausted`; it always reports `account_erasure_complete:false`.
Even an exhausted invocation does not establish account erasure.

## Explicit boundaries

Print-only uses both a read-only connection and an explicit read-only transaction.
Neither mode runs migrations, writes deletion requests/progress/stage success,
or connects to Redis. The normal workflow dispatcher remains unchanged. The
offline command has no request lease and must not be substituted for a durable
workflow handler. Integration needs the existing parent lease checks, atomic
progress and counts, a write gate, and durable cache/session cleanup.

The start record counts deferred archive and replay-watermark rows in this
community. Archive rows remain so the retained profile does not reappear.
Watermarks remain until durable write restrictions replace their replay
protection. Required actor fields, moderation/audit erasure, device ownership,
global records and cache invalidation remain unsupported and are named in every
start record. Profile and agent data remain for their separate cleanup steps.
Events, workflows, deliveries and media belong to the separate content/media
commands and are not selected here.

For testing, compare database snapshots before and after each preview mode,
then execute with a small `--max-items`, inspect the partial result, rerun, and
verify only the listed rows and fields changed. Restart live services only as
part of the surrounding deletion procedure, after required cache and session
invalidation and durable write restrictions are in place.
