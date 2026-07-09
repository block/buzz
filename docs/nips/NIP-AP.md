NIP-AP
======

Agent Personas
--------------

`draft` `optional`

This NIP defines `kind:30175` persona events — public, addressable definitions that describe how to instantiate an AI agent. A persona carries identity (display name, avatar), behavioral configuration (system prompt, model, runtime), and an optional name pool. It is the "blueprint" from which agents are spawned.

## Kind

This NIP claims `kind:30175` for agent persona definitions. It is in the NIP-33 parameterized replaceable range (30000–39999) per [NIP-01](01.md): addressed by `(pubkey, kind, d_tag)`, with only the latest event per address retained.

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
  "avatar_url": "<string | null>",
  "runtime": "<string | null>",
  "model": "<string | null>",
  "provider": "<string | null>",
  "name_pool": ["<string>", ...],
  "respond_to": "<string | null>",
  "respond_to_allowlist": ["<64-hex pubkey>", ...],
  "mcp_toolsets": "<string | null>",
  "parallelism": "<integer | null>"
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
| `avatar_url` | string \| null | `null` | URL to an avatar image. |
| `runtime` | string \| null | `null` | ACP runtime identifier (e.g. `"goose"`, `"claude-code"`). |
| `model` | string \| null | `null` | Model identifier (e.g. `"claude-opus-4"`). |
| `provider` | string \| null | `null` | Model provider (e.g. `"anthropic"`). |
| `name_pool` | string[] | `[]` | Pool of display names for agent instances spawned from this definition. When non-empty, the spawning system picks a name from this pool for each new agent instance, enabling multiple concurrent agents from the same definition to have distinct identities. |
| `respond_to` | string \| null | `null` | **Reserved.** Default respond-to policy for instances spawned from this definition: `"anyone"`, `"owner-only"`, or `"allowlist"`. `null` defers to the client default. |
| `respond_to_allowlist` | string[] | `[]` | **Reserved.** Allowlisted author pubkeys (64-char lowercase hex) when `respond_to` is `"allowlist"`. Ignored otherwise. |
| `mcp_toolsets` | string \| null | `null` | **Reserved.** MCP toolset selector string passed to spawned instances. |
| `parallelism` | integer \| null | `null` | **Reserved.** Default max concurrent turns for spawned instances. `null` defers to the client default. |

The behavioral fields (`respond_to`, `respond_to_allowlist`, `mcp_toolsets`,
`parallelism`) are definition-level *defaults*: a spawned instance copies them
at creation and may be reconfigured independently afterwards. They were
previously carried only on the kind:30177 projection (see
"Slimming: kind:30177" below).

**Status: reserved.** In the current implementation these four fields are
*parsed but not yet applied*: readers tolerate and preserve them at the wire
layer, but the local definition store does not yet carry them and writers do
not emit them. The instance-copy-at-creation behavior activates in a
subsequent release (the create-path unification). Until then a definition
carrying these fields round-trips through the wire type but the values do not
survive a local edit-and-republish cycle.

Unknown fields MUST be ignored by readers (forward compatibility).

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
  allowlist, `parallelism`, `mcp_toolsets`).
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
  event both carry a field, the 30175 head is authoritative.
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

## Relay behavior

- The relay MUST accept `kind:30175` events that pass standard NIP-33 validation (valid signature, exactly one `d` tag with a non-empty value).
- The relay stores persona events globally (`channel_id = NULL`); they are not channel-scoped.
- The relay is NOT required to validate that `content` parses as valid `PersonaEventContent` JSON. Relays are dumb stores per Nostr convention; content validation is a client responsibility.
- The relay MUST enforce that the `d` tag is non-empty (standard NIP-33 requirement for parameterized replaceable events).

## Security considerations

- **No encryption.** System prompts, model names, runtime identifiers, and all configuration are visible to anyone with relay read access. Operators MUST NOT store secrets in persona event content.
- **System prompt sensitivity.** System prompts may contain security-relevant behavioral instructions. Publishing them unencrypted enables adversarial prompt extraction. Operators who consider system prompts confidential SHOULD NOT publish them in persona events, or SHOULD use a relay with appropriate access controls.
- **Write authority.** Only the holder of `seckey_o` can publish or replace persona events. NIP-33 replacement is scoped by pubkey — no spoofing risk from other relay members.
- **Slug collision across pubkeys.** Two different owners can publish personas with the same slug. Clients MUST always scope queries by author pubkey, not just slug.
- **Metadata exposure.** The `(pubkey, kind:30175, slug)` triple reveals persona existence. Event timestamps reveal edit history.
- **No owner write authority over agents.** Persona events define *what* an agent should be; they do not grant runtime control over a running agent. The agent consumes the persona at spawn time. Updates to the persona event do not automatically propagate to running agents.

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
