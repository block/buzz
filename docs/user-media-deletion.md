# User media inventory and deletion

Media deletion uses recorded upload ownership. If Alice uploaded a file and Bob
only copied its URL, deleting Alice deletes eligible stored versions and Bob's
link stops working. If Bob also uploaded identical bytes, his upload record keeps
the file. The decision considers uploaders across communities sharing storage.

The workflow reserves `gate_user(None)` as the first slot, then
`purge_media(None)` once for the request, then community content/account stages.
The gate implementation is separate work: the current destructive workflow
stops at that unimplemented slot. Media implements no gating or serving-write
guards. Relay and external writers must remain stopped throughout deletion.

## Preview and connections

```sh
cargo run -p buzz-admin -- user-deletions media \
  --pubkey <npub> --include-owned-agents --print-only
```

`--print-only` is the default; `--read-only` is an alias. Optional owned agents
come from `users.agent_owner_pubkey`, separately in each community. Membership
is not required to discover past uploads. No media URL setting is needed.

Use an already-migrated control database for `DATABASE_URL` and set
`BUZZ_MEDIA_SOURCE_DATABASE_URL` to the source database. Preview connects with
`default_transaction_read_only=true`; upload evidence is read in a repeatable-read
transaction. Use SELECT-only source credentials with access to historical upload
records across all communities sharing the storage namespace. Do not migrate a
live source just to preview it.

Configure `BUZZ_S3_ENDPOINT`, `BUZZ_S3_BUCKET`, region/addressing style and normal
S3 credentials or workload identity. Preview needs version listing, GET,
GetObjectLockConfiguration, GetObjectLegalHold and GetObjectRetention permissions.
It reads upload-record JSON versions, not photo/video bytes.

Stdout is JSON. Stderr prints a run UUID; `--run-id <uuid>` resumes that saved
observation. Use a new UUID to observe changed data. The binding includes subject,
agent-selection flag, source identity excluding rotating credentials, storage
namespace, scan budget and policy version. Old link-based observations cannot be
resumed under the new policy. Preview exit 0 means no unknown decisions; exit 2
prints unresolved decisions. `execution_authorized` remains false in either case.

## Discovery and decisions

All entry points use the same [pipeline](../crates/buzz-deletion/src/user/media/pipeline.rs):
load saved inventory or discover it once, then classify unknown files with one
loop. Workflow preview keeps everything in memory. Standalone preview saves an
isolated observation journal. Delete journals decisions and applies them.

The [source query](../crates/buzz-db/src/store/media_deletion/sources.sql) reads:

| Source | Evidence |
|---|---|
| `media_uploaders` | Community, file hash and uploader, including uploads not posted in messages |
| `audit_log`, action `media_uploaded` | Community, actor and matching object/detail hash |
| Storage `_uploads/<community>/<hash>/<event>.json` | Validated uploader and hash at an exact object version |

`users` is read only to resolve optional owned-agent scope. Messages, profiles,
workflows, canvases and other URL-bearing fields are not scanned. Copied links
neither add candidates nor protect files. `_meta` paths identify stored sidecars;
a sidecar in another community alone does not establish another uploader.

Discovery reads database upload records once, lists the bucket's versions and
delete markers once, and validates upload JSON against its path, hash, actor and
ULID. It groups evidence and original/derived/sidecar versions by file hash, then
selects files attributed to the requested scope. Missing tables, malformed
records, denied reads or exceeded bounds fail discovery; no partial inventory is
published. Historical absent attribution cannot prove that nobody else uploaded
the file before tracking was enabled. This remains an explicit coverage limit.

Classification consumes that inventory without another source scan:

1. Another recorded uploader outside scope keeps the file.
2. Missing selected-uploader attribution leaves an error.
3. Invalid object paths or missing/null version IDs leave an error.
4. Active legal hold or retention keeps the file. Denied retention APIs leave an error.
5. Otherwise the exact observed versions become deletion targets. No remaining
   versions produces a keep decision.

Object Lock checks use bucket configuration and exact-version hold/retention
APIs. Missing HEAD headers never establish permission. Governance retention is
never bypassed. Errors remain `unknown`, with a recorded reason; deletion cannot
start while any decision is unknown.

## Destructive execution and recovery

```sh
cargo run -p buzz-admin -- user-deletions media \
  --pubkey <npub> --delete --offline --request-id <approved-request-uuid>
```

The approved request must already be at `purge_media`. The command verifies the
subject and approved scope, claims/renews the request lease, runs media, records
success and releases the lease. It does not advance the workflow or implement
its missing gate. `--offline` asserts that relay and external writers have stopped
and their in-flight work has finished. They must stay stopped through retries.
The command does not enforce this operator obligation.

Execution uses the serving writable `DATABASE_URL`; the source URL must be unset
or identical. The stage binding includes `policy="upload-ownership-v1"` and
rejects old decisions rather than silently applying new semantics to old targets.
`--run-id` is only for preview and conflicts with deletion.

The shared pipeline publishes discovery atomically into
`entity_deletion_media_files`, with a receipt in `entity_deletion_stage_history`.
Zero candidates still gets a receipt. Preview's parent request is terminal and
its stage is never current, so it cannot authorize execution. File statuses are
`unknown`, `dont_delete`, `to_be_deleted` and `deleted`.

Delete validates approved source communities. Classification uses saved evidence.
Immediately before applying saved targets, a separate bounded query revalidates
only the candidate hashes' database upload claims. New other-uploader or changed
source evidence stops execution; it never replaces already-frozen targets.
Retries reuse the discovery and targets, and repeat this candidate revalidation.
The audit schema currently has no action/actor/object-hash lookup index, so a
hash predicate bounds returned evidence without guaranteeing an indexed read.

Storage attribution from discovery is retained, and execution checks presence
and retention for each exact target version. No second bucket-wide discovery is
performed. The caller must exclude concurrent source/storage writes and other
deletion jobs operating on the same storage. A request lease coordinates only
workers for that request; it does not block other requests or normal writers.

Byte-bearing versions are deleted before markers. After deletion, listing must
prove each exact version absent; permission errors/timeouts are not absence.
Only then is the file marked `deleted`. A lost acknowledgement or interrupted
multi-version deletion retries those same versions without deleting replacements.
After all selected work succeeds, cleanup removes captured departing
`media_uploaders` rows, including for kept files.
Audit history and `media_publications` remain. The media handler does not delete
messages, profiles or automations.

Every journal write verifies generation and execution lease. Unfinished work
retains immutable targets and progress. A `deleted` result means the selected
versions are absent, not that intentionally kept files or future reuploads have
been erased. This is a media stage, not proof of complete account erasure.

## Bounds and fixtures

Defaults: one million database source rows, one million storage versions,
100,000 hashes, 128 MiB evidence/storage descriptors, 1 MiB per source/file and
64 KiB per upload record. `--max-source-rows` and `--scan-timeout-secs` can raise
positive database budgets on a new run; they do not permit partial publication.

The disposable media fixture includes exclusive and unposted uploads, shared
uploaders in one/two communities, other-community metadata, and a copied-link
case. Exclusive, unposted, metadata-only and copied-link cases delete; recorded
other uploaders keep their files. The copier's message remains after media and
content cleanup for the departing user. Fixture commands mutate services and
must only run when local execution is authorized.
