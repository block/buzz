---
name: buzz-multi-session
description: >
  Coordinate several independent Claude Code sessions — typically parallel git
  worktrees of one repo — over a shared Buzz channel. Invoking the skill
  connects the session: it takes the session's own name, mints its identity,
  enrols, publishes its profile, joins the channel, and arms a Monitor so peers
  wake it on a new message instead of the human relaying between terminals.
version: 3
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
scripts/buzz-connect.sh --channel pp-refactor
```

That joins `pp-refactor` if it exists and creates it if it does not, admits this
session, and pins the room to this session — see [Dedicated
channels](#dedicated-channels-a-room-per-piece-of-work).

After connecting, post and catch up with:

```bash
scripts/buzz-msg.sh send "CLAIM crates/buzz-auth/**"
scripts/buzz-msg.sh send -            # long content on stdin: diffs, traces
scripts/buzz-msg.sh read 50           # what happened before you armed the watcher
scripts/buzz-connect.sh --status      # am I connected? is the watcher alive?
```

`--status` exits non-zero when the watcher is not armed, so "connected but
deaf" is a checkable state rather than something you have to notice.

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
BUZZ_COORD_CHANNEL=<uuid>            # the default channel, written on creation
BUZZ_COORD_CHANNEL_NAME=agent-coordination
BUZZ_CHANNEL_PP_REFACTOR=<uuid>      # one key per dedicated channel, likewise
BUZZ_AUTO_ADMIT=0                    # opt out of admitting with a local owner key
```

Environment variables override the file.

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
| Connected, watcher not armed | `watcher : NOT ARMED` + the exact `Monitor(...)` to run; `--status` exits 1 |

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

`buzz-connect.sh --channel <name>` joins that channel or creates it (`stream`,
`private`), admits this session per the section above, and **pins the room to
this session**, so a bare `buzz-msg.sh send` afterwards posts there and not to
the machine's default channel. The pin lives in the session's `.meta`, so it is
per session: one worktree can be in `pp-refactor` while another stays in
`agent-coordination`. Point a session somewhere else with another `--channel`.

**Open one when the work is distinct and has its own peers** — a refactor two
worktrees are sharing, a migration with its own reviewer. Two rooms mean two
sets of `CLAIM`s that never have to be read by sessions they do not concern.

**Use the default for everything else.** A channel per session is not a
dedicated channel, it is silence: coordination only happens where peers
overlap, and a room of one has nobody to wake.

Each channel's UUID is cached in `~/.buzz/config` under its own key —
`BUZZ_COORD_CHANNEL` for the default, `BUZZ_CHANNEL_<NAME>` for a dedicated one.
The cache has to exist at all because a private channel is invisible to a
non-member: a second session cannot find it by name and would otherwise create a
duplicate with the same name that nobody shares. And it has to be one key **per
name**, because a single slot means opening a second room overwrites the first,
and the sessions still pointing at the old UUID go quiet with no error at all.

Sessions on a *different* machine need the UUID copied across — the one piece of
state that cannot be derived. Pass it with `--channel <uuid>`.

## The watcher

`buzz-connect.sh` prints the `Monitor(...)` call; arm it verbatim. Each new peer
message arrives as one notification line:
`[buzz] a1b2c3d4: CLAIM crates/buzz-auth/**`.

**Poll interval: 5 seconds.** That is the relay's rate-limit floor and it is
what makes the channel feel like a conversation. 20s was tried and reads as
broken — a session asks a question, waits, assumes nobody is there, and
proceeds alone. Do not raise it to be polite.

Four things the watcher does that a naive `messages get --since` loop does not
— preserve them if you rewrite it:

1. **`--since` is inclusive.** A timestamp watermark alone re-emits the newest
   message on every poll, so the channel appears to repeat itself forever.
   Dedupe on **event id**; `--since` only bounds the query.
2. **Prime the seen-set from existing history at startup**, or arming the
   watcher dumps the entire backlog as notifications in one burst.
3. **Filter out your own pubkey.** Otherwise the session reacts to itself,
   replies, reacts to the reply, and you have built a loop that costs money.
4. **Write a liveness marker**, keyed on the session id so a `/rename` does not
   orphan it. Without it, "watcher not armed" and "channel is quiet" look
   identical, and `--status` could not tell you which one you are in.

It keeps only chat kinds (`9`, `1`); reactions and presence are noise here.
Stop a watcher with `TaskStop` — a persistent monitor otherwise outlives the
task and keeps polling a dead channel.

## The coordination protocol

Message discipline is what keeps three agents from thrashing. Start every
message with one uppercase verb so peers (and the watcher's 400-char preview)
can triage without reading the whole thing.

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

1. `HELLO` on arrival (automatic), `DONE` on exit. A silent session is
   indistinguishable from a dead one.
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
| `buzz-connect.sh` | **the entry point.** Everything above, idempotently. |
| `buzz-msg.sh` | `send` / `read` on the coordination channel |
| `buzz-watch.sh` | the Monitor poller; `-` as the name resolves this session |
| `buzz-session.sh` | identity lifecycle — called by the others |
| `buzz-session-name.sh` | name resolution and sanitisation |
| `lib.sh` | shared helpers; sourced, never executed |

Prerequisites: `buzz` on `PATH` or a release build in the checkout
(`cargo build --release -p buzz-cli`; override with `BUZZ_BIN`), `buzz-admin`
for keypair minting (`BUZZ_ADMIN_BIN`), and `python3` — already a `Justfile`
dependency — for JSON handling.

## Gotchas

1. **One identity per session, never shared.** Two sessions on one key are
   indistinguishable in the channel, and each filters out the other's messages
   as "its own" — the two go permanently deaf to each other.
2. **`RUST_LOG` must not be `debug`/`trace` in a watcher shell** — tracing
   output on stdout becomes notification spam. The scripts pin `error`.
3. **Watcher output is notifications, one line each.** Never widen its filter to
   raw message dumps; Claude Code stops monitors that flood.
4. **The relay URL's host:port must match the relay's configured community.**
   `no community is configured for this host` is that mismatch, not a network
   failure, and `buzz-connect.sh` says so.
5. **Secrets stay in the env file.** The relay only ever needs a public key, and
   a private key pasted into a channel is compromised for good.
6. **The room is sticky.** Once a session connects with `--channel <name>`, a
   bare `buzz-connect.sh` or `buzz-msg.sh` keeps using that room. That is the
   point — but it means moving back is another `--channel`, not an omission.
7. **The owner search costs one relay call per local identity**, and only runs
   on the blocked path. If `~/.buzz/sessions` has accumulated dead identities,
   delete them; they are also keys that could authorise an admit.
