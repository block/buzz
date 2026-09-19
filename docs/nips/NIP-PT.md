NIP-PT
======

Projects and Tasks
------------------

`draft` `optional` `relay`

**Depends on:** NIP-CS (conversation subjects).

## Kinds

| Kind | Name | Location | Claim |  
|------|------|----------|-------|
| `45010` | Project | channel | exclusive |
| `45012` | Task | thread | primary |

These are regular stored events, not addressable replacements. Identity,
controller authority, revision chaining, and common tags follow NIP-CS.
A project groups tasks, not repositories. Neither type requires a Git reference.

## Project event

```jsonc
{
  "kind": 45010,
  "tags": [
    ["cs", "1"],
    ["d", "<project-uuid>"],
    ["h", "<channel-uuid>"],
    ["location", "channel"],
    ["claim", "exclusive"]
  ],
  "content": "{\"title\":\"Checkout reliability\",\"description\":\"Reduce failed checkouts\",\"archived\":false}"
}
```

Standard signed-event fields are omitted in examples. An update also supplies
`prev` as defined in NIP-CS.

| Content field | JSON type | Constraints |
|---------------|-----------|-------------|
| `title` | string | Required; non-whitespace; at most 512 UTF-8 bytes |
| `description` | string | Required; Markdown; at most 65536 UTF-8 bytes |
| `archived` | boolean | Required |

Content MUST be a JSON object without duplicate or unknown fields. Archiving
prevents creation of additional tasks but does not archive the channel, hide
existing tasks, revoke access, or release the channel claim. Unarchiving is an
ordinary authorized revision.

## Task event

```jsonc
{
  "kind": 45012,
  "tags": [
    ["cs", "1"],
    ["d", "<task-uuid>"],
    ["h", "<project-channel-uuid>"],
    ["location", "thread"],
    ["root", "<discussion-anchor-event-id>"],
    ["claim", "primary"],
    ["project", "<project-uuid>"]
  ],
  "content": "{\"title\":\"Handle payment timeout\",\"description\":\"Preserve the pending payment\",\"status\":\"open\"}"
}
```

| Additional tag | Cardinality | Meaning |
|----------------|-------------|---------|
| `project` | 1 | Project subject UUID |
| `parent` | 0 or 1 | Parent task subject UUID |

Both tags have exactly two elements and use NIP-CS UUID encoding. They are
immutable after creation. A subtask remains a task, with its own primary thread
claim. Parentage alone does not require nested thread ancestry or create a new
authorization boundary.

| Content field | JSON type | Constraints |
|---------------|-----------|-------------|
| `title` | string | Required; non-whitespace; at most 512 UTF-8 bytes |
| `description` | string | Required; Markdown; at most 65536 UTF-8 bytes |
| `status` | string | Required; `open`, `in_progress`, `done`, or `cancelled` |

Content MUST reject duplicate and unknown fields. Any listed status may
transition to any other listed status under ordinary update authority.
Completion or cancellation does not delete the discussion or retire attached
subjects. There is no implied transition from Git merge state to task status.

## Relationship validation

The relay MUST resolve `project` to an accepted kind-45010 subject in the same
community. Its home channel MUST equal the task's `h`. At creation, the project
MUST NOT be archived. `parent`, when present, MUST resolve to a different
kind-45012 subject in the same project. Cyclic parentage MUST be rejected.

A task's primary claim allows attachment subjects, but not another primary or
exclusive subject. Project and task creation require the NIP-CS claim authority;
updates require the controller and continued channel access. A task's controller
need not be the project's controller.

Example rejections include a missing project, a parent from another project,
a task rooted in another channel, a second task on an occupied primary thread,
and an update changing `project` or `parent`.

## Queries

Project listing filters kind 45010 by `h`. Task listing filters kind 45012 by
`project`; subtask listing additionally filters `parent`. These relationship
filters use the generic relay query extension and MUST run before LIMIT.
Results are signed revisions under NIP-CS history semantics, not a separate
project-maintained task list. A client materializes current task state by
subject ID and accepted revision chain.

## Compatibility

Kind 30621 retains its multi-repository grouping semantics. Kind 1621 retains
its NIP-34 issue semantics. Neither is a project or task under this NIP merely
because it carries a channel reference. Conversion creates an explicit new
subject and preserves old event identities and links.
