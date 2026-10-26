# Test inventory through the user-deletion workflow

Both demo and real-key submissions run through the existing workflow:

```text
inventory preview → submit → inspect → approve → run → inspect completed
```

Request, inventory, approval, lease and stage records are real. All execution
handlers still print dummy messages: `gate_user`, then `purge_db` and
`purge_agents` for each community. They do not fence users or delete data.
There is no background worker to start; `user-deletions run` is the worker process.

While execution uses dummy handlers, submit and execute requests only in an
isolated, disposable development database. Production inventory preview may use
read-only access. A completed dummy request permanently occupies that target's
request row in the database where it ran.

## Build and choose a database

```bash
. ./bin/activate-hermit
cargo build -p buzz-admin
```

Set `DATABASE_URL` explicitly. Preview needs SELECT on `communities`,
`relay_members`, and `channel_members`. The complete workflow also needs write
access to the `entity_deletion_*` tables. A production read-only account cannot
submit, approve, or checkpoint a run.

For a fresh, disposable local database:

```bash
docker run --detach --name buzz-user-inventory \
  --publish 127.0.0.1:55435:5432 \
  --env POSTGRES_USER=buzz --env POSTGRES_PASSWORD=buzz-local-demo \
  --env POSTGRES_DB=buzz postgres:17-alpine
export DATABASE_URL=postgres://buzz:buzz-local-demo@127.0.0.1:55435/buzz
# Wait until pg_isready reports accepting connections.
docker exec buzz-user-inventory pg_isready -U buzz -d buzz
./target/debug/buzz-admin migrate
```

Use a different container name and port if those are occupied. No relay, Redis,
or object storage is required. These commands use `jq` to extract JSON fields.

## 1. Choose demo or a public key

For demo, no membership fixtures are needed:

```bash
SUBJECT=demo
```

For real discovery, set your hex public key or npub:

```bash
SUBJECT=<your-hex-key-or-npub>
```

Hex and npub are two encodings of the same public key. They normalize to the
same lowercase hex target and therefore the same request. No private key is
needed. Discovery only sees memberships in the database selected by
`DATABASE_URL`; a fresh local database will return no communities for your key.
The optional local fixture below exercises discovery using that key without
connecting to production.

Preview the inventory:

```bash
./target/debug/buzz-admin user-deletions inventory --pubkey "$SUBJECT"
```

The default is demo if `--pubkey` is omitted. Its two fixed communities have the
same shape as real discovery:

```json
{
  "communities": [
    {
      "community_id": "00000000-0000-0000-0000-00000000000b",
      "label": "demo:community-b",
      "relationship": "participant"
    },
    {
      "community_id": "00000000-0000-0000-0000-00000000000c",
      "label": "demo:community-c",
      "relationship": "former_participant"
    }
  ]
}
```

There are no actions, subjects or request targets inside this inventory.

## 2. Submit and inspect

```bash
./target/debug/buzz-admin user-deletions submit \
  --pubkey "$SUBJECT" --requested-by operator-a > /tmp/user-deletion-request.json
REQUEST_ID=$(jq -r '.id' /tmp/user-deletion-request.json)
./target/debug/buzz-admin user-deletions inspect "$REQUEST_ID" > /tmp/user-deletion-inspection.json
jq '{id, target, status, inventory_manifest, inventory_digest}' /tmp/user-deletion-inspection.json
DIGEST=$(jq -r '.inventory_digest' /tmp/user-deletion-inspection.json)
```

Submit runs the same inventory generator as preview, then atomically saves one
`entity_deletion_requests` row with `status = inventoried`, the manifest and
its digest. Preview is optional; approval always refers to the saved inventory.
Real targets are canonical public keys. The default demo target is `demo:default`.

The same target, operator and reason return the existing request. Resubmission
does not refresh its inventory or restart a completed run. For another demo run,
use `submit --target demo:another-run --requested-by operator-a`. For another
full real-key test, use a fresh disposable database.

## 3. Approve the inspected inventory

```bash
./target/debug/buzz-admin user-deletions approve "$REQUEST_ID" \
  --approved-by operator-b --inventory-digest "$DIGEST"
```

This writes an `entity_deletion_approvals` row and changes the parent status to
`approved`. No stage has run yet. A wrong digest is rejected, and `run` before
approval is rejected. Operator identities remain self-asserted demo inputs.

## 4. Step once, then resume to completion

```bash
./target/debug/buzz-admin user-deletions run "$REQUEST_ID" --next-step
./target/debug/buzz-admin user-deletions inspect "$REQUEST_ID" \
  | jq '{status, execution, stage_history}'
./target/debug/buzz-admin user-deletions run "$REQUEST_ID"
./target/debug/buzz-admin user-deletions inspect "$REQUEST_ID" \
  | jq '{status, stages: [.stage_history[] | {stage, target_community_id, status, is_current}]}'
```

The first run claims the request, creates and completes `gate_user`, saves the
next current stage, and releases ownership. The second process resumes from
that saved stage and processes communities in inventory order. Each completion
updates `entity_deletion_stage_history` and advances the current marker under
the parent lease. On completion the parent is `completed` and no stage is current.

Expected counts: demo has five completed stages; one discovered community has
three. An empty inventory has only the request-wide dummy gate, so an empty
result does not test community processing. Runs are bounded to 100 handlers;
for larger inventories, repeat `run` until inspection shows `completed`.

To test a failure, use a fresh request and, after approval, run:

```bash
./target/debug/buzz-admin user-deletions run "$REQUEST_ID" --fail-stage purge_db
# Expected nonzero exit: inspect the saved error and retry deadline.
./target/debug/buzz-admin user-deletions inspect "$REQUEST_ID" \
  | jq '{status, execution, stage_history}'
# Once execution.next_attempt_at has passed (normally 30 seconds):
./target/debug/buzz-admin user-deletions run "$REQUEST_ID"
```

The failed stage stays current; its retry updates the same row. No new approval
is needed. See [the executor walkthrough](operator-user-deletion-executor.md)
for leases, shutdown, and takeover details.

## Optional: local membership fixture for your key

Run this **only against the disposable container created above**. It creates one
fake community and a channel membership, with no `relay_members` row. This tests
the channel-only discovery path. It does not reproduce your production inventory.
Supply a hex key here (the CLI itself also accepts npub).

```bash
HEX_KEY=<your-64-character-hex-public-key>
docker exec -i buzz-user-inventory psql -X -U buzz -d buzz \
  -v ON_ERROR_STOP=1 -v subject="$HEX_KEY" <<'SQL'
BEGIN;
INSERT INTO communities (id, host)
VALUES ('00000000-0000-0000-0000-000000000101', 'inventory-fixture.example');
INSERT INTO channels (community_id, id, name, created_by)
VALUES ('00000000-0000-0000-0000-000000000101',
        '00000000-0000-0000-0000-000000000102', 'fixture', decode(:'subject', 'hex'));
INSERT INTO channel_members (community_id, channel_id, pubkey)
VALUES ('00000000-0000-0000-0000-000000000101',
        '00000000-0000-0000-0000-000000000102', decode(:'subject', 'hex'));
COMMIT;
SQL
SUBJECT="$HEX_KEY"
```

Now repeat steps 1–4. Create the fixture before submitting; a saved request's
inventory is not regenerated on resubmission.

## Discovery and limits

1. Normalize the hex key or npub.
2. Find explicit `relay_members` rows by the normalized hex key.
3. Find `channel_members` rows using the byte-form key and `removed_at IS NULL`.
   Hidden memberships count, including token-authenticated participants without
   an explicit relay membership row.
4. Combine community IDs, deduplicate, and join `communities` for each host.
   Include archived and deleting communities when membership evidence exists.
5. Preserve an explicit relay owner relationship; otherwise use participant.
6. Return communities once each, ordered by UUID.

Discovery uses one statement snapshot, a 30-second timeout, and a 1,000-community
cap. Errors and oversized results fail rather than return partial inventories.
A failed discovery during submission rolls back the request too.
Preview uses a read-only transaction and needs no workflow tables or migrations.

This covers current memberships only. It does not discover past participation,
other keys owned by the person, or communities in other databases. Owners are
reported by discovery, but submission rejects owner inventories and rolls back
the request until owned-community execution is implemented. Approval also rejects
previously saved owner inventories. Both report "owned-community execution is not
supported yet". The dummy fixture's former participant is synthetic.

## Schema and compatibility

Apply migrations 0072 and 0073 before real-key submission. They widen the target
check to accept a nonempty identifier of up to 256 bytes with no surrounding
spaces, then validate in a separate transaction. Identity formats belong in the
application, so this column can also store account IDs when account resolution
is implemented. Today the CLI resolves only demo and hex/npub inputs.
The user-only entity-type check remains in place. Workflow-specific commands also
validate the entity type before executing a request.
Both use a 5-second lock timeout and 30-second statement timeout. The desired-state
schema has the same final constraint. No data backfill, trigger, function, or
foreign key is added. Existing demo targets remain valid. If you applied an
earlier draft of these migrations, use a fresh disposable test database; do not
rewrite migration checksums.

The new reader accepts both community-only inventories and saved version-1 demo
inventories from #8520. Old manifests and their handler inputs are preserved on
resume. Old binaries reject the new manifest format; use this binary to approve
and run newly submitted requests. The serving relay does not automatically execute
user requests. Older auto-migrating binaries refuse unknown migrations, so use the
deployment's rollback procedure rather than assuming binary rollback is sufficient.

## Automated verification

With the PostgreSQL test lane configured (see `crates/buzz-db/TESTING.md`):

```bash
scripts/postgres-test-run.sh -p buzz-db -p buzz-admin -p buzz-deletion \
  -E 'test(/user_deletion::/) or binary(postgres_user_inventory) or test(/^user::/)'
```

Coverage includes actual CLI preview, submit, digest approval, step/resume and
completion for demo and npub, with hex resubmission; unchanged membership rows;
SELECT-only discovery; channel-only participants; owner precedence; hidden and
removed memberships; lifecycle inclusion; query errors; size cap; and migration
constraint parity. Existing worker tests cover failure, retries and lease takeover.

## Adding real stages without changing the executor

The user workflow's stage rules live in `crates/buzz-deletion/src/user/workflow.rs`;
handler dispatch lives in `user/handlers.rs`. Add a rule and its handler there,
then connect the predecessor's decision to it. CLI stage-name validation reads
that same rule list. Unknown stages fail explicitly. No stage-name constraint
exists in SQL, and adding a stage does not require changing the worker loop.

Input identity parsing and demo fixture selection live in the inventory generator
in `buzz-db/src/store/user_deletion/discovery.rs`. Approval and recovery validate
the saved inventory and workflow history, without reparsing the target identifier.
The shared request store has no demo or public-key formatting rules. An account-ID
resolver remains future work; accepting saved account-shaped identifiers does not
implement that resolver. Dummy side effects remain confined to the user handlers.

Permanent uniqueness on `(entity_type, target)` is unchanged by this cleanup.
