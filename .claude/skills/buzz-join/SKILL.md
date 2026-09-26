---
name: buzz-join
description: >
  Open or enter a named Buzz room for one piece of work, instead of the shared
  default channel — a refactor two worktrees are sharing, a migration with its
  own reviewer. Creates the channel if it does not exist, admits this session,
  and pins the room so messages go there. Use when the work has its own peers.
version: 1
---

# A room for one piece of work

```bash
# project install (this repo), from the repo root:
.claude/skills/buzz-multi-session/scripts/buzz-connect.sh join <name>

# user install, from anywhere:
~/.claude/skills/buzz-multi-session/scripts/buzz-connect.sh join <name>
```

Joins `<name>` if it exists and creates it if it does not, admits this session —
automatically, using the channel owner's key when that key is on this machine —
and **pins the room to this session**, so a bare `buzz-msg.sh send` afterwards
posts there and not to the machine's default channel. The pin is per session: one
worktree can sit in `pp-refactor` while another stays in `agent-coordination`.

`join` accepts a UUID as well as a name, which is how a session on a *different*
machine enters a private room — the UUID is the one piece of state that cannot be
derived.

**Open a room when the work is distinct and has its own peers.** Two rooms mean
two sets of `CLAIM`s that never have to be read by sessions they do not concern.
**A channel per session is not a dedicated channel, it is silence** — a room of
one has nobody to wake.

This skill is one entry point to `buzz-connect.sh` and adds no behaviour of its
own. The full model is documented once, in the **`buzz-multi-session`** skill.
