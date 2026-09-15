NIP-RB
======

Repositories and Branches
-------------------------

`draft` `optional` `relay`

**Depends on:** NIP-CS (conversation subjects), NIP-34 (Git announcements).

## Kinds

| Kind | Name | Location | Claim |
|------|------|----------|-------|
| `45011` | Repository | channel | exclusive |
| `45014` | Branch | thread | attachment |

These are regular stored events. NIP-CS defines identity, revision chaining,
controller authority, and common tags. A repository subject maps a Buzz identity
to a NIP-34 repository; it does not create another Git repository. A branch
subject identifies a collaboration lifecycle, not a ref-state snapshot.

## Repository event

```jsonc
{
  "kind": 45011,
  "tags": [
    ["cs", "1"],
    ["d", "<repository-uuid>"],
    ["h", "<repository-channel-uuid>"],
    ["location", "channel"],
    ["claim", "exclusive"],
    ["a", "30617:<owner-public-key>:checkout"]
  ],
  "content": "{\"title\":\"Checkout\",\"archived\":false}"
}
```

Standard signed-event fields are omitted. Updates include `prev`.

| Additional tag | Cardinality | Meaning |
|----------------|-------------|---------|
| `a` | 1 | NIP-34 repository coordinate; exactly two tag elements |

The coordinate MUST start with `30617:` followed by a 64-character lowercase
hex public key, a colon, and the repository identifier. Split on the first two
colons only; the remaining identifier is literal. The relay MUST resolve the
coordinate to an existing repository announcement in the same community.
The coordinate is immutable and unique among repository subjects in that
community, including archived subjects.

| Content field | JSON type | Constraints |
|---------------|-----------|-------------|
| `title` | string | Required; non-whitespace; at most 512 UTF-8 bytes |
| `archived` | boolean | Required |

Duplicate and unknown fields MUST be rejected. Archiving prevents new branch
subjects but does not delete code, hide discussions, or release the home claim.
Clone URLs, commit tips, and ref snapshots remain NIP-34 state, not independently
writable fields on this subject.

## Branch event

```jsonc
{
  "kind": 45014,
  "tags": [
    ["cs", "1"],
    ["d", "<branch-uuid>"],
    ["h", "<discussion-channel-uuid>"],
    ["location", "thread"],
    ["root", "<discussion-anchor-event-id>"],
    ["claim", "attachment"],
    ["repository", "<repository-uuid>"],
    ["ref", "refs/heads/fix-timeout"]
  ],
  "content": "{\"retired\":false}"
}
```

| Additional tag | Cardinality | Meaning |
|----------------|-------------|---------|
| `repository` | 1 | Buzz repository UUID using NIP-CS encoding |
| `ref` | 1 | Full Git branch ref |

Tags have exactly two elements. Both values are immutable. The repository MUST
resolve to kind 45011 in the same community and MUST NOT be archived on branch
creation. The branch resolves its NIP-34 target through that repository; it MUST
NOT repeat an independently authoritative coordinate.

`ref` MUST start with `refs/heads/` and satisfy Git ref-name rules: no empty or
dot-prefixed path component, `.lock` component suffix, trailing dot, `..`, `@{`,
ASCII control/space, DEL, or any of `~ ^ : ? * [ \\`. A branch subject may precede
the first push; ref existence is not required. At most one non-retired subject
may occupy `(community, repository, ref)`.

Content contains exactly the required boolean `retired`. Retirement is terminal:
a retired branch cannot become active again. A new incarnation uses a new UUID
and may claim the released ref slot, but the retired subject retains its
conversation association. Ref deletion alone does not retire a subject; rename
is retirement followed by creation, not a silent identity rewrite.

## Authorization and disclosure

Repository creation requires control of the NIP-34 announcement and channel
administration. Its Git access binding MUST match the subject's channel; a
conflicting binding MUST be rejected rather than silently overwritten. Later
Git metadata writes MUST NOT contradict an adopted binding.

Branch creation and updates require the NIP-CS controller rules, destination
channel write permission, and repository administration authority. This is
stronger than code read or ordinary push permission: placing a ref name in
another channel constitutes disclosure to that audience.

A branch's discussion channel MAY differ from its repository channel. Repository
readers do not thereby gain access to the discussion, and discussion readers do
not gain code access. Rendering a branch MUST NOT implicitly fetch or expose Git
content without a separate repository access check. Git-originated notifications
and agent output are subject to the same disclosure boundary.

## Queries and compatibility

Query repositories by kind 45011 and `h`; query branches by kind 45014 and
`repository`, optionally `ref`. Query discussion attachments by `h` and `root`.
Relationship filtering MUST precede pagination and use NIP-CS authorization and
revision-history semantics.

NIP-34 announcement 30617 and ref state 30618 retain their identities. A 30618
event may describe many refs and MUST NOT serve as a branch subject identity.
Pull requests, patches, and their statuses remain separate Git protocol objects.
Adopting a repository does not retrospectively restrict copies of previously
published metadata or reinterpret legacy comments as private channel replies.
