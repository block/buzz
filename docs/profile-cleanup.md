# Standalone profile cleanup

`buzz-admin user-deletions profile-batches` previews or removes profile data in
one explicit community. This is the identity step after content, account and
media cleanup. It does not enable the user-deletion workflow or declare complete
account erasure.

```sh
# DATABASE_URL selects the operator database. Preview is the default.
buzz-admin user-deletions profile-batches \
  --pubkey <human-hex-or-npub> --community <uuid> --print-only

# --read-only is an alias for --print-only.
# Stop the relay and external writers and finish earlier cleanup first.
buzz-admin user-deletions profile-batches \
  --pubkey <human-hex-or-npub> --community <uuid> --delete
```

The human and directly owned agents are discovered afresh from `users` in the
selected community on every preview, delete and retry. Ownership in another
community never authorizes cleanup here. A changed ownership set or unselected
child agent blocks a write batch. The output prints the discovered `target_pubkeys`
for inspection; saved agent keys are not accepted as deletion authority.
This command requires privileged database access.

The supported families are ordinary profiles (0/10100), owner-authored managed
agent records (30177/30179), verified profiles (30382), archive requests and
deltas (9035/9036/8002/8003), archive snapshots (13535), `nip_fi_profile_heads`,
`archived_identities`, and final `users` rows. Event-dependent mention, reaction
and delivery projections are removed before their source events. Agent identity
rows go before the human row. Reserved kind 30179 is cleaned only if stored;
this command does not enable its ingestion or decrypt its payload.

Verified profiles are selected by their matching `d`/`p` subject, not the event
author. Supply `--relay-pubkey <hex-or-npub>` for each independently verified
relay signer. A missing or untrusted signer stops the operation. The same trust
rule applies to archive deltas and snapshots. Human-signed requests about another
identity and archive actor/replacement references on surviving identities need
an explicit shared-history disposition. Selected archive events with these
references stop planning with an error before `profile_batches_start` is printed;
they are not included in its blocker list. Historical `replaced-by` references in
archive requests and deltas still stop planning after unarchive removes the
canonical archive row.

Shared archive snapshots are rebuilt from the retained canonical archive rows.
The snapshot replacement is a fresh signed event; no old signature is reused.
Deletion requires `BUZZ_RELAY_PRIVATE_KEY` matching the affected snapshot signer.
Preview never requires or reads this key. Historical snapshots that require a
different signing key remain blocked pending reconciliation. Do not pass secrets
on the command line or include them in logs.

Each JSON line describes the scope, proposed batch, committed batch, or final
summary. Batches default to 100 row mutations; `--batch-size` accepts 1–100.
Replacing a snapshot costs two mutations (delete and insert in one transaction),
so it requires a batch size of at least two. `--max-items` defaults to 100,000;
exceeding it fails before deletion. Additional memory limits cap a plan at 65,536
operations and candidate tags at 32 MiB (64 KiB per event); archive snapshots
support up to 10,000 retained identities. Exceeding any limit is an error, never
a successful partial preview. SQL statements have a 30-second timeout and locks
a three-second timeout; large accounts can require a separately reviewed plan.

Before mutations, the command rejects remaining credentials, memberships,
workflows, subscriptions, non-profile authored content other than repository
announcements/state (30617/30618), and unresolved archive references. Repository
events and name mappings remain intact and do not block identity removal.
It retains replay watermarks, deletion fences, requests, approvals,
history, repositories and media objects. Media cleanup must preserve its source
evidence before this command removes avatar/configuration references.

Each committed batch survives interruption. Rerun with the same human and community
to discover the remaining agents. All profile data is removed before any identity
row, so interruption during identity removal leaves no unfinished profile data
for an agent that can no longer be discovered. Source fingerprints and the full
current ownership set are rechecked inside the batch transaction; a mismatch rolls back that batch. A failure after commit
but before output is an error and does not mean the batch rolled back. No success
marker is printed if prerequisites or final profile checks fail. The final
summary always has `account_erasure_complete: false`.

The stopped-writer requirement includes pending login-triggered verified-profile
publication and external agent/config writers. This command does not add online
admission fencing or invalidate client caches. Restart/refresh the supported
serving processes and clients as part of the full deletion procedure. Retaining
replay records alone does not prove every writer is fenced. Ordinary verified
profiles remain historical records; explicit operator erasure is a separate
exception to normal refresh behavior.

## Local verification

Use a disposable database and relay, following `TESTING.md`; do not reuse a
Desktop database. Seed a human, two owned agents, a survivor, and a second
community. Include ordinary/verified profiles, managed configuration and a
shared archive snapshot. Complete content/account cleanup in the first community.

1. Export every table and the object inventory. Run print-only with the known
   relay public key. Compare every selected key with the expected inventory and
   confirm all exports are unchanged.
2. Stop writers. Run delete with the same scope and relay signing key. Compare
   proposed and committed keys and the surviving data, including the second
   community. Verify the new shared snapshot signature and survivor list.
3. Rerun with the same human and community; expect zero operations. Interrupt a
   multi-batch run and repeat to confirm recovery. Introduce an unresolved dependency or a
   wrong signer and confirm no deletion occurs.
4. Restart the isolated relay and reload clients. Check profile-event queries,
   directory/name search and NIP-05. Test old-event replay and fresh admission
   under the full deletion procedure's write restrictions. Report any recreation;
   the standalone database result does not certify online enforcement.
