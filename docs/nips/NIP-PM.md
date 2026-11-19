NIP-PM
======

Pinned Messages
---------------

`draft` `optional` `relay`

This NIP defines how members pin and unpin messages in a channel. A member signs one small command for each action. The relay applies the commands one at a time and publishes the channel's pins as one relay-signed list. Clients read the list. They never write it. A Buzz DM is a channel, and its participants pin in it the same way.

[NIP-29](29.md) has the member send the whole list. Two members who pin at the same time then overwrite each other, and one command can clear a channel. Here the member never sends the list, so there is nothing to conflict on. NIP-29 also puts group pins at `kind:39005`, which Buzz already uses for thread summaries.

## Kinds

| Kind | Name | Signer | Purpose |
|------|------|--------|---------|
| `40010` | Pin | member | Command: pin one message |
| `40011` | Unpin | member | Command: unpin one message |
| `30623` | Channel pins | relay | The channel's pin list |

`kind:40004` is an older Buzz kind named for pinned messages. It is not a pin here: it never changes the list, and clients MUST NOT send it to pin a message. This NIP uses new kinds because a relay without it stores `kind:40004` unchecked, and rejects the kinds above as unknown.

## Commands

```jsonc
{
  "kind": 40010, // 40011 to unpin
  "content": "",
  "tags": [
    ["h", "<channel id>"],
    ["e", "<message id>"],
    ["client-id", "<random value>"]
  ]
}
```

A command MUST have exactly one `h` tag and exactly one `e` tag. The `e` value MUST be a 64-character lowercase hex event ID. `content` MUST be empty.

A client MUST put a fresh random value, such as a UUID, in the `client-id` tag of each new command. Without it, two commands from one member for one message in the same second are the same event, and the relay answers the second as a resend of the first. The relay does not read the value.

Other tags, and elements after a tag value, carry no meaning.

## Pin list

`kind:30623` is an addressable event keyed by `(relay pubkey, kind, d)` as defined in [NIP-01](01.md). A channel has at most one.

```jsonc
{
  "kind": 30623,
  "pubkey": "<relay pubkey>",
  "content": "",
  "tags": [
    ["d", "<channel id>"],
    ["h", "<channel id>"],
    ["e", "<newest pin>"],
    ["e", "<older pin>"]
  ]
}
```

The tags follow NIP-29's `kind:39005`, plus the `h` tag that clients [read](#reading) the list by.

The `e` tags are the pinned messages, newest pin first. Their order is the display order. A list with no `e` tags means the channel has no pins. So does a missing list.

Each new list has a `created_at` greater than the list it replaces.

## Relay behavior

A relay that supports this NIP MUST reject `kind:30623` from clients.

A channel's list changes through a command or through the deletion of a pinned message. The relay applies a channel's changes one at a time, in one order. Each change reads the current list and replaces it.

For each command the relay:

1. Checks that the author may post a message in the channel. The rule for posting decides, not membership alone: a guest cannot pin or unpin, and nobody can in an archived channel.
2. For a pin, checks that the target is a message in this channel that is not deleted. It puts the target at the front of the list.
3. For an unpin, removes the target from the list.
4. Stores the command, the new list and the notice together, then answers.

A message is a stream message (`kind:9`, `kind:40002` or `kind:40008`), a forum post (`kind:45001`) or a forum comment (`kind:45003`).

A channel holds at most 250 pins; a pin past that is rejected. An entry whose message is gone does not count.

Pinning a message that is already pinned, or unpinning one that is not, is accepted and changes nothing.

| Outcome | `OK` |
|---------|------|
| Applied, or nothing to change | `true`, empty message |
| Same event sent again | `true`, `duplicate:` |
| Target cannot be pinned | `false`, `invalid: target is not a message in this channel` |
| Channel is at the limit | `false`, `invalid: pin limit reached` |
| Author may not post, or the channel is archived | `false`, as for a message |
| Relay does not accept the commands | `false`, as for an unknown kind |

### Retries

A client that gets no answer MUST send the same signed event again. It MUST NOT sign a new one. The relay remembers command IDs and does not apply a command twice. Otherwise a repeated pin could undo an unpin that another member made in between.

A relay refuses an event whose `created_at` is too far from its clock (15 minutes on a Buzz relay), and it does so before it looks for the command ID. So this refusal does not say whether the command was applied. A client whose retry is refused for that reason MUST stop retrying and read the list. It MUST NOT sign a new command by itself. It signs one only when the member acts again.

### Deleted messages

When the relay accepts the deletion of a pinned message, it removes the message from the list and publishes the new list in the same step. This holds whoever deletes it: its author, the owner of the agent that wrote it, a channel admin or a relay admin. The removal takes its turn in the channel's order, like a command, and the relay decides in that turn whether the message is pinned.

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
{ "kinds": [30623], "#h": ["<channel id>"] }
```

The relay returns the list only to those who can read the channel, and sends each new list to open subscriptions with this filter.

Clients read by `#h` because it names the channel under [NIP-CL](NIP-CL.md), which a read of a system channel, or of an unlisted channel the reader has not joined, needs. A `#d` filter does not name the channel for this kind and gets no new lists.

A relay that supports this NIP MUST give its `self` key in its [NIP-11](11.md) document. Clients MUST ignore a list that key did not sign. Clients keep the list with the newest `created_at`.

The list holds IDs only. Clients fetch the messages by ID, in a filter that also names the channel with `#h`. They load a pinned message's edits as they do for any other message.

## Support

A relay does not advertise this NIP. A client learns that a relay does not accept the commands when the relay rejects one as an unknown kind.

A relay that stops accepting commands keeps serving the lists it has, and keeps removing deleted messages from them.

## Limits summary

| Item | Limit |
|------|-------|
| Pins per channel | 250 |
| `h` tags on a command | exactly 1 |
| `e` tags on a command | exactly 1 |
| Command `content` | empty |
