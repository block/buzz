NIP-CL
======

Channel Identity and Discovery
------------------------------

`draft` `optional` `relay`

This NIP adds identity facts to the relay-signed NIP-29 channel-state events
(kinds 39000–39002). With these facts, a client can find channels with
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
| first `t` | 39000–39002 | Channel type. |
| later `t` | 39000–39002 | Channel labels. |
| `P` | 39000–39002 | Public key that signed the kind:9007 that created the channel. |
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

**Older events.** A relay adds these tags only when it publishes a
channel-state event. An event that a relay version without this NIP published
can lack them, including one published during an upgrade. Such an event stays
until the channel next changes, for example on an edit or a membership
change. A client MUST treat a missing tag as unknown. A missing `P` does not
mean that a key did not create the channel.

## Channel labels

A label groups channels across channel types, for example `workspace` or
`agent-config`. Labels have no meaning to the relay.

**Set labels.** A kind:9007 (create group) MAY carry up to 8 distinct
labels as `["t", <label>]` tags. On kind:9002 (edit metadata):

- `t` tags replace the whole label set;
- `["t", ""]` tags and no other `t` tag clear the set;
- no `t` tag leaves the labels as they are.

Only an owner or admin of the channel MAY change its labels.

**Rules.** A label is 1–64 characters from `a-z`, `0-9`, `.`, `:` and `-`. A
label MUST NOT be `stream`, `forum`, `dm`, `workflow` or `system`. These are
channel type names, so this rule stops a label from making a channel look like
another type. The relay MUST reject the whole event if it has more than 8
distinct labels, an invalid label, a reserved name, or `["t", ""]` together
with other labels. The relay stores repeated labels once, so a repeat does
not count toward the 8, and repeated `["t", ""]` tags clear the set like one.

## Finding channels

Use ordinary NIP-01 filters on the channel-state kinds.

| To find | Filter |
| --- | --- |
| Channels that a key created | `{"kinds": [39000], "#P": ["<creator>"]}` |
| Channels with a label | `{"kinds": [39000], "#t": ["<label>"]}` |
| One creator's channels with a label | `{"kinds": [39000], "#P": ["<creator>"], "#t": ["<label>"]}` |
| Channels of one type | `{"kinds": [39000], "#t": ["<type>"]}` |

A `#P` filter does not return a channel whose events lack `P` (see
**Older events**). So it can miss channels that a key created before the relay
supported this NIP.

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

The oldest channel has the lowest `created_at` tag value, compared as an
integer. If two channels have the same value, the lowest channel ID (`d` tag,
compared as a string) wins. A channel without a `created_at` tag ranks after
every channel that has one, and two channels without one compare by channel
ID.
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
change.

A client sees a channel only if it knows the channel's type. A read that does
not name a channel returns only the **legacy types**: `stream`, `forum`, `dm`
and `workflow`, the types that every client knew before this NIP. This list is
frozen. The relay leaves out every other type, so a channel type added later
is also hidden from clients that do not know it. A filter that names such a
channel (see below) reads it under the ordinary rules, and no other filter in
the same request gains it. The opt-in in "Asking for system channels"
(`#t:["system"]` or `#P`) adds system channels only, never a channel of
another type. Each new type needs its own way to ask for all its channels.
This applies to:

- a filter without `#h`, for example the channels or messages a reader can see;
- kind:39002 `#p:["<reader>"]`, the channels a reader belongs to;
- COUNT and NIP-50 search; and
- any relay-specific query API that does the same reads.

A filter **names** a channel when it has the channel ID in `#h`, or in `#d` on
a filter whose kinds are all 39000–39003. This is the same for a system
channel and for a type added later. Then the ordinary read rules apply: the
reader can read the channel if they are a member or if it is open.
A client that has the ID, for example from a link, can open the channel. Thus
a system channel stays an ordinary NIP-29 group.

The relay decides this for each filter. A filter that names a channel does
not add it to another filter in the same request.

Push notifications (NIP-PL) use the same rule. A lease filter wakes its
author for an event in a system channel or a channel of a type added later
only when the filter names that channel. A lease filter cannot have `#t` or
`#P`, so here "names" means `#h`. The relay still checks that the author can
read the event, as for every other channel.

### Membership notices carry the channel type

Every kind:44100 (added) and kind:44101 (removed) notice has one more tag,
`["t", <channel_type>]`, for example `["t", "stream"]` or `["t", "system"]`.
The relay sets it from the channel type when it sends the notice. A channel's
type does not change. The relay stores the notice like any other 44100/44101.

A client sees a notice only if it knows the notice's channel type. Old
clients do not know types added later, such as `system`, and would show the
channel from such a notice. So the relay MUST apply this rule:

- **A filter with `#t`** matches a notice only when the notice's type is one
  of the `#t` values.
- **A filter without `#t`** matches a notice only when its type is one of the
  **legacy types**: `stream`, `forum`, `dm`, `workflow`. This list is fixed.
  A type added later is never on it, so old clients never see its notices.
- **A notice without a `t` tag** was stored before this rule. Every such
  notice is about a legacy type, so the relay treats it as all of them: it
  matches a filter without `#t`, and a filter whose `#t` has a legacy type.
  On these two kinds, this replaces the plain NIP-01 `#t` match.

The rule applies on every path: REQ history, live delivery, COUNT, and any
relay-specific query API. Existing clients do not send `#t` on these kinds,
so they see no change.

A client that knows every current type sends one filter:

```json
{"kinds": [44100, 44101], "#p": ["<me>"], "#t": ["stream", "forum", "dm", "workflow", "system"]}
```

When Buzz adds a type, a client adds its value to `#t` when it supports that
type.

The `#p` rule for 44100/44101 still applies: it must be the reader's own
pubkey. An `ids` filter does not lift it: the relay returns, counts or
delivers a 44100/44101 only to the pubkey in its `p` tag, even to a reader
who knows the event id.

Because the notices are stored, a client that reconnects can set `since` to
the newest notice it has seen. One subscription then returns the notices it
missed and the new ones, so no change falls between a read and a
subscription. A notice is a hint: the channel's group-state events decide
membership.

### Asking for system channels

A filter whose kinds are all 39000–39003 also returns the system channels that
the reader can read when it has:

- `"#t"` with the value `system`; or
- any `"#P"`. A query for the channels of named creators returns a small
  result, so it does not fill generic channel lists.

The filter then matches as usual. A label alone does not ask for system
channels. These filters add only system channels. A channel of a type added
later reaches only a filter that names it.

| To find | Filter |
| --- | --- |
| My system channels | `{"kinds": [39002], "#p": ["<me>"], "#t": ["system"]}`, then their kind:39000 by `#d` |
| Open system channels | `{"kinds": [39000], "#t": ["system"]}` |
| One creator's channel with a label, system or not | `{"kinds": [39000], "#P": ["<creator>"], "#t": ["<label>"]}` |

`"#t": ["system", "<label>"]` means `system` **or** the label. It does not
mean "a system channel with this label". Use `#P` and the label instead.

Private system channels stay hidden from non-members, as for every private
channel.

## Advertising support

Until this draft has an upstream integer NIP number, relays MUST NOT advertise
it in NIP-11 `supported_nips`. A relay that supports this NIP adds `"nip-cl"`
to its NIP-11 `supported_extensions` string array.

A relay without this NIP drops label tags on create and refuses
`channel_type=system`. So a client SHOULD NOT create a channel with labels or
a system channel on a relay whose NIP-11 document does not contain `"nip-cl"`.
Lookups by label on such a relay find nothing, which is safe.

## Trust

A client that relies on these tags MUST check that the relay's key signed the
event, as for all NIP-29 group-state events.

Labels give no trust. Any owner or admin can set them. Only `P` says who
created a channel.

## Relation to other NIPs

- NIP-29: the group-state events that this NIP adds tags to.
- NIP-01: the filters that find channels.
- NIP-22, NIP-34 and NIP-72: the convention for uppercase `P`.
