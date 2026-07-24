NIP-BP
======

Buzz Projects
-------------

`draft` `optional`

This NIP defines `kind:30621` project announcements. A project is an
owner-authored grouping above one or more NIP-34 repositories. It does not
change repository identity, Git transport, repository permissions, issues, or
pull requests.

## Kind and address

Kind `30621` is addressable under NIP-01. Its coordinate is:

```
30621:<project-owner-pubkey>:<project-id>
```

The plaintext `d` tag is the stable project identifier. It follows the same
identifier rules as Buzz repository IDs: `[a-zA-Z0-9._-]{1,64}`, with no
leading dot and no `..`.

## Event envelope

```jsonc
{
  "kind": 30621,
  "pubkey": "<project-owner-pubkey>",
  "tags": [
    ["d", "sprout"],
    ["name", "Sprout"],
    ["h", "<optional-project-channel-uuid>"],
    ["a", "30617:<repo-owner-pubkey>:frontend", "", "primary"],
    ["a", "30617:<repo-owner-pubkey>:backend"]
  ],
  "content": "Project description"
}
```

Writers MUST emit exactly one `d` tag and one non-empty `name` tag. The
description is plaintext `content` and MUST NOT exceed 1,024 bytes.

Each repository member is a full NIP-34 repository coordinate in an `a` tag.
Repositories MAY have different owners. Duplicate coordinates are invalid.
A non-empty project MUST mark exactly one repository with the `primary` marker
in position four. An empty project has no repository tags and no primary
repository.

The optional `h` tag associates a NIP-29 channel with the project. It is an
association for clients, not an authorization grant.

## Replacement and deletion

Publishing a later event with the same `(pubkey, kind:30621, d_tag)` replaces
the previous project definition under NIP-01 addressable-event semantics.
Adding, removing, reordering, or changing the primary repository therefore
requires one project update; repository announcements are unchanged.

Owners MAY delete a project with a NIP-09 deletion request that references the
project coordinate using an `a` tag. Deleting a project MUST NOT delete any
member repository.

## Authorization and security

Project membership is presentation metadata. It MUST NOT grant repository
read, push, administration, issue, or pull-request permissions. Those remain
controlled by each referenced NIP-34 repository and its relay policy.

Readers MUST NOT infer project ownership from a channel or from repository
authors. The signer of the kind `30621` event is the project owner and the
authoritative source of membership.

References to missing, malformed, or inaccessible repositories MUST be ignored
without making the whole project unreadable. Clients SHOULD visibly distinguish
unavailable members from valid members when enough metadata is available.

Project events are public plaintext and MUST NOT contain credentials, private
clone URLs with embedded tokens, or other secrets.

## Backward compatibility

Clients that do not understand kind `30621` continue to see standard NIP-34
repositories. Supporting clients MAY present an unreferenced kind `30617`
repository as an implicit single-repository project, preserving existing
repository links and data without migration.
