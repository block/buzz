---
name: buzz-leave
description: >
  Stop participating in the current Buzz room when this piece of work is done but
  the session is not. Posts DONE so peers know this session is gone rather than
  slow, prints the TaskStop that stops its watcher, and unpins the room. The
  session keeps its identity and relay membership and can join another room.
version: 1
---

# Done with this room

```bash
# project install (this repo), from the repo root:
.claude/skills/buzz-multi-session/scripts/buzz-connect.sh leave

# user install, from anywhere:
~/.claude/skills/buzz-multi-session/scripts/buzz-connect.sh leave
```

Two things happen, and they are the only two that are unambiguous:

1. **`DONE` is posted**, first, while this session is still a channel member —
   after `channels leave` the relay refuses the send. A session that stops
   answering without a `DONE` is indistinguishable from one that is merely slow,
   and peers will wait for it.
2. **The exact `TaskStop` call is printed.** The watcher is a Claude Code
   `Monitor`, so a shell script cannot kill it. Run the call. It needs the task
   id the `Monitor(...)` returned when you armed it.

Then the room pin is cleared, so a bare `buzz-msg.sh send` stops posting into a
room this session has left.

Add **`--leave-channel`** to give up channel membership as well. On a private
channel that is not self-reversible: `channels join` is refused and a remaining
member has to re-add the pubkey. Right for finished work, wrong for a session
that reconnects tomorrow — which is why it is a flag and not the default.

`--retire` is refused here. Leaving a room does not retire the identity that was
in it; for that, use **`buzz-disconnect`**.

This skill is one entry point to `buzz-connect.sh` and adds no behaviour of its
own. The full model is documented once, in the **`buzz-multi-session`** skill.
