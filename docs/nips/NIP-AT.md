NIP-AT
======

Agent Attention Configuration
-----------------------------

`draft` `optional` `buzz-only`

This NIP defines how an AI agent stores its attention policy on a relay. The policy is the set of objects that decide which events wake the agent: **Interests**, **event watches** and **timers**. Each object is one addressable `kind:30183` event ([NIP-01](01.md)), signed by the agent and encrypted with [NIP-44](44.md) to the conversation key between the agent and its owner. Only the agent and the owner can read it.

The envelope is close to [NIP-AE](NIP-AE.md) engrams. The kind is separate so that runtimes that apply a policy do not mix it with agent memory.

Kind choice (2026-10-07): `30183` is not listed in the NIPs README or in `nostr-protocol/registry-of-kinds`, and a GitHub code search for `kind 30183`, `Kind(30183`, `kinds 30183` and `30183 nostr` found no Nostr use. The earlier choice `30173` is used by ridestr (`DRIVER_AVAILABILITY`), and the same search found Nostr uses of `30181`, `30182` and `30184`. This is a bounded search, not proof that the kind is unused.

This kind is supported only on Buzz relays that enable it (`BUZZ_AGENT_ATTENTION_ENABLED=true`). Other relays do not apply the read gate, the delete rules or the limits below. Writers MUST publish only to a Buzz relay that supports this NIP. The [NIP-70](70.md) `-` tag makes a standard relay refuse the event unless the author has authenticated.

## Roles

- **agent** — the identity (`pubkey_a`) that signs every config event and every delete.
- **owner** — the identity (`pubkey_o`) named in the optional single `p` tag.

## Event

```
K_c  = nip44_conversation_key(seckey_agent, pubkey_owner)
d(x) = lower_hex(HMAC-SHA256(K_c, "agent-attention/v1/d-tag" || 0x00 || x))
```

An agent with no registered owner uses its own public key as `pubkey_owner` and omits the `p` tag.

```jsonc
{
  "kind": 30183,
  "pubkey": "<agent>",
  "tags": [
    ["d", "<d(slug)>"],
    ["p", "<owner>"],
    ["-"],
    ["alt", "encrypted agent attention configuration"]
  ],
  "content": "<nip44(K_c, body)>"
}
```

The tags are exactly `d`, `p` (unless there is no owner), `-` and `alt`. Any other tag, including [NIP-40](40.md) `expiration`, makes the event invalid.

The decrypted body is a UTF-8 JSON object of at most 65,535 bytes:

```jsonc
{ "schema": "agent-attention/v1", "slug": "watch/project-messages", "value": { } }
```

- `slug` is `interest/<id>` or `watch/<id>`. Event watches and timers share the `watch/` space; `value` says which type it is. IDs match `^[!-~]{1,64}$`.
- A reader MUST check that `d(slug)` equals the event's `d` tag and MUST ignore the event if it does not.
- The event's `created_at` is the object's last-modified time.
- Objects have no order. Readers list them by slug.
- If a `watch/<id>` changes between an event watch and a timer, the runtime treats it as a removal of the old object and an addition of the new one.
- The `value` schema for `agent-attention/v1` (value types, event watch matching, filter grammar, classifier, timer schedule and limits) is defined in the design document "Agent Attention configuration on the relay". A reader that does not know a `schema` skips that object and reports it.

## Removal

An object is removed with a [NIP-09](09.md) `kind:5` event signed by the agent, with exactly one `a` tag, `30183:<agent>:<d>`, and a `["k", "30183"]` tag. It removes only versions with `created_at` at or before its own.

The deletion event is not encrypted. It shows the agent key, the time, the kind and the hashed `d` tag. It does not show the owner, the slug or the content.

## Relay behavior

A relay that supports this NIP:

- MUST reject `kind:30183` unless it has exactly one `d` tag (64 lowercase hex), at most one `p` tag (64 lowercase hex), exactly one `-` tag, exactly one `alt` tag, no other tag, and NIP-44 v2 content.
- MUST store it as a global (non-channel) addressable event.
- MUST answer a filter that can match `kind:30183` only when `authors` contains only the authenticated pubkey, or `#p` contains only the authenticated pubkey. Explicit `ids` do not exempt a filter: knowing an event ID is not authorization. This applies to `REQ`, `COUNT`, search and any HTTP query surface.
- MUST deliver or count a stored `kind:30183` event only to its author or to the pubkey in its `p` tag, on every read path (history, live delivery, `COUNT`, search and HTTP), including filters with no `kinds`.
- MUST keep a delete in effect. For each address it keeps the time of the newest delete and of the newest accepted version, and rejects:
  - an event dated at or before the newest delete, with `OK false` and the message `invalid: agent-attention address deleted at <t>`, where `<t>` is the delete's `created_at`;
  - an event that is older than the newest accepted version, or has the same `created_at` and a higher ID, with `invalid: agent-attention address has a newer version`.
  An exact resend of the current version is accepted as a duplicate.
- MUST store an accepted delete and apply it (remove the versions it covers and record its time) as one atomic step. If it cannot apply the delete, it MUST reject the `kind:5` and MUST NOT store or deliver it.
- MUST reject a `kind:5` that targets `kind:30183` unless it is signed by the agent, has exactly one `a` tag and has a `["k", "30183"]` tag. It MUST reject an `e`-tag deletion of a `kind:30183` event.
- MUST send `CLOSED` with an `error:` prefix, instead of `EOSE`, when it cannot read stored events for a subscription with a filter that can match `kind:30183`. An empty result followed by `EOSE` therefore means there are no objects.

## Writing

A write to an object (a new version or a delete) uses `created_at = max(now, head.created_at + 1)`, where `head` is the object's current event, if any. If the relay answers `deleted at <t>`, the writer publishes again with `created_at = max(now, t + 1)`, and fails with a retryable error if that is more than 60 seconds ahead of `now`.

## Reading

A reader opens one subscription:

```
[ { "kinds": [30183], "authors": ["<agent>"], "limit": L },
  { "kinds": [5], "authors": ["<agent>"], "#k": ["30183"], "limit": 0 } ]
```

`L` is the relay's NIP-11 `limitation.max_limit`, or 1,000 if it is missing. An owner viewer uses `{ "kinds": [30183], "authors": [<owned agents>], "#p": ["<owner>"] }` instead of the first filter.

The first filter returns the stored policy and then live changes. The second returns no stored events and delivers live deletions; the relay's delete rule makes stored deletions unnecessary. For each address the reader keeps the event with the newest `created_at`, and on a tie the lowest event ID. A deletion removes an object whose `created_at` is at or before the deletion's. On `CLOSED` the reader keeps its last good objects and subscribes again later. On reconnect it replaces its whole policy with the new result.

If a page holds `L` config events, the reader requests the next page with `until` set to the oldest `created_at` in the page, skips IDs it has already seen, and stops when a page has fewer than `L` events.
