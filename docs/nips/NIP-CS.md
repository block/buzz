NIP-CS
======

Compare-and-Set for Replaceable Events
--------------------------------------

`draft` `optional` `relay`

This NIP lets a writer ask the relay to replace a replaceable or addressable event only if the writer's view of the current head is still correct. The writer names the head it read in a signed `expected-revision` tag. The relay checks the tag and the replacement in one atomic step.

## Motivation

[NIP-01](01.md) replacement keeps the event with the latest `created_at`. Two writers can read the same head, change it, and publish a full replacement each. The relay accepts both. One replacement wins by timestamp. The other replacement is lost, and both writers see success.

A writer that publishes once and exits cannot merge live updates. It needs the relay to refuse a write that was built on an old head.

## Terminology

- **coordinate**: for a replaceable kind (`0`, `3`, `10000`-`19999`), the pair `(kind, pubkey)`. For an addressable kind (`30000`-`39999`), the triple `(kind, pubkey, d)`.
- **head**: the live event of a coordinate with the highest `created_at`. On a tie, the event with the lowest `id` is the head.

## The `expected-revision` tag

```json
["expected-revision", "<value>"]
```

`<value>` is one of:

- a 64-character hexadecimal event ID. The head of the coordinate MUST have this ID.
- the literal `none`. The coordinate MUST have no live head.

The tag has exactly two elements. An event MUST NOT carry more than one `expected-revision` tag. The tag is part of the signed event, so a proxy cannot remove it without invalidating the signature.

This tag is the same tag that Buzz canvas writes (`kind:40100`) and workflow definitions (`kind:30620`) already use. It has the same values and the same meaning.

## Relay behavior

A relay that supports this NIP MUST apply these rules to a replaceable or addressable event that carries the tag.

1. The relay MUST read the head, check the tag, and store the event in one atomic step. No other write to the same coordinate can happen between the check and the write.
2. If the event ID is already the head, the relay MUST accept the event and report `duplicate:`. This makes a retry safe.
3. If `<value>` is an event ID and the head has a different ID, the relay MUST reject the event.
4. If `<value>` is an event ID and the coordinate has no head, the relay MUST reject the event.
5. If `<value>` is `none` and the coordinate has a head, the relay MUST reject the event.
6. If the check passes but the event does not replace the head under NIP-01 ordering (it is older, or it ties and has a higher `id`), the relay MUST reject the event. A tagged writer must not believe that a dropped write succeeded.
7. If the tag is malformed, or appears more than once, the relay MUST reject the event with the `invalid:` prefix.
8. A rejected event MUST NOT be stored, and the head MUST NOT change.

A relay MUST NOT change how it handles an event that has no `expected-revision` tag.

### Rejection

The relay answers with `["OK", <event-id>, false, "conflict: <reason>"]`. The message starts with `conflict:`. A client uses the prefix to tell a lost race from a malformed or unauthorized event. Over HTTP, the relay answers with status `409`.

## Client behavior

1. Read the head of the coordinate. Remember its `id` and `created_at`. If no head exists, use `none`.
2. Build the replacement from that head.
3. Set `created_at` to the larger of the current time and the head's `created_at` plus one. This makes the event sort ahead of the head, so it cannot lose the `id` tie-break.
4. Add the `expected-revision` tag and publish.
5. On `conflict:`, read the head again, apply the change again, and publish again. A client MUST NOT retry with the same expected head.

## Security considerations

The tag does not grant any permission. The relay applies the normal authentication and authorization rules before it checks the tag.

A conflict response tells a writer that a head exists, and its ID is not the expected one. The writer can already read the head with the same permissions, so this leaks nothing new.
