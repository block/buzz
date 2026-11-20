# Private-channel media revocation

`BUZZ_MEDIA_READ_MODE` is a startup setting: `off` (default), `shadow`, or
`enforce`. Unknown values fail startup. Channel revocation is effective only
in `enforce`. Blossom authentication, tenant binding, and community admission
remain mandatory in every mode.

## Read policy

Every GET and HEAD, including each Range request, authorizes the parent hash
before returning blob bytes, thumbnail bytes, existence, size, or range errors.
A tenant sidecar lookup precedes the new database lookup. Authorization uses
a fresh primary-database repeatable-read snapshot, without membership caches
or replicas. A removal committed before that snapshot excludes the member.
A request authorized before removal may finish afterward.

A file is readable through any current readable reference. Shared bytes may
therefore remain readable through another open channel or a DM the person
still belongs to. Channel owners/admins have no blanket exception to private
channel membership. Ordinary REQ event visibility, recipient-gated kinds and
encrypted engrams retain their restrictions. Gift-wrap ciphertext itself
cannot supply hidden attachment references.

Counted references are imeta `url` and `image` fields, link-preview snapshot image/favicon fields, profile `picture` (or `image` when
`picture` is absent) and `banner` fields, group metadata `picture` tags, and emoji tags on emoji lists
and sets and per-emoji entries (kind 30180). Persona definitions (kind 30175)
count `avatar_url`, with the same owner-only/shared visibility as their events.
Root-relative paths and the tenant's HTTP/HTTPS media bases count.
Animated profile descriptors count their poster and decoded animation URL
independently, with the same tenant check on each. The indexed profile lookup
expects hash characters to remain literal, as in the desktop descriptor.
Republish over-encoded hashes such as `%61` or JSON Unicode-escaped hash characters such as `\u0061` using canonical URLs. Ordinary Unicode escapes in profile names are supported; the URL hash must remain plain ASCII. A pasted URL in free text alone grants nothing; messages must publish imeta.
Deleted events/channels grant nothing. Only the newest live edit supplies a
message's attachments; its original must remain live. Original link-preview
snapshots remain readable through an edit unless the original or any live edit carries the exact
`["link-preview","none"]` marker; later edits do not undo suppression; this does not retain removed attachments.
Superseded profiles
are excluded. Current community icons grant community-wide access.

The read path probes the existing tags GIN index for imeta hashes and exact
local URL aliases. It scans content only for current kind-0 profiles and kind-30175 persona avatar fields in the
community, never all message bodies. LLVM/JIT is disabled only in this short authorization transaction: compiling
partitioned permission probes costs seconds without amortizing that cost.
Other database work retains its configured JIT policy. A two-second statement deadline and a
1,024-candidate budget per probe and a 16 MiB aggregate work budget bound each lookup. An accessible candidate can succeed
before an overflow refusal. Candidates stream one at a time rather than allocating an entire page of message bodies. Budget exhaustion or a database failure returns
503; an inaccessible/missing reference returns 404. NIP-FI strict mode keeps
its existing authorization-unavailable response contract.

## Uploads and publication

After the upload pipeline verifies the supplied bytes, the relay records
`media_uploaders` through community write admission. This is independent of
optional audit/upload moderation records. An unposted file is previewable by
its uploader only. Real reuploads may prove possession for another uploader.
Possession authorizes publication; a known URL/hash alone does not. A registered
agent must hold its own upload proof or readable reference; owner proof is never inherited.

The shared publication check runs before common ingestion dispatch (WS and
HTTP, including metadata commands) and before direct workflow persistence.
It uses the same reference extractor as reads, plus structured command URLs.
A durable `media_publications` marker consumes preview authority before
publication dispatch and survives source deletion. A later rejected operation
may conservatively consume a preview; reupload still proves publication
rights. Reuploads also record a boundary when a legacy reference already exists, so later purging that reference cannot revive previews. The marker grants no download access. Both tables are purged during
community deletion, and every writer uses admission.

Deployment-admin feedback attachment reads retain their independent admin
and feedback-row provenance checks. Feedback does not grant community-wide
media access through the ordinary media endpoint.

## Rollout and recovery

1. Before any pod enters shadow, [build and verify the edit-target index](private-media-index-deployment.md).
   Small databases may auto-migrate through 0086: a missing index builds
   within a one-second lock and five-second statement budget, even with data.
   Large deployments disable auto-migration and prebuild concurrently per
   partition before applying 0086 or desired-state schema. A budget failure
   leaves migration retryable; a valid prebuild skips the blocking build.
   Bypassing the schema step can cause lookup timeouts/503 in enforce. Restore
   off, finish index construction/verification, then begin shadow.
   The migrations add plain tenant tables and an index, with no SQL functions,
   triggers, foreign keys or data rewrite.
2. Deploy the new binary on every writer with mode `off`.
3. Restart all writers in `shadow`; inspect
   `buzz_media_read_access_total{mode="shadow",reason="no_ref"}`,
   `reason="not_member"`, and `reason="lookup_error"`, plus
   `buzz_media_publication_access_total`. Exercise avatars, icons, emoji,
   uploads, private messages and old attachments during a staging soak.
4. Repair unsupported/historical URLs or republish them through an authorized
   uploader before restarting every serving pod in `enforce`.

**Enforce prerequisite: upgrade every edit producer.** The updated
`buzz messages edit` resolves the latest live attachment state on the primary and
carries its complete imeta set for kinds 9/40002/40008/45001/45003. For those kinds,
`--clear-attachments` explicitly removes all attachments; later text edits
preserve that empty set. Other channel-tagged kinds keep their original media references after edits,
and the CLI rejects `--clear-attachments` for them. SDK
integrations must use `buzz_sdk::build_edit_with_media` with the latest
effective metadata. The compatibility `build_edit` helper still emits an empty
attachment set and therefore removes attachments on kinds 9/40002/40008/45001/45003. A URL in the
body does not retain access.

Deploy the relay with the [versioned effective-edit
lookup](effective-message-edits.md) before releasing the new CLI. When editing
kinds 9/40002/40008/45001/45003, a new CLI against an older relay aborts without publishing; an
old CLI against a new relay emits empty imeta and revokes attachments on all
five kinds, including diff/forum attachments that previously remained readable.
Other kinds use plain edits without a lookup or a relay-version dependency. The
`buzz-dev-mcp` multicall named `buzz` delegates to the same CLI entrypoint. The
PostgreSQL regression `cli_successive_edits_preserve_and_revoke_attachments`
exercises that entrypoint through HTTP and checks GET/HEAD, removal, and
mixed-case edit targets.

**Remaining enforce blockers include the separate `block/buzz-app` MCP edit producer
and the active producers listed below.**
Its direct publisher at `src/buzz-mcp/tools.ts` copies the original event's
imeta rather than resolving successive edits. It must adopt effective state
and pass the same removal regression before enabling enforce for its users.
Audit other third-party SDK/direct publishers too; upgrading the CLI alone
cannot change their signed events.

The relay rejects a retaining edit when its signer cannot publish any carried
attachment. Agent ownership does not grant private-channel media access: an
owner outside that channel cannot retain its files. Restore access or explicitly
remove the attachments with `--clear-attachments`; the CLI itself also requires
read access to the original. Concurrent edits remain ordered by `(created_at,id)`;
coordinate writers when attachment changes overlap. There is no compare-and-swap
precondition on message edits.

This client change has no feature flag. Observe
`buzz_media_publication_access_total{mode="shadow",reason="denied"}` before
enforcement. Roll back the CLI by reverting its release; retain the additive
relay lookup. Rolling the relay back first causes new CLI/MCP edits on all five kinds to
abort without publishing. Already-published imeta edits remain valid; rollback
restores the older relay's media-reference rules, so recheck access before
resuming enforcement.

There is no grant backfill: existing stored references are evaluated directly,
and signed event tags are never rewritten. Legacy unreferenced/orphan blobs
have no upload proof and are denied in enforce mode; their owner must reupload.
Pre-enforcement references remain authoritative, including a preexisting avatar
that points to the same bytes as a private attachment. Custom historical media
hosts beyond the tenant's current authority require explicit repair.

Roll back policy by restarting with `off`; retain the additive schema and
publication markers. If rolling back to a pre-0086 binary, keep auto-migration
disabled and suspend community deletion: its exact-set deletion catalog does
not recognize the two added media tables, so deletion fails closed. Resume
deletion after deploying a binary whose deletion catalog includes them. Mixed off/enforce serving pods cannot provide revocation.
Shadow/off do not close the original exposure. Shadow authorization lookup failures, including sidecar failures, are counted
without refusing reads. The serving path can still fail if storage cannot supply
bytes. Enforcement fails closed.

Successful media responses, authorized range errors and media denials use
`private, no-store` in all modes, including off. Off preserves the previous
authorization behavior; cache headers intentionally change before enforce to
avoid creating more year-long immutable entries during the preparatory rollout. New
responses therefore increase repeat-download bandwidth. The desktop proxy
passes this policy to the WebView; its local file cache and the mobile client's
local copies can still display previously fetched bytes. Old immutable HTTP
cache entries, files already downloaded, screenshots and copied bytes cannot
be recalled. The guarantee applies to requests reaching the relay; migrating
existing client caches is a separate client rollout concern.

Any authorized member may deliberately republish or reupload bytes into a
readable channel or avatar. Revoking one channel cannot revoke that independent
share. This policy does not prevent authorized recipients from copying data.

Media authorization uses a shared semaphore across database-handle clones,
limited to one quarter of the writer pool, capped at eight lookups. Saturation
waits up to 500 ms before returning authorization-unavailable, counted with
`reason="capacity"`, before pool acquisition. Every Range
request reauthorizes; all events, including emoji lists and sets, have a 512-reference cap, and checks run
sequentially within that connection bound. Missing sidecars reject publication
in enforce mode; shadow counts failures and accepts the write, including stale
avatar metadata edits and upload-proof/publication-marker errors.

Eight slots provide an upper bound of `8 / mean_lookup_seconds` lookups per
second per pod before queueing (for example, 20 ms means about 400/s; 100 ms
means about 80/s). A smaller writer pool reduces that bound proportionally.
These are capacity calculations, not measured throughput targets. Measure full
request latency and concurrent image/avatar bursts, including all probes,
sidecars and pool acquisition. Keep sustained traffic below the measured bound;
the 500 ms queue absorbs bursts but cannot increase throughput. Each lookup
uses a writer connection and several statements; no-store repeats that work on
every view. The synthetic query-plan times exclude storage/network/pool costs.

During staging soak, abort enforcement if writer authorization acquire p95
exceeds 50 ms for five minutes, event-write acquire p95 doubles its pre-rollout
baseline, or media lookup_error/capacity refusal exceeds 0.1% of requests for
five minutes. Investigate query latency, pool utilization and storage errors
before retrying. Shadow failures must be repaired before enforce, since a failed
recording does not establish possession. Uploads made in off mode record no
proof: uploads not yet posted when enforce starts require reuploading.

Before enforce, every client that uploads avatars with one key and publishes
profiles with another must transfer the actual poster/animation bytes to an
upload signed by the profile key. Owner relationships never inherit possession.
Validate this in each supported client during shadow; upgrading only this
repository's desktop does not migrate other clients. This includes the new
desktop/web app in `block/buzz-app`, mobile, and CLI/plugin integrations that
publish agent profiles: verify their signer and upload flow before enabling
enforce for their users.

The legacy desktop transfers both poster and animation bytes for changed
persona avatars before saving the edit; transfer failures leave the old local
avatar intact. Team and individual agent snapshot imports also transfer selected
target-local avatars before publishing each new agent profile. Imports report
transfer failures in their profile-sync results. Validate all three workflows
against enforce mode before rollout. Unchanged-avatar reconciliation never
borrows the owner's signer.

New managed-agent creation explicitly downloads the chosen target-local avatar
with the captured owner's signer, verifies its image bytes and hash, then uploads
those bytes with the new agent's signer. The agent holds its own possession
proof. The captured community stays pinned through profile publication, even
if the owner changes workspaces during transfer. Automatic reconciliation never
borrows the owner's proof or signer.

Changed-avatar edits refuse the entire save before any transfer when a linked
instance has no usable signing key. The error names affected instances; restore
their keys, or remove/unlink those instances, then retry. The previous persona
and instance avatars remain unchanged, so the same requested avatar is retryable.
Keys may be permanently lost; waiting alone does not repair a lost key.

The relay folds effective attachment state for the same five kinds accepted by
the lookup: 9/40002/40008/45001/45003. CLI and MCP text edits preserve that state;
`--clear-attachments` replaces it with an empty set on all five. After removal,
access through that reference is revoked; another live reference or upload
proof may independently authorize the blob. An older community relay without
the lookup refuses these edits through the new CLI/MCP, so upgrade relays first.

## Historical diff and forum edits

**Readiness blocker:** before enforcement, audit existing kind 40008/45001/45003
originals and every active edit producer. Expanding read folding applies to
stored events, including old CLI edits that emitted no imeta. Upgrading clients
does not repair those signed historical edits. An authorized writer must review
and republish the intended full set; never infer that an empty edit meant to
retain the original's files. No automatic historical rewrite is included.

Run the read-only [preflight query](sql/private-media-edit-preflight.sql), binding
`$1` to a community UUID. For psql, prepare its SELECT as
`PREPARE private_media_edit_preflight(uuid) AS ...;`, then execute
`EXECUTE private_media_edit_preflight('<community UUID>');` inside a read-only
repeatable-read transaction. Use a bounded statement timeout. Each row counts
live added-kind originals with imeta whose selected live edit omits imeta. The
selection uses the same normalized first-valid target, same channel/community,
and descending `(created_at,id)` ordering as the read path. Missing kinds have
zero matches. These are affected references, not necessarily inaccessible blobs:
another live reference or upload proof can independently authorize the bytes.
Partial attachment changes require separate review. No production count is
claimed by this PR.

| Producer | Current behavior and rollout requirement |
| --- | --- |
| Old CLI (through `96af488d`) | Plain edits emit no imeta; on the new relay they revoke diff/forum attachments that previously remained readable. |
| Updated CLI | Resolves the full effective set for all five kinds; explicit clear stays empty. Release after the relay. |
| SDK | `build_edit` emits no imeta. `build_edit_with_media` copies the supplied set; callers must resolve latest state. Third-party callers remain a prerequisite. |
| Legacy desktop chat | Carries rendered attachment state through `submitMessageEdit.ts` and `events.rs`; its local edit selection does not provide the primary lookup's full ordering guarantees. Audit deployed versions. |
| Legacy native generic edit | Copies supplied media, defaulting to empty; no effective lookup. Its forum readers keep showing original metadata and only fold preview suppression. Audit any direct generic callers before enforcement. |
| Mobile editor | `message_actions.dart` passes only custom emoji tags to `editMessage`; no imeta is retained. This remains an uncorrected producer blocker. |
| App MCP | Current main copies original metadata; the paired MCP fix adopts latest state. The fix must ship before enforcement for its users. |
| App UI | `fold.ts` selects the latest edit having imeta, falling back to the original. The UI editor can restore older files after an empty removal; it remains an uncorrected producer blocker. |
| ACP | Edit routing consumes events; generic publication forwards already-built signed events. It does not resolve attachments for producers. |

Source paths are relative to this repository, or to `block/buzz-app` for app
entries. App audit snapshot: `cae3922574`; paired MCP implementation is tracked
in [the counterpart PR](https://github.com/block/buzz-app/pull/2937). This audit
identifies source behavior, not installed versions.

Mode effects are precise: `off` skips reference checks for media reads and
publication. `shadow` evaluates the five-kind policy and records its metrics,
but does not deny reads/publication for policy failures; it also records sticky
publication boundaries best effort. `enforce` denies reads without a current
readable reference and rejects inaccessible carried attachments. Thus historical
empty edits change shadow observations immediately, and deny their superseded
original reference in enforce. Storage and tenant admission still apply in all
modes. The lookup and new CLI behavior apply independently of read mode.
