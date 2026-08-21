# triage-service

External backend for the Buzz Inbox fibre engine. It turns channel and DM
messages into **fibres** — ideas, asks, decisions, commitments, questions,
blockers, and FYIs — and keeps the open set up to date as new messages arrive.

A fibre holds one or more messages. The desktop app posts every new message
here, including ones you wrote and ones that turn out to be noise, because
context is what makes the judgment calls possible.

It runs as a standalone process and shares nothing with the relay. Only
`data.json` (its runtime state) is gitignored.

## Two stages

Triage is two passes over different units of work.

**Stage 1 — extraction and consolidation** ([extract.mjs](extract.mjs)) works on
messages. For each one it decides `ignore`, `attach` to an existing fibre,
`create` a new one, or `merge` two that turned out to be the same work. Every
message gets exactly one decision; nothing is dropped silently. No scoring
happens here.

**Stage 2 — prioritisation** ([prioritize.mjs](prioritize.mjs)) works on fibres.
It runs only over the fibres stage 1 touched, sees the whole message set rather
than the latest arrival, and produces the score that drives the lane. Existing
fibres are rescored on every touch, so a fibre that collects a third unanswered
ping climbs.

Splitting them resolves a tension in the rules: a `bump` carries no new
information, so stage 1 attaches it rather than creating a fibre, and stage 2
then sees a fibre with three pings and no reply from you and raises its score.

### Message bursts

People split one thought across several lines, and each line on its own would
become its own fibre. Stage 1 keeps a trail of what every earlier message in
the batch landed on, because a message that continues one of them cannot attach
to a fibre that does not exist yet.

A message joins the previous one when the same author posted within
`BURST_WINDOW_SECONDS` (5 minutes) *and* it opens by referring back — "so …",
"and …", "it …", "that said …". Timing alone is not enough; a new subject from
the same author seconds later still gets its own fibre. The model sees
`secondsAfterPrevious` and `previousFromSameAuthor` and is told to group these
by default, and it can veto with `"newTopic": true` when the author has genuinely
moved on. Differing kinds are not a reason to split: an ask and the consequence
its author spells out a line later are one fibre.

This also applies to lines that would otherwise be dropped. "so i'd imagine
there'd be a ton of conflicts" says nothing standalone, so it is ignored on its
own merits — as a continuation it joins the ask it belongs to instead. A
continuation can only ever join a fibre this way, never start one.

[classify.mjs](classify.mjs) orchestrates the two.

## Lanes

Every open fibre lands in exactly one column, tried in order:

| Lane | Rule |
|------|------|
| `important` | `score > 70` |
| `hot` | `engagement >= 60` |
| `other` | everything else |

`score` is the model's importance judgment plus deterministic adjustments.
`engagement` is computed purely from thread velocity and distinct participants
([signals.mjs](signals.mjs)) — no model call, so the `hot` lane is free and
predictable.

## Context and signals

The service keeps a rolling per-channel **transcript** (300 messages) in
`store.mjs`. Everything else is derived from it: the twelve messages preceding
a candidate, the full thread when you were tagged inside one, unanswered pings,
thread velocity, whether you have worked on this thread before, and whether an
incident channel is still moving.

Adjustments applied on top of the model's score, so they are guaranteed rather
than hoped for:

- unanswered pings above one: `+10` each, capped at `+25`
- follow-up with no new information: `-15`
- you have worked on this before: `+10`
- ongoing incident channel: `+25`

## Run

No dependencies. Requires Node 18+ (Node 24 ships with the repo's Hermit
toolchain).

```bash
cd triage-service
OPENAI_API_KEY=sk-... node server.mjs   # http://localhost:8787
```

Or from the repo root: `./scripts/triage-up.sh`. Point the desktop app at it
with `VITE_TRIAGE_API_URL` if you change the port.

```bash
node --test "*.test.mjs"
```

## Classification modes

**The LLM is the default and the intended mode.** The contextual rules — blast
radius, whether an FYI matters to you, whether an architectural change is
expensive to undo — are judgment calls a regex cannot make.

Set `TRIAGE_MODEL` to change the model (default `gpt-4o-mini`).

The built-in heuristic is the fallback when `OPENAI_API_KEY` is missing or a
call fails, and can be forced with `TRIAGE_LLM=0` (`./scripts/triage-up.sh
--heuristic`). It is much blunter: it reads keywords and thread shape, so
expect coarse lanes and no real sense of consequence. It exists so the app is
demoable offline, not as parity.

## Endpoints

- `POST /ingest` — `{ pubkey, messages }` records the transcript, runs both
  stages, and returns `{ fibres, done, openCount, doneCount, clearedCount,
  laneCounts, changes, ignored }`
- `GET /fibres?pubkey=` — open fibres with lanes, plus the done list
- `GET /ignored?pubkey=` — audit trail of messages triage did not surface, with
  the reason for each and `addressedCount` for the ones aimed at you. Nothing in
  the product reads this; it exists so a missed true positive can be found
  instead of disappearing.
- `PATCH /fibres/:id` — `{ pubkey, status: "done" | "dismissed" | "open" }`
- `POST /fibres/restore` — `{ pubkey }` reopens every done/dismissed fibre
- `POST /feedback` — `{ pubkey, fibreId, userAction, note? }` (done, dismissed,
  delegated); returns the feedback id
- `PATCH /feedback/:id` — `{ pubkey, note }` attaches a written reason after the
  fact, so dismissing stays instant
- `GET /health`
- `POST /reset` — purge everything

Written reasons are fed verbatim into both stage prompts as standing
instructions, which is the strongest correction signal the engine has.

State persists to `data.json` beside the server.
