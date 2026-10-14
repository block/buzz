NIP-AP
======

Agent Personas
--------------

`draft` `optional`

This NIP defines `kind:30175` persona events — public, addressable definitions that describe how to instantiate an AI agent. A persona carries identity (display name, avatar), behavioral configuration (system prompt, model, runtime), and an optional name pool. It is the "blueprint" from which agents are spawned.

This NIP also defines teams of agents (see "Teams") and how an agent's and a team's config are stored as [NIP-AR](NIP-AR.md) artifacts in [NIP-CL](NIP-CL.md) system channels (see "Agent and team config").

## Kind

This NIP claims `kind:30175` for agent persona definitions and `kind:30178` for the shareable team-catalog projection (see "Team catalog projection: kind:30178"). Both are in the NIP-33 parameterized replaceable range (30000–39999) per [NIP-01](01.md): addressed by `(pubkey, kind, d_tag)`, with only the latest event per address retained.

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
  agent's instructions, its current `buzz.agent.instructions` artifact is
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

**A share is a snapshot, never history.** A writer embeds the team's current instructions at the time it publishes (see "Team instructions") and no earlier revisions. Editing the team does not change an existing share; the owner updates a share by publishing a new head. A client MAY show the owner that a share is older than the team's current instructions. A reader that adopts a shared team creates a new team with a new id; the adopted team starts its own instructions history.

## Teams

A **team** is an owner-defined group of agents that share optional **team instructions**. A team's name and instructions live in its team config channel (see "Agent and team config"), and its members are agents in that channel. The shareable form of a team is `kind:30178`.

### Team identity

A team is identified by `(pubkey_o, community, team-id)`: the owner, the relay community holding the team's config channel, and the team id.

A team id MUST be non-empty, MUST be at most 64 Unicode scalar values, and MUST NOT contain Unicode control or whitespace characters. This is the same grammar as the `kind:30178` `d` tag. Team ids are compared byte for byte, with no trimming, case folding, or Unicode normalization.

Clients SHOULD mint a new UUID for each new team, including a team created by importing or adopting another team. A client MUST NOT give a new team the id of a deleted team.

### Ownership

An agent's owner is the owner that the relay has verified from the agent's [NIP-OA](NIP-OA.md) owner attestation. A tag on an arbitrary event does not establish ownership. A team's owner is the creator of its team config channel, as named by the channel's NIP-CL `P` tag.

Changing an agent's or a team's owner is not supported. Transferring a config channel's owner role to someone else does not change who owns the agent or team, and a second attestation does not move an agent's config or history.

### Membership

A team's **members** are the agents in its team config channel whose owner is the team's owner. The owner, humans, and agents owned by anyone else can be in the channel but are not members. An agent's channel role does not affect membership: an agent made an admin or a guest of the channel is still a member.

Team membership is channel membership, so it is also access: every member can read the team's config. A member follows the team's current instructions; it does not pin the text in effect when it joined. An agent MAY be a member of more than one team.

Changing who is on a team is a change to the channel's membership, under the channel's role rules:

- a member of the channel, other than a guest, can add someone who is not yet in the channel as a member or guest, subject to that person's add policy: a policy of only their owner admits additions by their owner alone, and a policy of nobody rejects every addition by someone else, including their owner;
- only an owner or admin can add someone as an owner or admin, or change an existing member's role, up or down;
- only an owner or admin can remove someone else, except that a member can remove an agent they own;
- a guest can add and remove no one;
- anyone can leave;
- the channel's last owner cannot leave, be removed, or be demoted.

### Team instructions

A team's current instructions are the `instructions` field of its current `buzz.team` artifact (see "Config artifacts"). A team whose `buzz.team` artifact is absent has no name and empty instructions. A team whose `buzz.team` artifact is invalid, deleted, or unavailable has invalid instructions (see "Artifact state").

### Launch with teams

At each session start, the client resolves the current instructions of every team the agent is a member of. Each team reaches the agent as its own **team section**, which carries the team's name, its members, and its instructions. A team without instructions has empty instructions; its section still names the team and its members.

The client MUST NOT merge, deduplicate, or compare the instructions of different teams, and MUST NOT refuse to start an agent because its teams' instructions differ. If a team's instructions are invalid, the client MUST NOT start the session, and SHOULD name the team. An active session need not be interrupted.

An agent that is a member of no team has no team sections. Team sections are delivered after the agent's own instructions (see "Launch composition").

### Deleting a team

A client deletes a team by archiving its team config channel. Archiving keeps the channel and its artifacts, so the team can be recovered by unarchiving it. Purging the channel is a separate action. A team whose config channel is archived has no members.

No later than each member's next session start, the client removes the deleted team's section and resolves the remaining teams' sections. An active session need not be interrupted, and a client MAY apply the removal sooner. A team config channel that cannot be read is not a deleted team.

## Agent and team config

An agent's config and a team's config are [NIP-AR](NIP-AR.md) artifacts. Each lives in its own private [NIP-CL](NIP-CL.md) system channel, the **config channel**, so channel membership decides who can read and edit it.

### Relay support

A client stores config on the relay only when the relay's [NIP-11](11.md) document shows that it supports all of these:

- NIP-CL system channels and labels;
- NIP-AR artifacts;
- guest roles that are read-only: a guest cannot write artifacts and cannot add anyone to the channel;
- exempting config channels from age-based retention (see "History");
- artifact sync that can read an artifact's complete history;
- telling an artifact address that has never been used apart from one whose revisions have all been redacted.

How a relay advertises each one is defined by the specification of that feature. Otherwise the client MUST keep config local, and MUST NOT create config channels.

### Config channels

| Channel | One per | Created by | Family label | Subject label |
| --- | --- | --- | --- | --- |
| Agent config channel | agent | the agent's owner | `agent-config` | the agent's 64-character lowercase hex pubkey |
| Team config channel | team | the team's owner | `team-config` | `team-` followed by the first 59 lowercase hex characters of the SHA-256 hash of the team id's UTF-8 bytes |

The owner creates a config channel with a kind:9007 that carries `["channel_type", "system"]`, both labels as `t` tags, and no `visibility` or `ttl` tag. A config channel MUST be private and MUST NOT have a TTL. The owner MUST NOT change a config channel's labels.

Both subject labels are 64 characters. A team id can contain characters a label cannot, so the team label is derived from a hash; the team id itself is stored on the `buzz.team` artifact (see "Config artifacts").

An agent config channel's members are the owner, the agent, and any others the owner adds. A team config channel holds the owner and the team's members, and can hold others who are not members (see "Membership").

### Discovery

To find a config channel, a client queries by creator and subject label:

```
Agent: {kinds: [39000], "#P": [pubkey_o], "#t": [<agent-pubkey>]}
Team:  {kinds: [39000], "#P": [pubkey_o], "#t": [<team-label>]}
```

To find the teams an agent is on, the agent's client queries the channels the agent belongs to, opting in to system channels:

```
{kinds: [39002], "#p": [<agent-pubkey>], "#t": ["system"]}
```

then reads those channels' kind:39000 by `#d`.

In every case the client keeps only channels whose type is `system`, whose labels include the family label, and whose `P` is the expected owner. It MUST NOT put the family label in the same `#t` filter, because values in one filter combine with OR. When more than one channel remains for one agent or team, the client uses the oldest, by NIP-CL's "Choosing one channel" rule. A client creates a config channel only when this query returns none. A client MUST NOT delete or archive a config channel because it was not selected.

### Config artifacts

| Type | Channel | `title` | Extra tags | Content |
| --- | --- | --- | --- | --- |
| `buzz.agent.settings` | agent config | `Agent settings` | | `{"version": 1, "settings": { ... }}` |
| `buzz.agent.instructions` | agent config | `Agent instructions` | | `{"version": 1, "text": "<string>"}` |
| `buzz.team` | team config | the team name | `["team", "<team-id>"]` | `{"version": 1, "name": "<string>", "instructions": "<string>"}` |

A config channel holds at most one artifact of each of its types. Its `d` is the UUIDv5 of the type name in the namespace of the channel's UUID, so two clients creating the same artifact collide on `d` and one create fails. The client whose create fails fetches the current revision and edits it; it MUST NOT choose a different `d`. A client accepts a config artifact only when its `d`, `type`, and `h` match the config channel it was read from. A `buzz.team` artifact has no member list; members come from the channel (see "Membership").

**Team id.** A `buzz.team` artifact carries exactly one `team` tag, whose value is the team id. A client MUST check that the team id's subject label equals the channel's subject label, and MUST treat the artifact as invalid otherwise.

**Team name.** `name` is also the artifact's `title`, so it MUST be nonblank and at most 512 UTF-8 bytes, and MUST equal the `title` tag.

**Instructions.** `text` and `instructions` MAY be empty, which means "no instructions". They MUST be valid UTF-8 and MUST NOT contain U+0000.

**Settings.** `settings` is a JSON object of the agent's launch configuration. Its fields are defined by the client and harness that launch the agent, not by this NIP. It MUST NOT contain credentials, secrets, machine-local paths, or device state. A client MUST NOT launch an agent with a setting it does not support silently replaced by a default; it shows the error instead.

**Size.** A config artifact's `content` MUST NOT exceed 131,072 bytes after JSON encoding, and `buzz.agent.settings` content MUST NOT exceed 65,535 bytes. The signed event MUST also fit within the relay's advertised event-size limits. Writers check these limits on the encoded content and the signed event, reject an edit that exceeds them, and MUST NOT truncate.

An artifact whose content does not parse, has an unknown `version`, or breaks these rules is **invalid**.

### Artifact state

When a client reads a config artifact, the artifact is in one of these states:

- **present:** its current revision was read and is valid;
- **invalid:** its current revision was read and is invalid;
- **absent:** the relay confirmed that the artifact's address has never been used;
- **deleted:** a lookup by `d` returned a deletion;
- **unavailable:** anything else, including a failed read, an unreadable channel, an artifact moved to another channel, a redacted current revision, or a lookup that returns nothing without that confirmation.

Only an absent artifact lets a client fall back to another source (see "Launch composition"). An invalid, deleted, or unavailable artifact blocks the start: the client MUST show the error and MUST NOT substitute an older revision, another source, or local config. A deleted artifact is recovered with NIP-AR `restore`.

### Editing

Every channel member who is not a guest can edit the channel's artifacts. This includes members who are not on the team, such as humans and agents with another owner. Guests can only read them. The owner makes an agent read-only for a config channel by giving it the guest role there.

Every edit is a new NIP-AR revision and names the current revision in `prev`. On conflict, the client reconciles with the current revision as NIP-AR requires. An edit MUST keep any JSON fields and tags the client does not recognise.

### History

Every revision of a config artifact has a stable ID, its event ID. A config artifact's revisions are its history. History is kept until the owner deliberately purges it or a moderator removes a revision. Config channels MUST be exempt from age-based community retention, and a client MUST NOT configure a config artifact to keep only its latest revision. A client shows a removed revision as a gap in history, not as the absence of history.

**Revert** publishes an older revision's content as an `op=update` revision carrying a complete, valid snapshot. Reverting a team's instructions keeps the team's current name. Clients MUST NOT redact revisions to roll back.

### Launch composition

An agent's own instructions layer is the first of these that exists:

1. the `text` of its current `buzz.agent.instructions` artifact. Empty text yields no instructions and stops resolution. An invalid, deleted, or unavailable artifact blocks the start (see "Artifact state");
2. while that artifact is absent, instructions held in local configuration;
3. the `system_prompt` of its linked `kind:30175` definition;
4. for a definition-less instance, the `system_prompt` in its legacy `kind:30177` event.

A client moving local instructions to the relay MUST create the `buzz.agent.instructions` artifact and confirm that the relay accepted it before it stops using the local copy.

Team sections are resolved independently (see "Launch with teams"). The client delivers the agent's own layer and each team section to the agent's harness separately, the agent's own first.

A change to the agent's instructions or to any of its teams MUST take effect no later than the agent's next session start. A client MAY apply it sooner, including to an active session between turns, and MAY start a new session to apply it. An active session need not be interrupted. A client that lets local configuration override the resolved instructions SHOULD show the owner that an override is in effect.

### Deleting an agent

A client deletes an agent by archiving its agent config channel. A client MUST NOT start a new session for an agent whose config channel is archived. An active session need not be interrupted. The archived channel stays discoverable, so the agent can be recovered by unarchiving it. Purging the channel is a separate action. Deleting an agent never purges a team's config or history.

### Older clients

Clients that do not implement config channels do not see them, because NIP-CL leaves system channels out of reads that do not name them. They resolve an agent's instructions from `kind:30175` and `kind:30177` as before, so they launch agents with the definition's prompt rather than the agent's current instructions, and they do not see teams stored on the relay.

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

- **No encryption for definitions.** Persona and team-catalog events store system prompts, model names, runtime identifiers, and all configuration unencrypted. Shared persona events are readable community-wide. Operators MUST NOT store secrets in persona event content.
- **System prompt protection.** System prompts and `respond_to_allowlist` pubkeys are sensitive. The relay's author-only-unless-shared gate ensures they are not visible to other community members unless the owner explicitly opts in by publishing a `["shared", "true"]` head. Shared persona events are readable community-wide; operators who need additional confidentiality should use relay-level access controls or choose not to share.
- **Write authority.** Only the holder of `seckey_o` can publish or replace persona events. NIP-33 replacement is scoped by pubkey — no spoofing risk from other relay members.
- **Slug collision across pubkeys.** Two different owners can publish personas with the same slug. Clients MUST always scope queries by author pubkey, not just slug.
- **Metadata exposure.** The `(pubkey, kind:30175, slug)` triple reveals persona existence. Event timestamps reveal edit history.
- **No owner write authority over agents.** Persona events define *what* an agent should be; they do not grant runtime control over a running agent. The agent consumes the persona at spawn time. Updates to the persona event do not automatically propagate to running agents.
- **Config is readable by every config channel member.** Agent settings, agent instructions, and team instructions are not encrypted. Every member and guest of a config channel can read its artifacts and their history, so adding someone to a config channel shares that config with them. Every member who is not a guest can also edit it, including humans and agents that are not on the team. The relay operator can read them too. Removing someone, archiving the channel, or redacting a revision does not revoke copies already read, exported, or shared.
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
