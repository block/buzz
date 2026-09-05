# Buzz Channels — One Conversation, Many Views

## The Model

A **channel** is where people and agents discuss work. It has one stable UUID
and one shared conversation. Task details, documents, and code diffs are views
alongside that conversation, not separate chats.

Projects, tasks (including subtasks), repositories, branches, and living documents
each have one home channel. A channel can be home to several objects, or just
be a conversation with none. These objects do not impose mutually exclusive
channel types.

For example, a task and its implementation branch can share **the same channel
UUID**. Switching from task details to the diff changes the view, not the
conversation. Opening either object leads back to that channel; adding a view
does not create another chat.

Object state stays in its own representation: a task has status and assignees,
a repository has refs, and a document has content. The channel brings these views
and their conversation together.

## Projects and Repositories

Projects organize a time-bound outcome. Repositories are durable homes for code
and ongoing conversation. A project can span several repositories, and a
repository can support many projects over time or concurrently. Their channels have
different purposes and do not normally share a one-to-one relationship.

For example, a **Simplify sign-in** project spans the identity service and mobile
app repositories:

```text
Project channel: Simplify sign-in
├── Task channel: Improve account recovery
│   ├── Channel S: Fix recovery tokens     [subtask + identity-service branch]
│   └── Channel M: Add recovery screen     [subtask + mobile-app branch]
└── Design channel: Recovery experience   [document]

Repository channel: identity-service      [ongoing code and maintenance discussion]
└── Branch view: fix-recovery-tokens → channel S

Repository channel: mobile-app            [ongoing code and maintenance discussion]
└── Branch view: recovery-screen → channel M
```

The subtask and branch in S open the same channel; likewise for M. The repository
branch listings link to those channels rather than creating new conversations.
Completing the sign-in project does not end either repository's ongoing work.

## Nested Channels and Relationships

Channels nest to organize work: a project contains task channels, a task contains
subtask channels, and a document can have its own channel or be a view of an existing
channel. A canvas is the channel’s document view.

A channel has at most one containment parent. The navigation tree is not the entire
relationship model: a task's channel can sit under its project while its branch
still belongs to a repository elsewhere. Project-to-repository links and
branch-to-repository links remain available independently of that tree. Multiple
routes to a channel all lead to the same conversation.

## Access

A channel and its object views should feel like one workspace. Someone admitted to
that workspace should be able to read its conversation and the objects presented
as part of it, rather than encountering an arbitrary inaccessible subset. The
same principle applies when presenting nested channels as a shared project workspace.
Reading the work and permission to push, merge, or administer it are distinct
capabilities.

The permissions design must make that experience consistent across linked channels
and objects. For repositories, it must choose whether access is governed through
the canonical channel relationship or through repository permissions coordinated
with the channel. The model does not prescribe a separate, unrelated channel link
just for Git authorization. The choice of permission authority and inheritance
rules remains an explicit design decision; the required user experience is
coherent access, not disconnected permission islands.

## Backend Specification

Persist channel relationships on the channel:

- `parent_channel_id`: nullable channel UUID, in the same community, indexed for
  child lookup.
- `object_bindings`: a list of typed object references stored as JSONB. A channel
  can bind several objects, including a task and its branch.

Bindings reference the object they represent: an issue ID for a task, an
addressable project or repository coordinate, or a repository coordinate plus
full Git ref for a branch. Object records retain their own state; the bindings
connect them to their conversation home.

Expose these fields through signed channel create (`9007`), edit (`9002`), and
metadata discovery (`39000`) events. No feature-specific HTTP endpoint is needed.
Edits can set or clear the parent and replace the binding list. Omitted fields
remain unchanged; parent clearing and an empty binding list are explicit actions.
Persist a metadata edit atomically and return the same relationships on discovery.

The relay validates management authority, bounded binding shapes, and an existing
same-community parent. It rejects self-parenting and cycles, including concurrent
moves that would create a cycle.

All clients use the same relationships to resolve an object's canonical channel.
Creating another view reuses that channel. Repeated creation requests must converge
on the same channel rather than create parallel conversations. Conflicting bindings
must be reconciled consistently across clients, not resolved by local discovery
order. The metadata storage does not require a global unique-binding constraint.
