NIP-AP
======

Agent Personas
--------------

`draft` `optional`

This NIP defines `kind:30175` persona events — public, addressable definitions that describe how to instantiate an AI agent. A persona carries identity (display name, avatar), behavioral configuration (system prompt, model, runtime), and an optional name pool. It is the "blueprint" from which agents are spawned.

This NIP also defines teams of agents (see "Teams") and `kind:44300` instructions versions, an owner-private history of each agent's and each team's instructions (see "Instructions versions: kind:44300").

## Kind

This NIP claims `kind:30175` for agent persona definitions and `kind:30178` for the shareable team-catalog projection (see "Team catalog projection: kind:30178"). Both are in the NIP-33 parameterized replaceable range (30000–39999) per [NIP-01](01.md): addressed by `(pubkey, kind, d_tag)`, with only the latest event per address retained.

This NIP also claims `kind:44300` for instructions versions (see "Instructions versions: kind:44300"). It is a regular event: stored, append-only, and never replaced.

A dedicated kind (rather than encoding personas as NIP-78 `kind:30078` "Application-specific Data") is taken for the same reasons as [NIP-AE](NIP-AE.md): (1) it isolates this NIP's address space from any other application using the same pubkey — persona slugs cannot collide with another app's `d` tag choices; (2) it lets observers, indexers, and unknown-kind viewers identify persona events from the kind alone, without parsing content as a namespace demultiplexer.

## Roles

- **owner** — a Nostr identity (`pubkey_o`) that publishes and manages persona definitions. Typically the workspace operator.
- **agent** — a Nostr identity instantiated from a persona. Agents do NOT author persona events; they consume them. An agent MAY store a private snapshot of its originating persona in a [NIP-AE](NIP-AE.md) engram at `mem/persona` (encrypted, owner-readable).

## Slugs

The `d` tag of a persona event is the **plaintext persona slug**. A valid slug matches:

```
^[a-z0-9][a-z0-9_-]{0,63}$
```

Total length: 1–64 bytes. Slugs are flat identifiers (no path separators), unlike [NIP-AE](NIP-AE.md) memory slugs which are hierarchical (`mem/…`).

### Plaintext rationale

The d-tag is deliberately NOT blinded (contrast with [NIP-AE](NIP-AE.md) which HMAC-blinds d-tags to protect memory slug confidentiality). Personas are public definitions meant for discovery:

- Direct filter queries: `{kinds: [30175], authors: [pubkey], "#d": ["my-persona"]}`
- Human-readable addressing in UIs
- Cross-workspace sharing without a shared secret

## Event envelope

```jsonc
{
  "kind": 30175,
  "pubkey": "<pubkey_o>",
  "created_at": <unix_seconds>,
  "tags": [
    ["d", "<persona-slug>"]
  ],
  "content": "<json_body>"
}
```

There MUST be exactly one `d` tag and it MUST contain a valid slug per the grammar above. The relay enforces this constraint on ingest. There is no `p` tag — persona events are owner-to-self definitions, not directed at a counterparty.

Implementations MAY include a [NIP-31](31.md) `["alt", "agent persona definition"]` tag to give unknown-kind viewers a non-leaking summary. Additional tags beyond `d` and `alt` are not defined by this NIP and have no effect on validity.

## Content body

The `content` field is a **plaintext** (unencrypted) JSON object:

```jsonc
{
  "display_name": "<string>",
  "system_prompt": "<string | null>",
  "acp_command": "<string | null>",
  "avatar_url": "<string | null>",
  "runtime": "<string | null>",
  "model": "<string | null>",
  "provider": "<string | null>",
  "name_pool": ["<string>", ...],
  "respond_to": "<string | null>",
  "respond_to_allowlist": ["<64-hex pubkey>", ...],
  "parallelism": "<integer | null>",
  "session_policy": "<channel | thread>"
}
```

### Required fields

| Field | Type | Description |
|-------|------|-------------|
| `display_name` | string | Human-readable name for the agent definition. |

### Optional fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `system_prompt` | string \| null | `null` | The system prompt injected into agent sessions. Optional since the unified agent model: a definition can be pure configuration (e.g. provider/model only). Readers MUST treat an absent or `null` prompt as "no prompt". |
| `acp_command` | string \| null | `"buzz-acp"` | ACP transport command, distinct from the runtime/harness. See transport portability below. |
| `avatar_url` | string \| null | `null` | URL to an avatar image. |
| `runtime` | string \| null | `null` | ACP runtime identifier (e.g. `"goose"`, `"claude-code"`). |
| `model` | string \| null | `null` | Model identifier (e.g. `"claude-opus-4"`). |
| `provider` | string \| null | `null` | Model provider (e.g. `"anthropic"`). |
| `name_pool` | string[] | `[]` | Pool of display names for agent instances spawned from this definition. When non-empty, the spawning system picks a name from this pool for each new agent instance, enabling multiple concurrent agents from the same definition to have distinct identities. |
| `respond_to` | string \| null | `null` | **Reserved.** Default respond-to policy for instances spawned from this definition: `"anyone"`, `"owner-only"`, or `"allowlist"`. `null` defers to the client default. |
| `respond_to_allowlist` | string[] | `[]` | **Reserved.** Allowlisted author pubkeys (64-char lowercase hex) when `respond_to` is `"allowlist"`. Ignored otherwise. |
| `parallelism` | integer \| null | `null` | **Reserved.** Default max concurrent turns for spawned instances. `null` defers to the client default. |
| `session_policy` | string | `"channel"` | ACP conversation boundary for instances launched from this definition. `"channel"` shares context across a channel; `"thread"` isolates context per channel thread. Direct messages remain conversation-scoped. |

The behavioral fields are definition-level defaults. `respond_to`, its
allowlist, and `parallelism` are copied when an instance is created.
`session_policy` remains definition-authoritative: changing it marks running
linked instances for restart, and the next restart launches with the current
definition value without rewriting an already-deployed instance in place.

Unknown fields MUST be ignored by readers (forward compatibility).

### Transport portability

`acp_command` is optional; legacy absence/null selects stock `buzz-acp` for
new definitions. Portable commands are `buzz-acp` or a name of at most 255
ASCII bytes matching `buzz-[A-Za-z0-9_-]+-acp`. Discovery resolves these aliases
on the receiving device; publication does not guarantee local availability.

Catalog publications carry `["shared", "true"]`. Their writers MUST omit
nonportable commands (including machine-local paths), and MUST emit explicit
`"buzz-acp"` when stock is selected. Foreign catalog readers MUST reject a
present nonportable value, and use stock when the field is omitted/null.

Owner-to-self synchronization of non-catalog heads retains existing custom
command compatibility; it is not a foreign adoption path or a sandbox. On an
owner's existing definition, absent/null transport on a shared head MUST
preserve an existing nonportable local command, because that value may have
been redacted. Otherwise absence selects stock. An explicit portable value,
including `"buzz-acp"`, replaces the previous command. A shared custom command
therefore remains local rather than synchronizing its path to another device.
The `shared` tag is catalog presentation, not confidentiality: all kind:30175
content remains plaintext, including non-catalog heads.

Writers serialize optional `acp_command` after `system_prompt` and before
`avatar_url`; omission preserves the existing reference vector's bytes.

### Prohibited: secrets in content

The content body is **public and unencrypted**. It MUST NOT contain secrets (API keys, tokens, credentials, or any sensitive environment variables). In particular, an `env_vars` field MUST NOT appear in the content body.

Secrets required by agents spawned from a persona MUST be conveyed through a separate encrypted channel — specifically, the [NIP-AE](NIP-AE.md) engram at `mem/persona` (which is NIP-44 encrypted to the agent↔owner conversation key) or through out-of-band injection at spawn time.

## Encryption rationale

Persona events carry no encryption. This is deliberate:

- Personas are *configuration*, not *state*. They describe what an agent should be, not what it has learned.
- Encryption would prevent relay-side indexing, search, and third-party client rendering — all desirable for definitions that workspace members should browse.
- Operators who need confidentiality should use relay-level access control ([NIP-42](42.md) authentication + [NIP-29](29.md) group membership) rather than event-level encryption.

## Replacement semantics

Standard NIP-33: for a given `(pubkey, kind:30175, d_tag)`, only the event with the greatest `created_at` is the **head**. Ties are broken by lowest event `id` per [NIP-01](01.md). Relays SHOULD return only the head; clients MUST select the head from any multi-event response.

## Writing

To write or update a persona with slug `s` and body `b`:

1. Validate `s` against the slug grammar. Reject if invalid.
2. Serialize `b` to JSON. Reject if the serialized body exceeds 65,535 bytes.
3. Compute the head of `s` per NIP-33 and let `T` be its `created_at` (or 0 if no head exists). Set `created_at := max(now, T + 1)`. Monotonicity ensures fresh writes always supersede prior heads regardless of clock skew.
4. Tags: `[["d", s]]`.
5. Sign with `seckey_o` and publish to configured relays.

## Reading

To read a single persona by slug `s`:

```
Filter: {kinds: [30175], authors: [pubkey_o], "#d": [s]}
```

Select the head per NIP-33 rules. Parse `content` as JSON. Validate required fields.

To list all personas for an owner:

```
Filter: {kinds: [30175], authors: [pubkey_o]}
```

Returns all heads. Clients scope by author pubkey — two different owners MAY publish personas with the same slug; these are independent events.

## Deletion

Owners MAY publish [NIP-09](09.md) deletion requests targeting persona events. A deletion request MUST be authored by the same key (`pubkey_o`). Such requests SHOULD include `["k", "30175"]` and use an `a`-tag identifier `30175:<pubkey_o>:<slug>`.

A subsequent write with a later timestamp resurrects the slug under NIP-33 replacement semantics.

The same applies to `kind:30178`: a deletion request SHOULD carry `["k", "30178"]` and the `a`-tag identifier `30178:<pubkey_o>:<team-id>`. Unsharing is distinct from deletion — it is a newer valid head at the same coordinate published *without* the `shared` tag, which keeps the projection readable to its author while retracting it from foreign readers.

## Relationships to other NIPs

### NIP-AE (Agent Engrams)

Agents spawned from a persona MAY store a private snapshot at the reserved engram slug `mem/persona`. This engram:

- Is NIP-44 encrypted (confidential to agent + owner)
- MAY contain secrets (env vars, API keys) that the public persona event must not carry
- Serves as the agent's private, mutable copy of its originating configuration
- References back to the persona event by slug convention, not by event ID

The `mem/persona` slug conforms to [NIP-AE](NIP-AE.md)'s slug grammar and requires no amendment to that spec.

### Slimming: kind:30177 (instance state)

Kind:30177 is keyed by **agent pubkey** (one event per instance) while
kind:30175 is keyed by **definition slug** — they occupy different key
spaces and serve different roles. 30177 remains the per-instance
cross-device sync channel; with the unified agent model it is **slimmed**
to carry only instance-level state:

- Writers MUST NOT include definition-level fields
  (`system_prompt`, `model`, `provider`, `persona_source_version`) in new
  kind:30177 events **for definition-linked instances**. Those resolve
  through the linked kind:30175 definition. Writers continue to publish
  instance-level fields (name, linked definition id, `respond_to` +
  allowlist, `parallelism`).
- **Exception — definition-less instances:** an instance with no linked
  definition is its own definition; writers MUST keep emitting the
  definition-level fields for such instances. (Rationale: old readers
  parse a slimmed event successfully and would overwrite their local
  snapshot with absent values; a definition-linked instance self-heals
  from its definition at next spawn, but a definition-less one has no
  restore path.) This exception retires naturally once all instances are
  definition-backed.
- Readers SHOULD continue to accept legacy "fat" kind:30177 events
  during the transition. Where the linked 30175 head and a legacy 30177
  event both carry a field, the 30175 head is authoritative. For an
  agent's instructions, a current `kind:44300` version for that agent is
  authoritative over both (see "Launch composition").
- Deletion/retention rules for kind:30177 are unchanged so historical
  tombstones keep working.

### Mixed-version note

Clients released before this revision require `system_prompt` in 30175
content and will fail to parse (and therefore silently drop) prompt-less
definitions published by newer clients. This is a benign divergence —
old devices simply do not see new-style definitions until upgraded — not
data corruption. Implementations SHOULD log dropped events rather than
surface per-event errors.

### NIP-OA (Owner Attestation)

Agents spawned from a persona carry [NIP-OA](NIP-OA.md) owner attestation — an `auth` tag proving that `pubkey_o` authorized the agent's key. The persona event itself does not contain attestation; it is the *definition* from which attestation is issued at spawn time.

## Team catalog projection: kind:30178

Kind `30178` is the **shareable projection of a team**: owner-authored, parameterized replaceable, addressed by `(pubkey_o, 30178, d)` where `d` is the team id (see "Team identity"). Its `content` is a versioned JSON body carrying sanitized team fields plus ordered, *embedded* member definition projections. The content schema is defined by the client that publishes it; this section specifies only the envelope and the relay's contract.

```jsonc
{
  "kind": 30178,
  "pubkey": "<pubkey_o>",
  "created_at": <unix_seconds>,
  "tags": [
    ["d", "<team-id>"],
    ["shared", "true"]     // optional; presence opts the projection into community reads
  ],
  "content": "<json_body>"
}
```

**Why a separate kind rather than a `shared` tag on the team event (kind:30176).** A team's members are `kind:30175` definitions, which are author-only unless individually shared — so a foreign reader of a shared team could never hydrate its members. Kind `30178` embeds the member projections instead of referencing them: the share is atomic, it covers built-in members that have no `30175` head at all, it is immune to local-id/`d`-tag divergence, and an unshared `30175` stays private.

**The `d` tag is a team id, not a persona slug.** It is either a UUID or a built-in identifier such as `builtin-team:welcome`. The colon is illegal under the persona slug grammar, and rewriting ids to fit would change the team's identity — so the relay applies a laxer rule (see below) to `30178` than to `30175`.

**Content carries only sanitized fields.** No environment variables, no `respond_to` allowlist pubkeys, no source or local ids, no filesystem paths, no secrets. Sharing a team makes the team's and every member's instructions community-readable plaintext.

**A share is a snapshot, never history.** A writer embeds the team's current instructions at the time it publishes (see "Team instructions") and no `kind:44300` versions. Saving a version does not change an existing share; the owner updates a share by publishing a new head. A client MAY show the owner that a share is older than the team's current instructions. A reader that adopts a shared team creates a new team with a new id; the adopted team starts its own instructions history.

## Teams

A **team** is an owner-defined group of agents that share optional **team instructions**. This section defines what a team is, what membership means, and how team instructions reach an agent at launch.

A client stores its own team records (name, members, other settings) in a client-defined, owner-private form. This NIP does not define that storage, so it does not let one client discover another client's private teams or memberships. The shareable form of a team is `kind:30178`.

### Team identity

A team is identified by `(pubkey_o, community, team-id)`: the owner, the relay community holding the team's events, and the team id.

A team id MUST be non-empty, MUST be at most 64 Unicode scalar values, and MUST NOT contain Unicode control or whitespace characters. This is the same grammar as the `kind:30178` `d` tag. Team ids are compared byte for byte, with no trimming, case folding, or Unicode normalization.

Clients SHOULD mint a new UUID for each new team, including a team created by importing or adopting another team. A client MUST NOT give a new team the id of a deleted team.

A client MUST NOT publish instructions versions for a team whose id does not match this grammar. Such a team keeps its client-defined instructions and has no version history.

### Membership

An agent is a **member** of a team when its owner binds it to that team. A binding is scoped by owner and community. It is not channel membership and grants no relay access. A binding follows the team's current instructions; it does not pin the text in effect when the agent joined. An agent MAY be a member of more than one team.

### Team instructions

A team's current instructions are its current team-subject version (see "Instructions versions: kind:44300"). An invalid current version makes the team's instructions invalid.

Before a team has any version, a client MAY use instructions held in its private team record. A client moving those instructions into versions MUST publish the first version and confirm that the relay stored it before removing the private copy. After that, the private copy is not authoritative. A client that cannot fetch or decrypt a team's versions MUST NOT treat that failure as the team having no versions.

### Launch with teams

When an agent starts, the client resolves the current instructions of every team the agent is a member of and compares them after trimming leading and trailing whitespace. A team without instructions has empty instructions, which is a value like any other.

- If every team's instructions are equal, the agent receives the trimmed text once, as its team layer.
- If any differ, or any are invalid, the client MUST NOT start the agent, and SHOULD name the teams involved. A running agent is not affected. The owner resolves a conflict by changing a team's instructions or removing a membership.

An agent that is a member of no team has no team layer. The team layer is delivered after the agent's own instructions (see "Launch composition").

### Deleting a team

A client that deletes a team SHOULD also delete the team's instructions versions (see "Deletion: kind:44300"). It MUST first confirm that the deletion of its private team record is durable in the store that is authoritative for that record (for a record held on a relay, that the relay stored the deletion), so the team never appears with an older version as current while versions are being deleted. After that, a client MUST NOT publish a version for the deleted team, including a save that was pending when the team was deleted.

At each member's next start, the client removes the deleted team's membership and resolves the team layer from the remaining teams. A running agent is not affected. Missing versions alone never mean that a team was deleted.

## Instructions versions: kind:44300

Kind `44300` records one saved version of an agent's own instructions or of a team's instructions. Every save is a new event, so a subject's events are its full history.

### Event envelope

```jsonc
{
  "kind": 44300,
  "pubkey": "<pubkey_o>",
  "created_at": <unix_seconds>,
  "tags": [
    ["p", "<agent-pubkey>"]   // agent subject, or:
    // ["t", "<team-id>"]     // team subject
  ],
  "content": "<NIP-44 v2 ciphertext>"
}
```

- The owner authors every version. Agents do not author instructions versions.
- An event MUST carry exactly one **subject tag**: either one `p` tag whose value is the agent's 64-character lowercase hex pubkey, or one `t` tag whose value is a team id (see "Team identity"). A subject tag MUST have exactly two elements. Tags are counted by their first element, so an event with both a `p` and a `t` tag, two `p` tags, two `t` tags, or a valueless `["p"]` or `["t"]` is invalid.
- `t` values are case-sensitive. Clients and relays MUST NOT lowercase them, unlike [NIP-24](24.md) hashtags.
- The event MUST NOT carry an `h` tag; it belongs to no channel.

Implementations MAY include a [NIP-31](31.md) `["alt", "agent instructions version"]` tag. Other tags are not defined by this NIP and, apart from the prohibited `h` tag, have no effect on validity.

### Content

`content` MUST be [NIP-44](44.md) v2 ciphertext encrypted with the conversation key of `(seckey_o, pubkey_o)`: the owner encrypts to themselves.

The plaintext is the three ASCII bytes `v1:` followed by the instructions text in UTF-8. The prefix lets an empty text be encrypted; NIP-44 cannot encrypt an empty plaintext.

- The text MAY be empty. An empty version means "no instructions" for its subject.
- The text MUST be valid UTF-8, MUST NOT contain U+0000, and MUST NOT exceed 131,072 bytes.
- A plaintext of 65,536 bytes or more uses NIP-44 v2's extended length prefix: two zero bytes followed by the plaintext length as a 32-bit big-endian integer. An implementation that supports only the two-byte prefix cannot decrypt such a version and MUST treat it as invalid, not absent.
- With these bounds, `content` is at most 218,548 bytes.

Writers MUST enforce the text rules before encrypting. Readers MUST check, in this order:

1. `content` is at most 218,548 bytes, before decoding or decrypting it;
2. it decrypts under NIP-44 v2;
3. the plaintext starts with `v1:`;
4. the text after the prefix is valid UTF-8, contains no U+0000, and is at most 131,072 bytes.

A version that fails any check is **invalid**. Ciphertext length cannot enforce the text bound by itself: a maximum-length text and a text one byte longer produce `content` of the same length.

### Current version and history

A subject's **current version** is its event with the greatest `created_at`; ties are broken by lowest event `id`, as in [NIP-01](01.md).

```
Agent: {kinds: [44300], authors: [pubkey_o], "#p": [<agent-pubkey>]}
Team:  {kinds: [44300], authors: [pubkey_o], "#t": [<team-id>]}
```

With `limit: 1`, these filters return the current version. Clients query one subject per filter.

History is the same filter, ordered by `created_at` descending and then by `id` ascending, and paged with a composite cursor: the `created_at` and `id` of the last event on the previous page, sent as `until` and `before_id`. The next page holds events with `created_at < until`, or with `created_at = until` and `id > before_id`. `before_id` is an extension to the NIP-01 filter, not a standard field. A timestamp-only cursor is not sufficient: concurrent saves on different devices can give several versions of one subject the same `created_at`, and paging by `until` alone either repeats or skips them. A relay MAY return fewer events than `limit`, so a client has read a subject's complete history only when a page returns no events.

If a subject's current version is invalid, clients MUST show the error and MUST NOT substitute an older version, a definition's prompt, or a legacy copy.

### Writing

To save text `x` for a subject:

1. Validate `x` against the text rules. Reject if invalid.
2. Fetch the subject's current version and let `T` be its `created_at` (or 0 if none). Set `created_at := max(now, T + 1)`.
3. Encrypt `v1:` followed by `x` (see "Content"). Tag the event with the subject.
4. Sign with `seckey_o` and publish.

There is no compare-and-swap. If two devices save concurrently, the relay stores both, the newer becomes current, and the other remains in history. A client SHOULD fetch the current version again after publishing and tell the user when its save was stored but is not current.

Before a subject's first version, a client SHOULD publish the text the subject currently resolves to (see "Launch composition" and "Team instructions"), so the text from before the first edit can be restored.

- **Restore** publishes an older version's text as a new version.
- **Reset to template**, for an agent, publishes its linked definition's current `system_prompt` as a new version, or empty text when it has none.

Clients MUST NOT delete versions to roll back. Deleting the current version makes the previous version current; it is not a restore.

### Launch composition

An agent's own instructions layer is the first of these that exists:

1. its current agent-subject version. An empty version yields no instructions and stops resolution; an invalid version blocks the start;
2. the `system_prompt` of its linked `kind:30175` definition;
3. for a definition-less instance, the `system_prompt` in its legacy `kind:30177` event.

The team layer is resolved independently (see "Launch with teams"). The client delivers both layers to the agent's harness as separate layers, the agent's own first.

A version takes effect at the agent's next start; a running agent is unchanged. A client MAY restart an agent to apply a new version. A client that lets local configuration override the resolved instructions SHOULD show the owner that an override is in effect.

This NIP does not define how `kind:44300` relates to the private managed-agent aggregate reserved by [NIP-PMA](NIP-PMA.md). That integration is left to NIP-PMA.

### Launch records

A client SHOULD record, for each agent start, which instructions it used:

- the agent-subject version id, or that the layer came from a definition, came from a legacy event, or was empty;
- the team id and version id of every team that contributed;
- a SHA-256 hash of each layer's text as delivered, after any local override, and the source of any override;
- a run identifier and the start time.

A client that records launches SHOULD write the record once the agent has started and retry until the relay stores it. The record MUST be encrypted to the owner, for example as a [NIP-AE](NIP-AE.md) engram. Version ids and hashes MUST NOT appear in plaintext tags. A launch record shows what the client handed to the harness, not that the harness used it.

### Privacy

Instructions versions are readable only by their author. The agent's key has no access to them; the client decrypts the agent's instructions and hands them to the agent at start. The `p` tag names the subject, not a reader, and grants the agent nothing.

The relay still sees each event's subject tag, timestamp, and padded size, so it can observe how often each agent's or team's instructions change and roughly how long they are.

Clients MUST publish instructions versions, and deletion requests for them, only to the relay community that holds the subject.

### Deletion: kind:44300

Owners MAY publish [NIP-09](09.md) deletion requests for instructions versions, one `e` tag per version. A deletion request for a `kind:44300` event MUST carry `["k", "44300"]`; the relay classifies it by that tag, so it stays private after its target is removed (see "Access control: kind:44300 author-only"). A relay MAY accept only one target per request. Clients SHOULD publish one request per version, retry until every version is deleted, and MUST NOT report a subject's history deleted after deleting only part of it.

- A client that deletes an agent SHOULD delete all of that agent's versions. It MUST NOT delete any team's versions.
- A client that deletes a team follows "Deleting a team".

Deletion withholds versions from readers. It does not erase copies held by the relay operator, and it does not reach copies in exports, `kind:30178` shares, or launch records.

### Older clients

Clients that do not implement `kind:44300` ignore it. They resolve an agent's instructions from `kind:30175` and `kind:30177` as before, so they launch agents with the definition's prompt rather than the agent's current version, and they do not see team versions.

## Relay behavior

### Ingest validation

- The relay MUST accept `kind:30175` events that pass standard NIP-33 validation (valid signature, exactly one `d` tag with a non-empty value).
- The relay stores persona events globally (`channel_id = NULL`); they are not channel-scoped.
- The relay is NOT required to validate that `content` parses as valid `PersonaEventContent` JSON. Relays are dumb stores per Nostr convention; content validation is a client responsibility.
- The relay MUST enforce that the `d` tag is non-empty (standard NIP-33 requirement for parameterized replaceable events).
- The relay MUST enforce shared-tag shape: if a `shared` tag is present, it MUST consist of **exactly two elements** — `["shared", "true"]`. Extra elements (e.g. `["shared","true","extra"]`), wrong values (`["shared","false"]`), missing values (`["shared"]`), or duplicate `shared` tags are all rejected with `invalid:`. The two-element exact-shape constraint is required so that the relay's SQL visibility clause (`tags @> '[["shared","true"]]'`) never matches a stored malformed tag via JSONB containment supersets.

### Ingest validation: kind:30178

Kind `30178` is stored globally and its content is unvalidated, exactly as for `30175`. The envelope rules differ in one respect — the `d` grammar:

- The relay MUST enforce the same `shared`-tag exact shape as `30175`, for the same reason: the read gate and the SQL containment clause must agree on every stored event.
- The relay MUST enforce **exactly one** `d` tag whose value is non-empty, at most 64 characters, and free of Unicode control characters and whitespace. Tags are counted by their first element, so a valueless `["d"]` counts toward the total and fails the value check on its own — otherwise `["d"]` alongside `["d","<team-id>"]` would pass, and a consumer that reads `["d"]` as an empty-valued first `d` tag would address the event at `""` while this relay addresses it at `<team-id>`. Without the non-empty check, generic NIP-33 storage maps a missing or empty `d` to the empty coordinate, collapsing every team into the single `(pubkey_o, 30178, "")` slot — last-write-wins data loss. The character bound keeps the value usable as a NIP-33 coordinate and as a log field.
- The relay MUST NOT apply the persona slug grammar to a `30178` `d` tag; team ids legitimately contain characters (notably `:`) that the slug grammar forbids.

### Ingest validation: kind:44300

- The relay MUST reject, with `invalid:`, a `kind:44300` event that does not carry exactly one subject tag as defined in "Event envelope", whose `p` value is not 64-character lowercase hex, or whose `t` value does not match the team id grammar.
- The relay MUST reject a `kind:44300` event with an empty `content` or a `content` longer than 218,548 bytes. It cannot validate the plaintext; that is a client responsibility.
- The relay MUST reject a `kind:44300` event that carries an `h` tag.
- The relay MUST reject a `kind:5` deletion request that targets a stored `kind:44300` event and does not carry `["k", "44300"]`.
- The relay stores `kind:44300` events globally, outside any channel.

### Access control: kind:44300 author-only

The relay MUST withhold `kind:44300` events, and their existence, from every reader except their authenticated author, on every read surface: historical REQ delivery, NIP-01 `ids` lookup, live fan-out, COUNT, and the HTTP bridge's query and count endpoints.

The relay MUST apply the same rule, on the same read surfaces and in search results, to every `kind:5` deletion request that carries `["k", "44300"]`, whether or not its target is still stored. A deletion request reveals a version's id and the time it was deleted.

The relay MUST exclude `kind:44300` from full-text search results for every reader, including the author.

The relay MUST apply a `#p` or `#t` filter on `kind:44300` before `ORDER BY … LIMIT`, so that `limit: 1` returns a subject's current version even when the owner has newer versions for other subjects.

The relay MUST support the composite history cursor (see "Current version and history") for `kind:44300`. A filter carrying `before_id` without `until`, or a `before_id` that is not 64 hexadecimal characters, MUST be rejected rather than ignored.

### Access control: author-only-unless-shared

Kind `30175` uses **shared-tag-gated read semantics** to protect system prompts and `respond_to_allowlist` from being visible to all community members as a side-effect of device sync.

The gate is kind-generic: the relay applies it to every kind in `SHARED_GATED_KINDS` (`buzz-core/src/kind.rs`), currently `30175` and the `30178` team-catalog projection described below. The rules and enforcement surfaces are identical for each member kind.

**Rules:**

| Event state | Author reads | Foreign reads |
|---|---|---|
| No `shared` tag | ✅ allowed | ❌ withheld |
| `["shared", "true"]` tag | ✅ allowed | ✅ allowed |

These rules are enforced at the following relay read surfaces (content and event existence are withheld on all of them):

- **REQ historical delivery** — foreign requests silently omit unshared persona events, even in mixed-kind filters (`{kinds:[30175,9]}`). The visibility check is applied **before `ORDER BY … LIMIT`** at the SQL level (`shared_gated_reader` field in `EventQuery`), so a page of newer private personas cannot starve an older shared persona off the candidate set — the catalog's primary all-author query pattern is correctly served.
- **NIP-01 `ids` lookup** — knowing an event id does NOT grant access to an unshared persona. The result gate returns nothing.
- **Live fan-out** — unshared personas are delivered only to the author's connections. Shared personas fan out community-wide.
- **COUNT** — the fast SQL `count_events()` path is bypassed when the filter can match a shared-gated kind. A per-event fallback applies the shared-tag check, preventing existence-leak via COUNT.
- **NIP-98 HTTP bridge `/query`** — the same per-event visibility check is applied to the catchall post-processing loop. The SQL-level `shared_gated_reader` clause also applies before `LIMIT`, preventing older shared personas from being starved by newer private ones on paginated catalog queries. A foreign caller POSTing `{kinds:[30175],authors:[victim]}` or a kindless `{ids:[...]}` filter to `/query` receives no unshared persona content.
- **NIP-98 HTTP bridge `/count`** — `needs_shared_gate_filtering` forces the per-event fallback path for any filter that can match a shared-gated kind; the fast SQL `count_events()` path is not used. Both the channel-scoped and unconstrained fallback loops apply `event_visible_to_reader`, preventing existence-leak via COUNT over HTTP.
- **FTS (NIP-50 search) and `/search`** — every search result passes the same per-event visibility check before delivery, so no search result shows an unshared event to a foreign reader. Unshared events can still be indexed and returned to their author.

**Owner read access is unaffected.** The sync subscription (`{kinds:[30175], authors:[self]}`) reads the author's own events regardless of shared state. Shared heads redact nonportable transport commands, however, so those commands remain device-local under the transport replay rules above.

**Opting in to community sharing.** Publish a NIP-33 replacement head for the persona with a `["shared", "true"]` tag. Unsharing is the reverse: republish without the tag. NIP-33 replacement semantics apply (newest `created_at` wins).

**`shared` is a tag, not a content field.** The tag controls visibility, but sharing also changes the transport projection: stock becomes explicit and nonportable commands are omitted. NIP-01 hashes the published event, including those projected content bytes and tags. Local `source_version` drift detection instead hashes the unredacted, spawn-relevant definition content; it need not equal a hash of the catalog body. A sharing-only change does not change that local definition hash.

**Non-goal: side-band existence oracles.** Reaction, report, and event-deletion validation resolves target events by id to check that they exist. These paths intentionally accept arbitrary event references by design — they leak one bit (existence) but never content, and exploiting them requires already possessing a 64-hex event id that unshared personas never expose through any gated read path. Gating these side-band resolvers would require teaching reaction/report validation about persona read semantics with no realistic attack mitigated. If a stricter "zero existence leakage" property is required in future, it is a separate scoped task.

## Security considerations

- **No encryption for definitions.** Persona and team-catalog events store system prompts, model names, runtime identifiers, and all configuration unencrypted. `kind:44300` instructions versions are the exception: they are encrypted to the owner. Shared persona events are readable community-wide. Operators MUST NOT store secrets in persona event content.
- **System prompt protection.** System prompts and `respond_to_allowlist` pubkeys are sensitive. The relay's author-only-unless-shared gate ensures they are not visible to other community members unless the owner explicitly opts in by publishing a `["shared", "true"]` head. Shared persona events are readable community-wide; operators who need additional confidentiality should use relay-level access controls or choose not to share.
- **Write authority.** Only the holder of `seckey_o` can publish or replace persona events. NIP-33 replacement is scoped by pubkey — no spoofing risk from other relay members.
- **Slug collision across pubkeys.** Two different owners can publish personas with the same slug. Clients MUST always scope queries by author pubkey, not just slug.
- **Metadata exposure.** The `(pubkey, kind:30175, slug)` triple reveals persona existence. Event timestamps reveal edit history.
- **No owner write authority over agents.** Persona events define *what* an agent should be; they do not grant runtime control over a running agent. The agent consumes the persona at spawn time. Updates to the persona event do not automatically propagate to running agents.
- **Instructions history is owner-private, not secret from the owner's clients.** `kind:44300` versions are encrypted to the owner and withheld from other readers. Any client holding `seckey_o` can read every version; deleting versions does not revoke copies already exported, shared, or recorded at launch.
- **Sharing a team shares every member's instructions.** A `kind:30178` head carrying `["shared","true"]` exposes the team's own fields *and* the embedded projection of every member — including members whose own `kind:30175` heads are unshared and therefore still private. Clients MUST make this explicit at the point of sharing; the relay cannot infer it.

## Reference test vectors

> **TEST KEYS — DO NOT USE IN PRODUCTION.** The keys below are pinned for reproducibility. Production code MUST source randomness from a CSPRNG.

### Inputs

```
seckey_o    = 0000000000000000000000000000000000000000000000000000000000000001
schnorr_aux = 0000000000000000000000000000000000000000000000000000000000000000
```

### Derived

```
pubkey_o = 79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798
```

### Event 1 — create persona with all fields

```jsonc
// Body (exact UTF-8, no trailing whitespace):
{"display_name":"Test Agent","system_prompt":"You are a test assistant.","avatar_url":"https://example.com/avatar.png","runtime":"goose","model":"claude-opus-4","provider":"anthropic","name_pool":["Alpha","Beta"]}
```

```
kind            = 30175
pubkey          = 79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798
created_at      = 1700000000
tags            = [["d", "test-agent"]]
content         = {"display_name":"Test Agent","system_prompt":"You are a test assistant.","avatar_url":"https://example.com/avatar.png","runtime":"goose","model":"claude-opus-4","provider":"anthropic","name_pool":["Alpha","Beta"]}
id              = <derived per NIP-01: sha256([0, pubkey, created_at, kind, tags, content])>
sig             = <BIP-340 Schnorr signature with aux=0x00…00>
```

### Event 2 — minimal definition (required fields only)

A definition need not carry a prompt — pure-configuration definitions
(e.g. provider/model presets) are valid:

```jsonc
// Body:
{"display_name":"Minimal"}
```

```
kind            = 30175
pubkey          = 79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798
created_at      = 1700000001
tags            = [["d", "minimal"]]
content         = {"display_name":"Minimal"}
id              = <derived per NIP-01>
sig             = <BIP-340 Schnorr signature with aux=0x00…00>
```

### Event 3 — replacement (same slug, higher `created_at`)

```jsonc
// Updated body (system_prompt changed):
{"display_name":"Test Agent","system_prompt":"You are an updated test assistant.","avatar_url":"https://example.com/avatar.png","runtime":"goose","model":"claude-opus-4","provider":"anthropic","name_pool":["Alpha","Beta","Gamma"]}
```

```
kind            = 30175
pubkey          = 79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798
created_at      = 1700000002
tags            = [["d", "test-agent"]]
content         = {"display_name":"Test Agent","system_prompt":"You are an updated test assistant.","avatar_url":"https://example.com/avatar.png","runtime":"goose","model":"claude-opus-4","provider":"anthropic","name_pool":["Alpha","Beta","Gamma"]}
id              = <derived per NIP-01>
sig             = <BIP-340 Schnorr signature with aux=0x00…00>
```

After Event 3, the head for slug `test-agent` is Event 3 (greatest `created_at`). Event 1 is superseded.

### Head selection with tiebreak

If two events share `created_at = 1700000000` and slug `test-agent`, the head is the event with the lexicographically lowest `id` (hex comparison per NIP-01).

### Implementation notes

Unlike [NIP-AE](NIP-AE.md), persona events involve no encryption, no HMAC derivation, and no conversation key. The test vectors are standard NIP-33 events with JSON content — implementations need only:

1. Correct NIP-01 event-id serialization: `json.dumps([0, pubkey, created_at, kind, tags, content], separators=(",", ":"), ensure_ascii=False)` over UTF-8 bytes.
2. BIP-340 Schnorr signing with the pinned aux value.
3. JSON serialization of the content body with no trailing whitespace or BOM.
