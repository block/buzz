NIP-ER
======

Event Reminders
---------------

`draft` `optional` `relay`

This NIP defines encrypted, author-only reminders as `kind:30300` addressable events. A pending reminder carries a public `not_before` tag that tells supporting relays when the reminder is due, while the reminder target, note, and state are encrypted to the author with [NIP-44](44.md). A reminder without `not_before` is a bookmark (saved item with no due time) or a terminal state (done/cancelled).

The relay learns that an author has a reminder due at a time. It does not learn what the reminder is about.

Delivery is relay-dependent: relays that advertise push-mode support MUST emit a due reminder to matching live subscriptions when `not_before` passes, and clients MUST enforce `not_before` locally.

## Motivation

Nostr has primitives for private state, deletion, expiration, and relay-authenticated reads, but no standard way to represent an author's private reminder that becomes due in the future. [NIP-40](40.md) `expiration` closes a visibility window; it does not open one. [NIP-51](51.md) private lists can store reminder-like data, but relays cannot discover a due time without decrypting list contents.

This NIP defines the smallest interoperable reminder primitive: encrypted author-owned state plus one public due-time tag. Relays can schedule or surface due reminders without learning the reminder target or note, and clients can recover reminder state across devices.

## Non-Goals

This NIP does not define recurrence, shared reminders, push notifications, calendar events, or cryptographic time-locking. Recurrence is a client-side policy: a client that wants recurring reminders creates a new reminder with a fresh `d` tag for each occurrence.

Relay due-time delivery is not guaranteed notification delivery. Clients remain responsible for recovery queries, deduplication, and final `not_before` enforcement. Clients that do not rely on local long-horizon scheduling can use the stateless notification profile in [Client behavior](#client-behavior), but that profile requires a relay that advertises push-mode due delivery; on lazy or non-supporting relays it fires only for reminders already within `W` seconds of due at query time.

## Terminology

- **reminder address**: the [NIP-01](01.md) addressable-event coordinate `(pubkey, 30300, d)`.
- **head**: the winning latest event for a reminder address under NIP-01 replacement ordering.
- **pending reminder**: a head whose decrypted `status` is `pending` and whose outer event has exactly one valid `not_before`.
- **due reminder**: a pending reminder whose `not_before` is less than or equal to the client's current time.
- **terminal reminder**: a head whose decrypted `status` is `done` or `cancelled`.
- **due signal**: an `EVENT` message sent by a relay when a reminder becomes due. A due signal is not a new event and is not a delivery guarantee.

## Relationship to Other NIPs

This NIP uses [NIP-01](01.md) addressable-event replacement semantics, [NIP-09](09.md) deletion requests for hard deletion, [NIP-11](11.md) relay information documents for capability and limitation hints, [NIP-40](40.md) `expiration` for cleanup after terminal states, [NIP-42](42.md) authentication for author-only reads, [NIP-44](44.md) encryption for private content, and [NIP-65](65.md) relay lists for write-relay selection.

This NIP intentionally does not use [NIP-59](59.md) gift wrapping. Reminders are self-addressed state: the relay must know which author is allowed to recover and receive due signals, and it must read the public `not_before` tag to schedule them.

If this draft receives an upstream NIP number, implementations SHOULD migrate discovery to `supported_nips` for that number.

## Event

`kind:30300` is an addressable event keyed by `(pubkey, kind, d)` as defined in [NIP-01](01.md). Each reminder MUST use a fresh random `d` tag.

Required tags for a reminder that may become due:

```jsonc
[
  ["d", "<random-id>"],
  ["not_before", "<unix-timestamp-seconds>"],
  ["alt", "Encrypted reminder"]
]
```

For bookmarks (saved items) or terminal states (done/cancelled), `not_before` is omitted:

```jsonc
[
  ["d", "<random-id>"],
  ["alt", "Encrypted reminder"]
]
```

`d` MUST be an opaque random value with at least 128 bits of entropy and MUST NOT be derived from the target event, reminder text, or reminder time. Events with no `d` tag, an empty `d` tag, or more than one `d` tag are invalid.

`not_before` MUST be a decimal Unix timestamp string. It MUST contain only ASCII digits, with no sign, whitespace, decimal point, or leading zero except `"0"`. It MUST parse exactly as an integer in the range 0 through 9007199254740991 inclusive. Implementations MUST NOT parse it through lossy floating-point conversion, and MUST treat values outside this range or values that overflow their parser as malformed. Events MUST contain at most one `not_before` tag. Supporting relays SHOULD reject events with an invalid or duplicate `not_before` tag using `invalid: malformed not_before`. A pending reminder that may become due MUST include exactly one valid `not_before`. Bookmarks and terminal states (done/cancelled) MUST omit `not_before`. Clients MUST ignore pending reminders without exactly one valid `not_before`.

`alt` is RECOMMENDED for [NIP-31](31.md) fallback text.

`expiration` MAY be used as in [NIP-40](40.md), but SHOULD NOT be used on pending reminders. Completed or cancelled reminders SHOULD set `expiration` to a jittered cleanup time, for example 30-90 days after completion. If `not_before` is present, clients MUST NOT set `expiration` less than or equal to `not_before`.

## Content

`.content` MUST be a [NIP-44](44.md) ciphertext encrypted to the author's own public key, using the same self-encryption pattern as [NIP-51](51.md) private lists.

The decrypted plaintext is a UTF-8 JSON object:

```jsonc
{
  "target": {
    "id": "<event-id>",
    "a": "<kind>:<pubkey>:<d>",
    "relays": ["wss://relay.example"],
    "preview": "optional cached text"
  },
  "status": "pending",
  "note": "optional private note"
}
```

`status` MUST be one of:

- `pending` -- reminder has not been completed
- `done` -- reminder was shown or acknowledged
- `cancelled` -- author cancelled it without deleting history

A pending reminder MUST contain either a `target` object or a non-empty `note`. A reminder MAY be note-only and need not reference an existing Nostr event; clients can create standalone private reminders by setting `note` and omitting `target`.

When `target.a` is present, clients SHOULD resolve the current addressable event. When both `target.a` and `target.id` are present, `id` is only a snapshot fallback; it MUST NOT override a resolvable `a` reference. If `a` is absent or cannot be resolved, clients MAY use `id`. `relays` are hints only.

Clients MUST validate the outer event signature before decrypting. Clients MUST ignore plaintext they cannot decrypt, plaintext that is not a JSON object, plaintext with duplicate member names in any object, or plaintext with an unknown `status`. Unknown non-duplicate fields are ignored.

For deterministic convergence, clients MUST apply these content-validity rules before treating a head as actionable:

- `target.id`, when present, MUST be a 64-character lowercase hex event id.
- `target.a`, when present, MUST be a syntactically valid NIP-01 address (`<kind>:<pubkey>:<d>`).
- `target.relays`, when present, MUST be an array; clients MUST ignore entries that are not absolute `ws://` or `wss://` URLs with a non-empty host.
- `target.preview` and `note`, when present, MUST be strings.
- A pending reminder MUST have either a valid target reference (`id` or `a`) or a non-empty `note`.

## State

Reminder updates are normal addressable-event replacements. The winning event for `(pubkey, 30300, d)` is the event with the highest `created_at`; ties are broken by lowest lexicographic `id`, per [NIP-01](01.md).

Common transitions:

| Operation | Replacement |
| --- | --- |
| create | `status: "pending"` with future `not_before` |
| snooze | `status: "pending"` with a later `not_before` |
| complete | `status: "done"`, omit `not_before`, add `expiration` |
| cancel | `status: "cancelled"`, omit `not_before`, add `expiration` |

After a reminder becomes `done` or `cancelled`, clients SHOULD create a new reminder with a fresh `d` tag rather than reusing the old address.

For hard deletion, use [NIP-09](09.md) with an `a` tag referencing `30300:<pubkey>:<d>` and a `k` tag of `30300`. A deletion request only deletes versions with `created_at` less than or equal to the deletion event's `created_at`, as defined by NIP-09. To cancel a pending notification, clients SHOULD publish a `cancelled` replacement before any NIP-09 deletion; deletion requests are `kind:5` events and are not guaranteed to reach `kind:30300` notification receive paths before a held reminder fires.

## Relay behavior

Until this draft has an upstream integer NIP number, relays MUST NOT advertise it in [NIP-11](11.md) `supported_nips`. Relays advertise draft support by adding `"nip-er"` to a NIP-11 `supported_extensions` string array. NIP-11 permits implementation-specific fields; clients that do not understand this field ignore it.

Supporting relays MUST enforce [NIP-42](42.md) authentication for all `kind:30300` reads. A relay MUST NOT reveal the existence, count, tags, content, schedule, or search matches of a `kind:30300` event to anyone except the authenticated event author.

For unauthenticated single-kind `30300` requests, relays SHOULD close with `auth-required:`. For authenticated requests for another author's reminders, relays SHOULD close with `restricted:`. For mixed-kind filters, unauthorized `30300` matches MUST be omitted while other kinds are handled normally.

Supporting relays MUST NOT reject a valid `kind:30300` event solely because `not_before` is in the future. Supporting relays SHOULD reject `kind:30300` events where both `not_before` and `expiration` are present and `expiration` is less than or equal to `not_before`, using `invalid: expiration before not_before`. Relays MAY enforce normal write policy, storage quotas, rate limits, proof-of-work, and a maximum reminder horizon. A maximum horizon SHOULD be advertised in NIP-11 as `limitation.max_not_before_delta`.

Relays MUST store only the latest version for each `(pubkey, 30300, d)` address. When a replacement wins, older versions MUST be discarded and any due-time delivery for older versions MUST be cancelled. This rule is based only on addressable-event replacement ordering; relays do not decrypt `status`.

### Due-time delivery

For authenticated author subscriptions matching a latest event with a valid `not_before`, a supporting relay SHOULD send that event as an `EVENT` message when `not_before` passes. A relay that advertises `limitation.due_delivery_mode` as `"push"` MUST send that due-time `EVENT` message. For push-mode relays, an authenticated author subscription opened after `not_before` has passed SHOULD receive the latest due reminder during the stored-event replay, subject to normal filter matching and replacement and deletion state. This is a due signal. It does not change the event, create a relay-authored event, or imply guaranteed notification delivery.

If a replacement with a future `not_before` is accepted while an authenticated author subscription is open, the relay SHOULD send that replacement immediately as state sync. The relay SHOULD send it again when it becomes due. Clients MUST deduplicate persisted reminder state by event `id` and address.

Relays MAY implement due-time delivery with a timer, cron, sorted queue, or lazy query-time evaluation. Lazy implementations that do not proactively push due events still conform if they preserve the author-only privacy rules and return due reminders on later authenticated queries. Relays SHOULD advertise `limitation.due_delivery_mode` as `"push"` when they proactively emit due signals and `"lazy"` when they only surface due reminders on query.

## Client behavior

Clients SHOULD publish reminders to the author's [NIP-65](65.md) write relays whose NIP-11 documents advertise both `supported_extensions: ["nip-er"]` and NIP-42 in `supported_nips`. Clients SHOULD NOT publish reminders to relays that do not advertise both unless the user accepts the metadata and read-access risk.

Clients subscribe to their own reminders:

```jsonc
{"kinds": [30300], "authors": ["<own-pubkey>"]}
```

Clients that expect due-time `EVENT` messages SHOULD keep reminder subscriptions unbounded by `since` and `until`, or use periodic recovery queries. `since` and `until` compare against `created_at`, not `not_before`, so a reminder created long ago may become due after the client's last cursor.

For notification-only use, clients SHOULD ensure the receive path for `kind:30300` notifications does not suppress repeated `EVENT` messages by id; pool-level duplicate-id filtering can otherwise drop due-time redelivery before application code runs. On each delivery, if `not_before` is more than `W` seconds in the future, the notification path SHOULD discard the event without recording it; otherwise it SHOULD hold the event and notify at `not_before`, or immediately if `not_before` is already past. Every delivery supersedes any held event for the same reminder address, including deliveries that are themselves discarded or terminal; the notification path therefore needs to parse the reminder address from a delivery before discarding it. After notifying, the notification path SHOULD NOT notify the same event id again. The RECOMMENDED value of `W` is 60 seconds; `W` SHOULD exceed worst-case client clock skew, or due-time redelivery can itself be discarded as too far in the future. Reminder management and synchronization SHOULD use separate one-shot queries. This stateless notification profile requires a relay that advertises push-mode due delivery; on lazy or non-supporting relays it fires only for reminders already within `W` seconds of due at query time.

Clients MUST enforce `not_before` locally even when a relay serves an event early or does not support this NIP. A pending reminder with a future `not_before` may be shown in a reminder-management UI, but MUST NOT notify the user or be marked `done` before it is due. Because relays cannot read `status`, clients MUST omit `not_before` on `done` and `cancelled` replacements. If a latest replacement decrypts to `done` or `cancelled` but carries `not_before`, clients MUST treat it as terminal state and MUST NOT schedule or display a due notification for it.

Clients SHOULD persist the latest known version for each reminder address. Before notifying or publishing `done`, a client SHOULD refresh the latest version from its write relays when practical and verify:

1. the event is still the latest known replacement for the address;
2. decrypted `status` is `pending`;
3. the event has exactly one valid `not_before`; and
4. `not_before` is less than or equal to the client's current time.

This reduces stale and duplicate notifications, but does not eliminate simultaneous multi-device races. Two devices may notify at the same time before either observes the other's `done` replacement.

Clients SHOULD paginate reminder recovery with `until` and `limit`.

## Privacy

NIP-44 protects reminder content: target, note, preview, and status. It does not hide all metadata.

Visible to supporting relays and storage observers:

| Metadata | Source |
| --- | --- |
| reminder owner | event `pubkey` |
| scheduled time | `not_before` tag |
| reminder count | distinct `d` tags |
| creation/update times | `created_at` |
| approximate payload size | ciphertext length |
| lifecycle timing | replacements and `expiration` |

`not_before` is not a security boundary. A malicious relay can serve early, serve late, refuse to serve, or leak metadata. Clients must treat relay scheduling as best-effort.

## Security Considerations

Relays can observe reminder ownership, due times, approximate payload sizes, and lifecycle timing. Users who do not want a relay to learn that they have a reminder due at a particular time should not publish reminders to that relay.

A malicious or faulty relay can send due signals early, late, repeatedly, or not at all. Clients MUST enforce `not_before` locally and MUST deduplicate persisted reminder state by event `id` and reminder address.

A relay that violates the NIP-42 author-only read requirement can leak reminder metadata or ciphertext. NIP-44 protects reminder contents from passive readers, but it does not hide schedule metadata and does not protect against compromise of the author's private key. A compromised author key can decrypt, modify, complete, cancel, or delete that author's reminders.

## Worked Examples

These examples are illustrative wire shapes, not cryptographic test vectors.

Create a reminder:

```jsonc
{
  "kind": 30300,
  "pubkey": "<author-pubkey>",
  "created_at": 1769990000,
  "tags": [
    ["d", "a3f8c2e1b4d79600e5d2f1a8c3b6094d"],
    ["not_before", "1770000000"],
    ["alt", "Encrypted reminder"]
  ],
  "content": "<nip44-ciphertext>",
  "id": "<event-id>",
  "sig": "<signature>"
}
```

Decrypted content for a target-backed reminder:

```jsonc
{
  "target": {
    "a": "30023:79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798:proposal",
    "id": "7b4f3c2a1e9d8c7061524334aabbccddeeff00112233445566778899aabbccdd",
    "relays": ["wss://relay.example"],
    "preview": "Can you review this before Friday?"
  },
  "status": "pending",
  "note": "Follow up before planning"
}
```

Decrypted content for a note-only reminder:

```jsonc
{
  "status": "pending",
  "note": "Submit travel receipt"
}
```

Snooze by replacing the same address with a later `not_before`:

```jsonc
{
  "kind": 30300,
  "pubkey": "<author-pubkey>",
  "created_at": 1770000100,
  "tags": [
    ["d", "a3f8c2e1b4d79600e5d2f1a8c3b6094d"],
    ["not_before", "1770086400"],
    ["alt", "Encrypted reminder"]
  ],
  "content": "<nip44-ciphertext-with-status-pending>",
  "id": "<event-id>",
  "sig": "<signature>"
}
```

Complete by replacing the same address without `not_before`:

```jsonc
{
  "kind": 30300,
  "pubkey": "<author-pubkey>",
  "created_at": 1770086410,
  "tags": [
    ["d", "a3f8c2e1b4d79600e5d2f1a8c3b6094d"],
    ["alt", "Encrypted reminder"],
    ["expiration", "1777542730"]
  ],
  "content": "<nip44-ciphertext-with-status-done>",
  "id": "<event-id>",
  "sig": "<signature>"
}
```

Delete stored reminder data with NIP-09:

```jsonc
{
  "kind": 5,
  "pubkey": "<author-pubkey>",
  "created_at": 1770086420,
  "tags": [
    ["a", "30300:<author-pubkey>:a3f8c2e1b4d79600e5d2f1a8c3b6094d"],
    ["k", "30300"]
  ],
  "content": "",
  "id": "<event-id>",
  "sig": "<signature>"
}
```

AUTH-gated read:

```
R: ["AUTH", "<challenge>"]
C: ["AUTH", <signed-event-json>]
R: ["OK", "<auth-event-id>", true, ""]
C: ["REQ", "r1", {"kinds": [30300], "authors": ["<author-pubkey>"]}]
R: ["EVENT", "r1", <latest-reminder>]
R: ["EOSE", "r1"]
... not_before passes ...
R: ["EVENT", "r1", <same-latest-reminder>]
```


## Registry

This NIP registers:

- `kind:30300`: Event reminder
- `not_before`: earliest due time for `kind:30300` reminders, encoded as a decimal Unix timestamp string
- NIP-11 `supported_extensions`: string array; contains `"nip-er"` when the relay supports this draft before an upstream integer NIP number is assigned
