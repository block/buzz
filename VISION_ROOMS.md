# Rooms in Buzz

Buzz, at its core, is a place where people and agents exchange messages. A room
is a view into a collection of messages: a place to talk about something specific
with the right group.

Under the hood, Buzz uses tags on messages to group them into *rooms*. An `h` groups
messages at the top level and forms a *channel*. Channels have a permission system:
people can join and leave them, and channels can be private. An `e` tag specifies
a `root` for a message, forming a *thread*. Threads inherit their permission from
their channel.

Every work object in Buzz has exactly one canonical room: one clear place to talk
about it. This is an intentional simplification that avoids ambiguity. Any human
or agent knows where to read up or ask about an object in Buzz.

What are the objects in Buzz? We expect to expand them over time. For now we have:
- Projects: a collection of work that achieves a goal
- Tasks: a smaller piece of work with a tracked status
- Repos: a collection of code (a git repository)
- Branches: a specific set of changes in a repo (a git branch)
- Documents: an editable markdown file

## Objects, Channels, and Threads

An *object* has its own meaning and state: a task has status and assignees,
a repository has code and refs, and a document has content. The interface or
plugins in Buzz can provide ways to work with these objects: a task board, diff viewer,
or document editor, but the conversation always happens in the corresponding room.

| Object     | Room                                                              |
|------------|-------------------------------------------------------------------|
| Project    | One exclusive channel for the outcome it organizes                |
| Repository | One exclusive channel for ongoing code and maintenance discussion |
| Task       | One exclusive thread in its project's channel                     |
| Branch     | One home thread discussing the code                               |
| Document   | One exclusive thread commenting on the content                    |

Channels and threads don't need to have any object associated with them, they are
often just conversations. Projects and repositories always have their own channel;
the channel for a project can't be the channel for another project or repository.
Tasks and documents each have their own thread.

Objects typically have their own room. Branches are a pragmatic exception: completing
one task often involves multiple branches and resulting PRs. The goal of exclusive homes
for the other objects is to make organization and authorization as unambiguous as possible.

Every task, including a subtask, must belong to a project. Its thread must be
rooted in a message in that project's channel. When there is no natural originating
message, Buzz can create a hidden message to provide the thread root.

## Authorization

Channels hold authorization in Buzz. A repo's channel defines who can see the
code, including the code on any branches. A project's channel defines who can see
the project and its tasks: every task's thread lives in that project's channel.
A document's thread likewise inherits access from the channel containing it.

A branch belongs to a repository, but its conversation may live with the task it
implements. Access to code follows the repository's channel; access to the
conversation follows the channel containing its thread. Neither grants access to
the other. This lets a task keep one conversation across changes to several
repositories, without changing who can read those repositories.

## Lifecycles

Projects organize time-bound outcomes. Repositories are durable homes for code.
A project can span several repositories, and a repository can support several
projects, concurrently or over time. Grouping them does not merge their audiences
or lifecycles.

For example, **Simplify sign-in** spans an identity service and a mobile app. In
this example, the project audience is appropriate for both implementation
conversations:

```text
Project home channel: Simplify sign-in
  Task thread T: Improve account recovery
  Subtask thread S: Fix recovery tokens  [identity-service branch]
  Subtask thread M: Add recovery screen  [mobile-app branch]
  Document thread D: Recovery experience [editable markdown document]

Repository home channel: identity-service
  Branch listing: fix-recovery-tokens → thread S

Repository home channel: mobile-app
  Branch listing: recovery-screen → thread M
```

Task details and the identity-service diff both lead to thread S. The mobile
implementation uses thread M. Requirements, progress, and review stay together
for each piece of work. The parent task gives people a place to coordinate the
overall outcome. The repositories live on even after the project is finished.
