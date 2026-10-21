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

1. Deploy share recording to every pod with reads in `log-only`.
2. Run `buzz-admin backfill-media-shares` after all pods record shares.
3. Inspect `buzz_media_read_access_total{mode="log-only",outcome="deny"}` and `outcome="error"` in Prometheus.
4. Investigate denied workflows and repair missing provenance before enforcing.
5. Set `BUZZ_MEDIA_READ_MODE=enforce` and restart every pod.

The counter uses only fixed `mode` and `outcome` labels: `allow`, `report`, `deny`, or `error`.
It measures authorization evaluations on GET and HEAD, before checking whether storage contains the file.
It counts requests, not distinct files, and includes thumbnail requests checked against the original hash.
Use controlled client smoke tests to investigate counts; hashes, keys, and URLs are not metric labels.
Log-only evaluations create no shares and do not grant future access.

Backfill reevaluates references received after recording began using their authors' current permissions.
It does not remember earlier refusals: a rerun can grant an old reference after its author gains access.
For a community-wide note, that grant can survive removal of the author's channel access and deletion of the original private post.
The grant remains tied to the note and ends when that note is deleted.
This migration limitation is accepted for recording and log-only rollout; before enabling enforcement, explicitly accept it or prevent reevaluation.
Reruns deduplicate rows but do not preserve earlier authorization decisions.

Previously unshared uploads without uploader proof still need an upload of the exact bytes before they can be shared.
Historical uploader backfill and unparsed-reference recovery remain separate work.
To roll back file denials, set `off` or `log-only` and restart every pod.

## Open reports

A community owner or admin who can view the moderation queue may read an attachment
on a reported message outside their channels while that report has status `open`.
The report, message, and file grant must belong to the requesting community.
The source message must still exist and be undeleted.
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
