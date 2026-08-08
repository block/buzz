---
name: buzz-multi-session
description: >
  Coordinate several independent Claude Code sessions — typically parallel git
  worktrees of one repo — over a shared Buzz channel. Invoking the skill
  connects the session: it takes the session's own name, mints its identity,
  enrols, publishes its profile, joins the channel, and arms a Monitor so peers
  wake it on a new message instead of the human relaying between terminals.
  Also covers leaving a room and disconnecting a finished session.
version: 4
---

# Buzz Multi-Session Coordination

Three Claude Code sessions in three worktrees normally cannot talk. The human
becomes the message bus: copy an answer out of terminal A, paste it into
terminal B, notice ten minutes later that B and C edited the same file.

This skill removes the human from that loop. Each session gets **its own Buzz
identity, which is the session** — the name the user gave it with `/rename` is
the name that appears in the channel. All sessions join **one channel**, and
each arms a **Monitor** on it, so a peer's message becomes a notification you
wake on.

This is a Claude Code developer workflow, not something shipped to managed
agents — it depends on the `Monitor` tool. For the general relay CLI surface,
see the `buzz-cli` skill; this skill only documents what that one does not.

## The verbs

One script, five verbs, all run by you and never by the user:

| Verb | Skill | What it does |
|------|-------|--------------|
| `connect` | `buzz-connect` | the default. Identity, enrolment, profile, channel, `HELLO`, watcher |
| `join <name>` | `buzz-join` | a room for one piece of work — connect, but into that channel |
| `status [--all]` | `buzz-status` | am I connected, is the receiver alive, is the watcher armed, and with `--all`, every identity on this machine |
| `leave` | `buzz-leave` | stop participating in the current channel |
| `disconnect` | `buzz-disconnect` | stop participating entirely |
| — | `buzz-agent-provision` | an identity for a non-Claude-Code agent (`buzz-acp`) |

Every flag still works and nothing was renamed: `--status` **is** `status`, and
`--channel <name>` **is** `join <name>`. Verbs are an addition.

They live on `buzz-connect.sh` rather than in a dispatcher because all five need
the same first three steps — resolve this session's name, load its identity,
resolve the room it is in — and those steps *are* this script. A dispatcher would
either duplicate them or hand straight back here.

### Why there is a skill per verb

Verbs on one script are correct and undiscoverable. Claude Code's `/` menu lists
skill *names*, and there is no completion into a skill's arguments, so a user who
sees `buzz-multi-session` has no way to learn that `leave` exists. A session that
cannot be told to disconnect never disconnects, which is how watchers and
identities accumulate in the first place.

So each verb has a thin sibling skill whose **`description` is the whole
discoverability surface** — written for a human scanning a list. The siblings
carry no logic and no duplicated prose: each is a short `SKILL.md` naming the one
command and pointing here. This document and `scripts/` remain the only copies of
anything, which is what stops the family drifting.

They are directories with their own `SKILL.md` rather than symlinks to this one,
because a skill's identity *is* its frontmatter `name` and `description`. Six
symlinks to one file would be six skills with the same name and the same
description — precisely the problem being fixed. `sprout-cli` and
`desktop-screenshot` are single symlinked `SKILL.md` files with no scripts and no
siblings, so there was no existing pattern to follow here.

**This is not a menu of ways to solve a blocker.** A human picks a verb from the
slash menu; no agent deliberates over which one to try.

## Connect — one command, run by you, not by the user

```bash
# project install (this repo), from the repo root:
.claude/skills/buzz-multi-session/scripts/buzz-connect.sh

# user install, from anywhere:
~/.claude/skills/buzz-multi-session/scripts/buzz-connect.sh
```

Use whichever path this skill was loaded from — the scripts resolve their own
directory, so they work from any working directory once launched. A session
coordinating worktrees of some *other* repo needs the user install; the
project install only reaches sessions running inside this one.

That is the whole setup. It is idempotent, so run it again whenever you are
unsure of the state. In one pass it:

1. resolves this session's name,
2. mints or adopts its identity and loads it,
3. enrols on the relay if an invite code is configured,
4. publishes the display name so the session is findable in Buzz,
5. finds or creates the channel and gets this session into it — including
   admitting it with the owner's key when that key is on this machine,
6. announces `HELLO`,
7. prints the exact `Monitor(...)` call to arm — **arm it immediately**.

**Never ask the user to run setup commands, create identity files, or source an
env file.** Every script loads the identity itself from the session-derived
path. A human is asked for exactly two things, and only when nothing on the
machine can supply them: relay enrolment with no invite code available, and
channel membership for a channel no local key owns.

To work in a room of its own instead of the shared default, name it:

```bash
scripts/buzz-connect.sh join pp-refactor
```

That joins `pp-refactor` if it exists and creates it if it does not, admits this
session, and pins the room to this session — see [Dedicated
channels](#dedicated-channels-a-room-per-piece-of-work).

After connecting, post and catch up with:

```bash
scripts/buzz-msg.sh send "CLAIM crates/buzz-auth/**"
scripts/buzz-msg.sh send -            # long content on stdin: diffs, traces
scripts/buzz-msg.sh read 50           # what happened before you armed the watcher
scripts/buzz-connect.sh status        # am I connected? is the watcher alive?
```

`status` exits non-zero when this session cannot hear, so "connected but deaf"
is a checkable state rather than something you have to notice: 1 when the watcher
is not armed, 6 when the receiver itself is down, 4 when this session is not a
channel member, 2 when it is in no room at all. It restarts a dead receiver
itself — see [the resumption rule](#if-the-monitor-dies--the-resumption-rule).

**`status` reports; it never acts.** It will not create a channel and — the case
that matters — it will not re-admit a session that has just left one. A status
call that silently undid a `leave` would make `leave` look broken.

## An identity belongs to a session, and the name follows `/rename`

The point of this design: a message in the channel must be attributable to a
session the user can actually go and find. So the Buzz member **is** the
session, not the directory it happens to be sitting in.

`scripts/buzz-session-name.sh` resolves the name. Claude Code exports
`CLAUDE_CODE_SESSION_ID` and writes the transcript to
`~/.claude/projects/<cwd with / and . replaced by ->/<session-id>.jsonl`, where
the title set by `/rename` appears as `customTitle`. The first tier that
produces a usable name wins:

| | Source | Example |
|-|--------|---------|
| 1 | `customTitle` from this session's transcript (last occurrence — `/rename` can be re-run) | `Auth Refactor A` |
| 2 | the git worktree directory name | `wt-a` |
| 3 | `session-<first 8 of the session id>` | `session-8c5d0d2c` |
| 4 | `session-<first 8 of sha256(cwd)>` | `session-56af72ca` |

It never fails and never returns an empty name. A session with no `/rename`, no
session id and no git repo still gets a stable, per-directory identity.

**When the user runs `/rename`, the identity follows.** The identity file
records the session id, so a later run finds the existing keypair under its old
name, renames the file, and republishes the profile under the new display name.
The keypair is preserved — a `/rename` is not a new member.

Two forms of the name, from the same source:

- **slug** — lowercased, reduced to `[a-z0-9._-]`, collapsed, stripped of
  leading and trailing `-._`, capped at 64 characters. It is a filename
  (`~/.buzz/sessions/<slug>.env`), so `/` can only ever become `-` and a
  leading `..` is stripped: a title cannot escape the directory.
- **display** — the original characters, emoji included, with control and
  format characters (including bidi overrides) removed, whitespace collapsed,
  capped at 64 characters. This is what `buzz users set-profile --name` gets.

Pass an explicit name to override: `buzz-session.sh ensure "some-name"`. It is
sanitised the same way, and it disables the follow-a-rename behaviour.

### Secrets

Two files per identity, both mode 600, and the split is deliberate:

- `~/.buzz/sessions/<slug>.env` is **sourced**, so it holds only shell-safe
  values: two 64-char hex keys and a validated relay URL. It is checked against
  that shape before being sourced, and refused otherwise.
- `~/.buzz/sessions/<slug>.meta` is **never sourced**, only grepped. Everything
  derived from free text lives here. A `/rename` title is arbitrary user input;
  a title like `x$(rm -rf ~)` in a sourced file would execute.

**Never print, cat, grep, echo or otherwise surface `BUZZ_PRIVATE_KEY`** — not
into the transcript, not into a Buzz message, not into a log. `buzz-admin
generate-key` is piped straight into the 600-mode file for exactly this reason.
Only the **public** key is ever quotable.

## Setup the user does once (not per session)

`~/.buzz/config` is read by every session on the machine. `KEY=value`, parsed
rather than sourced, `chmod 600` — the scripts warn if it is readable by others,
because an invite code is a bearer token.

```
BUZZ_RELAY_URL=https://relay.example
BUZZ_INVITE_CODE=<code>              # sessions self-enrol with this
BUZZ_COORD_CHANNEL_NAME=agent-coordination
BUZZ_AUTO_ADMIT=0                    # opt out of admitting with a local owner key
```

Environment variables override the file.

### Anything a relay minted is cached per relay

A channel UUID and an invite code both belong to one relay and mean nothing on
another — but a UUID is structurally valid everywhere, so pointing
`BUZZ_RELAY_URL` at a different relay used to make every session resolve the old
relay's UUID and post into a channel that does not exist, with no error at all.
The cache keys therefore carry the relay, and these are written rather than set
by hand:

```
BUZZ_COORD_CHANNEL__<RELAY>          the default channel's UUID on that relay
BUZZ_CHANNEL_<NAME>__<RELAY>         a dedicated channel's UUID on that relay
BUZZ_INVITE_CODE__<RELAY>            the code that worked on that relay
```

`<RELAY>` is the host, uppercased, plus eight characters of its hash — so
`wss://` and `https://` on the same host are one relay, and two hosts sharing a
long prefix are not.

**Scoped rather than invalidated**, for three reasons: verifying a cached UUID on
every resolve would cost a relay round trip on the hot path, and `buzz-msg.sh`
resolves on every send; invalidating throws the old value away, so switching back
to the first relay would create a duplicate channel instead of finding the
original room; and keys that cannot collide beat detecting a collision after it
has happened.

The unscoped `BUZZ_COORD_CHANNEL`, `BUZZ_CHANNEL_<NAME>` and `BUZZ_INVITE_CODE`
are still read, so an existing config keeps working, and are adopted into the
scoped form on first use. Adoption is silent when `channels get` proves the
channel is on this relay and **announced when it cannot** — a private channel
this identity is not in is indistinguishable from one that is somewhere else, so
the ambiguity is stated rather than guessed at.

Three other things move with the relay:

- **The session's room pin** records its relay and is dropped, with a message,
  when that changes. A pin is per-session state, not a cache worth keeping.
- **A failed invite claim** says plainly when the code came from the unscoped key
  and may belong to a relay you have switched away from. An invite is minted by
  one relay and is meaningless to another; that is not a broken code.
- **The identity file's `BUZZ_RELAY_URL` is a record of where the key was minted,
  not configuration.** It no longer outranks `~/.buzz/config` — before, editing
  the relay in the config did nothing whatsoever for an existing identity, and
  every session silently kept talking to the relay it was born on. Precedence is
  environment, then config, then the mint record. When they differ the run says
  so: **the keypair carries over, relay membership does not**, so the identity
  needs enrolling again. The mint record is deliberately left alone, so switching
  back needs no repair.

**The intended setup is one invite code.** With
[#4479](https://github.com/block/buzz/pull/4479)'s `buzz invites claim`, a
session enrols itself: put the code in `~/.buzz/config` once and every session
on the machine gets onto the relay with no further human involvement. `buzz
invites` is not in the CLI yet — until it lands, `buzz-connect.sh` says so and
falls back to the single ask below.

**Without a code**, ask for the invite link and nothing else:

> In Buzz Desktop: **Invite to community → Copy link**. Paste it here.

Then run `buzz-connect.sh --invite "<what they pasted>"`. It takes the whole URL
or a bare code, saves it, and enrols. One ask, one paste, and every session on
the machine is solved from then on.

Ask that and stop. Do not present alternatives, do not weigh routes, and do not
offer `buzz-admin add-member`: it writes to the relay's Postgres directly, so it
is inert on any machine that is not the relay host, and it is the operator's
decision regardless. A menu of options is a worse answer than one instruction —
the user asked to be connected, not to choose an enrolment strategy.

## The two gates, and the three failures worth naming

**Relay membership and channel membership are separate gates.** A pubkey that
is a relay member still sees nothing in a private channel until the channel
owner adds it. Forgetting this looks exactly like an agent ignoring you: an
empty channel, no error.

`buzz-connect.sh` checks both and names the fix rather than surfacing a 403.
The three states, and what you will see:

| State | What is printed |
|-------|-----------------|
| Not a relay member | `BLOCKED: this session is not a member of the relay yet` + the exact sentence to say to the user, asking for the invite link |
| Relay member, not a channel member | `auto-admit:` lines naming the local key that granted it — or, if no local key owns the channel, `BLOCKED: relay membership is not channel membership` + the exact `buzz channels add-member` line for the owner |
| Connected, watcher not armed | `watcher : NOT ARMED` + the exact `Monitor(...)` to run; `status` exits 1 |

`buzz-msg.sh read` on an empty channel says the same thing rather than printing
nothing, because "nothing here" and "you cannot see it" are indistinguishable.

### Channel membership is granted automatically when the machine holds the owner's key

Every session on this machine mints its identity into `~/.buzz/sessions`, so the
key that created a channel is almost always sitting right there. Asking a human
to run `channels add-member` for the fourth session is asking them to relay a
decision they already made. So `buzz-connect.sh` does it:

1. the blocked session is not a member, so it cannot read the member list —
   a non-member gets `[]` and exit 0, and `channels get` returns `null`;
2. so each key in `~/.buzz/sessions` is asked in turn whether the relay reports
   *it* as this channel's owner;
3. the owner's key runs `channels add-member --role member` for the blocked
   session, and connecting continues.

**It is never silent.** Every use prints the identity name, the owner pubkey,
the file the key came from, and the exact command that was run under it.

**It is scoped.** Only keys already in `~/.buzz/sessions`, only the channel
being joined, only role `member`. Nothing is minted, no role is promoted, relay
membership is not touched. Those are literals in `join_channel`, not options.

**It is refusable.** `BUZZ_AUTO_ADMIT=0` skips the owner search entirely and
falls back to the single ask above.

**Relay membership deliberately does not work this way**, even though a local
owner/admin key could mint an invite. Channel membership is one room and one
scoped grant; relay membership is the whole community, and the artefact is a
bearer token that outlives the action and sits in a config file where anything
that can read it can join. The invite-link flow already reduces that to one
paste, and the human should stay the one who authorises it.

## Dedicated channels: a room per piece of work

`buzz-connect.sh join <name>` joins that channel or creates it (`stream`,
`private`), admits this session per the section above, and **pins the room to
this session**, so a bare `buzz-msg.sh send` afterwards posts there and not to
the machine's default channel. The pin lives in the session's `.meta`, so it is
per session: one worktree can be in `pp-refactor` while another stays in
`agent-coordination`. Point a session somewhere else with another `join`, or out
of every room with `leave`.

**Open one when the work is distinct and has its own peers** — a refactor two
worktrees are sharing, a migration with its own reviewer. Two rooms mean two
sets of `CLAIM`s that never have to be read by sessions they do not concern.

**Use the default for everything else.** A channel per session is not a
dedicated channel, it is silence: coordination only happens where peers
overlap, and a room of one has nobody to wake.

Each channel's UUID is cached in `~/.buzz/config` under its own key, one per
channel name **per relay** — see [Anything a relay minted is cached per
relay](#anything-a-relay-minted-is-cached-per-relay). The cache has to exist at
all because a private channel is invisible to a non-member: a second session
cannot find it by name and would otherwise create a duplicate with the same name
that nobody shares. And it has to be one key per name, because a single slot
means opening a second room overwrites the first, and the sessions still pointing
at the old UUID go quiet with no error at all.

Sessions on a *different* machine need the UUID copied across — the one piece of
state that cannot be derived. Pass it with `join <uuid>`.

## Receiving and waking are two different jobs

`buzz-connect.sh` prints the `Monitor(...)` call; arm it verbatim. Each new peer
message arrives as one notification line:
`[buzz] a1b2c3d4: CLAIM crates/buzz-auth/**`.

Behind that there are two processes, and the split between them is the most
important property of this design:

```
relay --wss--> buzz-stream.sh --appends--> ~/.buzz/stream/<id>.<chan>.log
               (the RECEIVER,                       ^
                outside Monitor)                    | tail -F from a stored offset
                                              buzz-watch.sh
                                              (the WAKE, under Monitor)
```

**Why they are split.** Monitor-hosted watchers have been observed dying with
exit 144 after running for hours, including on a channel their session had just
created and was alone in, while the identical command under plain `nohup` bash
stayed healthy. Nobody has a mechanism, and Monitor reaps a task's output before
anyone can read it, so three deaths produced no diagnosis. Rather than explain
it, the split removes the consequence: **a Monitor death now costs the wake, not
the messages.** They keep landing in the log, and re-arming replays every one of
them from the offset. Before the split a dead watcher meant those messages were
never fetched at all, and were simply gone.

**Read the watcher's exit code before theorising:**

| Exit | Meaning |
|------|---------|
| `143` | 128+15, SIGTERM. Someone stopped it — a `TaskStop`, a `kill`, a shell going away. Normal. |
| `144` | 128+16, SIGURG. It died on its own. **Treat this as a bug and report it**, with the receiver's `.err` file and the fact that reception continued. |

That distinction is worth the two lines: 143 was confirmed by deliberately
SIGTERMing a live Monitor watcher, which is what ruled out "the harness reaped
it" and "someone stopped it" as explanations for the 144s. Whatever produces 144
is a distinct mechanism, and it arrives without warning.

One hypothesis is already ruled out, so do not spend time on it either: bash
ignores SIGURG by default (verified on Darwin 25), so a bare SIGURG to the
watcher cannot produce 144 on its own. Both scripts carry `trap '' URG` anyway —
an ignored disposition is inherited across `exec`, so it costs nothing and covers
the CLI and `python3` too.

Either way the response is the same, which is the point of the split: run
`status` and re-arm. Nothing was lost.

### If the Monitor dies — the resumption rule

Nothing is lost, but nothing is delivered either until you act. `status` is the
check and it exits non-zero, so it is testable rather than something to notice:

| Exit | Meaning |
|------|---------|
| `0` | receiver live, watcher armed |
| `1` | watcher NOT ARMED — messages are queueing, nothing is waking you |
| `6` | the receiver itself is down or wedged — messages are **not being fetched** |
| `2` / `4` | not in a room / not a channel member |

```bash
scripts/buzz-connect.sh status
```

It restarts a dead receiver itself, reports how many messages are queued, and
reprints the exact `Monitor(...)` to arm. **Re-arm with that call verbatim** —
the offset is stored per identity and channel, so every message that arrived
while nothing was armed is delivered in order, once, and then live delivery
resumes. Never assume a quiet channel means you heard everything: if the watcher
died, the channel was never quiet.

`buzz-msg.sh send` and `read` both run that check first and warn before doing
the work. They warn rather than fail — a send refused because the *receive* path
is broken would be a second outage on top of the first.

### The receiver is pushed, not polled

`buzz messages subscribe` holds a NIP-42-authenticated WebSocket open and prints
one event per line the instant the relay pushes it. Measured against a local
relay, same messages, end to end from `send` to a notification line out of the
Monitor command:

| | push | poll (5s) |
|---|---|---|
| median | 44 ms | 2.0 s |
| worst observed | 90 ms | 4.5 s |

That is the difference between a peer answering and a peer appearing absent.

**HTTP reads have not gone away, and must not.** `messages get --since` runs
before every stream and again every time one ends. It is the safety net for what
push cannot see: a subscription the relay has quietly stopped matching against is
silent and still heartbeating, exactly like a quiet channel. `--reconnect-after`
ends a healthy stream every 5 minutes so that read gets a turn, and its `--since`
covers whatever the socket missed while it was down.

**A CLI with no `subscribe` verb falls back to polling**, automatically, with no
error. Latency is a nicety; hearing your peers is not. The receiver is then
byte-for-byte the loop this skill has always used.

Losing push is written to the log, once, and so is getting it back:

```
[buzz] relay stream is down, polling every 5s instead — <reason from the relay>
[buzz] relay stream restored — back to push delivery
```

Never let those go silent. A receiver that quietly degrades is worse than one
that never had push, because the session believes it is listening at full speed.

**Poll interval: 5 seconds.** Still the fallback interval, and still the sweep's
floor while push is down. That is the relay's rate-limit floor and it is what
makes the channel feel like a conversation. 20s was tried and reads as broken —
a session asks a question, waits, assumes nobody is there, and proceeds alone.
Do not raise it to be polite.

Tunable through the environment, all with working defaults:
`BUZZ_WATCH_RESUBSCRIBE` (300s), `BUZZ_WATCH_IDLE` (90s — must clear the relay's
30s heartbeat), `BUZZ_WATCH_UP_AFTER` (25s — must clear the CLI's 20s NIP-42
challenge timeout), `BUZZ_WATCH_WINDOW` (300s, first sweep only),
`BUZZ_STREAM_TICK` (15s heartbeat), `BUZZ_STREAM_STALE` (60s).

### Liveness is a heartbeat, not a pid

The receiver rewrites `<id>.<chan>.hb` with its pid and the time every 15
seconds. A receiver wedged on a socket is still a running process, so a pid check
alone would call it healthy; `status` reports `live`, `stale`, `dead` or `none`
and treats stale as broken. Its stderr goes to `<id>.<chan>.err` and is kept
across restarts — that file is the post-mortem Monitor's own output never was.

### Nine things to preserve if you rewrite this

1. **`--since` is inclusive.** A timestamp watermark alone re-emits the newest
   message on every poll, so the channel appears to repeat itself forever.
   Dedupe on **event id**; `--since` only bounds the query. The dedupe is also
   what lets push and HTTP feed the same filter without double-notifying.
2. **Prime the seen-set from existing history at startup**, or starting a
   receiver appends the entire backlog. A prime that *failed* is not a prime that
   found an empty channel: if the relay was unreachable at start, the first read
   that succeeds must be treated as backlog, or the whole room replays the moment
   the relay returns.
3. **Filter out your own pubkey.** Otherwise the session reacts to itself,
   replies, reacts to the reply, and you have built a loop that costs money. This
   is also why the log is per identity and not per channel: three worktree
   sessions sharing one log would each be woken by their own messages.
4. **Write a liveness marker**, keyed on the session id so a `/rename` does not
   orphan it. Without it, "watcher not armed" and "channel is quiet" look
   identical, and `status` could not tell you which one you are in.
5. **Advance the offset only after the line has been written out**, and keep the
   delivery loop in the watcher's own shell rather than a pipeline subshell. A
   subshell survives its parent: when the watcher was SIGKILLed during testing,
   the orphan went on reading and advancing the offset with nobody receiving,
   and re-arming then skipped messages that had never been delivered. Silent
   loss, caused by the code meant to prevent it. Observed, not theorised.
6. **Run the relay stream as a backgrounded job under `set -m`, and `wait` on
   it.** Bash defers a trap until a foreground command returns, so a foreground
   stream ignores TERM for as long as it lives and leaves an authenticated
   WebSocket behind. `wait` returns on a trapped signal at once, and `set -m`
   gives the job its own process group so the trap can take the CLI down with it.
   A blocked `read` builtin needs none of this — bash services traps during it.
7. **Never run the stream inside `$(...)`.** Command substitution captures
   stdout, and stdout is the notifications.
8. **One receiver per identity per channel**, enforced with an atomic `mkdir`
   lock whose pid is checked. Two receivers on one log double every message.
9. **Clean up the previous tail on the way in.** Nothing runs in a SIGKILLed
   process, so the next watcher to arm is the only thing that can do it.

It keeps only chat kinds (`9`, `1`); reactions and presence are noise here.

**Keep the task id the `Monitor(...)` call returns.** `leave` and `disconnect`
print the `TaskStop` that needs it. Both also stop the receiver, which is an
ordinary process and really is stopped rather than described.

## Leaving, and disconnecting

Nothing above tears anything down, so a session can only accumulate. It arms a
watcher, joins rooms, becomes a permanent relay member, and then the terminal
closes and every one of those outlives it. Two verbs end that:

```bash
scripts/buzz-connect.sh leave         # done with this room
scripts/buzz-connect.sh disconnect    # done, full stop
```

There are four separable things a departing session could do, and only the first
two are unambiguous. **Only the first two happen by default:**

1. **Stop the watcher.** It is a Claude Code `Monitor`, so a shell script cannot
   kill it. Both verbs print the exact call, the mirror of the `Monitor(...)`
   that connect prints:

   ```
   TaskStop(
     task_id: "<the id returned when you armed the Monitor for 'buzz coordination: pp-refactor'>"
   )
   ```

   Run it. If the id is lost the printed pid still works — the watcher clears its
   own marker on `TERM` — but `TaskStop` is what stops Claude Code tracking the
   task. Confirm with `status`, which then reports `NOT ARMED` and exits 1.
2. **Say goodbye.** A `DONE` is posted before anything else, while this session is
   still a channel member — after `channels leave` the relay refuses the send. A
   session that stops answering without a `DONE` is indistinguishable from one
   that is merely slow, and peers will wait for it.

Then the room pin is cleared, so a bare `buzz-msg.sh send` no longer posts into a
room this session has left.

The other two are opt-in, because each is right in one case and wrong in the
other:

| Flag | Does | Right when | Wrong when |
|------|------|------------|------------|
| `--leave-channel` | `buzz channels leave` | the piece of work is finished | the session reconnects tomorrow and would have to be re-admitted |
| `--retire` | archives this identity (NIP-IA kind:9035) | a throwaway worktree | anything resumable |

**`leave` and `disconnect` do the same two things by default.** The difference is
what they say and what they offer: `leave` says this session is done with this
room, `disconnect` says the session itself is finished, reports what is left
behind, and is the only verb that accepts `--retire`. Pretending to a deeper
difference would mean inventing a third teardown action that nothing needs.

### `--leave-channel`

On a private channel this is not self-reversible. `channels join` is refused with
`restricted: channel is private`, so some **remaining member** has to re-add the
pubkey — any member can, not only the owner. `buzz-connect.sh join <name>` does
it with no human involved when the owner's key is in `~/.buzz/sessions`, which is
why keeping channel membership is the cheap default and giving it up is a flag.

The relay evicts the departing session's live subscriptions, disables its
workflows in that channel, and posts `member_left`, so peers see the exit twice —
once as the `DONE` and once as a system message.

**A session that opened its own room owns it, and an owner cannot leave**:
`cannot remove the last owner`, because an ownerless private room can never admit
anyone again. That is the normal outcome of `join <name>` followed by
`disconnect --leave-channel`, not an edge case, so the script names it and offers
the two real options — hand ownership to a peer, or `channels delete` the room.

### `--retire`, and what archiving is not

`--retire` submits a NIP-IA archive request (kind:9035) for **this session's own
pubkey**. The relay's self path is `actor == target`, so a session can retire
itself with no owner or admin involved. It can never retire anything else: the
pubkey is this session's, not an argument.

What it does: one row in `archived_identities`, and a republished kind:13535
snapshot. Clients and peers can then see the identity is retired and stop
addressing it; Buzz Desktop gives it an "Archived" flair.

**What it does not do, and this is the part worth stating plainly: archival is a
signal, not a lock.** It does not stop the key reading, writing or connecting, it
does not hide anything already published, it does not touch relay membership, and
it does not remove the identity from any channel. A retired identity that posts
is still a posting identity.

`buzz agents unarchive <pubkey>` is a clean inverse **of the state** — the archive
is that one row and unarchiving deletes it, and nothing else was mutated, so
nothing else needs restoring. It is not a clean inverse **of the record**: the
9035 and 9036 requests are stored, publicly readable events; the row's reason and
timestamp are destroyed rather than rolled back; and re-archiving later keeps the
first reason and publishes no new delta. Reversible, not private, not free.

### Relay membership survives everything

`relay_members` has no TTL, no expiry and no last-seen column, so every session
name ever used is a permanent member until somebody deletes the row. **Nothing
this skill can run deletes it**, and `disconnect --retire` does not either:

- The relay does implement a self-service leave — NIP-43 kind:28936, which
  removes the sender's own row — but **no client builds that event.** It exists in
  the relay's ingest handler and in `buzz-core`'s kind table and nowhere else:
  not in `buzz`, not in `buzz-sdk`, not in Desktop, not in the web app. The
  capability is real and unreachable from here.
- The admin remove (kind:9031) explicitly refuses self-removal.
- `buzz-admin remove-member` writes to the relay's Postgres directly, so it does
  nothing unless the operator runs it on the relay host.

So retiring the identity is the strongest thing a session can do about itself, and
the honest thing to tell the user is that the membership stays. Every teardown
prints what remains rather than implying the session has been erased.

## Roster hygiene

```bash
scripts/buzz-connect.sh status --all
```

Because membership is permanent and every `/rename` mints or adopts an identity,
`~/.buzz/sessions` accumulates. `--all` lists every identity on the machine, asks
the relay whether it is still a member (one call each, which is why it is on
request), and says whether anything is listening for it:

```
  IDENTITY                       PUBKEY             RELAY            WATCHER        ROOM
  buzz-init                      0550845571d4322b   member           live pid 4137  agent-coordination
  hermes                         592b948b9ff4906a   member           unbound        -
  localowner                     9f33902767b7cbf6   not-a-member     unbound        -
  spec-kit-arch-governance-init  ce24afa247e2674c   member           live pid 49820 none pinned; still watching 6c61c7b4
```

Three states are worth acting on:

- **`unbound`** — no `.meta`, so no Claude Code session ever adopted it. It was
  minted by hand, or belongs to a session that never actually ran. It is still a
  relay member and its key can still authorise a channel admit.
- **`none pinned; still watching`** — a live watcher for an identity that is no
  longer in a room. That is a `Monitor` whose session moved on; it holds an
  authenticated WebSocket open against the relay and wakes nobody. `TaskStop` it.
- **`member/archived`** — retired, and still able to write. See above.

**It prunes nothing.** An identity with no watcher is usually a session between
runs, not a dead one, and the script cannot tell the difference. Retiring is
`disconnect --retire`, run from that session, for itself. Deleting a `.env`
destroys the keypair: it can never sign again and its name in old messages can
never be reclaimed.

## Provisioning an agent that is not a Claude Code session

`buzz-acp` runs goose, codex, `claude-agent-acp`, hermes and anything else that
speaks ACP. What each of them needs to reach a relay is identical, and it is
exactly what `buzz-connect.sh` already does for a session: a keypair, relay
membership, a published name, and channel membership. **buzz-acp does none of
it.** It never claims an invite, never publishes a profile and never joins a
channel — it assumes all four and, given none of them, boots to `no channel
subscriptions resolved — agent will sit idle`.

```bash
scripts/buzz-agent-provision.sh <name> [--channel <name>] [--command <harness>]
                                       [--owner <pubkey>] [--auth-tag <json>]
                                       [--force]
```

It prints the env block for a Dockerfile, a fly secret or a systemd unit. **The
private key is never printed** — only its path, and two ways to load it that do
not put it in a terminal, a log, a shell history or `ps`.

Three differences from a session identity, and they are why this is its own
command rather than a flag on `buzz-connect.sh`:

1. **The name is given, not resolved.** There is no `/rename` to follow, so the
   identity is deliberately **not** bound to any session id. `buzz-session.sh`
   used to record `CLAUDE_CODE_SESSION_ID` even for an explicit name, which meant
   a `/rename` in the terminal that provisioned an agent would rename the
   daemon's identity out from under it. It no longer does.
2. **No watcher.** The harness is its own event loop; a Monitor would be a second
   reader of the same channel.
3. **The output is configuration**, not a session that starts talking.

`--command` is not validated against a list, because buzz-acp does not have one:
it normalises the command to an identity (basename, lowercased, `.exe`/`.cmd`/
`.bat` dropped, space and `_` to `-`) and only looks up default arguments.
`goose` gets `acp`; `codex`, `codex-acp`, `claude-agent-acp`, `claude-code-acp`,
`claude-code`, `claudecode` and `buzz-agent` get none. **Everything else,
`hermes` included, gets no defaults — and the built-in default for
`BUZZ_ACP_AGENT_ARGS` is the literal string `acp`**, so an unrecognised harness
is launched as `<cmd> acp`. When the command is one buzz-acp does not know, the
env block sets `BUZZ_ACP_AGENT_ARGS=` explicitly and says why.

`BUZZ_ACP_CHANNELS` does not join anything either. It narrows channels the
harness has already discovered from its own membership events, so a UUID it is
not a member of is dropped without a word. `--channel` is what makes the agent
hear anything.

### Ownership, and the gap that has no bridge

A self-enrolled agent lands in `relay_members` as `role: member` with no owner.
That costs more than it sounds like:

- buzz-acp's `--respond-to` **defaults to `owner-only`**. An unowned agent under
  the default gate forwards nothing — it connects and ignores everyone.
- `buzz agents draft-create` / `draft-update` fail with exit 3, have no `--owner`
  flag, and end in a human's Buzz Desktop regardless.
- Agent turn metrics are rejected: the relay requires the `p` tag to be the
  agent's registered owner.
- **`buzz mem` is not what breaks.** Every `mem` subcommand takes `--owner <hex>`
  and the relay gates engrams on author-or-`p`, not on a registered owner.

An owner is a NIP-OA attestation — `["auth", <owner pubkey>, <conditions>,
<signature>]` — and only the owner's **secret key** can produce one. There is no
CLI command to mint it, no relay endpoint to request one, and no event kind that
registers ownership. The one shipped tool is
`cargo run --release --example compute_auth_tag -- <owner_secret_hex> <agent_pubkey> ""`.
So provisioning does the only honest thing: `--auth-tag` uses a real attestation,
`--owner` records the pubkey and says plainly that it is not the same thing, and
neither prints the full cost above rather than leaving it to be discovered.

**The finding that matters: enrolling makes ownership unrecordable.** The relay
writes `users.agent_owner_pubkey` only on the `ViaOwner` path — a key that is
*not* a direct member, admitted because its owner is one. A direct member's
membership check returns `Member` and short-circuits before the attestation is
looked at, on both the HTTP event submit and the NIP-42 WS AUTH. So an agent that
claims an invite can never have an owner recorded, and relay membership has no
self-service exit, so that cannot be undone — only replaced with a fresh key.

`--auth-tag` therefore **does not claim an invite**, and says so. The attested
agent reaches the relay through its owner, which needs the owner's pubkey to be a
relay member and the relay to run with `BUZZ_ALLOW_NIP_OA_AUTH`. If the key is
already a direct member, the output says the owner will never be recorded, what
still works (everything that reads the tag: buzz-acp owner resolution,
`--respond-to`, NIP-IA owner consent) and what stays refused.

### Why this is not a mirrored skill

The other skills in this repo are symlinked into `.agents/`, `.goose/` and
`.codex/`, and provisioning is runtime-agnostic, so mirroring looks right. It is
not, for two reasons.

**The mirror is not a doc mirror, it is a shipping channel.** All four symlinks
point at `desktop/src-tauri/src/managed_agents/<name>_skill.md`, which
`nest.rs:44` `include_str!`s into Buzz Desktop and installs for every managed
agent. Mirroring this would ship "mint a keypair, enrol it, publish a profile"
into agents that already have an identity Desktop minted and owns. That is a
capability increase aimed at the one audience that does not need it.

**And it would have to split `lib.sh`.** Four of the six steps here are the same
functions `buzz-connect.sh` calls — `ensure_relay_membership`, `publish_profile`,
`resolve_channel`, `join_channel` — and the `BUZZ_AUTH_TAG` isolation that makes
auto-admit safe for an attested agent lives in the shared `_as_identity`. A
separate skill would either duplicate that or depend on this skill's scripts, and
there is no precedent for either: both mirrored skills are a single
self-contained `SKILL.md` with no scripts at all.

So it stays here, `.claude/` only, alongside the identity code it is 90% made of.

## The coordination protocol

Message discipline is what keeps three agents from thrashing. Start every
message with one uppercase verb so peers (and the watcher's 400-char preview)
can triage without reading the whole thing.

### What is worth waking a peer for

Every wake is an inference in every listening session, so an ambient channel
scales badly: three sessions turn one `STATUS` into two model turns with
nothing to answer. `buzz-acp` defaults to mention-only for exactly this reason.

Mention-only is too strict here — a `CLAIM` is how a peer learns not to touch a
path, and nobody @mentions everyone. So the default (`BUZZ_WAKE=addressed`)
splits by audience, which the verbs already encode:

| | Wakes a peer |
|---|---|
| `ASK` / `ANSWER` naming them, or a `p` tag mention | yes — addressed to them |
| `CLAIM` `RELEASE` `BLOCKED` | yes — shared state they may collide with |
| `HELLO` `DONE` `STATUS` from someone else | no — logged, read on next catch-up |

**Nothing is dropped.** Every message still lands in the log, so
`buzz-msg.sh read` and the next catch-up show the whole conversation. The only
judgement is whether it was worth interrupting for.

`BUZZ_WAKE=all` restores wake-on-everything. `BUZZ_WAKE=mentions` is strictest —
`p` tags only, no verb awareness — and it will miss claims, so use it only where
peers are not editing shared paths.

This is why the verb matters more than it looks: it is not decoration, it is the
routing header. A message that opens with prose rather than a verb is
informational by default and will not wake anyone.

### Write for the human in the room, thread the evidence

**A person is reading this channel.** Left to itself an agent writes for its
peers, and a peer needs everything: absolute paths, line numbers, commit SHAs,
the whole chain of reasoning, because that is what lets it verify a claim
without asking. The result is correct and unreadable — a screen of dense text
where a conversation should be, and a human who stops reading it.

Split the two audiences:

```bash
buzz-msg.sh send "ANSWER Auth B: the challenge expires first — auth.rs:141." \
  --detail - <<'EOF'
The gate runs after the challenge check, so an expired challenge is rejected
before membership is consulted. Reproduced against a local relay with
BUZZ_REQUIRE_RELAY_MEMBERSHIP=true: ...
EOF
```

The channel gets one or two sentences a person can scan. The evidence goes in a
threaded reply — one click away, collapsed until someone wants it. Nothing is
discarded, and peers lose nothing: `messages thread` returns the whole exchange.

The test before sending: **would this read as a sentence someone says out
loud?** If it needs a code block, a table, or three clauses of qualification,
that part belongs in `--detail`.

Verb, one claim, evidence threaded. Not a report.

| Verb | Meaning | Example |
|------|---------|---------|
| `HELLO` | joining; who and where (sent for you by `buzz-connect.sh`) | `HELLO Auth Refactor A branch=feat/auth dir=wt-a` |
| `CLAIM` | taking exclusive ownership of a path glob | `CLAIM crates/buzz-auth/**` |
| `RELEASE` | done with a claim | `RELEASE crates/buzz-auth/**` |
| `STATUS` | progress, no reply needed | `STATUS tests green on auth` |
| `ASK` | question addressed to one peer | `ASK Auth Refactor B: did you rename Session?` |
| `ANSWER` | reply to an `ASK` | `ANSWER Auth Refactor A: yes, now AuthSession` |
| `BLOCKED` | stuck, needs someone | `BLOCKED waiting on RELEASE of migrations/**` |
| `DONE` | this session's work is finished | `DONE Auth Refactor A: pushed feat/auth` |

Address peers by their session name — that is the name the user gave them with
`/rename` and the name on their Buzz profile, so it points at a session the
user can find. Prefer plain `@name` text over `--mention`: an unresolved or
ambiguous name **stops the send before publishing**.

Rules that make it work:

1. `HELLO` on arrival (automatic), `DONE` on exit (automatic — `leave` and
   `disconnect` post it). A silent session is indistinguishable from a dead one,
   so a session that vanishes without a `DONE` leaves peers waiting on it.
2. **`CLAIM` before editing shared paths.** If a peer has an unreleased `CLAIM`
   overlapping yours, do not edit — `ASK` them or work elsewhere. Claims are
   advisory; nothing enforces them but the agents.
3. Answer every `ASK` addressed to you, even with "don't know". A session
   blocked on an unanswered question burns its whole budget waiting.
4. Never reply to your own message, and never `STATUS` on a timer — noise costs
   every peer a notification and a wake-up.
5. `buzz-msg.sh read 50` before your first edit, to catch claims made before you
   armed the watcher.

## Scripts

| Script | Role |
|--------|------|
| `buzz-connect.sh` | **the entry point.** `connect` / `join` / `status` / `leave` / `disconnect`, idempotently. |
| `buzz-msg.sh` | `send` / `read` on the coordination channel |
| `buzz-stream.sh` | **the receiver.** A daemon outside Monitor: holds the relay connection, filters, and appends notifications to `~/.buzz/stream/`. Started by `connect` and by the watcher |
| `buzz-watch.sh` | **the wake.** All Monitor runs: `tail -F` the receiver's log from a stored offset. `-` as the name resolves this session |
| `buzz-session.sh` | identity lifecycle — called by the others |
| `buzz-session-name.sh` | name resolution and sanitisation |
| `lib.sh` | shared helpers; sourced, never executed |

Prerequisites: `buzz` on `PATH` or a release build in the checkout
(`cargo build --release -p buzz-cli`; override with `BUZZ_BIN`), `buzz-admin`
for keypair minting (`BUZZ_ADMIN_BIN`), and `python3` — already a `Justfile`
dependency — for JSON handling.

Push delivery additionally needs a `buzz` that has `messages subscribe`. An
older binary — the one Buzz Desktop bundles, for instance — simply polls, and
says nothing about it because there is nothing wrong. Check with
`"$BUZZ" messages subscribe --help`.

## Gotchas

1. **One identity per session, never shared.** Two sessions on one key are
   indistinguishable in the channel, and each filters out the other's messages
   as "its own" — the two go permanently deaf to each other.
2. **`RUST_LOG` must not be `debug`/`trace` in a watcher shell** — tracing
   output on stdout becomes notification spam. The scripts pin `error`.
3. **Watcher output is notifications, one line each.** Never widen its filter to
   raw message dumps; Claude Code stops monitors that flood. `buzz messages
   subscribe` writes raw NDJSON, which is a transport and not output — it must
   always go through the filter, never straight to the Monitor.
4. **The relay URL's host:port must match the relay's configured community.**
   `no community is configured for this host` is that mismatch, not a network
   failure, and `buzz-connect.sh` says so.
5. **Secrets stay in the env file.** The relay only ever needs a public key, and
   a private key pasted into a channel is compromised for good.
6. **The room is sticky.** Once a session runs `join <name>`, a bare
   `buzz-connect.sh` or `buzz-msg.sh` keeps using that room. That is the point —
   but it means moving back is another `join`, not an omission. `leave` is what
   clears the pin.
7. **The owner search costs one relay call per local identity**, and only runs
   on the blocked path. `status --all` is how you find out what has accumulated
   in `~/.buzz/sessions`; every one of those keys could authorise an admit.
8. **`--retire` is never implicit.** No verb archives an identity without it,
   `leave` refuses the flag outright, and it only ever targets the running
   session's own pubkey — there is no argument that could point it elsewhere.
