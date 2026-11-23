NIP-SL
======

Send Later
----------

`draft` `optional` `relay`

This NIP defines how an author writes a message now and has the relay post it later, while every device of the author is offline.

1. The author's client signs the real message, dated for the moment `T` it should post, encrypts it to the relay, and publishes it inside a private instruction.
2. The relay checks the instruction and keeps it. Until `T` the author can edit the message, change the time, send it now or cancel it.
3. At `T` the relay checks that the author may still post, and stores the signed message as it stores any other.
4. The relay tells the author how the send ended, in a status that only the author can read.

A relay cannot write a message for an author, because it never holds the author's key, and a client cannot sign while it is closed. So the author signs ahead of time. The message is dated for the moment it posts, and the relay releases it inside the timestamp window it already enforces, so the signer rule and the timestamp rule stay as they are. The alternative is a future-dated event that the relay hides until it is due. That needs an exception to the timestamp rule and a filter on every read path, and one missed filter posts a message early.

Readers get an ordinary event signed by its author. They cannot tell that it was sent later, and they need nothing from this NIP to display it. It can reach them some time after the `created_at` it carries; see [Late arrival](#late-arrival).

Out of scope: recurring sends (a client creates one send for each occurrence), forum posts and any other kind the relay does not list, and [NIP-17](17.md) gift-wrapped direct messages, which are signed by a one-time key with a randomized timestamp and cannot be prepared as a dated, author-signed event. There is no cryptographic time lock: the relay can read a message as soon as it accepts the instruction.

## Kinds

| Kind | Name | Signer | Purpose |
|------|------|--------|---------|
| `30360` | Instruction | author | Holds one message to post later, or cancels it |
| `30361` | Status | relay | Tells the author how a send ended |

Both kinds are addressable, and only the author can read them.

`kind:40006` is an older Buzz kind named for scheduled messages. It is not part of send later: a relay stores it as an ordinary channel event that channel members can read. Implementations MUST NOT use it to send a message later.

## Terms

- **send**: one message waiting to be posted. Its address is `(author pubkey, 30360, d)`, and its `d` value is the **send id**.
- **instruction**: a `kind:30360` event. It either holds a message or is a **cancel**, which is empty.
- **message**: the signed event inside an instruction. It is an ordinary event of a kind the relay lists.
- **`T`**: the moment the message should post. It is the instruction's `not_before` and the message's `created_at`.
- **generation**: the version counter of a send. It starts at `1` and grows by one with each accepted replacement.
- **release**: the relay storing the message as an ordinary event. The **release window** runs from `T` through `T + release_window`.
- **pending**: a send whose newest accepted instruction holds a message, and which has not ended.
- **ended**: a send that was sent, failed, missed or cancelled. An ended send never becomes pending again.
- **executor key**: a relay key that instructions are encrypted to and statuses are signed with.
- **owner**: the key that the relay has recorded as owning an agent key in the community ([NIP-OA](NIP-OA.md), and "Agent keys" in [NIP-FI](NIP-FI.md)).

All times are integer Unix seconds. Every comparison of times uses the relay's clock.

## Instruction

`kind:30360` is an addressable event keyed by `(pubkey, kind, d)` as defined in [NIP-01](01.md).

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
    ["exec", "2026-11"],
    ["alt", "Send later"]
  ]
}
```

| Tag | Value |
|-----|-------|
| `d` | The send id. It MUST be exactly 32 lowercase hex characters that encode 16 bytes from a cryptographically secure random source. It MUST NOT be derived from the message, the channel or the time. |
| `not_before` | `T`, in the grammar [NIP-ER](NIP-ER.md) defines for this tag: a decimal Unix timestamp of ASCII digits only, with no sign, whitespace, decimal point or leading zero, from 0 through 9007199254740991. |
| `generation` | A decimal string in the same grammar, at least `1`. |
| `exec` | The `id` of the executor key that the content is encrypted to ([Support](#support)). |
| `alt` | Optional ([NIP-31](31.md)). |

An instruction that holds a message MUST have exactly one `d`, one `not_before`, one `generation` and one `exec` tag, and at most one `alt`. Each tag has exactly one value. Any other tag, a repeated tag or an extra value makes the instruction malformed. So an instruction has no `h`, `p` or `e` tag, and no channel, mention or thread query can match it.

`content` MUST be a [NIP-44](44.md) version 2 payload, encrypted with the conversation key of the author's key and the executor key that `exec` names. The relay decrypts it with the executor secret key. Any device of the author decrypts it with the author's key and the executor public key.

The plaintext is the signed message as UTF-8 JSON: one object with exactly the members `id`, `pubkey`, `created_at`, `kind`, `tags`, `content` and `sig`, and no wrapper. A plaintext that is not one JSON object, that repeats a member, or that has a missing or extra member is invalid.

### Message

The message inside an instruction MUST meet all of these:

1. Its `id` and `sig` verify as defined in NIP-01.
2. Its `pubkey` is the instruction's `pubkey`.
3. Its `created_at` equals `not_before` exactly.
4. Its `kind` is one the relay lists ([Support](#support)).
5. It has exactly one `h` tag, which names a channel of the community the instruction was sent to.

It is otherwise an ordinary event of its kind. A thread reply has its usual `e` tags, a message in a DM names that channel in `h`, a mention has its `p` tag, an attachment has its usual media tags, and an agent's message has the NIP-OA `auth` tag it would have if the agent posted it live.

### Cancel

```jsonc
{
  "kind": 30360,
  "pubkey": "<author>",
  "created_at": 1769995000,
  "content": "",
  "tags": [
    ["d", "a3f8c2e1b4d79600e5d2f1a8c3b6094d"],
    ["generation", "3"]
  ]
}
```

A cancel MUST have exactly one `d` and one `generation` tag, at most one `alt`, and empty `content`. It has no `not_before` and no `exec`.

## Changing a send

The first instruction accepted at an address MUST hold a message and have `generation` `1`. Each later instruction at that address MUST:

- have a `generation` one greater than the stored one; and
- win NIP-01 ordering against the stored instruction: a greater `created_at`, or the same `created_at` and a lexically lower `id`.

The first rule is a compare-and-swap. A device can replace only the generation it has seen, so two devices that change the same generation cannot both succeed. To meet the second rule, a client sets `created_at` to the later of its current time and the stored instruction's `created_at` plus one.

| Action | Instruction |
|--------|-------------|
| Send later | A message, a fresh `d`, `generation` `1` |
| Edit | The next generation, with a newly signed message |
| Change the time | The next generation, with a new `not_before` and a message newly signed for that time |
| Send now | The next generation, with `not_before` set to the current time and a message signed for that time. The relay releases it as soon as its checks pass |
| Cancel | A cancel at the next generation |

A replacement is a whole new instruction. It can change the content, the channel, the thread and the time together, and the relay checks all of it again.

A send that has ended closes its address. The relay MUST refuse every later instruction there. It MUST keep the address closed for as long as it stores a status or a cancel for it, and for at least `status_retention` after the send ended. To try again, a client creates a new send with a fresh `d`.

[NIP-09](09.md) does not apply to `kind:30360`. A relay MUST NOT delete an instruction or change a send because of a deletion request, and SHOULD reject a deletion request that names a `kind:30360` event or address. A cancel is the only way an author stops a pending send, so every change to a send has one path.

## Relay behavior

### Accepting

The relay accepts an instruction wherever it authenticates the sender, such as a [NIP-42](42.md) WebSocket session. It runs these checks in order, and the first one that fails decides the answer.

| # | Check | `OK` when it fails |
|---|-------|--------------------|
| 1 | The sender is authenticated as the event's `pubkey`, and `id` and `sig` verify | as for any event |
| 2 | The event is not the instruction already stored at its address | `true`, `duplicate: already have this event`; nothing changes |
| 3 | `created_at` is within the relay's usual tolerance of its clock | as for any event |
| 4 | The tags are those of an instruction that holds a message, or of a cancel, and `content` is at most `max_content_len` bytes | `invalid: malformed not_before` when that value breaks the grammar, otherwise `invalid: malformed instruction` |
| 5 | The address is not closed | `invalid: send ended` |
| 6 | The generation and ordering rules hold, and a cancel names an existing send | `invalid: stale generation`, `invalid: stale replacement`, `invalid: no such send` |
| 7 | A cancel is accepted here. The checks below apply to an instruction that holds a message | |
| 8 | The relay is accepting new sends | `restricted: send later disabled` |
| 9 | `now - 60 <= T` and `T <= now + max_not_before_delta` | `invalid: not_before in the past`, `invalid: not_before too far` |
| 10 | `exec` names a key the relay lists as `current` or `standby`, and `content` decrypts under it | `invalid: unknown executor key`, `invalid: undecryptable content` |
| 11 | The plaintext and the message meet [Instruction](#instruction), and the message passes every validity check the relay applies to an ordinary event of its kind, apart from the `created_at` tolerance | `invalid: message: <reason>` |
| 12 | The author may post the message to its channel at this moment: membership, permission to post, channel state, bans and timeouts | `restricted: message: <reason>` |
| 13 | Where the relay applies NIP-FI, no disconnect record fences the admission that carried the instruction ([Federated identity](#federated-identity)) | `restricted: authorization denied` |
| 14 | For a new address, the author has fewer than `max_pending` pending sends in the community | `restricted: send later quota exceeded` |

Clients match an `OK` message up to the end of the text shown. A relay MAY add detail after it.

Notes on the checks:

- Check 2 comes before the timestamp check, so that a client can confirm an acceptance however late it asks.
- Check 3 is the ordinary one. This NIP makes no exception to the signer rule or the timestamp rule.
- At check 7 the relay MUST NOT refuse a cancel because it is not accepting new sends, because the author is under a timeout, or because the author may no longer post to the channel.
- The 60 seconds in check 9 allow for clock difference on a send now.
- Under check 10 a relay MUST NOT accept a message under a `retiring` key or under a key it does not list.
- `max_pending` in check 14 is counted for each author key in each community. A replacement does not count against it. An agent's sends count against the agent's key.

When it accepts, the relay answers `OK true` and stores the instruction, the generation and the state of the send in one atomic change. Checks 5, 6, 13 and 14 MUST run inside that change, so that two racing instructions cannot both pass, and an instruction cannot slip past a disconnect that is being recorded. A refused instruction MUST change none of the three.

An accepted replacement discards the instruction it replaces. The relay MUST NOT release a replaced generation or keep it readable. An accepted cancel ends the send.

### Retries

A relay MUST answer a failure whose outcome it does not know, such as a storage error, with `error:`. A client that gets `error:`, or no `OK` at all, MUST NOT assume either outcome. It sends the same event again.

`OK true` settles the instruction as accepted. Any other answer leaves it open, because the instruction may have been accepted and then replaced or ended. The client then makes a [complete read](#complete-read). Before it creates a new send in place of an open one, it MUST also check that the message is not readable in its channel, so that the message is not posted twice.

### Releasing

For each pending send the relay does the following.

1. It waits for `T`. Before its clock reads `T` it MUST NOT store the message, deliver it to any subscriber, or let any query return it.
2. It MUST apply every check it would apply if the author posted the message at that moment on an authenticated session: membership, permission to post, channel state, bans, timeouts, attachments, content rules and the author's rate limit. It MUST also apply [Ending a send early](#ending-a-send-early).
3. It MUST store the message and mark the send `sent` in one atomic change, and that change MUST apply only if the send is still pending at the generation being released.
4. It then does everything it does for a message posted live: delivery to subscribers, push, mentions, thread counters and search indexing. The stored event is the author's signed event, unchanged.

Proof of a live session is the only thing step 2 does not require. The author's signatures on the instruction and on the message are the authority to post without one.

Step 3 settles every race with the author. A replacement or a cancel either commits first, and nothing from the old generation is posted, or commits second, and is refused with `invalid: send ended`. A relay MUST NOT answer `OK true` to a cancel and then post the message.

When a check in step 2 refuses the message:

- For a reason that will not clear by itself, the relay ends the send as `failed` and does not post.
- For a reason that may clear, such as a rate limit or an unavailable dependency, the relay retries with backoff inside the release window.

The relay MUST NOT release after `T + release_window`. A send that is still pending then ends as `missed`. That includes time during which the relay was down or releasing was switched off. A message is never posted late.

If the message is already stored when the relay comes to release it, for example because the author published the same event directly, the relay ends the send as `sent` with that event's id.

A relay SHOULD release each send as soon after `T` as its checks and the author's rate limit allow. It MAY spread sends that share one `T` over the following seconds or minutes. The rest of the window is for retries and recovery, and a relay SHOULD NOT use it to put off a release that it could make. Clients MUST NOT assume that a message posts at exactly `T`.

### Ending a send early

Some events end a pending send without the author. In this table "the key" also covers every key that the key owns in that community, so a restriction on an owner reaches its agents' sends.

| Event | Sends | What the relay does | Outcome |
|-------|-------|---------------------|---------|
| Ban | The key's, in that community | Cancels at once | `cancelled`, reason `banned` |
| NIP-FI disconnect | See [Federated identity](#federated-identity) | Cancels at once, before it answers the issuer | `cancelled`, reason `disconnected` |
| Channel deleted | That channel's | Cancels at once | `cancelled`, reason `channel_deleted` |
| The operator withdraws sends before a version change ([Rolling back](#rolling-back)) | The ones the operator names | Cancels them and stores their statuses before it stops | `cancelled`, reason `withdrawn` |
| Account or community deleted | That account's or community's | Removes them with the other data | none; no status is published |
| Timeout | The key's, in that community | Nothing when the timeout starts. Refuses at release if the timeout is still running | `failed`, reason `restricted` |
| Removal from the channel or the community, or loss of permission to post | That channel's or community's | Refuses at release | `failed`, reason `not_permitted` |
| Channel archived | That channel's | Refuses at release | `failed`, reason `archived` |
| Attachment no longer available | That send | Refuses at release | `failed`, reason `attachment` |

"At once" means in the same atomic change that records the ban, the disconnect or the deletion. A disconnect is also serialized against acceptance ([Federated identity](#federated-identity)).

A relay MUST apply the ban row and the channel-deleted row again at release, with the outcome the table gives. That catches an instruction that was being accepted while the ban or the deletion was being recorded.

A send that the relay cancelled stays cancelled. Lifting a ban or restoring access MUST NOT make it pending again. A timeout that ends before `T` has no effect on a send.

## Status

When a send ends for any reason other than the author's own cancel, the relay publishes one status. `kind:30361` is addressable.

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

| Tag | Value |
|-----|-------|
| `d` | The author's pubkey in lowercase hex, a colon, and the send id |
| `p` | The author's pubkey |
| `status` | `sent`, `failed`, `missed` or `cancelled` |
| `generation` | The generation that ended |
| `e` | Only for `sent`: the id of the stored message |
| `reason` | A machine-readable code. A relay SHOULD include it for `failed` and `cancelled`, and MUST omit it otherwise. Clients MUST treat a code they do not know as an absent reason |
| `expiration` | `created_at + status_retention` ([NIP-40](40.md)) |

`created_at` is the relay's clock when it signs the status. It is never earlier than the end of the send. `content` is empty.

| Status | When |
|--------|------|
| `sent` | The message is stored |
| `failed` | A release check refused it permanently |
| `missed` | It could not be released inside its release window |
| `cancelled` | The relay cancelled it |

`cancelled` always means that the relay cancelled the send. The author's own cancel produces no status; the cancel is the record.

| Reason | Status | Meaning |
|--------|--------|---------|
| `not_permitted` | `failed` | The author is no longer a member of the channel or the community, or may not post to the channel |
| `restricted` | `failed` | The author, or the author's owner, was under a timeout at release |
| `archived` | `failed` | The channel was archived |
| `attachment` | `failed` | An attachment was no longer available |
| `rejected` | `failed` | Any other permanent refusal |
| `banned` | `cancelled` | The author, or the author's owner, was banned |
| `disconnected` | `cancelled` | A NIP-FI disconnect applied to the author or the author's owner |
| `channel_deleted` | `cancelled` | The channel was deleted |
| `withdrawn` | `cancelled` | The relay operator ended the send before a version change that could not keep it |

A send has at most one status, and the relay MUST NOT revise it.

### Signer

The relay signs a status with the executor key that is `current` at that moment. A relay MUST reject a `kind:30361` event that a client submits.

Clients MUST accept a status only when its `pubkey` is listed in the relay's `keys`, whatever the entry's mark, and MUST ignore one with any other signer. A status therefore stays verifiable after the relay changes its keys, for as long as the relay serves it.

A client whose copy of the descriptor does not list the signer MUST read the descriptor again before it ignores the status. It does so once for each unlisted key. A read that returns the descriptor unchanged does not permit another read for that key.

### When the status is stored

The outcome of a send is final when the relay commits the change that ends it. A relay SHOULD store the status in that same atomic change.

A relay that stores the status afterwards MUST record the outcome durably in the change that ends the send, MUST sign the status when it stores it, and MUST keep trying until the status is stored. A crash may delay a status and MUST NOT lose one. [Reading](#reading) defines what the author can read in the meantime.

## Reading

Both kinds are private to the author. A relay MUST NOT reveal the existence, count, tags, content, time or search matches of a `kind:30360` or `kind:30361` event to anyone else, on any read path: `REQ`, `COUNT`, live subscriptions, lookups by id, search, HTTP bridges and push.

- `kind:30360` is readable only by a requester authenticated as the event's `pubkey`.
- `kind:30361` is readable only by a requester authenticated as the pubkey in the event's `p` tag.
- The owner of an agent cannot read the agent's instructions or statuses.
- For an unauthenticated request for only these kinds, a relay SHOULD close with `auth-required:`. In a filter that mixes kinds, the relay MUST omit the matches the requester may not read and handle the other kinds as usual.

A client follows its sends with one live subscription:

```jsonc
["REQ", "send-later",
  { "kinds": [30360], "authors": ["<own pubkey>"] },
  { "kinds": [30361], "#p": ["<own pubkey>"] }]
```

The relay delivers each accepted instruction and each published status to the author's own live subscriptions, so that the author's other devices stay in step. The relay's default limit can cut short the stored events this subscription returns, so a client derives state from a complete read.

### Complete read

A complete read is made on a connection authenticated as the author. It has three parts, in this order, and no request in it carries `since` or `until`.

1. **Live subscription.** The client holds an open subscription for both kinds, such as the one above, that the relay has answered with `EOSE`. It stays open until the read ends.
2. **Instructions.** One request, answered through `EOSE`, where `<max_limit>` is the relay's NIP-11 `max_limit`:

   ```jsonc
   { "kinds": [30360], "authors": ["<own pubkey>"], "limit": <max_limit> }
   ```

3. **Statuses.** One or more requests, each answered through `EOSE` and none naming more than `max_limit` values:

   ```jsonc
   { "kinds": [30361], "#p": ["<own pubkey>"], "#d": [<status d values>], "limit": <number of values> }
   ```

   Together they name every send that the client holds as pending or as ended with an unknown outcome, and every send for which part 2 returned an instruction that holds a message.

The read covers the sends that part 3 names and the sends whose cancel part 2 returned. An event that arrives on the live subscription before the read ends counts as returned by the read. Where the read returns two instructions for one address, the client keeps the one that wins NIP-01 ordering.

For each send the read covers, the first rule that applies gives its state:

1. A status was returned. The send ended as the status says.
2. An instruction was returned. One that holds a message means pending. A cancel means cancelled by the author.
3. Neither was returned. The send is not pending and its outcome is unknown.

Under rule 3, a client that knows the send from an earlier read shows it as ended with an unknown outcome until a status arrives. It MUST NOT show it as sent or as cancelled. A client MAY drop a send that has stayed under rule 3 for `status_retention`, and then stops naming it in part 3.

The relay is what makes the read complete:

- No request of a complete read is paged, and none can be cut short. The relay MUST return every event that matches the request of part 2, a number that [Retention](#retention) keeps within `max_limit`. It MUST return every event that matches a request of part 3, which is at most one status for each value named.
- It MUST answer each of these requests from one consistent view of its storage, so that a replacement in progress cannot hide a pending send.
- It MUST deliver to a live subscription every instruction it accepts and every status it stores after that subscription's `EOSE`. A change that commits while parts 2 and 3 run therefore reaches the client through part 1.
- It MUST NOT serve the instruction of an ended send, other than a cancel, until that send's status is readable. This keeps rule 2 true. A relay that stores the status in the change that ends the send meets it without further work. A relay that stores the status afterwards withholds the instruction of a `failed` or `missed` send until then.

### Other reads

Every other read is incremental: a live subscription update, or any request that is not part of a complete read, such as one that carries `since` or a status filter without `#d`. It updates the sends whose events it returns and says nothing about any other send.

A client MUST NOT use a `since` cursor to refresh `kind:30360`. An instruction carries its author's clock, which can be behind the relay's clock by as much as the relay's `created_at` tolerance. So an edit or a cancel from another device can carry a `created_at` that is earlier than events the client already holds.

Nothing else is evidence of a state. A request that failed, that the relay closed, or that ended before `EOSE`, a lost connection, a read that is not authenticated as the author, a complete read that lacks one of its parts, and the absence of a live update all leave the state unknown. The client keeps the last state it derived and MUST NOT infer an outcome from them.

A client's stored copy of a pending send can be out of date for a short time. The instruction of a `sent` or relay-cancelled send is deleted without an event, and the status is what tells the author's devices. Before the status arrives, two observations are authoritative:

- `invalid: send ended` in answer to an edit or a cancel. The send is not pending, and its outcome is unknown until the status arrives.
- The message, whose id the client knows, being readable in its channel. The send was `sent`.

## Retention

| Send ended as | Instruction | Status |
|---------------|-------------|--------|
| `sent`, or `cancelled` by the relay | Deleted when the send ends | Readable for `status_retention` |
| `failed`, `missed` | Stays readable to the author for `text_retention` after the status is stored, so that the author can recover the text. It is then deleted | Readable for `status_retention` |
| Cancelled by the author | The cancel replaces the instruction it cancels, which is deleted. The cancel stays readable for `status_retention` and is then deleted | none |

The bound in [Ended instructions](#ended-instructions) can delete the instruction in the last two rows earlier.

Deleting an instruction means that the relay no longer stores its ciphertext or the decrypted message in any form. A cancel holds no ciphertext, so keeping it keeps no message text.

### Ended instructions

The instructions of one author in a community MUST fit in one request ([Complete read](#complete-read)). `max_pending` bounds the pending ones. A relay MUST also keep at most `max_pending` ended instructions for one author in a community, counting the cancels and the instructions of `failed` and `missed` sends together. When the end of a send would exceed that number, the relay first deletes the ended instruction whose send ended earliest.

The address stays closed either way, but the two kinds of ended instruction lose different things:

- For a `failed` or `missed` send, the author loses the text before `text_retention` has passed. The status still records the outcome for `status_retention`.
- For a send that the author cancelled, the cancel is the only record of the outcome. Once it is deleted the relay holds no readable outcome for that send, before `status_retention` has passed, and a complete read reports the send under rule 3: not pending, outcome unknown.

A relay MUST advertise a NIP-11 `max_limit` of at least twice `max_pending`, and MUST NOT change either limit in a way that lets the instructions of one author exceed `max_limit`.

### Executor keys

A relay MUST keep an executor key in the descriptor for as long as an instruction encrypted to it is pending or readable, or a status signed with it is readable. It MUST keep the secret key for as long as an instruction encrypted to it is pending. A relay accepts no new instruction under a `retiring` key and signs no new status with it, so what depends on that key only shrinks.

## Support

Until this draft has an upstream NIP number, relays MUST NOT list it in [NIP-11](11.md) `supported_nips`. A relay advertises send later for the community that a NIP-11 request resolves to, by adding `"buzz-send-later"` to `supported_extensions` and by serving this descriptor:

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
| `accepting` | `true` when the relay accepts new sends. When `false` it accepts only cancels and reads |
| `keys` | The executor keys, described below |
| `kinds` | The kinds a message may have |
| `limitation` | The limits in [Limits summary](#limits-summary) |

Each entry of `keys` has an `id`, a `pubkey` and exactly one of these marks, set to `true`:

| Mark | Meaning |
|------|---------|
| `current` | Clients encrypt new instructions to this key, and the relay signs new statuses with it. Exactly one entry is `current` |
| `standby` | The relay also accepts new instructions under this key while a key change is in progress ([Changing the executor key](#changing-the-executor-key)) |
| `retiring` | The relay accepts no new instruction under this key. It stays listed while something that depends on it is readable ([Executor keys](#executor-keys)) |

Ids are unique, non-empty and at most 64 bytes. A relay MAY list its NIP-11 `self` key as an executor key.

A relay MUST enforce the limits it advertises. A relay that hosts several communities MUST serve the descriptor only for a community in which send later is enabled.

A client MUST treat the descriptor as absent when `supported_extensions` does not contain `"buzz-send-later"`, when `v` is not `1`, when a member shown above is missing or has the wrong type, or when `keys` does not contain exactly one `current` entry. A client MUST NOT publish `kind:30360` to a relay without a valid descriptor, and MUST NOT offer send later there.

A relay without this NIP either rejects both kinds as unknown or stores them as ordinary events that any reader can query. That is why a client checks the descriptor before it publishes. A client without this NIP subscribes to neither kind.

## Client rules

- Offer send later only against a valid descriptor with `"accepting": true`, and keep `T` and the number of pending sends inside the advertised limits.
- Encrypt to the key marked `current`. On `invalid: unknown executor key`, read the descriptor again and encrypt to the key it now marks `current`.
- To decrypt a stored instruction, look its `exec` value up in `keys`. Read the descriptor again if the client's copy does not list it, within the same limit as for an unlisted status signer ([Signer](#signer)).
- Verify the signature of each instruction before decrypting it, and verify that each status is signed by a key listed in the descriptor.
- Treat a send as cancelled only after `OK true` for the cancel. Treat `invalid: send ended` as final, and read the status to learn the outcome.
- On `invalid: stale generation` or `invalid: stale replacement`, read the address again. Another device changed the send.
- Do not publish the message directly while its send is pending. To post immediately, use send now.
- After a reconnect, make one complete read. Do not issue one request for each send, and do not refresh instructions with a `since` cursor.
- A client that holds pending sends and finds that the relay no longer serves a valid descriptor treats their state as unknown. It MUST NOT publish `kind:30360` there, and it reads again when the descriptor returns.
- The id of the message is known in advance. A client MAY use it to find the released message before the status arrives.

## Late arrival

A released message carries `created_at` `T` and can reach readers up to `release_window` later. The relay's `created_at` tolerance already permits the same late arrival for an event from an author whose clock is slow, but this NIP makes it routine. A reader therefore cannot take `created_at` for the time a message arrived.

A reader that opens a live subscription with `since` set to its current time, or that fetches new messages from the time of its last read, does not receive a message that is released after that point with an earlier `created_at`. Readers SHOULD set such a `since` back by at least the relay's `created_at` tolerance, and discard by `id` the events they already hold.

### Unread

Setting `since` back delivers a late message. It does not settle what is unread.

A reader that records a channel as read up to a `created_at` value treats a message that is released after that record, with a `created_at` at or before it, as already read. A reader with that kind of read state displays a late message and counts it as read. This NIP does not remove that limitation.

The time at which a device first received a message does not correct it, whether the message arrived on a live subscription or in a later read:

- A device that was offline would count messages that the user read on another device as unread.
- A device that receives a message live can process it after a read that another device has already recorded.

Arrival at one device is therefore not evidence that a message is unread.

This NIP defines no read state. Counting a late message as unread on every device of a user needs read state that those devices share and that does not depend on `created_at`: an order that follows when the relay stored a message, or a complete record of the messages read by `id`. A client with such read state SHOULD count a late message as unread. A client without it MUST NOT substitute a time or an order that is local to one device.

A relay limits the effect by releasing promptly ([Releasing](#releasing)).

## Remote signers

This NIP adds no delegation. A client whose key is held by a [NIP-46](46.md) remote signer asks the signer to sign the message with `created_at` set to `T`, to perform `nip44_encrypt` to the executor key, and to sign the `kind:30360` instruction. Reading a stored instruction needs `nip44_decrypt` with the executor key. The relay applies the same rules wherever the key is held.

Signing takes time. A client that prepares a send now through a remote signer SHOULD expect `invalid: not_before in the past` and retry with a newly signed message.

## Federated identity

Where a community requires NIP-FI, a pending send posts only while the identity issuer still stands behind its author. A release happens without a session, so the session rules of NIP-FI cannot stop it. [Deferred actions](NIP-FI.md#deferred-actions) in NIP-FI defines what does, and a send is a deferred action under that section:

- The relay records, with each accepted generation, the issuer that admitted the session or request that carried it, and the owner recorded for that admission.
- An accepted disconnect command cancels the pending sends of the target key, and of every key whose owner is the target key, that were accepted under the command's issuer, in every community. They end as `cancelled` with reason `disconnected`.
- The relay refuses an instruction whose admission a disconnect record fences. It decides that in the atomic change that would store the instruction, and that change is serialized against the disconnect command.

Nothing else in NIP-FI ends an accepted send. The relay does not contact the issuer at release, does not require a recent assertion, and does not evaluate the community's issuer policy again. A community that starts to require NIP-FI, or that stops authorizing an issuer, does not by that change alone end a send it already accepted. The ordinary release checks still apply.

A relay MUST NOT enable send later in a community that requires NIP-FI unless it implements "Deferred actions". An operator MUST NOT enable it there unless every issuer the community authorizes sends a disconnect command when an identity loses access. An issuer that only stops issuing assertions does not stop a pending send.

On a relay or in a community without NIP-FI, bans, timeouts and the release checks are what stop a pending send.

## Several relay processes

A deployment MAY run any number of relay processes against the same storage, and any of them MAY release any send.

- Every change to the state of a send (accept, replace, cancel, release, an early end by the relay, `missed`) MUST be a conditional change in the shared durable storage. It MUST apply only if the send is in the state and at the generation that the process read. The process whose change applies owns the outcome. Every other process finds nothing to do.
- A process MUST NOT rely on state held only in its own memory, such as a NIP-FI deny set, to decide whether a send may be accepted or released.
- The message MUST be stored at most once, however many processes attempt the release.
- A send and the message it becomes MUST be committed in the same storage transaction. A deployment that partitions its storage MUST keep both in the same partition.

### Changing the executor key

An executor key changes in three steps. Every process MUST complete a step before any process begins the next.

1. Every process holds the new key pair and lists the new key as `standby`.
2. Every process marks the new key `current` and the old key `standby`.
3. Every process marks the old key `retiring`.

A process accepts an instruction under its `current` key and under every key it lists as `standby` (check 10 of [Accepting](#accepting)). So while the processes disagree about a step, each of them accepts an instruction under the key that any of them advertises as `current`, and each lists the key that any of them signs a status with.

Every process holds the secret key of each key it lists that still has a pending instruction.

## Rollout

A deployment MUST introduce send later in this order:

1. Every relay process that shares the storage, and every tool that reads or deletes stored events, enforces [Reading](#reading) for both kinds and refuses both kinds at ingest. No process advertises send later. A deployment MUST NOT roll back below this version once step 2 has begun.
2. Send later is enabled for a community. Only now may a process accept `kind:30360`, publish `kind:30361` or serve the descriptor.

### Switching off

A relay SHOULD provide two controls for each community:

- **Accepting.** When it is off, the descriptor carries `"accepting": false` and the relay refuses new sends. Pending sends are still released, cancels are still accepted, and reads still work.
- **Releasing.** When it is off, nothing is released. A send whose release window passes meanwhile ends as `missed`.

While a pending send, a readable instruction or a status exists in a community, a relay with send later MUST keep serving reads and accepting cancels there, and MUST end every pending send with an outcome.

### Rolling back

A deployment SHOULD switch off the two controls rather than roll back to the step 1 version while sends exist.

The step 1 version keeps both kinds private and releases nothing, so no message is posted after a cancel. It also refuses every instruction, including a cancel, and publishes no status, so the guarantee in [Switching off](#switching-off) is suspended for as long as it runs. When a version with send later runs again, it MUST end each send whose release window has passed as `missed`.

A version without send later also cancels nothing:

- A ban or a channel deletion that it records, and that still stands at release, ends the send then, because the relay applies those rows of [Ending a send early](#ending-a-send-early) again at release.
- A NIP-FI disconnect command leaves nothing to apply again. A version that does not apply "Deferred actions" answers the command `200`, keeps no disconnect record and cancels nothing. The sends of the target and of its agents would be released once a version with send later runs again.

A deployment therefore MUST NOT run a version that does not apply NIP-FI "Deferred actions" while a send whose recorded admission names an issuer is pending. Either its step 1 version applies that section to the stored sends, or the deployment does the following, in this order:

1. It switches Accepting off for every community on every process.
2. It ends every such send as `cancelled` with reason `withdrawn`, and a process with send later stores those statuses.
3. It confirms that no such send is pending, and repeats step 2 if one is. An instruction that was being accepted when Accepting was switched off can still commit afterwards.
4. Only then does it start a process without send later.

## Privacy

| Party | Learns |
|-------|--------|
| Readers of the channel | Nothing about send later. They receive an ordinary event dated `T` |
| Other users of the relay | Nothing. Both kinds are readable only by the author |
| The relay | The message, its channel and its time, from the moment it accepts the instruction |
| Someone who can read the relay's storage but not the executor secret key | That an author has a send, its `T`, its generation and how it ended. For a `sent` status, also the id of the posted message. Not the content or the channel of a pending send |

The public `not_before` tag is what lets a relay find due sends without decrypting every instruction. An author who does not want a relay to learn that they have a message pending at a given time should not use send later there.

## Security

The relay is trusted with the signed message, which is a valid event from the moment it is signed. A relay that misbehaves can post it inside the timestamp window around `T` after a cancel, or withhold it. It cannot forge a message, change one, or post one at a time other than the one the author signed. A cancel is a promise that the relay enforces, not a cryptographic revocation. Erasure at the end of a send limits how long the relay holds a message its author may want back.

There is no live session at release. Authority to post comes from the author's two signatures, and everything that would stop the author from posting at release time also stops the send. Under NIP-FI the exception is an identity that lost access without a disconnect command: its sends keep posting for up to `max_not_before_delta`, in channels the key still belongs to. A ban stops them.

An agent's message carries the same NIP-OA `auth` tag as one it posts live. A `created_at` condition in that tag is evaluated against the message's `created_at`, which is `T`, so an owner's time bound on an agent also bounds how far ahead the agent can send later.

An old instruction cannot be submitted later: the relay accepts one only from its authenticated author, inside the ordinary `created_at` tolerance, at the next generation of an address that is not closed. Whoever holds the author's key can read, replace and cancel that author's sends, as they can sign any other event.

The conversation key of an author and an executor key protects every instruction that author encrypts to that key. A relay that uses the same key pair for another encrypted kind MUST NOT accept the ciphertext of one kind as content of the other.

Every message counts against its author's ordinary rate limit at release, and `max_pending` bounds what one key can queue. A relay SHOULD count each instruction against the author's ordinary rate limit as well, which bounds how many statuses one key can accumulate inside `status_retention`.

## Limits summary

The relay advertises these in `limitation` ([Support](#support)). Times are in seconds.

| Member | Limit | Recommended |
|--------|-------|-------------|
| `max_not_before_delta` | How far ahead `T` may be | 10368000 (120 days) |
| `max_pending` | Pending sends for one author in a community. The same number bounds the ended instructions the relay keeps for that author | 100 |
| `release_window` | How long after `T` the relay may still release. It MUST NOT exceed the tolerance the relay applies to `created_at` on ordinary events | 900 |
| `text_retention` | How long the instruction of a `failed` or `missed` send stays readable | 604800 (7 days) |
| `status_retention` | How long a status stays readable. It MUST be at least `text_retention` | 2592000 (30 days) |
| `max_content_len` | Maximum length of instruction `content`, in bytes | |
