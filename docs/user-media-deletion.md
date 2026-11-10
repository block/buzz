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
version listing and GET access. It scans historical upload-record versions as
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
subject, agent-selection flag, source-connection fingerprint, storage namespace,
aliases and parser version. Changing any of these requires a new run.

Exit 0 means no file decisions remain unknown, including a valid empty inventory.
Exit 2 prints the inventory but reports unresolved decisions. Other failures
produce an error, not partial discovery success. `execution_authorized` is always
false in a standalone observation, even when every file is kept.

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
| `workflow_runs` | Trigger context, trace and errors; owner joined through `(community_id, workflow_id)`; separate association through `trigger_context.author` |
| `workflow_approvals` | Note/spec, with the same composite owner join |
| `subscriptions`, `delivery_log` | Delivery URL and delivery errors, attributed through subscription ownership |
| `product_feedback` | Body/tags; retained feedback protects its files |
| `channels`, `communities` | Canvas copy, descriptions and community icon; shared state protects its files |
| `moderation_reports` | Notes, direct blob hashes and referenced event payloads; retained evidence protects files |
| `moderation_actions`, `relay_admin_actions`, `relay_admin_outbox` | Reasons, errors and notice payloads; retained state protects files |
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
recorded as observed objects. They are **not** executable targets until the
protected workflow checks each disposition and retention requirement.

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
2. **Decide.** Process only unknown rows. Standalone preview keeps a file when a
   definite surviving source or uploader claim protects it. Otherwise it records
   the outstanding checks and leaves it unknown. Protected execution atomically
   writes a complete immutable target list with `to_be_deleted`, or a keep reason
   with `dont_delete`. A failed check remains unknown. A restart processes the
   remaining unknown rows, without replacing a published inventory.
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
A shared original and separately eligible metadata need separate dispositions
inside that file's target/evidence fields.

Every control write checks the run generation. An older invocation cannot
publish after a replacement starts. Workflow writes additionally validate the
existing request lease and inventory approval. A worker lease does not by itself
protect media from new uploads or references.

## Shared handler modes and remaining integration

The library handler supports `print-only` (print and return), `print` (print then
execute) and `no-print` (execute without inventory output). Its exact-version
deletion and recovery loop is implemented and tested. The standalone command
rejects the latter two modes before connecting to any service.

The live deployment adapter must implement `ProtectedMedia`: verify approved
scope, exact downstream removal selectors/fingerprints, complete source and
uploader coverage, durable reservations respected by **all** writers, object
holds/retention and upload-record retention policy; perform idempotent uploader
cleanup. Main does not yet have that reservation protocol or the complete
removal/retention proof. There is no permissive production adapter or flag that
asserts these checks passed. Consequently standalone preview deliberately does
not produce `to_be_deleted` rows. Its observed storage versions are useful for
inspection, not approval to delete later. These are explicit integration gaps,
not functionality claimed by this PR.

The source catalog also needs downstream-selector integration for dependent
records owned by another author and historical relay-signed attribution. Missing
historical orphan-upload evidence cannot be reconstructed from file hashes.
The discovery marker certifies that the **defined scans** finished, not that
historical data was fully recorded or that all future source formats are covered.

Initial limits are one million source rows, one million storage versions,
100,000 distinct hashes, 128 MiB of evidence/storage descriptors, 1 MiB per
source/file document and 64 KiB per upload record. Exceeding a limit reports an
error without publishing partial discovery. Scale measurements may justify a
later unpublished-generation design; this implementation never quietly commits
individual discovery pages.
