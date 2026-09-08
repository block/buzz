# Channel-backed task experiment

This is a living record of experiments based on [Buzz Channels — One Conversation, Many Views](https://github.com/block/buzz/blob/2b1ccbab542c42f227f9714e13eb7e1377bcbe16/VISION_CHANNELS.md#L1). We are testing whether one Buzz channel can remain the shared home for a conversation as task, branch, and pull-request views appear.

## Currently testing

```text
Conversation → local task + new channel → implementation branch → pull request
```

- The task's mutable data lives in the channel canvas.
- No native Buzz task is created.
- The task has zero or one implementation branch.
- The branch, pull request, CI, and review use the task's channel.

## Canvas data model

Use this template:

```yaml
---
buzz_schema: channel-backed-task/v1
task:
  title: "<task title>"
  description: "<task description>"
parent_channel: "buzz://channel/<channel UUID>"
originating_thread: "buzz://message?channel=<channel UUID>&id=<message event ID>"
branch: null
---
```

## Agent decision flow

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
   - **No:** Create one and add its details to the canvas.
4. Start work.

## Future experiments

- Start with a bare channel, then add a task or branch view.
- Start with a branch channel, then add a task view.
- Represent work requiring multiple implementation branches as a parent task with branch-backed subtasks.
- Turn this decision flow into an API so agents declare their intent and Buzz creates or reuses the correct channel and bindings.

We will update this document with what each experiment teaches us and which behavior requires native client or relay support.
