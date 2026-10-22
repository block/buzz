# Media access rollout

File authorization is separate from authentication, NIP-FI, and community admission.
All three modes keep the existing authentication, host binding, ban, and membership checks.

Set `BUZZ_MEDIA_READ_MODE` in the relay deployment environment and restart every pod:

| Value | File-access behavior |
|---|---|
| `off` | Skip the new file-share check and its database lookups. |
| `log-only` (default) | Evaluate file access and count outcomes, but preserve existing reads, including on lookup errors. |
| `enforce` | Require file access; deny unauthorized reads with 404 and authorization failures with 503. |

Unknown values fail startup. This setting loads at startup; it is not a live feature flag.
Changing modes requires a rolling restart, but no code rebuild.
During that restart, pods use their own configured mode until replaced.

## Rollout

1. Before any backfill, explicitly accept that reconsidered references can create lasting grants; otherwise defer the backfill.
2. Deploy share recording to every pod with reads in `log-only`.
3. Run `buzz-admin backfill-media-shares` after all pods record shares.
4. Inspect `buzz_media_read_access_total{mode="log-only",outcome="deny"}` and `outcome="error"` in Prometheus.
5. Investigate denied workflows and repair missing provenance before enforcing.
6. Set `BUZZ_MEDIA_READ_MODE=enforce` and restart every pod.

The counter uses only fixed `mode` and `outcome` labels: `allow`, `report`, `deny`, or `error`.
It measures authorization evaluations on GET and HEAD, before checking whether storage contains the file.
It counts requests, not distinct files, and includes thumbnail requests checked against the original hash.
Use controlled client smoke tests to investigate counts; hashes, keys, and URLs are not metric labels.
Log-only GET/HEAD evaluations create no shares and do not grant future access.
Event recording and backfill still write grants in every read mode.

Backfill reevaluates references received after recording began using their authors' current permissions.
It does not remember earlier refusals: any run, including the first, can grant an old reference after its author gains access.
For a community-wide note, that grant can survive removal of the author's channel access and deletion of the original private post.
The grant remains tied to the note and ends when that note is deleted.
Accept those resulting grants before the first backfill, or defer backfill until refusal-preserving behavior exists.
Changing read modes does not revoke grants, and preventing reevaluation later does not remove grants already created.
Any unwanted existing grants need a provenance-based repair before enforcement.
Every invocation restarts the scan from the beginning; rows are deduplicated, but earlier authorization decisions are not preserved.

Before recording shares, the backfill restores uploader proof from retained server
`media_uploaded` audit rows with an attributed 32-byte pubkey and a valid SHA-256 object ID.
It scans each community's audit sequence in bounded batches and skips quiescing/deleting communities.
Missing, malformed, or unattributed audit evidence grants no ownership; those files still
need an upload of the exact bytes. Unparsed references need separate repair before enforcement.
To roll back file denials, set `off` or `log-only` and restart every pod.

## Open reports

A community owner or admin who can view the moderation queue may read an attachment
on a reported message outside their channels while that report has status `open`.
The report, message, and file grant must belong to the requesting community.
The source event and an edit's target message must still exist and be undeleted.
Report notes, arbitrary URLs, pubkey reports, and blob-only reports grant no extra access.
There is no role-based bypass for unrelated files, and reports create no public shares.

In enforce mode, each exceptional GET or HEAD requires a durable `moderation_media_read`
audit entry containing the actor, report, source message, and file hash before storage access.
If auditing is disabled or fails, the exceptional read returns 503.
Log-only mode evaluates this permission without writing an exceptional-access audit entry.
Ordinary reads allowed by existing shares do not need the moderation exception.

Closing the report, deleting the source message, or removing the queue role ends this exception
for subsequent requests; ordinary shares can still independently authorize access.
Escalated and processing reports do not qualify for this community-review exception.
Bytes already downloaded cannot be recalled.

## Inactive grant cleanup

The elected usage-poller leader inspects at most 1,000 media grants per tick,
in primary-key order, and removes inactive grants in community-admitted transactions.
The tick uses `BUZZ_USAGE_METRICS_INTERVAL_SECS` (300 seconds by default).
The scan wraps at the end; its cursor is held in memory and restarts after a relay restart.
Tenant-local failures retry on a later scan cycle, and quiescing/deleting communities are skipped.

A grant is inactive when its source event is missing or deleted, or its edit target is no longer live.
Live grants survive cleanup. Inactive public profile/emoji grants are removed completely.
Cleanup compacts inactive channel history to at most one row per community and file: removing every
channel row would incorrectly restore uploader preview access after message deletion.
Whole-community purge removes that final row as well.

Successful reclamation logs `reaped inactive media shares` with the number removed;
failures log `failed to reap inactive media shares` or a tenant-local sweep error.
This reclaims grant records; file retention follows the storage policy.
