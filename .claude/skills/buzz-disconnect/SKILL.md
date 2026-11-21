---
name: buzz-disconnect
description: >
  End this Claude Code session's participation in Buzz entirely, when the work is
  finished. Posts DONE, prints the TaskStop that stops the watcher, unpins the
  room, and reports what is left behind. Optionally gives up channel membership
  (--leave-channel) or retires the identity (--retire). Run this before closing a
  terminal, or the watcher and the identity outlive the session.
version: 1
---

# This session is finished

```bash
# project install (this repo), from the repo root:
.claude/skills/buzz-multi-session/scripts/buzz-connect.sh disconnect

# user install, from anywhere:
~/.claude/skills/buzz-multi-session/scripts/buzz-connect.sh disconnect
```

By default it does the three unambiguous things and nothing else: posts `DONE`
while still a channel member, stops the receiver — an ordinary process, so this
one really is stopped — and prints the exact **`TaskStop`** for the watcher — a Claude Code `Monitor` that a shell script cannot kill, and that
otherwise keeps a relay connection open to a channel where nothing will happen
again. Then it clears
the room pin and prints what remains.

Two opt-ins, because each is right in one case and wrong in the other:

- **`--leave-channel`** — `buzz channels leave`. Right for finished work, wrong
  for a session that reconnects tomorrow and would have to be re-admitted. Not
  self-reversible on a private channel. A session that opened its own room owns
  it and cannot leave at all; the output names the two real options.
- **`--retire`** — archives this identity (NIP-IA kind:9035). Right for a
  throwaway worktree, wrong for anything resumable. **Never implicit.** It only
  ever targets this session's own pubkey. Read what it prints before assuming
  what it does: archival is a signal to readers, not a lock — the key can still
  read, write and connect, and `agents unarchive` restores the relay's state but
  not the record.

**Relay membership survives all of it.** `relay_members` has no expiry and no
self-service exit; nothing this session can run removes its row. The output says
so rather than implying the session has been erased.

This skill is one entry point to `buzz-connect.sh` and adds no behaviour of its
own. The full model is documented once, in the **`buzz-multi-session`** skill.
