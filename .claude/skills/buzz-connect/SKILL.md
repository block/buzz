---
name: buzz-connect
description: >
  Connect this Claude Code session to the shared Buzz coordination channel so
  parallel sessions can talk to each other instead of the human copying answers
  between terminals. Mints this session's identity from its /rename title,
  enrols it, joins the default channel, announces HELLO, and arms a watcher that
  wakes this session when a peer posts. Start here.
version: 1
---

# Connect this session

```bash
# project install (this repo), from the repo root:
.claude/skills/buzz-multi-session/scripts/buzz-connect.sh

# user install, from anywhere:
~/.claude/skills/buzz-multi-session/scripts/buzz-connect.sh
```

Idempotent — running it again is how you check state, not a risk. In one pass it
resolves this session's name, mints or adopts its identity, enrols on the relay,
publishes the display name, joins the coordination channel, announces `HELLO`,
and prints the exact `Monitor(...)` call to arm.

**Arm that Monitor immediately, and keep the task id it returns.** Until you do,
peers can see this session but it cannot see them, which looks exactly like an
agent ignoring them. The task id is the only handle on the watcher when it is
time to stop.

For a room of its own rather than the shared default, use the `buzz-join` skill.

This skill is one entry point to `buzz-connect.sh` and adds no behaviour of its
own. The whole model — identities that follow `/rename`, the two membership
gates, the `CLAIM`/`RELEASE` protocol, how the watcher is pushed rather than
polled — is
documented once, in the **`buzz-multi-session`** skill. Read that when something
is surprising.
