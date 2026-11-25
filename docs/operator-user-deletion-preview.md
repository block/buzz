# Run and verify a read-only user deletion

**Never stop or restart a relay for user deletion, including rehearsals and production.**

Use this procedure to preview one user's deletion across their communities. It saves the request, approval, and stage history in PostgreSQL. User data and storage versions stay in place. For an owned community the preview records `would_submit` and its community ID. It creates zero child deletion requests.

## 1. Select the target and review the inventory

Use Bash with `buzz-admin` and `jq` on your path. Load the operator environment for the intended database and media store, including `DATABASE_URL`, `BUZZ_S3_ENDPOINT`, `BUZZ_S3_BUCKET`, and the usual storage credentials. Set `USER_PUBKEY` to the user's public key, and `REQUESTED_BY` and `APPROVED_BY` to the respective operator identities. Run the blocks in the same shell. The output includes sensitive identifiers.

```bash
set -euo pipefail
umask 077
OUT=$(mktemp -d "${TMPDIR:-/tmp}/user-deletion.XXXXXXXX")
echo "Preview files: $OUT"
user_deletion() { buzz-admin user-deletions "$@"; }

user_deletion inventory --pubkey "$USER_PUBKEY" \
  > "$OUT/inventory.json"
user_deletion submit --read-only --pubkey "$USER_PUBKEY" \
  --requested-by "$REQUESTED_BY" > "$OUT/request.json"
REQUEST_ID=$(jq -er '.id' "$OUT/request.json")
user_deletion inspect "$REQUEST_ID" > "$OUT/before.json"
jq '{id, target, status, inventory_digest, communities: .inventory_manifest.communities}' "$OUT/before.json"
```

Review the target, communities, and ownership labels in the **saved** inventory before approving. A fresh request has status `inventoried`. Identical submissions can return an existing request. For a completed request, inspect its saved results. To repeat a test from the beginning, use the disposable fixture procedure below.

## 2. Approve and run the full preview

```bash
DIGEST=$(jq -er 'select(.status == "inventoried") | .inventory_digest' "$OUT/before.json")
user_deletion approve "$REQUEST_ID" --approved-by "$APPROVED_BY" --inventory-digest "$DIGEST"
user_deletion run "$REQUEST_ID" --read-only --write-row-ids "$OUT/rows.ndjson" \
  > "$OUT/run.json"
user_deletion inspect "$REQUEST_ID" > "$OUT/inspect.json"
jq '{id, status, execution, stages: [.stage_history[] | {stage, target_community_id, status}]}' "$OUT/inspect.json"
```

A single invocation creates one file. Each handler appends entries in execution order, one JSON object per line, tagged with the stage name and ID. A stage with zero entries still appears in `inspect`. Each invocation runs up to 100 handlers. If the request is still running, inspect its current stage and retry deadline, then resume the same request with a fresh output path. Keep every part to verify the full request. For a step-by-step preview, add `--next-step` to each run and use a fresh path per step.

## 3. Verify the output

For a completed single-invocation preview, check every line against the saved stage history and print counts grouped by stage, community, and proposed action. For a resumed preview, pass the row files in invocation order in place of `"$OUT/rows.ndjson"`. Files from failed attempts can contain repeated proposals.

```bash
jq -en --slurpfile inspection "$OUT/inspect.json" '
  $inspection[0] as $i |
  ($i.stage_history | INDEX(.id)) as $stages |
  $i.status == "completed" and all($i.stage_history[]; .status == "completed") and
  all(inputs; . as $r | $stages[$r.stage_id] as $s |
    $s != null and $r.mode == "read-only" and $r.request_id == $i.id and
    $r.stage == $s.stage and $r.community == $s.target_community_id and
    (if $r.stage == "delete_owned_community" then
       $r.action == "would_submit" and $r.community_id == $s.metadata_json.result.community_id
       and ($r | has("child_request_id") | not)
     else true end))' "$OUT/rows.ndjson"

jq -r '[.stage, (.community // "global"), .action] | @tsv' "$OUT/rows.ndjson" | sort | uniq -c
jq '.stage_history[] | {stage, target_community_id, result: .metadata_json.result}' "$OUT/inspect.json"
```

Reconcile the counts with each handler's result. Content and account report selected items. A profile archive snapshot replacement occupies one line and counts as two writes. For media, count file decisions separately from version entries. Check selected hashes, object keys, and version IDs against the versioned bucket. In a seeded test, compare the selected keys and actions with the fixture expectations to catch missing or extra records.

If you have a verified reference file from the same fixture and bucket, set `EXPECTED_ROWS` to its path. You can concatenate verified per-step NDJSON files to produce that reference. Compare every entry, keeping duplicate counts and ignoring only the IDs generated for each run. An empty diff means the keys, proposed actions, communities, media versions, and stage names all match.

```bash
jq -cS 'del(.request_id, .stage_id)' "$EXPECTED_ROWS" | LC_ALL=C sort > "$OUT/expected.sorted"
jq -cS 'del(.request_id, .stage_id)' "$OUT/rows.ndjson" | LC_ALL=C sort > "$OUT/actual.sorted"
diff -u "$OUT/expected.sorted" "$OUT/actual.sorted"
```

For example, to spot-check a content event with `psql`, use the full key and community from the file. For a fixture containing a selected event, this should return `1`.

```bash
event=$(jq -cn 'first(inputs | select(.stage == "purge_content" and .table == "events"))' "$OUT/rows.ndjson")
if [[ -n "$event" ]]; then
  psql "$DATABASE_URL" -XAt -v ON_ERROR_STOP=1 \
    -v community="$(jq -r '.community' <<< "$event")" \
    -v event_id="$(jq -r '.key.id' <<< "$event")" \
    -v created_at="$(jq -r '.key.created_at' <<< "$event")" <<'SQL'
SELECT count(*) FROM events
WHERE community_id = :'community'::uuid AND id = decode(:'event_id', 'hex')
  AND created_at = :'created_at'::timestamptz;
SQL
fi
```

To spot-check a storage version, configure the AWS CLI for the same bucket, credentials, and region as the preview. This lookup covers both object versions and delete markers. A match prints `true`.

```bash
version=$(jq -cn 'first(inputs | select(.stage == "purge_media" and .version != null) | .version)' "$OUT/rows.ndjson")
if [[ -n "$version" ]]; then
  aws --endpoint-url "$BUZZ_S3_ENDPOINT" s3api list-object-versions \
    --bucket "$BUZZ_S3_BUCKET" --prefix "$(jq -r '.key' <<< "$version")" --output json |
    jq -e --argjson v "$version" '
      (if $v.kind == "delete_marker" then .DeleteMarkers else .Versions end) |
      any(.[]?; .Key == $v.key and .VersionId == $v.version_id and
          ($v.kind == "delete_marker" or .Size == $v.size))'
fi
```

## Repeat on a fresh disposable fixture

For a fresh test, create a separate database from a seeded local fixture. The following copy omits data from the deletion workflow tables, including fences, and preserves the original database and its history. Set the libpq connection variables (`PGHOST`, `PGPORT`, `PGUSER`, and authentication) for the local test server and `FIXTURE_DB` to the source database name. Use the fixture's versioned test bucket.

```bash
set -euo pipefail
umask 077
PREVIEW_DB="user_preview_$(date +%Y%m%d%H%M%S)"
dump_dir=$(mktemp -d)
pg_dump --format=custom --exclude-table-data='public.*_deletion_*' \
  --file="$dump_dir/fixture.dump" "$FIXTURE_DB"
createdb "$PREVIEW_DB"
pg_restore --exit-on-error --no-owner --no-acl --dbname="$PREVIEW_DB" "$dump_dir/fixture.dump"
export DATABASE_URL="postgresql:///$PREVIEW_DB"
```

Run steps 1–3 against the copy. In an isolated fixture you can also verify that the preview left the child request table empty, even when the user owns communities.

```bash
psql "$DATABASE_URL" -XAt -v ON_ERROR_STOP=1 -c 'SELECT count(*) FROM community_deletion_requests'
```

See [workflow execution and recovery](operator-user-deletion-executor.md) for retry behavior, handler scopes, and destructive-mode limitations.
