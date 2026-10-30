NIP-UG
======

User Groups
-----------

`draft` `optional` `relay`

This NIP defines `kind:30182`, an addressable event for a named, mentionable set of community members, like a Slack user group (`@ios-team`). It also defines how a message mentions a group.

A group is community-wide. One handle names one group in one community.

## Event

`kind:30182` is an addressable event as defined in [NIP-01](01.md), with the rules below that differ from [NIP-33](33.md).

```jsonc
{
  "kind": 30182,
  "content": "Builds and ships the iOS app",
  "tags": [
    ["d", "ios-team"],
    ["name", "iOS Team"],
    ["creator", "<hex pubkey of the person who created the group>"],
    ["admin", "<hex pubkey>"],
    ["p", "<hex pubkey of a member>"],
    ["p", "<hex pubkey of a member>"],
    ["policy", "open"],
    ["status", "disabled"]
  ]
}
```

### Tags

| Tag | Count | Value |
|-----|-------|-------|
| `d` | exactly 1 | The handle. Normalized. |
| `name` | exactly 1 | Display name. 1 to 128 bytes. No control characters. No leading or trailing space. |
| `creator` | exactly 1 | Hex pubkey of the person who created the group. |
| `admin` | 0 to 20 | Hex pubkey of a group admin. Distinct. |
| `p` | 0 to 500 | Hex pubkey of a member. Distinct. No fourth element. |
| `policy` | 0 or 1 | `admins` (default) or `open`. |
| `status` | 0 or 1 | `active` (default) or `disabled`. |

A group MUST NOT have an `h` tag. Groups are never scoped to a channel.

A handle is 1 to 64 bytes of ASCII letters, digits, `.`, `-`, and `_`. It starts with a letter or digit. The `d` tag holds the lowercase form. The handles `channel`, `here`, `everyone`, and `all` are reserved.

`content` is the description. It MUST NOT be longer than 1024 bytes.

A disabled group keeps its handle and its members. It notifies no one. This is the same as a disabled Slack user group.

## Who may write

The relay tells each signer one of these roles: community owner, community admin, or member.

- **Create.** Any community member may create a group. The `creator` tag MUST be the signer.
- **Edit.** The `creator` tag MUST NOT change. The signer MUST be one of:
  - the creator;
  - a pubkey in the current `admin` tags;
  - a community owner or admin;
  - any community member, when the current `policy` is `open`.
- **Open policy limit.** A signer who is allowed only by the `open` policy MUST NOT change the `admin` tags or the `policy` tag.

## Handle uniqueness and replace semantics

The live head of a handle is community-wide. It is not per author.

When the relay receives a valid event for a handle that has a live head by another author, it checks the rules above. If the signer may edit, the relay stores the event and removes the other author's head. A signer who may not edit gets `invalid:` and nothing is stored.

An edit by another author MUST have a `created_at` later than the current head. Edits by the same author follow the standard NIP-33 rule.

Heads by two authors can exist for a short time when two edits race. Readers MUST pick the head with the newest `created_at`, and the lowest event ID on a tie. This is the same rule as the custom emoji kind.

To delete a group, its current author sends a [NIP-09](09.md) deletion with an `a` tag of `30182:<author>:<handle>`. A signer who is not the author of the head uses `status` `disabled` instead.

## Reading

| Need | Filter |
|------|--------|
| One group | `{ "kinds": [30182], "#d": ["ios-team"] }` |
| All groups | `{ "kinds": [30182], "limit": 500 }` |
| Groups that a person is in | `{ "kinds": [30182], "#p": ["<pubkey>"] }` |

A client resolves duplicates as described above.

## Mentioning a group

A message mentions a group by adding one `p` tag per member. Each tag has the group marker as its fourth element:

```jsonc
["p", "<member pubkey>", "", "group:ios-team"]
```

This is the only change a message needs. Every client and agent already notifies on `p` tags, so the relay indexes them, leases match them, and agents wake for them without new code.

Rules for the sender:

- The sender MUST put the current members in the tags. A sender MAY leave out members it does not want to notify.
- A message MAY mention at most 5 groups.
- Group-marked tags do not count against per-message mention caps of clients, such as the 50 `p` tags of the SDK. A group has at most 500 members.
- A sender SHOULD also write the text `@ios-team` in the content, for display.

Rules for the relay:

- The relay MUST reject a message with `invalid:` when a group-marked `p` tag names a pubkey that is not a current member of an active group with that handle.
- The relay MUST reject a message that mentions more than 5 groups.

Rules for the readers that gate on the number of `p` tags (the push hellthread gate, [NIP-PL](NIP-PL.md)):

- A reader MUST count all group-marked tags of one handle as one mention.

A group mention is a snapshot. Members added after the message was sent are not notified. This is how Slack behaves.

## Limits summary

| Item | Limit |
|------|-------|
| Handle | 1 to 64 bytes, `[A-Za-z0-9._-]`, lowercase in `d` |
| Name | 128 bytes |
| Description | 1024 bytes |
| Members | 500 |
| Admins | 20 |
| Groups per message | 5 |
| `h` tags | 0 |

## Not in this NIP

- Adding a group to a channel. A client expands the members and sends one NIP-29 put-user event for each. The relay keeps no link between the group and the channel.
- A check that each member is a community member. A pubkey that is not a member gets no notification because it cannot read the message.
- Deletion of a head by a signer who is not its author.
