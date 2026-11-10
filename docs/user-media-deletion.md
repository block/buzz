# User media inventory and deletion

`buzz-admin user-deletions media --pubkey <hex-or-npub> --mode print-only`
collects media associated with a person across the source database's communities,
saves one row per file, classifies the observation, and prints JSON. Add
`--include-owned-agents` to include current agent ownership from
`users.agent_owner_pubkey`, separately in each community. Current memberships
are not required: old events and unposted uploads remain discoverable.

This direct command **never deletes**. It does not install a user fence, approve
a deletion request, migrate a database, or enqueue the subject for deletion.

## Connections and a first test

Use an already-migrated local control database for `DATABASE_URL`. Set
`BUZZ_MEDIA_SOURCE_DATABASE_URL` to the source database; the command connects with
`default_transaction_read_only=true` and scans in a read-only repeatable-read
transaction. Source access must include private and historical rows in every
community sharing the configured storage namespace. Use SELECT-only credentials
when testing against a live deployment. Never run the migrator against that
source merely to make a preview work.

Configure `BUZZ_S3_ENDPOINT`, `BUZZ_S3_BUCKET`, region/addressing style, and the
normal S3 credentials or workload identity. The direct command requires only
version listing, GET, GetObjectLockConfiguration, GetObjectLegalHold and
GetObjectRetention access. It scans historical upload-record versions as
well as current ones. A missing table, unreadable record, denied storage request,
malformed upload record, or exceeded resource limit fails discovery; none is
interpreted as an empty result.

```sh
# Configure connection and storage environment through the operator's normal setup.
cargo run -p buzz-admin -- user-deletions media \
  --pubkey <npub> --include-owned-agents --mode print-only
```

Stdout is inventory JSON. Stderr prints a run UUID immediately; retry the same
command with `--run-id <uuid>` after an interruption. Use a **new** run UUID to
observe changed source data. A retry intentionally reprints the saved observation.
Additional canonical storage aliases can be supplied with repeated
`--media-base https://files.example.org/media` arguments. The run binds the
subject, agent-selection flag, source-connection fingerprint (excluding rotating
credentials), storage namespace,
aliases and parser version. Changing any of these requires a new run.

Exit 0 means no file decisions remain unknown, including a valid empty inventory.
Exit 2 prints the inventory but reports unresolved decisions. Other failures
produce an error, not partial discovery success. `execution_authorized` is always
false in a standalone observation, including when it prints `to_be_deleted`.
Those decisions describe this observation; a later execution must use an approved
workflow stage and repeat its checks while writers respect the reservations.

## Tables read

The executable query catalog is
[`sources.sql`](../crates/buzz-db/src/store/media_deletion/sources.sql).
The database scan uses no visibility, membership, or soft-deletion filter.

| Tables | Fields/relationship inspected |
|---|---|
| `users` | Current `agent_owner_pubkey`; avatar, about and capabilities |
| `events` | Content/tags for every stored kind, including messages, edits, profiles, notes, canvases, personas, teams and artifacts |
| `nip_fi_profile_heads` | Stored verified-profile event |
| `workflows` | Definition, attributed to `owner_pubkey` |
| `workflow_runs` | Trigger context and trace; owner joined through `(community_id, workflow_id)`; separate association through `trigger_context.author` |
| `workflow_approvals` | Note/spec, with the same composite owner join |
| `subscriptions`, `delivery_log` | Delivery URL and delivery-log ownership; diagnostic errors are not attachment claims |
| `product_feedback` | Body/tags; retained feedback protects its files |
| `channels`, `communities` | Canvas copy, descriptions and community icon; shared state protects its files |
| `moderation_reports` | Notes, direct blob hashes and referenced event payloads; retained evidence protects files |
| `moderation_actions`, `relay_admin_actions`, `relay_admin_outbox` | Reasons and notice payloads; retained state protects files |
| `artifact_heads` | Live head's referenced event; shared head protects its files |
| `media_uploaders` | Recorded uploader/hash pairs, including unposted uploads |
| `audit_log` | `media_uploaded` actor/hash evidence; conflicting hashes fail the scan |

The application parses JSON strings, attachment tags, free-text local media URLs
and encoded animated-avatar descriptors. Absolute foreign-host URLs are not
mapped to local objects. Evidence contains source keys, field paths and payload
fingerprints; it does not persist message bodies, upload IP addresses or signed
URLs. Source association is not a claim that the author originally uploaded the
file or that the source is approved for removal.

Storage reads enumerate versions and delete markers. `_uploads/<community>/`
JSON records are read at their listed versions and validated against their path,
format, public key and ULID. Original/thumbnail keys and `_meta` sidecars are
recorded as observed objects. Step 2 checks retention and turns eligible versions
into exact targets. A missing version ID, denied lock API or unrecognized
attribution remains an error, including in print-only mode.

## Tables written and crash behavior

Only one new table is added: `entity_deletion_media_files`. Its primary key is
`(stage_id, storage_namespace, file_hash)`. Every file row has one of
`unknown`, `dont_delete`, `to_be_deleted`, `deleted`, with per-file evidence,
targets, decision reason and error.

The direct command creates an isolated `media-preview:<run UUID>` control record
in the existing `entity_deletion_requests`, terminal and ineligible for worker
claims. Its `media_preview` row in `entity_deletion_stage_history` is never the
current workflow stage. This records the observation without enrolling the
person in a destructive request. Control evidence intentionally survives
community deletion; no tenant data is owned by the new table.

1. **Discover.** Collect and deduplicate the complete bounded observation in
   memory. After DB and storage scans succeed, one transaction inserts all file
   rows as `unknown` and writes `metadata_json.media_discovery` with `complete`,
   count and scan receipt. A failed insert rolls back every row and the marker.
   Zero files still commits the marker with count zero. A lost commit response is
   resolved by reading the same stage. Partial rows never count as completion.
2. **Decide.** Process only unknown rows with the same function in every mode.
   Keep files with surviving references/uploader claims, another community's
   metadata, or an active storage hold/retention period. A file with references
   but no uploader attribution remains unknown. Otherwise save the exact target
   keys/versions and `to_be_deleted` together. `evidence_json.remove_sources`
   identifies the exact subject-owned source keys and fingerprints assumed to be
   removed by later handlers. Reads that fail leave the file unknown with the
   error. Print-only prints this inventory and returns here.
3. **Execute.** Require the discovery marker, matching row count, no unknown
   decisions, current request ownership and continuous external-write protection.
   Delete the exact recorded versions; verify their absence; only then change the
   file row to `deleted`. A crash after storage deletion but before the row update
   leaves `to_be_deleted`; retry verifies the same versions and finishes the row.
   If only a thumbnail was removed, retry still handles every saved target.
   Permission errors and timeouts are never absence. Finally remove the exact
   approved uploader proofs idempotently; preserve sticky `media_publications`.
   Source-message removal and workflow advancement belong to later handlers.

`deleted` describes **all selected targets being confirmed absent**. It does not
claim that intentionally retained components or a later re-upload were erased.
This implementation keeps all components of a shared file; it does not delete
community metadata separately while retaining the original.

Every control write checks the run generation. An older invocation cannot
publish after a replacement starts. Workflow writes additionally validate the
existing request lease and inventory approval. A worker lease does not by itself
protect media from new uploads or references.

## Workflow handler and writer protection

`run_stage(db, storage, lease, stage_id, options)` is the concrete handler. It
supports `print-only`, `print` and `no-print`. The workflow must own an approved
request lease and start its `purge_media` stage, and maintain its heartbeat.
The direct CLI stays print-only: it cannot enroll a person into deletion merely
by changing an output flag. Main workflow routing is still the separate dummy
workflow; this PR supplies the real media handler for that caller to use.

Execution additionally writes the existing `user_deletion_fences` table to stop
new subject uploads. It snapshots pre-existing upload leases from
`community_serving_write_leases` and waits for those to finish; unrelated new
uploads do not extend that drain. It then publishes discovery, checks that the
source communities are covered by the approved request, and sets `reserved` on
candidate rows in the one media table. Reservation batches are bounded; no
execution starts until all have committed. A stage receipt records the completed
drain and reservation checks, so per-version verification checks ownership and
that receipt without rescanning every file. It re-scans current references,
claims and storage before deciding. On a retry after targets were selected,
changed or newly surviving source evidence blocks deletion rather than replacing
the target list. It validates the saved source selection against approval again
after decisions finish, before the first deletion.

Reference writers take a shared lock for their media hashes, check the indexed
reservation flag, and retain the lock until their transaction finishes.
Reservation installation takes the corresponding exclusive locks first. The
flag persists through a worker crash. These checks cover event insert/replacement,
profiles, channel fields/canvas, community icons, workflows/runs/approvals,
feedback, moderation evidence, and admin notice payloads. Inventory and writer
checks share the reference parser; a `p` tag's public key is not a file hash.
Diagnostic error strings and immutable audit history are retained as history,
not new attachment publications. Subscription/delivery tables have no serving
write path in this revision.

Every relay upload path records its verified uploader claim before the first
storage read/write, including idempotent reuploads and media-read mode `off`.
This rejects subject upload restrictions and reserved hashes. The pre-storage
claim uses `media_uploaders.pending=true` and grants no preview/publication
authority. Existing publication checks run before confirmation clears that flag.
A failed upload (or an upload in media-read mode off) can leave a conservative
possession claim without preview authority, since the sender proved possession
of the bytes. Upload requests already in flight are covered by the drain above.
All relay writers must run this implementation before executing deletion;
pre-upgrade relay processes and direct administrative SQL/storage writes do not
participate in application reservations.

Object Lock checks call the bucket configuration API and, when enabled, the
legal-hold and retention APIs for the exact version. Missing HEAD headers never
establish permission. An explicit missing bucket Object Lock configuration is
accepted; a denied or malformed response is not. Governance retention is never
bypassed. Retention is checked again before deleting each byte-bearing version;
delete markers are removed only after byte-bearing versions.

After all work succeeds, execution removes only the captured departing rows
from `media_uploaders`, preserves `audit_log` and `media_publications`, then
releases reservations. Kept files release their reservation when the decision
commits. Failed/unfinished deletion targets keep theirs for recovery. The
existing user upload restrictions remain for the surrounding account-deletion
workflow. Source messages/profiles/workflows are not removed by this media
handler: later handlers must consume the saved source selection rather than
assuming every associated record belongs to the departing person.

Missing historical orphan-upload attribution cannot be reconstructed from a
file hash. The completion marker certifies that the defined scans finished;
missing historical records and future source formats cannot be inferred from
that marker. New media-bearing write paths must join the same reservation checks
and source catalog.

Initial limits are one million source rows, one million storage versions,
100,000 distinct hashes, 128 MiB of evidence/storage descriptors, 1 MiB per
source/file document and 64 KiB per upload record. Exceeding a limit reports an
error without publishing partial discovery. Scale measurements may justify a
later unpublished-generation design; this implementation never quietly commits
individual discovery pages.

For a larger database, set `--max-source-rows 10000000 --scan-timeout-secs 900`
on a new run. These positive limits are bound to that run and do not relax the
memory, per-file or storage limits. Choose a reader endpoint and a budget suited
to the deployment; increasing the budget allows more work, not partial success.
