# Channel-backed task experiment

This is a living record of experiments based on [Buzz Channels — One Conversation, Many Views](https://github.com/block/buzz/blob/2b1ccbab542c42f227f9714e13eb7e1377bcbe16/VISION_CHANNELS.md#L1). We are testing whether one Buzz channel can remain the shared home for a conversation as task, branch, and pull-request views appear.

## Currently testing

```text
Conversation → local task + new channel → implementation branch → pull request
```

- The task's mutable data lives in the channel canvas.
- No native Buzz task is created.
- The task has zero or one implementation branch.
- Each implementation branch has one home channel. Creating a branch from an appropriate task channel binds it to that channel instead of creating another one.
- A branch created without an appropriate current channel gets a new channel. Rebinding an established branch is for correcting mistakes, not routine restructuring.
- The branch, pull request, CI, and review use the task's channel.

## Canvas data model

Use this template:

```markdown
---
buzz_schema: channel-backed-task/v1
task:
  title: "<task title>"
  description: "<task description>"
parent_channel: "buzz://channel/<channel UUID>"
originating_thread: "buzz://message?channel=<channel UUID>&id=<message event ID>"
branch: null
---

Experimental task: You must read and follow the [channel-backed task experiment](https://github.com/block/buzz/blob/jtennant/task-channel-experiment/docs/experiments/channel-backed-tasks.md) before working in this channel.
```

When implementation starts, replace `branch: null` with the branch's repository and name:

```yaml
branch:
  repository: "<repository URL>"
  name: "<branch name>"
```

## Agent decision flow

Whenever an agent creates a task channel, it replies where the creation was triggered with a link to the new channel:

```markdown
I've created the task in [#<channel name>](buzz://channel/<channel UUID>).
```

### When asked to create a task

1. Is the current channel scoped only to this feature?
   - **Yes:** Add the task details to the current channel canvas.
   - **No:** Create a channel and add the task details to its canvas.

### When asked to start work on a feature

1. Is the current channel scoped only to this feature?
   - **No:** Create a channel for the feature and continue below using the new channel.
   - **Yes:** Continue below using the current channel.
2. Does the current channel have task details?
   - **No:** Add them to the canvas.
3. Does the current channel have a branch?
   - **No:** Create one and add its repository and name to the canvas.
   - **Yes, but the work needs another branch:** Create another task channel for that branch and relate the tasks when useful.
4. Start work.

## Valid progressions

- A conversation becomes a task channel, then gains a branch when implementation starts.
- A feature needs one branch in Berd and another in Voice Conversation CLI. The two task channels can be siblings under the Berd Voice channel, the Voice Conversation CLI task can be a subtask of the Berd runtime task, or a new branchless umbrella task can become the parent of both.
- An implementation attempt fails and a new approach starts, so the agent creates another task channel and branch. An umbrella task can be added later to encompass both attempts.
- An umbrella task remains branchless and coordinates branch-backed subtasks.

The same work can have multiple valid representations: sibling tasks, a parent and subtask, or an umbrella task with children. Task relationships should reflect how people understand the work and do not need to be predicted when the first task is created.

## Future experiments

- Automatically resolve or create a home channel when a branch is created: reuse an appropriate task channel when one exists, otherwise create a new channel.
- Start with a bare channel, then add a task or branch view.
- Start with a branch channel, then add a task view.
- Render a client-side "Task created" transition in an originating conversation when a task canvas references it through `originating_thread`.
- Have the task-creation API record the originating conversation as a native relationship so clients can render the transition without requiring a stored message.
- Turn this decision flow into an API so agents declare their intent and Buzz creates or reuses the correct channel and bindings.

We will update this document with what each experiment teaches us and which behavior requires native client or relay support.
