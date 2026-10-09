NIP-CL
======

Channel Identity and Discovery
------------------------------

`draft` `optional` `relay`

This NIP adds identity facts to the relay-signed NIP-29 channel-state events
(kinds 39000–39003). With these facts, a client can find channels with
ordinary NIP-01 filters. It needs no new filter key and no relay-specific API.

## Motivation

Plugins and agents often create a channel for their own use and must find it
again later, for example "the config channel that this agent created". With
plain NIP-29, a client cannot:

- tell who created a channel;
- read the channel type from any event except kind:39000; or
- tell which of two channels is older. The `created_at` of a kind:39000 event
  is the time of the last edit, not the time of creation.

A client also cannot mark its own channels so that it can find them again.
Channel labels (below) do this.

Also, every channel that a person belongs to shows in their client as a chat.
So a group that exists only to control who can read some data appears in
sidebars, search results and unread counts. System channels (below) fix this.

## Channel identity tags

On every channel-state event that the relay signs, the relay MUST put these
tags in this order, before any other `t` or `P` tag:

1. `["t", <channel type>]`, for example `stream`, `forum`, `dm`, `workflow`, `system`.
2. `["t", <label>]` for each channel label, in stored order (see
   [Channel labels](#channel-labels)).
3. `["P", <creator public key, hex>]`.

kind:39000 also carries `["created_at", <unix seconds>]`.

Example kind:39000 (signing fields and unrelated tags omitted):

```json
{
  "kind": 39000,
  "pubkey": "<relay public key>",
  "created_at": 1760025600,
  "tags": [
    ["d", "9b353519-f4fe-4757-aef4-bec6cc0ae54c"],
    ["name", "agent-config"],
    ["t", "stream"],
    ["t", "agent-config"],
    ["P", "<creator public key>"],
    ["created_at", "1759939200"]
  ]
}
```

| Tag | Kinds | Meaning |
| --- | --- | --- |
| first `t` | 39000–39003 | Channel type. |
| later `t` | 39000–39003 | Channel labels. |
| `P` | 39000–39003 | Public key that signed the kind:9007 that created the channel. |
| `created_at` | 39000 | Time the channel was created, in unix seconds, as a decimal string. |

**Channel type.** Clients MUST read the channel type from the first `t` tag.
Later `t` tags are labels.

**Creator.** The relay writes `P` from its own record of the kind:9007 that
created the channel. No client can set it. It does not change when ownership
moves to another member or when the channel is edited.

Uppercase `P` follows NIP-22, NIP-34 and NIP-72, where `P` names the author of
the root object. On these kinds, lowercase `p` already lists members, admins
and DM participants. A filter matches tag values, not their roles, so `#p`
would mix creators with members.

**Creation time.** The `created_at` tag does not change when the channel is
edited. Use it, not the event's `created_at`, to compare the age of channels.

## Channel labels

A label groups channels across channel types, for example `workspace` or
`agent-config`. Labels have no meaning to the relay.

**Set labels.** A kind:9007 (create group) MAY carry up to 8 `["t", <label>]`
tags. On kind:9002 (edit metadata):

- `t` tags replace the whole label set;
- one `["t", ""]` tag and no other `t` tag clears the set;
- no `t` tag leaves the labels as they are.

Only an owner or admin of the channel MAY change its labels.

**Rules.** A label is 1–64 characters from `a-z`, `0-9`, `.`, `:` and `-`. A
label MUST NOT be `stream`, `forum`, `dm`, `workflow` or `system`. These are
channel type names, so this rule stops a label from making a channel look like
another type. The relay MUST reject the whole event if it has more than 8
labels, an invalid label, a reserved name, or `["t", ""]` together with other
labels. The relay stores repeated labels once.

## Finding channels

Use ordinary NIP-01 filters on the channel-state kinds.

| To find | Filter |
| --- | --- |
| Channels that a key created | `{"kinds": [39000], "#P": ["<creator>"]}` |
| Channels with a label | `{"kinds": [39000], "#t": ["<label>"]}` |
| One creator's channels with a label | `{"kinds": [39000], "#P": ["<creator>"], "#t": ["<label>"]}` |
| Channels of one type | `{"kinds": [39000], "#t": ["<type>"]}` |

Values in one tag filter combine with OR, as in NIP-01. So
`"#t": ["workspace", "notes"]` finds channels with either label. To require a
label and a creator, use `#t` and `#P` together.

The relay MUST match `#t` and `#P` before it applies `limit`, as NIP-01
requires. A relay that reads the newest channel events first and matches tags
after can return a short or empty page while matching channels exist.

## Choosing one channel

The relay does not make labels unique. Many channels can share a label, and
any owner or admin can set any label. So two creates for the same label can
both succeed, for example when two processes of one agent start at the same
time.

A client that needs one channel per creator and label:

1. queries `{"kinds": [39000], "#P": ["<creator>"], "#t": ["<label>"]}`;
2. creates the channel only if the query returns nothing; and
3. when the query returns more than one channel, uses the oldest one.

The oldest channel has the lowest `created_at` tag value. If two channels have
the same value, the lowest channel ID (`d` tag, compared as a string) wins. A
channel without a `created_at` tag ranks after every channel that has one.
Every reader that follows this rule picks the same channel, even after either
channel is edited.

## System channels

A system channel exists only to control who can read some data, for example
config that only an agent and its owner can read. It has the channel type
`system`. In every other way it is an ordinary channel: it has members, roles,
messages and labels.

**Create.** A kind:9007 with `["channel_type", "system"]` creates a system
channel. It is private unless the kind:9007 also has `["visibility", "open"]`.

**Hidden.** While a system channel is private, its kind:39000 carries the
NIP-29 `hidden` tag.

### Reads that leave system channels out

The relay leaves system channels out of every read that does not name them.
Clients that do not know about system channels then stay correct with no
change. This applies to:

- a filter without `#h`, for example the channels or messages a reader can see;
- kind:39002 `#p:["<reader>"]`, the channels a reader belongs to;
- COUNT and NIP-50 search; and
- any relay-specific query API that does the same reads.

A filter **names** a system channel when it has the channel ID in `#h`, or in
`#d` on a filter whose kinds are all 39000–39003. Then the ordinary read rules
apply: the reader can read the channel if they are a member or if it is open.
A client that has the ID, for example from a link, can open the channel. Thus
a system channel stays an ordinary NIP-29 group.

The relay decides this for each filter. A filter that names a system channel
does not add it to another filter in the same request.

The relay MUST NOT send push notifications or kind:44100/44101 membership
notifications for system channels. A client that received one would show the
channel.

## Trust

A client that relies on these tags MUST check that the relay's key signed the
event, as for all NIP-29 group-state events.

Labels give no trust. Any owner or admin can set them. Only `P` says who
created a channel.

## Relation to other NIPs

- NIP-29: the group-state events that this NIP adds tags to.
- NIP-01: the filters that find channels.
- NIP-22, NIP-34 and NIP-72: the convention for uppercase `P`.
