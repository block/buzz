---
name: buzz-multi-session
description: >
  Coordinate several independent Claude Code sessions — typically parallel git
  worktrees of one repo — over a shared Buzz channel: per-session identities,
  channel enrolment, and a Monitor watcher so peers wake on new messages
  instead of the human relaying between terminals.
version: 1
---

# Buzz Multi-Session Coordination

Three Claude Code sessions in three worktrees normally cannot talk. The human
becomes the message bus: copy an answer out of terminal A, paste it into
terminal B, notice ten minutes later that B and C edited the same file.

This skill removes the human from that loop. Each session gets **its own Buzz
identity**, all of them join **one channel**, and each arms a **Monitor** on
that channel. A peer's message becomes a notification in your session — you
wake, read it, act, reply. No polling loops in the foreground, no copy-paste.

This is a Claude Code developer workflow, not something shipped to managed
agents — it depends on the `Monitor` tool. For the general relay CLI surface,
see the `buzz-cli` skill; this skill only documents what that one does not.

## Prerequisites

- `buzz` on `PATH`, or a release build in the checkout
  (`cargo build --release -p buzz-cli`). Bundled scripts fall back to
  `<repo>/target/release/buzz`; override with `BUZZ_BIN`.
- `buzz-admin` for keypair minting (`cargo build --release -p buzz-admin`),
  or `BUZZ_ADMIN_BIN`.
- `python3` (already a `Justfile` dependency) for JSON handling in the scripts.
- A relay you can reach, plus **either** an invite code **or** an owner willing
  to run `buzz-admin add-member`. See [Enrolment](#step-2--enrol-on-the-relay).

## The Two-Gate Rule

The single most common failure. **Relay membership and channel membership are
separate gates.** A pubkey that is a relay member still sees nothing in a
private channel until the channel owner adds it:

```bash
buzz channels add-member --channel <UUID> --pubkey <hex> --role member
```

Symptom of forgetting: `buzz messages get` returns `[]` forever, no error, and
the watcher stays silent while peers chat happily. If a session reports "the
channel is empty", check `buzz channels members --channel <UUID>` for its
pubkey **before** debugging anything else.

## Step 1 — Mint a per-session identity

Run once per session, inside its worktree:

```bash
.claude/skills/buzz-multi-session/scripts/buzz-session.sh new
```

The name defaults to `<repo>-<worktree-dir>`, so parallel worktrees get
distinct, attributable identities without anyone inventing names. Pass an
explicit name for sessions that are not worktrees.

Identities are stored at `~/.buzz/sessions/<name>.env`, mode 600, holding
`BUZZ_PRIVATE_KEY`, `BUZZ_PUBKEY`, `BUZZ_RELAY_URL`.

**Never print, cat, grep, echo or otherwise surface `BUZZ_PRIVATE_KEY`** — not
into the transcript, not into a Buzz message, not into a log. The script pipes
`buzz-admin generate-key` straight into the 600-mode file for exactly this
reason. Only the **public** key is ever quotable. Other subcommands:

```bash
buzz-session.sh pubkey        # public key only — safe to paste anywhere
buzz-session.sh env           # prints the `set -a; . <file>; set +a` line
buzz-session.sh list          # every known session identity + pubkey
```

Load it into the session's shell — every later `buzz` call reads these:

```bash
set -a; . ~/.buzz/sessions/<name>.env; set +a
```

## Step 2 — Enrol on the relay

**Preferred (pending [#3014](https://github.com/block/buzz/issues/3014)):**

```bash
buzz invites claim --code <token>
```

The relay endpoints `POST /api/invites` and `POST /api/invites/claim` exist
today, and claim is deliberately exempt from the relay-membership gate — but
**`buzz invites` is not yet in the CLI**. Until #3014 lands,
`buzz invites claim` exits 1 with `unrecognized subcommand 'invites'`. Do not
try to work around it by hand-rolling NIP-98 requests.

**Fallback until then** — the relay operator runs, once per session pubkey:

```bash
buzz-admin add-member --pubkey <hex> --role member
```

Then, either way, the channel owner runs the `channels add-member` from
[The Two-Gate Rule](#the-two-gate-rule). `buzz channels join --channel <UUID>`
publishes a kind:9021 join request and is rejected with
`403 relay_membership_required` for a non-member, so it is not a substitute for
either gate.

## Step 3 — Create or find the channel

One session (or the human) creates the coordination channel once:

```bash
buzz channels create --name refactor-auth --type stream --visibility private \
  --description "3-worktree coordination: auth refactor"
```

`buzz channels list` gives the UUID to everyone else. Export it so both the
watcher and your sends agree:

```bash
export BUZZ_COORD_CHANNEL=<UUID>
```

Use `--visibility private` for real work. Consider `--ttl <seconds>` to make
the channel ephemeral — the relay archives it after that long without a
message, which is a good fit for a coordination channel that outlives nothing.

## Step 4 — Arm the watcher

Use the **Monitor** tool, persistent, with the bundled poller:

```
Monitor(
  command: ".claude/skills/buzz-multi-session/scripts/buzz-watch.sh <session-name> <channel-uuid> 5",
  description: "buzz coordination channel <name>",
  persistent: true
)
```

Each new peer message arrives as one notification line:
`[buzz] a1b2c3d4: CLAIM crates/buzz-auth/**`.

**Poll interval: 5 seconds.** That is the relay's rate-limit floor and it is
what makes the channel feel like a conversation. 20s was tried and reads as
broken — a session asks a question, waits, assumes nobody is there, and
proceeds alone. Do not raise it to be polite.

Three things the watcher does that a naive `messages get --since` loop does not
— preserve them if you rewrite it:

1. **`--since` is inclusive.** A timestamp watermark alone re-emits the newest
   message on every poll, so the channel appears to repeat itself forever.
   Dedupe on **event id**; `--since` only bounds the query.
2. **Prime the seen-set from existing history at startup**, or arming the
   watcher dumps the entire backlog as notifications in one burst.
3. **Filter out your own pubkey.** Otherwise the session reacts to itself,
   replies, reacts to the reply, and you have built a loop that costs money.

It also keeps only chat kinds (`9`, `1`); reactions, presence and other kinds
are noise here. Stop a watcher with `TaskStop`.

## Step 5 — Post

```bash
buzz messages send --channel "$BUZZ_COORD_CHANNEL" --content "STATUS worktree-a: auth middleware extracted, tests green"
```

Long content: `--content -` reads stdin, which avoids shell-quoting pain for
diffs and stack traces. Content is GitHub-flavored Markdown, so fenced code
blocks render properly. Max 65,536 bytes.

Prefer plain `@name` text over `--mention` for peer sessions: an unresolved or
ambiguous name **stops the send before publishing**, and session identities
usually have no profile name set. Give a session a readable name once with
`buzz users set-profile --name worktree-a` if you want mentions to resolve.

## The coordination protocol

Message discipline is what keeps three agents from thrashing. Start every
message with one uppercase verb so peers (and the watcher's 400-char preview)
can triage without reading the whole thing.

| Verb | Meaning | Example |
|------|---------|---------|
| `HELLO` | joining; who and where | `HELLO worktree-a branch=feat/auth` |
| `CLAIM` | taking exclusive ownership of a path glob | `CLAIM crates/buzz-auth/**` |
| `RELEASE` | done with a claim | `RELEASE crates/buzz-auth/**` |
| `STATUS` | progress, no reply needed | `STATUS tests green on auth` |
| `ASK` | question addressed to one peer | `ASK worktree-b: did you rename Session?` |
| `ANSWER` | reply to an `ASK` | `ANSWER worktree-a: yes, now AuthSession` |
| `BLOCKED` | stuck, needs someone | `BLOCKED waiting on RELEASE of migrations/**` |
| `DONE` | this session's work is finished | `DONE worktree-a: pushed feat/auth` |

Rules that make it work:

1. `HELLO` on arrival, `DONE` on exit. A silent session is indistinguishable
   from a dead one.
2. **`CLAIM` before editing shared paths.** If a peer has an unreleased
   `CLAIM` overlapping yours, do not edit — `ASK` them or work elsewhere.
   Claims are advisory; nothing enforces them but the agents.
3. Answer every `ASK` addressed to you, even with "don't know". A session
   blocked on an unanswered question burns its whole budget waiting.
4. Never reply to your own message, and never `STATUS` on a timer — noise
   costs every peer a notification and a wake-up.
5. Read the channel before your first edit: `buzz messages get --channel
   "$BUZZ_COORD_CHANNEL" --limit 50` catches up on claims made before you
   armed the watcher.

## Gotchas

1. **Empty channel, no error** — almost always the channel-membership gate, not
   the relay one. See [The Two-Gate Rule](#the-two-gate-rule).
2. **`--since` is inclusive** — the top cause of a watcher that appears to
   repeat the same message every 5 seconds.
3. **One identity per session, never shared.** Two sessions on one key are
   indistinguishable in the channel, and each filters out the other's messages
   as "its own" — the two go permanently deaf to each other.
4. **`RUST_LOG` must not be `debug`/`trace` in a watcher shell** — tracing
   output on stdout becomes notification spam. The script pins `error`.
5. **Watcher output is notifications, one line each.** Never widen its filter
   to raw message dumps; Claude Code stops monitors that flood.
6. **Secrets stay in the env file.** The relay only ever needs a public key,
   and a private key pasted into a channel is compromised for good.
7. **Kill the watcher when the work ends** (`TaskStop`) — a persistent monitor
   outlives the task otherwise and keeps polling a dead channel.
