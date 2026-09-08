NIP-AK
======

Agent Keyrings
--------------

`draft` `optional`

This NIP defines a convention for delivering credentials to AI agents as encrypted, owner-authored `kind:30180` addressable events. A keyring entry carries one credential grant (an environment variable, an OAuth token, an MCP server definition, or a small file), encrypted with [NIP-44](44.md) using the conversation key between the agent and its owner, and addressed by a blinded `d` tag so relays learn neither the credential's name nor its existence. Because entries are addressable, **rotation is republication**: the newest head at an address is the live credential, everywhere the agent runs, without the agent or its harness restarting.

Together with [NIP-OA](NIP-OA.md) (identity: the owner attests the agent's key) and [NIP-AE](NIP-AE.md) (memory: encrypted state that follows the agent's key), this NIP makes the third machine-bound thing portable. An agent is two keys, an identity key and an inference key, and with a keyring both travel: a harness holding the agent's private key can materialize the agent's full working credential set on any machine, and the machine becomes disposable.

## Kind

This NIP claims `kind:30180` for agent keyring entries. It is in the addressable range per [NIP-01](01.md): relays store only the latest event per `(kind, pubkey, d)`. The replacement semantics are not incidental; they are the rotation mechanism (see [Rotation](#rotation)).

`30180` was verified unassigned as of 2026-08-15 in both this repository's kind registry (`crates/buzz-core/src/kind.rs`, where `30174`–`30179` are claimed by engrams, personas, teams, managed agents, team catalogs, and private managed agents) and the upstream `nostr-protocol/nips` kind table.

A dedicated kind (rather than reusing `kind:30174` agent engrams with a slug convention) is taken deliberately: keyring events are authored by the **owner**, engrams by the **agent** (see [Roles](#roles)), and a consumer must be able to enforce that authorship rule from the kind and author alone, before attempting decryption. Mixing the two under one kind would make "who may write here" a matter of decrypted content, which fails open.

## Motivation

An agent's identity already travels: [NIP-OA](NIP-OA.md) makes the agent a keypair whose authority is an owner-signed attestation, valid from any machine. Its memory already travels: [NIP-AE](NIP-AE.md) stores encrypted state on the relay, decryptable wherever the key wakes up. What does not travel is everything else the agent needs to work: the inference credential (an API key, or a subscription OAuth token minted by a local `login`), the tool credentials (a service API key, a git credential), and the MCP server grants that define which tools the agent may hold at all.

Today these are machine-bound. They live in the harness environment of one supervisor on one machine, injected by hand, invisible to the workspace. Moving an agent to another executor means re-plumbing them by hand; rotating one (a re-login on the owner's laptop) silently strands every remote copy with a dead credential. Worse, the by-hand path has no surface: a credential slipped into an agent through a wrapper script or an env file is a grant nobody can list, audit, or revoke from the product.

This NIP defines the smallest interoperable alternative: owner-sealed credential grants on the relay, one address per grant, latest-head-wins rotation, and a mandatory authorship rule that makes the keyring the *only* sanctioned path for credentials to reach an agent.

## Non-Goals

This NIP is not a general secret manager for humans, not a mechanism for sharing credentials between owners or between agents, and not an escrow: the relay stores ciphertext it cannot read and this NIP gives it no role in recovery. It does not transport the agent's own private key; the identity key bootstraps everything here and MUST be distributed by the supervisor out of band, never through a keyring it would be needed to decrypt. Large secrets (disk images, model weights, keystores beyond a relay's event size limit) are out of scope; entries are grants, not blobs.

## Roles

- **owner** — a Nostr identity (`pubkey_o`) that grants credentials. The author of every keyring event.
- **agent** — a Nostr identity (`pubkey_a`) that consumes them. Identified by the `p` tag.
- **harness** — the process holding `seckey_a` that fetches, decrypts, and materializes entries into a runtime's environment.

A keyring is scoped to a single `(pubkey_o, pubkey_a)` pair. An agent serving multiple owners holds an independent keyring per pair.

**Authorship is a hard rule.** A harness MUST ignore `kind:30180` events whose author is not the owner resolved from its [NIP-OA](NIP-OA.md) attestation, regardless of whether they decrypt. Memory ([NIP-AE](NIP-AE.md)) is agent-authored because agents own what they remember. Credentials are owner-authored because agents must never grant themselves capabilities: a compromised agent key can read its existing grants (it always could; it holds them in memory while working) but cannot mint new ones, widen an MCP allowlist, or re-target a credential. Write and read are split across the two halves of the same conversation key.

## Slugs

A **slug** names a grant. A valid slug matches:

```
^key/[a-z0-9][a-z0-9_-]{0,63}(/[a-z0-9][a-z0-9_-]{0,63})*$
```

with total length ≤ 255 bytes. Examples: `key/claude-sub`, `key/github-app`, `key/mcp/blotato`. Slugs are stable identities for grants: rotating a credential reuses its slug; renaming is a revoke plus a new grant.

## Addressing

The `d` tag of an entry is derived from its slug exactly as in [NIP-AE](NIP-AE.md), with a distinct domain prefix:

```
K_c = nip44_conversation_key(seckey_o, pubkey_a)
    = nip44_conversation_key(seckey_a, pubkey_o)          # symmetric per NIP-44
d   = lower_hex(HMAC-SHA256(K_c, utf8("agent-keyring/v1/d-tag") || 0x00 || utf8(slug)))
```

Implementations MUST NOT include the slug or any plaintext form of it in tags. A relay observing keyring traffic learns that an owner maintains *some* encrypted records addressed to an agent, and when they change. It learns neither what they are, nor how many distinct credentials exist beyond an upper bound, nor which entry a given update rotated.

## Event envelope

```jsonc
{
  "kind": 30180,
  "pubkey": "<pubkey_o>",
  "created_at": <unix_seconds>,
  "tags": [
    ["d", "<64-hex>"],
    ["p", "<pubkey_a>"],
    ["alt", "Encrypted agent keyring entry"]
  ],
  "content": "<nip44_ciphertext>"
}
```

`content` MUST be a [NIP-44](44.md) ciphertext under `K_c`. The `p` tag is required: it is what lets a harness subscribe to its own keyring with one filter (`kinds: [30180]`, `authors: [pubkey_o]`, `#p: [pubkey_a]`) on the same socket that carries its mentions.

## Content

The decrypted plaintext is a UTF-8 JSON object:

```jsonc
{
  "v": 1,
  "slug": "key/claude-sub",
  "status": "live",                    // "live" | "revoked"
  "type": "oauth",                     // "env" | "oauth" | "mcp" | "file"
  "rotated_at": 1786500000,            // unix seconds; see Rotation
  "expires_at": null,                  // optional; harness MUST fail closed past it
  "source": "claude-code-login",       // optional, informational: where this came from
  "grant": { ... }                     // shape depends on "type", below
}
```

`slug` MUST round-trip: a harness MUST verify that the decrypted slug re-derives the event's `d` tag and discard the entry otherwise (this binds ciphertext to address and defeats entry-swapping by a hostile relay).

Grant shapes:

- `"env"` — `{ "var": "BLOTATO_API_KEY", "value": "..." }`. One variable, injected into the runtime environment.
- `"oauth"` — `{ "var": "CLAUDE_CODE_OAUTH_TOKEN", "value": "...", "provider": "anthropic" }`. Like `env`, but marked as an inference/subscription credential so surfaces can present it as such and rotation watchers know what to watch.
- `"mcp"` — `{ "name": "blotato", "command": "...", "args": [...], "url": null, "env": { "BLOTATO_API_KEY": "..." } }`. A complete MCP server definition. The harness passes it in the ACP `session/new` MCP server list. An agent holds exactly the MCP servers its keyring grants: the keyring *is* the allowlist.
- `"file"` — `{ "path": "~/.config/tool/credentials.json", "mode": 384, "b64": "..." }`. A small file materialized before runtime spawn, `chmod` to `mode`. Intended for credential files tools insist on reading from disk; not a blob store.

## Rotation

Rotation is the reason this NIP exists, and it is deliberately boring: **publish a new event at the same address.** NIP-01 replacement makes it the head; every consumer converges on it. There is no rotation message, no version negotiation, no coordination round. The interesting work is in the two halves around that publish, specified here so the experience is seamless rather than merely eventual.

### The publishing half: watchers, not ceremonies

The owner-side client (in Buzz, the desktop) SHOULD maintain **source watchers** for credentials it has been asked to sync: observers of the local stores where credentials actually change. The canonical example: the owner runs `/login` in Claude Code on their laptop. The local credential store now holds a new subscription token and the old one is dying or dead. The watcher detects the change, seals the new value under the existing `key/claude-sub` slug, publishes the head, and notifies nobody, because there is nothing to decide. Rotation that asks the user a question has already failed; the user made their decision when they logged in.

Publishers MUST set `rotated_at` to the time of the underlying credential change and SHOULD publish within seconds of detecting it. Where a provider permits overlapping validity (an old key that keeps working after a new one is minted), publishers SHOULD grant the new before revoking the old at the provider, in that order, so consumers never observe a gap. Where the provider revokes on rotation (typical of subscription logins), the gap is unavoidable at the provider and the healing rule below covers it.

### The consuming half: push, then heal

A harness MUST obtain keyring heads at boot (standard addressable-event query) and MUST hold a live subscription to its keyring filter for the life of the process. On receiving a new head, the harness:

1. Validates authorship, decrypts, verifies the slug round-trip, and checks `created_at` monotonicity (below).
2. **Re-materializes at the next safe boundary.** New runtime sessions and MCP server spawns MUST use the new grant immediately. In-flight turns SHOULD complete on the environment they started with; a harness MUST NOT kill a running turn merely because a credential rotated.
3. If the rotated entry is of type `"mcp"`, the harness restarts that MCP server at the next session boundary with the new definition.

Push alone loses the race where a provider killed the old credential the instant the new one was minted and a turn was mid-flight. So the second rule: **on an authentication failure from a runtime, an MCP server, or a tool call, the harness MUST re-query its keyring heads before retrying, and SHOULD retry the failed operation once if a fresher head was found.** Push covers the seconds; heal-on-failure covers the race; boot-time fetch covers agents that were offline through any number of rotations, because addressable storage collapses missed rotations into one head. An agent that slept through five rotations wakes to the sixth with no catch-up protocol.

The combined guarantee, stated as UX rather than mechanism: *the owner logs in again on their laptop, and every copy of every agent, on the mini, on the fleet, in a sandbox, picks up the new credential without anyone touching any of them.* At worst, one in-flight turn retries once.

### Rollback resistance

A hostile or stale relay could serve an older head, quietly re-arming a revoked credential. Consumers MUST track, per slug, the highest `created_at` (and `rotated_at`) they have accepted, persisted in agent-local state (an [NIP-AE](NIP-AE.md) memory entry is suitable), and MUST reject heads older than what they have already accepted. A rejected regression SHOULD be surfaced through observer telemetry ([NIP-AO](NIP-AO.md)); it is either an infrastructure fault or an attack, and silence is the wrong response to both.

### Revocation

Revocation is rotation to a tombstone: publish a head with `"status": "revoked"` and an empty `grant`. On receipt a harness MUST remove the materialized value, MUST stop offering a revoked MCP server at the next session boundary, and SHOULD [NIP-40](40.md)-expire nothing prematurely: tombstones SHOULD carry an `expiration` tag some 30–90 days out so the address eventually clears, but MUST outlive any plausible offline consumer, because a consumer that never saw the tombstone and cannot fetch it will fail closed only when the credential itself dies at the provider. Owners revoking in earnest SHOULD also revoke at the provider; the keyring is delivery, not enforcement.

## Client behavior (the UI is normative in spirit)

This section is non-normative in mechanism but records the product rule the NIP exists to serve: **no credential reaches an agent except through its keyring, and the keyring is visible.**

A conforming owner-side client (Buzz Desktop) presents, per agent, a keyring panel showing every grant: slug, type, source, `rotated_at`, expiry, and per-executor freshness where observer telemetry provides it ("mini: current · fleet: current · pod: offline since Tue"). Grants are created there, revoked there, and *synced* there: the client enumerates syncable local sources (the Claude Code login, `gh auth`, MCP servers found in the user's local configuration) and offers each as a per-agent checkbox with a watcher attached. The MCP checkbox list is the allowlist made visible.

What this replaces deserves naming: today a credential can be smuggled into an agent through a wrapper script or a hand-edited env block, producing an agent whose real capabilities exceed anything any surface displays. That path must become both unnecessary (the keyring does it better: encrypted at rest, rotating, roaming) and culturally wrong (a grant that is not in the panel is a bug report). Env vars typed free-form into an agent's configuration SHOULD be reserved for non-secret tuning, and clients SHOULD warn when a value in that surface looks like a secret and offer to move it into the keyring.

Agents may need credentials nobody anticipated. The agent-side path is a request, not a write: an agent asks in-channel (or via a draft, as with agent creation), and the owner grants through the panel. The asymmetry is the point.

## Relationship to other NIPs

[NIP-OA](NIP-OA.md) supplies the owner binding that authorship enforcement depends on. [NIP-AE](NIP-AE.md) supplies the conversation-key and blinded-address pattern this NIP reuses, and a home for consumer-side rotation state. [NIP-01](01.md) addressable replacement supplies rotation; [NIP-44](44.md) supplies confidentiality; [NIP-42](42.md)/[NIP-AA](NIP-AA.md) supply authenticated delivery; [NIP-40](40.md) supplies tombstone cleanup; [NIP-09](09.md) supplies hard deletion where a relay honors it.

## Security considerations

- **The conversation key is the perimeter.** Compromise of either `seckey_o` or `seckey_a` exposes the keyring's plaintext. This is unchanged from the status quo, where the same credentials sit plaintext in a process environment on a machine holding `seckey_a`; the keyring narrows exposure at rest (ciphertext on the relay, materialized only in a running harness) and adds what the status quo lacks entirely: enumeration, rotation, and revocation.
- **Scope grants per agent.** Blast radius is the union of an agent's grants. Owners SHOULD grant each agent only its slice, SHOULD prefer short-lived or mintable credentials (a GitHub App that mints installation tokens, never a long-lived PAT), and SHOULD set `expires_at` where the provider supports bounded lifetimes, which consumers MUST fail closed on.
- **Subscription tokens are still subscription tokens.** A keyring makes an inference OAuth token portable; it does not change the provider's terms or rate limits, which remain shared across every holder of the token and remain the provider's lever. Treat `"oauth"` and API-key `"env"` grants as interchangeable slots so a policy change is a rotation, not an architecture change.
- **Relay metadata.** Blinded addresses hide names and identities of grants, but update timing is visible. An observer who can watch both a keyring update and, say, a public provider-side rotation may correlate. Owners for whom this matters SHOULD batch or jitter publishes.
- **Do not put the identity key in the keyring.** Restated from Non-Goals because it will be tempting: the nsec decrypts the keyring; a keyring entry containing the nsec is either useless or a circularity that ends with the key somewhere it should not be.
