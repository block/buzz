NIP-CP
======

Channel Pins
------------

`draft` `optional` `relay`

This NIP defines how members pin and unpin messages in a channel. A member signs one small command for each action. The relay applies the commands one at a time and publishes the channel's pins as one relay-signed list. Clients read the list. They never write it.

[NIP-29](29.md) has the member send the whole list. Two members who pin at the same time then overwrite each other, and one command can clear a channel. Here the member never sends the list, so there is nothing to conflict on. NIP-29 also puts group pins at `kind:39005`, which Buzz already uses for thread summaries.

## Kinds

| Kind | Name | Signer | Purpose |
|------|------|--------|---------|
| `40010` | Pin | member | Command: pin one message |
| `40011` | Unpin | member | Command: unpin one message |
| `30623` | Channel pins | relay | The channel's pin list |

## Commands

```jsonc
{
  "kind": 40010, // 40011 to unpin
  "content": "",
  "tags": [
    ["h", "<channel id>"],
    ["e", "<message id>"]
  ]
}
```

A command MUST have exactly one `h` tag and exactly one `e` tag. The `e` value MUST be a 64-character lowercase hex event ID. `content` MUST be empty. Other tags, and elements after a tag value, carry no meaning.

## Pin list

`kind:30623` is an addressable event keyed by `(relay pubkey, kind, d)` as defined in [NIP-01](01.md). A channel has at most one.

```jsonc
{
  "kind": 30623,
  "pubkey": "<relay pubkey>",
  "content": "",
  "tags": [
    ["d", "<channel id>"],
    ["e", "<newest pin>"],
    ["e", "<older pin>"]
  ]
}
```

The tags follow NIP-29's `kind:39005`.

The `e` tags are the pinned messages, newest pin first. Their order is the display order. A list with no `e` tags means the channel has no pins. So does a missing list.

Each new list has a `created_at` greater than the list it replaces.

## Relay behavior

A relay that supports this NIP MUST reject `kind:30623` from clients.

For each command the relay:

1. Checks that the author may post in the channel and that the channel is not archived. Guests cannot pin or unpin.
2. Applies the channel's commands one at a time.
3. For a pin, checks that the target is a message in this channel that is not deleted and that every reader of the channel can see. It puts the target at the front of the list. A channel holds at most 250 pins; a pin past that is rejected.
4. For an unpin, removes the target from the list.
5. Stores the command, the new list and the notice together, then answers.

Pinning a message that is already pinned, or unpinning one that is not, is accepted and changes nothing.

| Outcome | `OK` |
|---------|------|
| Applied, or nothing to change | `true`, empty message |
| Same event sent again | `true`, `duplicate:` |
| Target cannot be pinned | `false`, `invalid: target is not a message in this channel` |
| Channel is at the limit | `false`, `invalid: pin limit reached` |
| Author may not post, or the channel is archived | `false`, as for a message |

### Retries

A client that gets no answer MUST send the same signed event again. It MUST NOT sign a new one. The relay remembers command IDs and does not apply a command twice. Otherwise a repeated pin could undo an unpin that another member made in between.

### Deleted messages

When a pinned message is deleted by its author, a channel admin or a relay admin, the relay removes it from the list and publishes the new list in the same step.

A relay MAY remove messages in bulk, for example when it erases an account, without updating the list. Clients MUST skip an `e` tag whose message they cannot load. The relay drops such entries the next time the list changes.

### Notice

When a pin changes the list, the relay posts a `kind:40099` system message in the channel:

```jsonc
{
  "type": "message_pinned",
  "actor": "<member pubkey>",
  "target_event_id": "<message id>",
  "command_event_id": "<pin command id>"
}
```

`command_event_id` gives each pin its own notice, even when a member pins the same message twice. The notice never carries the message content. An unpin posts no notice. Clients MUST NOT count the notice as unread, and relays MUST NOT send a push notification for it.

## Reading

```jsonc
{ "kinds": [30623], "#d": ["<channel id>"] }
```

The relay returns the list only to those who can read the channel. It also sends each new list to subscriptions that name the channel with `#h`, as it does for the channel's other relay-signed state.

Clients MUST ignore a list that the relay's [NIP-11](11.md) `self` key did not sign. Clients keep the list with the newest `created_at`.

The list holds IDs only. Clients fetch the messages by ID.

## Discovery

A relay that accepts the commands lists `buzz-pins` in `supported_extensions` of its [NIP-11](11.md) document. Clients SHOULD offer pin and unpin only when they see it. A relay without it rejects `kind:40010` and `kind:40011` as unknown kinds.

A relay that stops accepting commands keeps serving the lists it has, and keeps removing deleted messages from them.

## Limits summary

| Item | Limit |
|------|-------|
| Pins per channel | 250 |
| `h` tags on a command | exactly 1 |
| `e` tags on a command | exactly 1 |
| Command `content` | empty |
