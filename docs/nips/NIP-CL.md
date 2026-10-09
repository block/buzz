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

## Channel identity tags

On every channel-state event that the relay signs, the relay MUST put these
tags in this order, before any other `t` or `P` tag:

1. `["t", <channel type>]`, for example `stream`, `forum`, `dm`, `workflow`.
2. `["P", <creator public key, hex>]`.

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
    ["P", "<creator public key>"],
    ["created_at", "1759939200"]
  ]
}
```

| Tag | Kinds | Meaning |
| --- | --- | --- |
| first `t` | 39000–39003 | Channel type. |
| `P` | 39000–39003 | Public key that signed the kind:9007 that created the channel. |
| `created_at` | 39000 | Time the channel was created, in unix seconds, as a decimal string. |

**Channel type.** Clients MUST read the channel type from the first `t` tag.
A relay can add other `t` tags after it.

**Creator.** The relay writes `P` from its own record of the kind:9007 that
created the channel. No client can set it. It does not change when ownership
moves to another member or when the channel is edited.

Uppercase `P` follows NIP-22, NIP-34 and NIP-72, where `P` names the author of
the root object. On these kinds, lowercase `p` already lists members, admins
and DM participants. A filter matches tag values, not their roles, so `#p`
would mix creators with members.

**Creation time.** The `created_at` tag does not change when the channel is
edited. Use it, not the event's `created_at`, to compare the age of channels.

## Finding channels

Use ordinary NIP-01 filters on the channel-state kinds.

| To find | Filter |
| --- | --- |
| Channels that a key created | `{"kinds": [39000], "#P": ["<creator>"]}` |
| Channels of one type | `{"kinds": [39000], "#t": ["<type>"]}` |

## Trust

A client that relies on these tags MUST check that the relay's key signed the
event, as for all NIP-29 group-state events.

## Relation to other NIPs

- NIP-29: the group-state events that this NIP adds tags to.
- NIP-01: the filters that find channels.
- NIP-22, NIP-34 and NIP-72: the convention for uppercase `P`.
