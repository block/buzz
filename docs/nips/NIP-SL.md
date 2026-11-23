NIP-SL
======

Send Later
----------

`draft` `optional` `relay`

This NIP defines how an author writes a message now and has the relay post it later, while the author's devices are offline. The client signs the real message ahead of time, dated for the moment `T` it should post, and gives it to the relay inside a private, encrypted instruction. At `T` the relay checks that the author may still post, stores the signed message as it stores any other, and tells the author how the send ended.

The relay never holds the author's key, and it releases the message inside the timestamp window it already enforces, so the signer rule and the timestamp rule stay as they are. Readers get an ordinary event and cannot tell that it was sent later.

Recurring sends, forum posts and [NIP-17](17.md) gift wraps are out of scope. There is no time lock: the relay can read a message as soon as it accepts the instruction. The rules for operating a relay with send later are in the [deployment guide](../send-later-deployment.md).

## Kinds

| Kind | Name | Signer | Purpose |
|------|------|--------|---------|
| `30360` | Instruction | author | Holds one message to post later, or cancels it |
| `30361` | Status | relay | Tells the author how a send ended |

Both kinds are addressable, and only the author can read them. One message waiting to be posted is a **send**, at the address `(author pubkey, 30360, d)`. A send is **pending** until it **ends** as sent, failed, missed or cancelled, and an ended send never becomes pending again.

`kind:40006`, an older Buzz kind named for scheduled messages, is an ordinary channel event that channel members can read. Implementations MUST NOT use it to send a message later.

## Instruction

```jsonc
{
  "kind": 30360,
  "pubkey": "<author>",
  "created_at": 1769990000,
  "content": "<NIP-44 ciphertext of the signed message>",
  "tags": [
    ["d", "a3f8c2e1b4d79600e5d2f1a8c3b6094d"],
    ["not_before", "1770000000"],
    ["generation", "1"],
    ["exec", "2026-11"]
  ]
}
```

- `d` is the send id. It MUST be 32 lowercase hex characters that encode 16 bytes from a cryptographically secure random source. It MUST NOT be derived from the message, the channel or the time.
- `not_before` is `T`, a Unix timestamp in the grammar [NIP-ER](NIP-ER.md) defines for this tag.
- `generation` is the version of the send, in the same grammar. It starts at `1`.
- `exec` is the `id` of the relay key that the content is encrypted to ([Support](#support)).

An instruction MUST have each of these tags exactly once, with one value, and MAY have one `alt` tag ([NIP-31](31.md)). Any other tag makes it malformed, so no channel, mention or thread query can match it.

`content` MUST be a [NIP-44](44.md) version 2 payload, encrypted between the author's key and the `exec` key. The plaintext is the signed message as one JSON object with exactly the members `id`, `pubkey`, `created_at`, `kind`, `tags`, `content` and `sig`. A missing, extra or repeated member is invalid.

The message MUST meet all of these:

1. Its `id` and `sig` verify.
2. Its `pubkey` is the instruction's `pubkey`.
3. Its `created_at` equals `not_before` exactly.
4. Its `kind` is one the relay lists ([Support](#support)).
5. It has exactly one `h` tag, which names a channel of the community the instruction was sent to.

It is otherwise an ordinary event of its kind. A thread reply, a message in a DM, a mention, an attachment and an agent's [NIP-OA](NIP-OA.md) `auth` tag look as they would if the author posted the message live.

### Cancel

```jsonc
{ "kind": 30360, "content": "", "tags": [["d", "<send id>"], ["generation", "3"]] }
```

A cancel MUST have exactly one `d` and one `generation` tag, at most one `alt`, no other tag, and empty `content`.

## Changing a send

The first instruction at an address MUST hold a message and have `generation` `1`. Each later one MUST have the stored generation plus one, and MUST win [NIP-01](01.md) ordering against the stored instruction: a greater `created_at`, or the same `created_at` and a lower `id`. Of two devices that change the same generation, only one succeeds. A client sets `created_at` to the later of its current time and the stored `created_at` plus one.

To edit a send or change its time, a client publishes the next generation with a newly signed message. To send now, it sets `not_before` to the current time and signs the message for that time. To cancel, it publishes a cancel at the next generation, which ends the send. A client MUST NOT publish the message directly while its send is pending.

A send that has ended closes its address. The relay MUST refuse every later instruction there, for as long as it stores a status or a cancel for the address and for at least `status_retention` after the send ended. A new attempt uses a fresh `d`.

[NIP-09](09.md) does not apply to `kind:30360`. A relay MUST NOT delete an instruction or change a send because of a deletion request, and SHOULD reject one that names a `kind:30360` event or address.

## Relay behavior

### Acceptance checks

The relay accepts an instruction wherever it authenticates the sender, such as a [NIP-42](42.md) session. It runs these checks in order, and the first that fails decides the answer. Clients match an `OK` message up to the end of the text shown; a relay MAY add detail after it.

| # | Check | `OK` when it fails |
|---|-------|--------------------|
| 1 | The sender is authenticated as the event's `pubkey`, and `id` and `sig` verify | as for any event |
| 2 | The event is not the instruction already stored at its address | `true`, `duplicate: already have this event` |
| 3 | `created_at` is within the relay's usual tolerance | as for any event |
| 4 | The tags are those of an instruction or a cancel, and `content` is at most `max_content_len` bytes | `invalid: malformed not_before`, otherwise `invalid: malformed instruction` |
| 5 | The address is not closed | `invalid: send ended` |
| 6 | The generation and ordering rules hold, and a cancel names an existing send | `invalid: stale generation`, `invalid: stale replacement`, `invalid: no such send` |
| 7 | A cancel is accepted here. The checks below apply to an instruction that holds a message | |
| 8 | The relay is accepting instructions that hold a message: new sends and replacements | `restricted: send later disabled` |
| 9 | `now - 60 <= T` and `T <= now + max_not_before_delta` | `invalid: not_before in the past`, `invalid: not_before too far` |
| 10 | `exec` names a `current` or `standby` key, never a `retiring` or unlisted one, and `content` decrypts under it | `invalid: unknown executor key`, `invalid: undecryptable content` |
| 11 | The plaintext and the message meet [Instruction](#instruction), and the message passes the relay's validity checks for an ordinary event of its kind, apart from the `created_at` tolerance | `invalid: message: <reason>` |
| 12 | The author may post the message to its channel now: membership, permission to post, channel state, bans and timeouts | `restricted: message: <reason>` |
| 13 | Under NIP-FI, no disconnect record fences the admission that carried the instruction ([Federated identity](#federated-identity)) | `restricted: authorization denied` |
| 14 | For a new address, the author has fewer than `max_pending` pending sends in the community | `restricted: send later quota exceeded` |

A relay MUST NOT refuse a cancel because check 8, a timeout or a lost permission to post would refuse a message. `max_pending` is counted for each author key in each community; an agent's sends count against the agent's key.

The relay stores an accepted instruction, its generation and the state of the send in one atomic change. Checks 5, 6, 13 and 14 MUST run inside that change. A refused instruction MUST change nothing. The relay MUST NOT release a replaced generation or keep it readable.

A relay MUST answer a failure whose outcome it does not know, such as a storage error, with `error:`. On `error:`, or no `OK`, a client MUST NOT assume either outcome, and sends the same event again. Only `OK true` settles an instruction as accepted. After any other answer the client makes a [complete read](#complete-read), and before it creates a new send in place of an unsettled one it MUST check that the message is not readable in its channel.

### Release

1. The relay MUST NOT store, deliver or return the message before its clock reads `T`.
2. At release it MUST apply every check it would apply if the author posted the message at that moment on an authenticated session: membership, permission to post, channel state, bans, timeouts, attachments, content rules and the author's rate limit. Only proof of a live session is not required.
3. It MUST store the message and mark the send `sent` in one atomic change, which MUST apply only if the send is still pending at the generation being released. A replacement or a cancel either commits first, and nothing old is posted, or is refused with `invalid: send ended`. A relay MUST NOT answer `OK true` to a cancel and then post the message.
4. It then does everything it does for a message posted live: delivery, push, mentions, thread counters and search. The stored event is the author's signed event, unchanged.

A check that refuses for good ends the send as `failed`. One that may clear, such as a rate limit, is retried. The relay MUST NOT release after `T + release_window`: a send still pending then ends as `missed`. If the message is already stored at release, the send ends as `sent` with that event's id.

A relay SHOULD release as soon after `T` as its checks allow. It MAY spread sends that share one `T`, and SHOULD NOT use the rest of the window to put off a release it could make. Clients MUST NOT assume that a message posts at exactly `T`.

## Status

When a send ends for any reason other than the author's own cancel, the relay publishes one status. For the author's cancel, the cancel is the record.

```jsonc
{
  "kind": 30361,
  "pubkey": "<executor key>",
  "created_at": 1770000004,
  "content": "",
  "tags": [
    ["d", "<author pubkey>:<send id>"],
    ["p", "<author pubkey>"],
    ["status", "sent"],
    ["generation", "2"],
    ["e", "<message id>"],
    ["expiration", "1772592004"]
  ]
}
```

`generation` is the one that ended. `e` is present only for `sent` and holds the id of the stored message. `expiration` ([NIP-40](40.md)) is `created_at + status_retention`. `created_at` is the relay's clock when it signs, never earlier than the end of the send.

| `status` | `reason` | The send ends this way when |
|----------|----------|-----------------------------|
| `sent` | | The message is stored |
| `missed` | | It was not released by `T + release_window` |
| `failed` | `not_permitted` | At release, the author is no longer in the channel or the community, or may not post to the channel |
| `failed` | `restricted` | At release, the author or the author's owner is under a timeout in the community |
| `failed` | `archived` | At release, the channel is archived |
| `failed` | `attachment` | At release, an attachment is no longer available |
| `failed` | `rejected` | Release is refused for good for any other reason |
| `cancelled` | `banned` | The author or the author's owner is banned in the community |
| `cancelled` | `disconnected` | A NIP-FI disconnect applies to the author or the author's owner ([Federated identity](#federated-identity)) |
| `cancelled` | `channel_deleted` | The channel is deleted |
| `cancelled` | `withdrawn` | The relay operator withdraws the send ([deployment guide](../send-later-deployment.md#rolling-back)) |

The relay cancels in the atomic change that records the ban, the disconnect or the channel deletion, and for a disconnect before it answers the issuer. It MUST apply the `banned` and `channel_deleted` rows again at release, which catches an instruction that was being accepted while the event was recorded. An owner is the owner of the author's key in that community (NIP-OA, and "Agent keys" in [NIP-FI](NIP-FI.md)). A cancelled send stays cancelled: lifting a ban or restoring access MUST NOT make it pending again. Deleting an account or a community removes its sends with the other data, and no status is published.

A relay SHOULD include `reason` for `failed` and `cancelled`, and MUST omit it otherwise. Clients MUST treat a code they do not know as no reason.

A send has at most one status, and the relay MUST NOT revise it. A relay SHOULD store the status in the atomic change that ends the send. One that stores it afterwards MUST record the outcome durably in that change, MUST sign the status when it stores it, and MUST keep trying until it is stored.

The relay signs a status with its `current` executor key, and MUST reject a `kind:30361` event that a client submits. Clients MUST accept a status only when its `pubkey` is listed in the descriptor's `keys`, whatever the mark. A client whose copy of the descriptor does not list the signer MUST read the descriptor again before it ignores the status, once for each unlisted key. The same limit applies when a client looks up the `exec` key of an instruction to decrypt it.

## Reading

A relay MUST NOT reveal the existence, count, tags, content, time or search matches of either kind to anyone but the author, on any read path: `REQ`, `COUNT`, live subscriptions, lookups by id, search, HTTP bridges and push.

- `kind:30360` is readable only by a requester authenticated as the event's `pubkey`.
- `kind:30361` is readable only by a requester authenticated as the pubkey in its `p` tag.
- The owner of an agent cannot read the agent's instructions or statuses.
- A relay SHOULD close an unauthenticated request for only these kinds with `auth-required:`. In a filter that mixes kinds, it MUST omit what the requester may not read.

A client follows its sends with one live subscription. After its `EOSE`, the relay MUST deliver to it every instruction it accepts and every status it stores. A client verifies the signature of an instruction before it decrypts it.

```jsonc
["REQ", "send-later",
  { "kinds": [30360], "authors": ["<own pubkey>"] },
  { "kinds": [30361], "#p": ["<own pubkey>"] }]
```

### Complete read

The relay's default limit can cut the stored events of that subscription short, and an instruction carries its author's clock, so a `since` cursor can miss an edit from another device. A client therefore MUST NOT refresh `kind:30360` with `since`. It derives state from a complete read: two requests made as the author, with no `since` or `until`, after the `EOSE` of the live subscription and while that subscription stays open.

1. Instructions: `{"kinds": [30360], "authors": ["<own pubkey>"], "limit": <max_limit>}`, with the relay's NIP-11 `max_limit`.
2. Statuses: `{"kinds": [30361], "#p": ["<own pubkey>"], "#d": [<status d values>], "limit": <number of values>}`, split so that no request has more than `max_limit` values. The values name every send the client holds as pending or as ended with an unknown outcome, and every send whose message-holding instruction request 1 returned.

An event that arrives on the live subscription before the read ends counts as returned. Of two instructions for one address, the client keeps the one that wins NIP-01 ordering. For each send that request 2 names, or whose cancel request 1 returned, the first rule that applies gives its state:

1. A status was returned: the send ended as the status says.
2. An instruction was returned: one that holds a message means pending, and a cancel means cancelled by the author.
3. Neither: the send is not pending and its outcome is unknown. The client MUST NOT show it as sent or as cancelled. It MAY drop a send that has stayed under this rule for `status_retention`.

The relay MUST return every event that matches either request, without paging; [Retention](#retention) keeps request 1 within `max_limit`. It MUST answer each request from one consistent view of its storage, so that a replacement in progress cannot hide a pending send. It MUST NOT serve the message-holding instruction of an ended send until that send's status is readable.

### Other reads

Any other read updates only the sends whose events it returns. A failed, closed, incomplete or unauthenticated read, a lost connection and the absence of a live update are not evidence of a state, and a client MUST NOT infer an outcome from them.

The instruction of a `sent` or relay-cancelled send is deleted without an event, so a client's copy can be out of date until the status arrives. Before it does, `invalid: send ended` in answer to an edit or a cancel means that the send is not pending and its outcome is unknown, and the message being readable in its channel means `sent`. A client treats a send as cancelled only after `OK true` for its cancel. On `invalid: stale generation` or `invalid: stale replacement`, another device changed the send, and the client reads the address again.

## Retention

| Send ended as | Instruction | Status |
|---------------|-------------|--------|
| `sent`, or `cancelled` by the relay | Deleted when the send ends | Readable for `status_retention` |
| `failed`, `missed` | Readable to the author for `text_retention` after the status is stored, then deleted | Readable for `status_retention` |
| Cancelled by the author | The cancel replaces the instruction it cancels, which is deleted. The cancel is readable for `status_retention`, then deleted | none |

Deleting an instruction means that the relay no longer stores its ciphertext or the decrypted message in any form.

A relay MUST keep at most `max_pending` ended instructions for one author in a community, counting cancels and the instructions of `failed` and `missed` sends together. When the end of a send would exceed that number, the relay first deletes the ended instruction whose send ended earliest. Its address stays closed. If it held the text of a `failed` or `missed` send, the text is lost early. If it was a cancel, a complete read then reports that send under rule 3.

So the instructions of one author in a community fit in one request. A relay MUST advertise a NIP-11 `max_limit` of at least twice `max_pending`, and MUST NOT change either limit in a way that lets them exceed `max_limit`.

A relay MUST keep an executor key in the descriptor for as long as an instruction encrypted to it is pending or readable, or a status signed with it is readable. It MUST keep the secret key for as long as an instruction encrypted to it is pending.

## Support

Until this draft has an upstream NIP number, relays MUST NOT list it in [NIP-11](11.md) `supported_nips`. A relay advertises send later for the community that a NIP-11 request resolves to, with `"buzz-send-later"` in `supported_extensions` and this descriptor:

```jsonc
{
  "supported_extensions": ["buzz-send-later"],
  "send_later": {
    "v": 1,
    "accepting": true,
    "keys": [
      { "id": "2026-11", "pubkey": "<64 hex>", "current": true },
      { "id": "2026-05", "pubkey": "<64 hex>", "retiring": true }
    ],
    "kinds": [9],
    "limitation": {
      "max_not_before_delta": 10368000,
      "max_pending": 100,
      "release_window": 900,
      "text_retention": 604800,
      "status_retention": 2592000,
      "max_content_len": 65536
    }
  }
}
```

| Member | Meaning |
|--------|---------|
| `v` | Version of this contract. This document defines `1` |
| `accepting` | `true` when the relay accepts instructions that hold a message: new sends and replacements. When `false` it accepts only cancels and reads |
| `keys` | The executor keys: relay keys that instructions are encrypted to and statuses are signed with. Each entry has a unique `id` of 1 to 64 bytes, a `pubkey` and exactly one mark, set to `true` |
| `kinds` | The kinds a message may have |
| `limitation` | The limits in [Limits summary](#limits-summary), each a positive integer. A relay MUST enforce the limits it advertises |

| Mark | Meaning |
|------|---------|
| `current` | Clients encrypt new instructions to this key, and the relay signs new statuses with it. Exactly one entry is `current` |
| `standby` | The relay also accepts new instructions under this key while it changes keys |
| `retiring` | The relay accepts no new instruction under this key and signs no new status with it. It stays listed while something that depends on it is readable |

A relay MAY list its NIP-11 `self` key as an executor key. A relay that hosts several communities MUST serve the descriptor only for a community in which send later is enabled.

A client MUST treat the descriptor as absent when `supported_extensions` does not contain `"buzz-send-later"`, when `v` is not `1`, when a member in the first table or a limit in [Limits summary](#limits-summary) is missing or has the wrong type, when a `keys` entry lacks a member or does not have exactly one mark, or when `keys` does not contain exactly one `current` entry. Without a valid descriptor a client MUST NOT publish `kind:30360` and MUST NOT offer send later: a relay without this NIP may store both kinds as ordinary events that any reader can query.

A client offers send later only while `accepting` is `true`, and keeps `T` and its number of pending sends inside the limits. On `invalid: unknown executor key` it reads the descriptor again and encrypts to the key now marked `current`. If the relay stops serving a valid descriptor, the client treats the state of its pending sends as unknown and reads again when the descriptor returns.

## Late arrival

A released message carries `created_at` `T` and can reach readers up to `release_window` later. A reader that fetches new messages with `since` SHOULD set it back by at least the relay's `created_at` tolerance, and discard by `id` the events it already holds.

That delivers a late message. It does not make it unread. A reader that records a channel as read up to a `created_at` value counts a late message as read, and this NIP does not remove that limitation. The time or order in which one device received a message is not evidence that the message is unread, and a client MUST NOT substitute it. Counting a late message as unread on every device of a user needs read state that those devices share and that does not depend on `created_at`: an order that follows when the relay stored a message, or a complete record of the messages read by `id`. A client with such read state SHOULD count a late message as unread.

## Federated identity

Where a community requires NIP-FI, a send is a deferred action under [Deferred actions](NIP-FI.md#deferred-actions), because a release happens without a session. That section defines what the relay records with each accepted generation, which sends a disconnect command cancels (those of the target key and of every key it owns, accepted under the command's issuer, in every community) and when check 13 refuses an instruction. A relay MUST NOT enable send later in such a community unless it implements that section.

Nothing else in NIP-FI ends an accepted send. The relay does not contact the issuer at release, require a recent assertion or evaluate the community's issuer policy again, and an issuer that only stops issuing assertions does not stop a pending send.

## Security

The relay is trusted with the signed message, which is a valid event from the moment it is signed. A relay that misbehaves can post it inside the timestamp window around `T` after a cancel, or withhold it. It cannot forge a message, change one, or post one at another time. A cancel is a promise that the relay enforces, not a cryptographic revocation.

No session exists at release, so authority to post comes from the author's signatures. A `created_at` condition in an agent's NIP-OA `auth` tag is evaluated against `T`, so an owner's time bound on an agent also bounds how far ahead the agent can send. A [NIP-46](46.md) remote signer needs no delegation: it signs the message for `T`, encrypts it to the executor key and signs the instruction. Signing takes time, so a client that sends now through one SHOULD expect `invalid: not_before in the past` and retry with a newly signed message.

A relay that uses an executor key pair for another encrypted kind MUST NOT accept the ciphertext of one kind as content of the other. A relay SHOULD also count each instruction against the author's ordinary rate limit.

Someone who can read the relay's storage but not the executor secret key learns that an author has a send, its `T`, its generation and how it ended, but not the content or the channel of a pending send.

## Limits summary

The relay advertises these in `limitation` ([Support](#support)). Times are in seconds.

| Member | Limit | Recommended |
|--------|-------|-------------|
| `max_not_before_delta` | How far ahead `T` may be | 10368000 (120 days) |
| `max_pending` | Pending sends for one author in a community, and ended instructions kept for that author | 100 |
| `release_window` | How long after `T` the relay may still release. It MUST NOT exceed the relay's `created_at` tolerance for ordinary events | 900 |
| `text_retention` | How long the instruction of a `failed` or `missed` send stays readable | 604800 (7 days) |
| `status_retention` | How long a status stays readable. It MUST be at least `text_retention` | 2592000 (30 days) |
| `max_content_len` | Maximum instruction `content`, in bytes | |
