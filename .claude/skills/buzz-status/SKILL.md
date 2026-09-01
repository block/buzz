---
name: buzz-status
description: >
  Check whether this Claude Code session is actually connected to its Buzz room
  and whether its watcher is alive — "connected but deaf" looks identical to a
  quiet channel. With --all, lists every Buzz identity on this machine, which are
  still relay members, and which have no live watcher. Reports only; changes
  nothing.
version: 1
---

# Am I connected, and is anyone listening?

```bash
# project install (this repo), from the repo root:
.claude/skills/buzz-multi-session/scripts/buzz-connect.sh status
.claude/skills/buzz-multi-session/scripts/buzz-connect.sh status --all

# user install, from anywhere:
~/.claude/skills/buzz-multi-session/scripts/buzz-connect.sh status [--all]
```

Exit codes are the answer, so this is checkable rather than something you have to
read: **1** the watcher is not armed, **4** this session is not a channel member,
**2** it is in no room at all, **0** everything is live.

**`status` reports; it never acts.** It will not create a channel and it will not
re-admit a session that has just left one — a status call that silently undid a
`leave` would make `leave` look broken.

`--all` is the roster: every identity in `~/.buzz/sessions`, whether the relay
still counts it as a member, and whether anything is listening for it. It costs
one relay call per identity, which is why it is on request. Three states are
worth acting on: `unbound` (no Claude Code session ever adopted it, yet it is
still a relay member holding a key that can authorise a channel admit), `none
pinned; still watching` (a watcher whose session moved on — `TaskStop` it), and
`member/archived` (retired, and still able to write).

**It prunes nothing.** An identity with no watcher is usually a session between
runs, and nothing here can tell the difference.

Two separate things can be broken, and the exit code says which: **1** the
Monitor is not armed, so messages are queueing and nothing is waking this
session; **6** the receiver itself is down or wedged, so messages are not being
fetched at all. This verb restarts a dead receiver and reprints the `Monitor(...)`
to arm — re-arming replays everything queued since it died.

This skill is one entry point to `buzz-connect.sh` and adds no behaviour of its
own. The full model is documented once, in the **`buzz-multi-session`** skill.
