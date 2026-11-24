# Inspect community memberships for user deletion

Inventory, read-only submission and deletion submission select the subject’s current community and channel memberships. A saved request keeps the list approved for that run.

## Build and choose a database

```bash
. ./bin/activate-hermit
cargo build -p buzz-admin
```

Set `DATABASE_URL` explicitly. Preview needs SELECT on the source tables used by community discovery. The complete workflow also needs write
access to the `entity_deletion_*` tables. A production read-only account cannot
submit, approve, or checkpoint a run.

For a fresh, disposable local database:

```bash
docker run --detach --name buzz-user-inventory \
  --publish 127.0.0.1:55435:5432 \
  --env POSTGRES_USER=buzz --env POSTGRES_PASSWORD=local-fixture \
  --env POSTGRES_DB=buzz postgres:17-alpine
export PGPASSWORD=local-fixture
export DATABASE_URL=postgres://buzz@127.0.0.1:55435/buzz
# Wait until pg_isready reports accepting connections.
docker exec buzz-user-inventory pg_isready -U buzz -d buzz
./target/debug/buzz-admin migrate
```

Use a different container name and port if those are occupied. A full workflow also needs a configured versioned media store. The examples use Bash and jq.

## 1. Preview the memberships

```bash
SUBJECT="$SUBJECT_PUBKEY"
MODE=(--read-only)
./target/debug/buzz-admin user-deletions inventory --pubkey "$SUBJECT"
```

The command accepts hex or npub. Each entry contains a community ID, host label and relationship. Use `MODE=(--delete)` only when executing an ordinary deletion request on disposable test data. Omit `--read-only` on submission for that mode.

## 2. Submit and inspect

```bash
./target/debug/buzz-admin user-deletions submit \
  --pubkey "$SUBJECT" --read-only --requested-by operator-a > /tmp/user-deletion-request.json
REQUEST_ID=$(jq -r '.id' /tmp/user-deletion-request.json)
./target/debug/buzz-admin user-deletions inspect "$REQUEST_ID" > /tmp/user-deletion-inspection.json
jq '{id, target, status, inventory_manifest, inventory_digest}' /tmp/user-deletion-inspection.json
DIGEST=$(jq -r '.inventory_digest' /tmp/user-deletion-inspection.json)
```

Submit runs the same inventory generator as preview, then atomically saves one
`entity_deletion_requests` row with `status = inventoried`, the manifest and
its digest. Preview is optional; approval always refers to the saved inventory.
Deletion targets are canonical public keys. Read-only requests use the same key with a `read-only:` prefix for independent progress.

The same target, operator and reason return the existing request. Resubmission
does not refresh its inventory or restart a completed run. Use a fresh disposable database to test another initial submission.

## 3. Approve the inspected inventory

```bash
./target/debug/buzz-admin user-deletions approve "$REQUEST_ID" \
  --approved-by operator-b --inventory-digest "$DIGEST"
```

This writes an `entity_deletion_approvals` row and changes the parent status to
`approved`. No stage has run yet. A wrong digest is rejected, and `run` before
approval is rejected. Operator identities remain self-asserted.

## 4. Step once, then resume to completion

```bash
./target/debug/buzz-admin user-deletions run "$REQUEST_ID" "${MODE[@]}" --next-step
./target/debug/buzz-admin user-deletions inspect "$REQUEST_ID" \
  | jq '{status, execution, stage_history}'
./target/debug/buzz-admin user-deletions run "$REQUEST_ID" "${MODE[@]}"
./target/debug/buzz-admin user-deletions inspect "$REQUEST_ID" \
  | jq '{status, stages: [.stage_history[] | {stage, target_community_id, status, is_current}]}'
```

The first run claims the request, creates and completes `gate_user`, saves the
next current stage, and releases ownership. The second process resumes from
that saved stage and processes communities in inventory order. Each completion
updates `entity_deletion_stage_history` and advances the current marker under
the parent lease. On completion the parent is `completed` and no stage is current.

Runs are bounded to 100 handlers per invocation. For larger inventories, repeat `run` until inspection shows `completed`.

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
2. Select communities from `relay_members` and active channel memberships (`channel_members.removed_at IS NULL`). Historical messages alone do not select a community.
3. Preserve the owner relationship when present. Other current members and channel-only participants are participants.
4. Include archived or deleting communities that meet the same membership rule. Return each community once, ordered by UUID.

Discovery uses one statement snapshot, a 30-second timeout, and a 1,000-community
cap. Errors and oversized results fail rather than return partial inventories.
A failed discovery during submission rolls back the request too.
Preview uses a read-only transaction and needs no workflow tables or migrations.

Preview and submission use the same selector. Read-only execution records the ID of each owned
community it would submit. Delete mode submits or reuses the community-deletion
request for independent approval.

A workflow uses the saved version-2 inventory. An unsupported workflow version is rejected before execution. Inspect an existing request to see its saved scope. Rerunning it resumes progress against that scope.

See [the executor guide](operator-user-deletion-executor.md) for the owned-community completion check and [the preview guide](operator-user-deletion-preview.md) for a full read-only rehearsal.
