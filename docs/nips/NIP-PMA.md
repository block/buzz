# NIP-PMA: Private Managed-Agent Config

`draft` — implemented codec (`crates/buzz-core/src/private_managed_agent.rs`)
and relay gates. This document describes the wire contract as shipped; where
an earlier reservation planned a stricter mechanism, the difference is called
out explicitly rather than left implied.

## Purpose and kind

Kind `30179` is an owner-authored, addressable, owner-readable event carrying
the runnable private configuration of one managed agent: its nsec, optional
NIP-OA attestation, secret environment, and the portable runtime settings a
second device needs to run the same agent. Its coordinate is
`(owner pubkey, 30179, agent pubkey)`.

Kinds `30175` (definition) and `30177` (instance) remain the public
projections. `30179` carries secrets plus the instance's runnable snapshot,
including public instance fields (name, definition link, parallelism,
`respond_to`, allowlist) and definition mirrors (runtime, model, provider,
system prompt). Definition/catalog display metadata (display name,
description, avatar, slug) does not enter the payload.

## Encryption exception

[VISION.md § Encryption](../../VISION.md#encryption) states Buzz's one model:
TLS in transit, storage-layer encryption at rest, server-readable content so
eDiscovery works on everything. Kind `30179` is the deliberate exception. Its
content is NIP-44 v2 ciphertext from the owner's key to itself, because the
payload holds agent private keys and API credentials that must never be
readable by relay operators, backups, or search. The relay treats the event as
opaque author-only data: its ciphertext is unavailable to relay-side
discovery, while the event's existence, coordinate, and tags, the public
`30175`/`30177` projections, kind:5 deletions, and the agent's own signed
messages remain server-readable as usual. Any change that widens `30179`'s
contents beyond secrets and runnable settings must revisit this exception.

## Relay contract (as enforced)

- `30179` is a parameterized-replaceable kind: the relay keeps one live head
  per coordinate and resolves concurrent writes by ordinary NIP-33
  last-write-wins on `created_at`.
- `30179 ∈ AUTHOR_ONLY_KINDS`: REQ, COUNT, and subscription fan-out return the
  event only to the authenticated author. Existence, tags, and counts are not
  revealed to anyone else.
- Global-only scope (`UsersWrite`), never channel-scoped; excluded from the
  FTS allowlist (`migrations/0033_private_managed_agent_fts.sql`).
- Deletion is an ordinary NIP-09 kind:5 naming the coordinate. Desktop
  enqueues kind:5s for both the `30177` and `30179` coordinates in one local
  transaction (`tombstone_managed_agent_at`); a kind:5 naming either
  coordinate is treated locally as covering both heads.

There is **no** transactional CAS, no aggregate submission endpoint, no
`state` tag, and no relay-side anti-resurrection rule. Generation and
predecessor below are advisory audit metadata, validated for shape only.

## Signed outer envelope

Exactly these two-element tags are permitted:

- `d = <64 lowercase hex agent pubkey>` exactly once;
- `g = <canonical positive decimal generation>` exactly once;
- `prev = <64 lowercase hex predecessor event id>` exactly once after
  generation 1 and absent at generation 1.

Any other tag, a duplicate tag, or a tag with a different arity is rejected
before decrypt. Content is non-empty NIP-44 v2 ciphertext no longer than
`MAX_CIPHERTEXT_BYTES`. Event ID, signature, kind, author, tag grammar, and
canonical curve-valid agent key are validated before decrypt. The decrypted
payload repeats owner, agent, generation, and predecessor; any mismatch is
rejected as corruption.

## Decrypted v1 payload

```json
{
  "format": "buzz-private-managed-agent",
  "version": 1,
  "agent_pubkey": "<hex>",
  "owner_pubkey": "<hex>",
  "generation": 3,
  "previous_event_id": "<hex>",
  "updated_at": "<RFC3339>",
  "identity": { "private_key_nsec": "nsec1…", "auth_tag": "[…]" },
  "config": { … },
  "extensions": { "namespace:key": … }
}
```

Duplicate JSON member names anywhere in the plaintext are rejected. Known
members are strictly typed. Unknown members at the top level and inside
`config` are **preserved verbatim** (`extra` maps) so an older writer editing
one known field never drops data authored by a newer Desktop; a matching
known key always binds to the typed field, so an unknown member can never
shadow one. `extensions` keys must be namespaced (`contains ':'`) and bounded.
Core semantics never depend on `extra` or `extensions`.

`identity.private_key_nsec` MUST derive the `d` coordinate. When present,
`identity.auth_tag` MUST be a cryptographically valid unconditional
(`conditions = ""`) owner-to-agent NIP-OA attestation whose owner equals the
event author and whose agent equals the nsec-derived coordinate.

`config` members (all optional unless noted):

| member | type | notes |
|---|---|---|
| `relay_url` | string, required (may be empty) | device-validated; empty = workspace relay |
| `name` | string, required, non-empty | instance handle |
| `persona_id` | string | definition linkage; carried verbatim. A device without the linked definition refuses to materialize the instance (no lifecycle row is written) until the definition arrives; it never detaches the link |
| `runtime`, `model`, `provider`, `system_prompt` | string | definition mirror (authoritative only for definition-less instances) |
| `parallelism` | u32 | instance projection mirror |
| `respond_to`, `respond_to_allowlist` | NIP-AP wire string, hex list | instance projection mirror |
| `agent_command_override`, `agent_args` | string, string list | device-validated before launch |
| `idle_timeout_seconds`, `max_turn_duration_seconds` | u64 | portable |
| `env_vars` | string map | secret-bearing; bounded by `MAX_ENV_*` |
| `backend` | JSON, required | versioned `BackendKind`; device/provider-validated |
| `backend_agent_id` | string | remote identity; ownership device-validated |
| `team_id`, `persona_name_in_team` | string | portable team linkage |
| `relay_mesh` | JSON | versioned mesh marker (definition mirror) |
| `effort_level` | string, ≤ `MAX_EFFORT_LEVEL_BYTES` | canonical harness-agnostic effort; each device normalizes it against the destination runtime at spawn |

Payloads authored before `effort_level` existed decode with it absent
(`None` = inherit); a writer that has never set it omits the key.

## Field authority

Every `ManagedAgentRecord` field belongs to exactly one class:

- **coordinate**: agent pubkey (`d`).
- **`30177` instance projection** (also carried in `30179` so a fresh device
  can reconstruct the instance): name, definition linkage, parallelism,
  `respond_to`, allowlist.
- **definition mirror**: prompt, runtime, model, provider, relay-mesh marker.
  `30175` is authoritative for a definition-linked instance; `30179` carries
  the runnable snapshot for definition-less instances.
- **`30175` definition/catalog projection only**: display name, description,
  slug, name pool, builtin/active flags, sharing/provenance, definition
  behavior defaults, public avatar. Never enters `30179`.
- **private portable canonical**: nsec, auth tag, env, timeouts, team
  linkage, secret-bearing backend configuration, `effort_level`.
- **private but device-validated**: relay URL, explicit command/args, backend
  remote identity.
- **local device policy/derived**: start-on-launch, auto-restart, provider
  policy pending, effective binary paths, installed team directory,
  catalog-derived commands.
- **legacy conversion only**: create-time command/model/provider mirrors,
  deprecated MCP/turn timeout, source-version drift markers, the retired
  `shared` flag.
- **transient local only**: PID and all last start/stop/exit/error receipts.
- **bookkeeping**: `updated_at` rides the payload as advisory metadata but is
  excluded from body equality; `created_at` stays local.

Adding a `ManagedAgentRecord` field fails to compile until it is classified in
`desktop/src-tauri/src/managed_agents/reconcile/tests/field_classification_tests.rs`,
whose exhaustive destructure is the tripwire; the same test asserts that each
class either does or does not change the `30179` payload body the writer
compares, so a misclassification fails at test time rather than drifting.

## Desktop write and read discipline

- A `30179` head is written only through `retain_private_agent_record`, which
  rebuilds the payload from the resolved record, preserves `extra`/`extensions`
  authored by newer clients, and skips the write when the decrypted body is
  unchanged (NIP-44 is randomized, so ciphertext is never compared).
- Interactive edits resolve the relay overlay onto the disk record before
  applying the user's patch, so a follower's edit is authored on top of the
  config it is actually running rather than stale disk.
- Boot publishes a `30179` only when no retained head exists for the
  coordinate; an existing head is left to the interactive paths.
- Inbound heads pass `validate_and_decrypt` and Desktop-side field validation
  (`PrivateConfigPatch::from_payload`) before entering the overlay; a rejected
  head leaves the previously cached patch in place.
- Local-only effort controls (the per-instance effort picker and its
  next-spawn restart semantics) are unchanged: they write `effort_level` on
  the record, and that column is what rides `30179`.

### Bootstrap, deletion, and recovery boundaries

Before boot may mint a private head from disk, native Desktop fetches the
owner's authenticated history for kinds `5`, `30177`, and `30179`. It verifies
signatures and owner scope and completes pagination before applying history.
HTTP `/query` pages use `(until, before_id)` in `created_at DESC, id ASC` order;
an empty or short page establishes exhaustion. Bootstrap is capped at 200
pages of 500 events, 32 MiB of serialized events, and 30 seconds of fetching.
The kind:5 query includes unrelated owner deletions: a generic `#a` filter is
not used because relay post-filtering could make a partial page look complete.
Cap/network errors suppress boot agent publication and remain visible on the
Agents page, with reconnect retry and operator guidance. Cached retained
configuration can still hydrate; corrupt retained authority is an error, never
an empty authoritative set.

Private heads are applied before historical deletions. Public `30177` heads
are used only as deletion-survival witnesses: a strictly newer public-only
recreation preserves the existing local lifecycle row and key. The witness is
not retained as an already-applied public update, so the ordinary subscription
still owns public policy application and its runtime transitions. A private
head covered by the historical deletion is removed from retention/overlay;
the public witness does not reconstruct private configuration. The retained
watermark also denies disk fallback when no newer private head exists: config
reads, edits, and launch must refuse rather than execute deleted settings.
Hydration and self-authored absorption reject covered retained private rows,
even if an older client left such a row behind. A strictly newer validated
private head restores config authority. The local identity/process receipt
remains available for explicit deletion and owned-process Stop; config exports
and persona cascades exclude identities without private authority.

A kind:5 watermark for either managed-agent coordinate fences both heads at
`created_at <= deletion.created_at`, including replay after restart. A newer
recreation does not discard that watermark. Local deletion atomically prepares
both signed tombstones, archive state, head removal, and an exact owner/agent
cleanup obligation before destructive process/JSON/key cleanup. Inbound
managed-agent deletion also prepares its obligation before cleanup. Failed
cleanup keeps the obligation for boot recovery, which runs before positive
reconciliation; pending cleanup gates authority readiness and head admission.
Recovery targets journaled identities, not every agent absent from local disk:
relay-only configurations are legitimate records, not deletion evidence.

These are client recovery rules, not relay CAS or a cross-device lease. A
concurrent edit arriving after the history snapshot remains an ordinary LWW
race. No live two-device or provider conformance guarantee follows from the
local bootstrap ordering alone.
