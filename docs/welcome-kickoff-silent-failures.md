# Welcome Kickoff — Failure Paths

Context: the Welcome-channel kickoff choreography
(`desktop/src/features/onboarding/welcomeKickoff.ts`) where Fizz posts an
opener, teammates introduce themselves in-thread, and Fizz posts a closer.

The file name says "silent-failures" for link stability (referenced from
[PR #2066](https://github.com/block/buzz/pull/2066) and
`useWelcomeKickoffStage.ts`); the scope is all kickoff failure paths.

## The one bug behind all of it

The kickoff fails in three directions, and they look unrelated until you notice
what they share:

| Class | Failure | Status |
|---|---|---|
| **Wrong story** | The team is announced as late/broken while it is working fine | **Open** — [§1](#1-wrong-story-the-closer-speaks-on-a-timer) |
| **Too loud** | Agents reply to each other indefinitely | **Fixed 2026-07-18** — [§2](#2-too-loud-runaway-reply-loop-fixed) |
| **Too quiet** | Nobody speaks; the user stares at an empty channel | **Open** — [§3](#3-too-quiet-silent-paths) |

**The shared root cause: the kickoff decides what to say from a timer and the
absence of evidence, then writes that guess in permanent ink.**

It has exactly one fact-based health check — `failedAfterKickoff`
(`welcomeKickoff.ts:282`), which reads real agent state (`status === "stopped"`
+ `lastError` + `lastStoppedAt` after the opener). That is a genuine fact: the
process died. Everything else that drives a user-visible decision is a stopwatch:

| Timer | Value | Decides |
|---|---|---|
| `TEAMMATE_READY_WAIT_MS` | 60s | whether to post the degraded opener |
| `TEAMMATE_INTRO_WAIT_MS` | **15s** (now `TEAMMATE_INTRO_BACKSTOP_MS`, 120s — [§1](#1-wrong-story-the-closer-speaks-on-a-timer)) | whether to announce teammates as slow |
| `WELCOME_KICKOFF_STAGE_TIMEOUT_MS` | 90s | whether to retire the kickoff stage |

**The facts decorate; the timers decide.** `failedAfterKickoff` only chooses
*wording* inside a message the 15s stopwatch already decided to send. Invert
that and most of this doc collapses: **facts decide, timers are a last-resort
backstop.**

The distinction the code is missing is between two things it treats as one:

- **"The agent crashed"** — a fact. We have it. Worth announcing.
- **"No intro yet"** — *not* a fact. That is ignorance. It is not news.

Announcing ignorance on a deadline is what produces the wrong story. Being
unable to announce anything is what produces the silent paths. And the loop was
the same disease one layer up: agents were *required to speak every turn*
regardless of whether they had anything true to add, so they said "got it"
forever.

**The principle, at both layers: don't mandate speech — mandate honesty.** The
prompt fix in §2 and the closer fix in §1 are the same change in two places.

## Plan

| # | Work | Where | PR |
|---|---|---|---|
| 1 | Stop "no intro yet" from writing the permanent closer | `welcomeKickoff.ts` | **✅ landed, this branch** |
| 2 | Loop hardening | `base_prompt.md` | **✅ landed, this branch** |
| 3 | Thread replies don't render live | `hooks.ts` / thread cache | **separate PR** — app-wide, not kickoff ([§4](#4-thread-replies-dont-render-live-separate-pr)) |
| 4 | Silent paths — surface a cause in the UI | `useWelcomeKickoff` + stage | later ([§3](#3-too-quiet-silent-paths)) |
| 5 | Loop circuit breaker | `buzz-acp` | backlog ([§2](#2-too-loud-runaway-reply-loop-fixed)) |
| 6 | `!cancel` unreachable from any surface | `buzz-acp` + CLI | backlog ([§5](#5-backlog)) |

---

## 1. Wrong story: the closer speaks on a timer

**Status: fixed on this branch. Observed 2026-07-18, 14:26.** Opener at 2:26. At 2:26+15s Fizz
posted *"Honey and Bumble are taking longer than expected. I'm still here to
help."* Honey and Bumble posted good intros at 2:27. The false story was never
corrected, because it was already stamped final.

### Mechanism

1. Opener posts. A timer is set for `15s − (now − opener.created_at)`
   (`welcomeKickoff.ts:716`).
2. It fires. `classifyWelcomeKickoffResolution` (`:292`) splits teammates into
   `failed` (fact-based, via `failedAfterKickoff`) and `unresolved` (**merely
   no intro seen yet**).
3. `unresolved.length > 0` → `buildWelcomeKickoffCloser([], ["Honey","Bumble"])`
   → the "taking longer" text + the CTA (`:253`).
4. It posts **with `closerMarker`** (`sendWelcomeKickoffCloser`, `:443`). That
   marker is **terminal**: every later pass early-returns on it (`:703`) and the
   `kickoffResolved` latch (`:513`) makes it permanent by design.
5. Intros arrive. Nothing re-runs. **There is no path that ever posts a real
   closer.**

### Why 15s is the wrong number *and* the wrong question

- Its neighbour allows **60s for a process to boot** (`TEAMMATE_READY_WAIT_MS`)
  but **15s for two cold agents to receive a dispatched event, run a full LLM
  turn, and publish** — 4× less budget for a far harder job.
- The clock starts at `opener.created_at`, so harness dispatch latency spends it
  before the agents hold the event.
- `Math.max(0, …)` (`:716`) means on a revisit the wait clamps to zero and the
  message fires **instantly**.
- Observed reality: intros took **~60s**. A 30s timer would also have misfired.

But the deeper problem is structural: **the closer welds a terminal fact to a
provisional guess.**

| Part of the closer | Nature | Wants |
|---|---|---|
| The CTA — *"What can we help you build?"* | terminal, exactly once | ✅ a one-shot marker |
| Teammate status — *"X is taking longer"* | **provisional, corrigible** | ❌ currently welded to that marker |

### The fix

Let facts decide; keep the stopwatch as a backstop only. The closer should fire
when one of these is true:

- **intros land** → clean closer + CTA (the ~95% path — needs no timer at all)
- **`failed` is non-empty** → the "couldn't start / check Agents" closer,
  immediately (fact-based, so it can be fast and still honest)
- **a long backstop elapses with teammates alive but silent** → the "taking
  longer" text, which by then is *true*

**The code already has this structure. Only the backstop's value was wrong.**
`classifyWelcomeKickoffResolution` (`:292`) already excludes `failed` from
`unresolved`, so once every teammate is intro'd-or-failed, `unresolved` is empty
and the closer fires on the 3s beat with the correct fact-based wording — the
timer is cleared and never speaks. The timer callback also re-classifies against
the latest events before posting, so it self-corrects if intros land between
timer-set and timer-fire.

So the whole fix is the constant: `TEAMMATE_INTRO_WAIT_MS = 15_000` →
`TEAMMATE_INTRO_BACKSTOP_MS = 120_000`. Renamed because the old name described it
as an expectation of how fast an intro arrives, which is what invited tuning it
like one. It is a give-up backstop. Because it doesn't gate the happy path,
raising it costs the normal case nothing — it only delays the moment we give up
on a teammate that is alive but silent. A real failure never waits for it;
`failedAfterKickoff` resolves crashed teammates immediately.

### Decided: keep the CTA bundled in the closer

The CTA only exists *inside* the closer, so waiting for intros delays the "you
can talk to us now" handoff from ~15s to ~60s. **Accepted** (Morgan, 2026-07-18):
the room is not dead while we wait — the opener is up and the stage shows
*"Fizz: Working"*. The alternative (post the CTA early on its own, and post
status only when there is status worth reporting) is honest but adds a second
message, with its own marker and idempotency, to solve a problem the stage
already solves.

**Note:** this failure is a *shrunken* §3. The channel is not dead — a CTA
arrives — but the story is false. Any fix here must not reopen §3: if we wait
longer and the wait ends in nothing, we are back to unexplained silence.

---

## 2. Too loud: runaway reply loop (fixed)

**Status: prompt hardening landed 2026-07-18. Verified once manually** (14:26
run: 3 replies, intros, stop). One good observation, not proof. Re-verify on
Codex specifically.

Observed on the Codex runtime (`codex-acp`), never reproduced on Claude Code.
21+ replies deep, each an acknowledgement of the previous acknowledgement:

> **Bumble:** `@Fizz` parked; no further replies from me until there's work.
> **Honey:** `@Fizz` understood. I won't reply again unless there's a task for me.
> **Fizz:** `@Honey` `@Bumble` acknowledged — stay parked until `@morgan` brings a real task.

**The content was the tell: every agent was trying to end the conversation, and
announcing it is what kept it alive.** The agents were not malfunctioning — they
were complying exactly. The loop was *correct* behavior given the prompt.

### Root cause

Two rules in `crates/buzz-acp/src/base_prompt.md` composed into a perpetual
motion machine:

1. *"**Every turn that processes a user message MUST publish a reply.** […] A
   turn that ends without a published message is a silent failure."*
2. *"When you finish delegated work, you MUST `@mention` the delegator […]. This
   is the #1 cause of stalled collaboration."*

Rule 1: *always speak*. Rule 2: *when you speak, tag whoever tagged you*. On a
mutual mention the circuit closes and never opens. Rule 1 said "user message"
but was phrased as an absolute with no exception for an agent-authored trigger.
Rule 2 was written to fix the *opposite* failure — the two hardenings worked
against each other and nothing reconciled them.

The Welcome kickoff was the worst case: the opener says *"Don't start any work
yet"* (`:162`), so teammates were told they **must** reply and that there is
**nothing to report** — stripping away every substantive thing a reply could
contain. The only output satisfying rule 1 was a content-free acknowledgement.
The kickoff didn't just permit the loop; its instructions selected for it.

### What shipped

Scoped both rules by **what the turn has to say**, not who triggered it:

- Rule 1 → publish if the turn produced something worth knowing (a result,
  answer, deliverable, decision, blocker, or needed question; asked-for work
  always counts).
- A human who asked you something must always get a reply — even "nothing to
  add". This preserves the anti-silent-failure floor rule 1 existed for.
- Otherwise publishing is optional and **silence is explicitly a success**.
- **No bare acknowledgements**, with the observed offenders named ("Got it",
  "Confirmed", "Standing by", "Parked", "I won't reply again") and the kicker:
  *if you are tempted to announce you are done replying, that is the message not
  to send.*
- Rule 2 scoped to **completed work only** — not assignment acks, not
  conversational loop-closing.
- Mentions rule hardened: naming someone while talking *about* them ("waiting on
  @morgan") is narrative — drop the `@`. The loop spammed Morgan with 4 such
  false notifications.

### Why it had to be a local test, not "don't loop"

**"Don't get into a loop" is not a rule an agent can follow.** A loop is a
global property of a conversation; each agent sees only its own turn, and every
individual reply looks locally reasonable — which is why the sign-offs read as
polite rather than broken. The rule had to become a **local, per-turn test**:
*does this add information the thread doesn't have?* An acknowledgement is
definitionally not new information, which makes "no bare acknowledgements" the
checkable form of the intent.

A soft caveat would also have failed: *"you may end the turn"* sitting next to
*"**MUST** publish a reply"* leaves a literal model correctly following the
stronger instruction. The mandate had to be **narrowed**, not exception-ed.

### Still open: the circuit breaker

Prompt-only means prose-compliance-only, and Codex is proof models don't
reliably comply. There is still **no reply-depth counter, hop limit, cooldown,
or agent-to-agent budget anywhere in the path.** Existing guards that don't help:

| Guard | Why not |
|---|---|
| `ignore_self` (`lib.rs:1864`) | Blocks self-replies only. The *only* loop guard, and A→B→A is exactly what it misses. |
| Author gate (`respond_to`) | **Admits siblings by design** — `is_owner_or_sibling` (`lib.rs:166`) verifies same-owner agents via NIP-OA. It's an *admission* mechanism; a loop needs *termination*. No setting stops this. |
| `max_turns_per_session` (`config.rs:372`) | Defaults 0 = disabled; it's session rotation for context hygiene, not a reply brake. |
| Queue caps (`queue.rs:24`) | Backpressure on *pending* events. A ping-pong is never backed up. |
| `closerMarker` | Idempotency for the client-authored closer only; never observes agent replies. |

Candidate: **consecutive agent-to-agent reply budget** — count unbroken
agent-authored turns in a thread; past N, drop the trigger. A human message
resets it. Set N high (~6–10) so it never fires on healthy work — a circuit
breaker, not a policy. `resolve_reply_anchor` deliberately allows deep
agent-only nesting, so a low cap would truncate legitimate coordination.

**A too-aggressive breaker manufactures §3.** A depth counter cannot tell a loop
from a productive chain; dropping a good reply produces exactly the unexplained
silence this doc is otherwise about. Hence: prompt primary, breaker high.

Add a `tracing` line when it fires — cheap and currently we'd be blind. A
user-facing surface is likely out of scope: when the breaker works, the desired
outcome is just that agents stop talking. The case for surfacing it is the
false-positive, not the success.

---

## 3. Too quiet: silent paths

**Status: open.** The *perception* gap is handled; the paths are not.

Every fallback message assumes Fizz — the lead and sender — is alive and able to
post. **When Fizz is the thing that failed, nobody speaks.**

The client-side kickoff stage (characters on the Welcome composer banner) covers
perception and has landed: after 90s with no message, they exit and the banner
drops to its normal mention hint. A failed kickoff degrades to an ordinary,
usable empty channel rather than claiming a team is still being set up.

What it does **not** do is explain anything:

- The stage reads only "is the timeline empty" + that timer
  (`useWelcomeKickoffStage.ts`). It never reads real kickoff state, so it cannot
  tell "Fizz crashed" from "the relay is slow" — **another stopwatch standing in
  for a fact.**
- The empty channel it degrades to invites the user to `@`-mention Fizz — who,
  in exactly these cases, is what isn't working. Honest, but a dead end.

### What the user CANNOT be told today

1. **Fizz fails to start.** `startManagedAgent` rejects (harness binary missing,
   spawn error). The effect logs `Failed to start Welcome agent…` and returns —
   by design only Fizz sends the opener, so nobody speaks.
2. **Any step throws.** The whole kickoff is one `try/catch` that logs
   `Failed to start the Welcome team kickoff.` and gives up. Seen in practice:
   relay unreachable / websocket down; `ensureWelcomeTeam` failure; the send
   itself rejected (relay rate-limiting — see [Related](#related)).
3. **Closer-path failures.** A failing closer send is caught-and-logged only;
   the thread ends without the CTA. Lower stakes (opener + intros already
   happened) but still dangling.

Navigating away mid-kickoff also cancels silently — intentional (it resumes on
next visit), not a failure.

### Messages the user CAN receive today

All hard-coded client-side; only teammate intro replies are LLM-generated.

| # | Message | Trigger | Sender |
|---|---|---|---|
| 1 | Provider fallback ("connect to an AI provider in Settings…") | Readiness check fails before kickoff | Fizz (`provider-required.v1`) |
| 2 | Happy-path opener | Team online | Fizz (`opener.v1`) |
| 3 | Degraded opener ("I'm here with Honey and Bumble…") | Fizz online, zero teammates online within 60s | Fizz (opener + closer markers) |
| 4 | Closer variants (clean / failed / slow) | 3s beat after intros resolve, **or the 120s intro backstop** — see [§1](#1-wrong-story-the-closer-speaks-on-a-timer) | Fizz (`closer.v1`) |
| 5 | Setup-mode nudge ("here's what you still need to configure") | Agent spawns but requirements check fails (e.g. missing API key) | The agent process itself (buzz-acp setup-listener mode) |

### Constraints for the fix

- **Fizz cannot be the messenger** — she is what failed. Any fallback must come
  from the client UI (banner, intro-block state, stage `timed-out` phase), not a
  channel message impersonating an agent.
- A relay-side/system-authored message is possible (kind-scoped system event)
  but heavier. The client already knows locally that the kickoff threw, so local
  UI state is the cheap, honest option.
- Must be **idempotent across revisits** — same rule as the opener markers.
  Don't re-alarm the user every time they click Welcome.
- Distinguish *retryable* (relay hiccup, rate-limit) from *actionable* (harness
  missing → point at Agents/Settings). `Requirement` in
  `desktop/src-tauri/src/managed_agents/readiness.rs` already classifies the
  actionable ones.

### Sketch (to validate later)

1. Surface a `kickoffError` phase from `useWelcomeKickoff` when the catch block
   fires or the lead's start rejects, with a coarse cause
   (`lead-start-failed` | `relay` | `unknown`).
2. The stage's `timed-out` phase renders that cause: quiet copy + a pointer to
   Agents (start failures) or a retry affordance (relay failures). Retry =
   re-run the effect (the coordinator already dedupes). The phase currently
   exits immediately on timeout, so giving it copy means holding it on screen —
   and it is `aria-hidden` decoration today, so anything it says must reach
   screen readers.
3. Consider a bounded auto-retry (once, short delay) for the relay class before
   showing anything.
4. Closer-path failure: retry the send once; otherwise leave the thread as-is
   (intros already delivered the core experience).

---

## 4. Thread replies don't render live (separate PR)

**Status: open, root cause unknown. Not a kickoff bug** — surfaced here, tracked
here only until it gets its own home. **App-wide; likely predates this work and
outranks everything else in this doc by blast radius.**

### Symptom (observed repeatedly, 2026-07-18 among others)

With a thread open, new replies from someone else **do not appear**. The
channel's reply count **does** increment. Closing and reopening the thread shows
every missing reply.

### Why it hides

One fact — "there are new replies" — travels three independent roads:

| What the user sees | Source |
|---|---|
| **Reply count** | Relay pushes a **kind 39005 thread-summary recount** → merged into the window-store overlay (`hooks.ts:266-277`). **Does not come from the replies themselves.** |
| **Thread pane rows** | A separate React Query cache `["thread-replies", channelId, rootId]` (`useThreadReplies.ts`), filled on open; live replies must be *filed into it* by `appendMessage` (`hooks.ts:282-291`) |
| **Channel timeline** | Window store — thread replies deliberately early-return before reaching it (`hooks.ts:292`) |

So the badge is **correct** and the pane is wrong — the count is the relay's own
tally, arriving whether or not the client ever received the reply. **The badge
moving is not evidence the message arrived**, which is why this is
undiagnosable from the UI and has gone unexplained across multiple sightings.
Close/reopen refetches from scratch (`staleTime: 0`) → everything appears.

Likely invisible in human-to-human threads because your own sends render
optimistically without waiting for the relay. The broken road is **replies
arriving from someone else while the thread is open** — in practice, mostly
agents.

### Cleared so far

- **Not `getThreadReference` normalization.** It returns `rootId: rootTag?.[1] ?? parentId`
  (`threading.ts:50`), so a direct reply to the opener *does* get a `rootId` —
  the thread-cache write condition should pass.
- **Not the live filter.** `buildChannelFilter` is `#h`-scoped across the broad
  `CHANNEL_EVENT_KINDS` set — thread replies carry the same `h` tag.
- **Not the initial-fetch race.** `useThreadReplies`' `queryFn` snapshots ids at
  start and re-merges anything received in-flight (`:108-113`).

### Suspect worth checking (unproven)

`welcomeKickoff.ts:504` calls `useThreadReplies` on the **same cache key** the
open thread pane uses (`ChannelScreen.tsx:186`) — in the Welcome kickoff the
open thread *is* the opener thread, so two features with different lifecycles
share one cache entry at `staleTime: 0`. When `kickoffResolved` latched, the
kickoff's observer passed `null`, flipping its key to
`["thread-replies","none",openerId]` and detaching — **~30s before the intros
failed to render.** The comment at `:492-499` documents this coupling as having
bitten once already.

**But Morgan reports this outside the Welcome flow, which argues against the
coupling being the cause.** Correlation only.

### Next step (do this before theorizing further)

Temporary log in `appendMessage` printing `event.kind`, `event.id`, and
`getThreadReference(event.tags)` for the channel; re-run with the thread open.
That splits the problem in half in one run:

- **Reply never arrives** → a delivery problem (subscription/relay fan-out).
- **Arrives but isn't filed** → a bookkeeping problem (the write condition).

---

## 5. Backlog

**`!cancel` / `!shutdown` / `!rotate` are unreachable from every product
surface.** `is_owner_control_command` (`lib.rs:2476`) requires *all* of: kind:9,
`content.trim() == "!cancel"` (**exact**), and a `p` tag naming the agent. But
every surface derives the `p` tag *from `@Name` text in the content* (Desktop:
`hasMention.ts:143`; CLI: `resolve_content_mentions`, `messages.rs:128` —
`SendMessageParams` has no mention flag). So `@Fizz !cancel` fails the exact
match, and bare `!cancel` produces no `p` tag. **Mutually exclusive on every
real surface.** Only a hand-crafted signed event via `POST /events` fires them.
The unit test passes only because it attaches the `p` tag independently of
content — a shape no product path can produce.

Options: relax the matcher to accept a leading `@Name` before the command, or
add a mention flag to `buzz messages send`. First confirm these were ever
intended for anything but hand-crafted/test use.

Even fixed, `!cancel` cancels **one turn, one agent, one channel**, and the
agent resumes on the next mention — not a loop breaker. Note stop/cancel
controls were explicitly descoped from the loop work (2026-07-18); this is
tracked as its own bug.

**What works today for a runaway team:** steering (just send a message —
`multiple_event_handling` defaults to `steer`, `config.rs:357`) redirects an
agent that is *working*, but a loop is many short *completed* turns, so steering
can't break it. The only real tool is **Stop in the Agents UI**
(`useManagedAgentActions.ts:245`), which also kills legitimate in-flight work
and requires the user to recognize the loop and know where the kill switch is.

## Reference: rejected approaches (do not retry)

**Scoping the reply mandate by sender identity (human- vs agent-triggered
turns).** Rejected 2026-07-18. The obvious fix is to key rule 1 on
`turn_is_human_facing` (`queue.rs:1150`). It does not work, and the reason is
invisible until you trace real transcript `p` tags:

`parse_thread_tags` (`queue.rs:835-837`) collects **every `p` tag with no notion
of who is being addressed**, and `turn_is_human_facing` returns `true` if *any*
mentioned pubkey is human (`:1167`). The loop's signature content is agents
narrating *"stay parked until `@morgan` brings a real task"* — which `p`-tags
the human:

| Turn | Trigger | `p` tags | Classified |
|---|---|---|---|
| Honey/Bumble | Fizz: *"…until `@morgan` brings a real task"* | Honey, Bumble, **morgan** | **human → MUST reply** |
| Fizz | Honey: *"@Fizz understood"* | Fizz | agent → optional |

It exempts only the leg that happens not to name the human — cutting 1 of 3 legs
**by luck**. Had Honey written *"@Fizz understood, waiting on @morgan"* —
entirely in character — the loop survives the fix intact. **The loop's own
content re-arms the rule meant to stop it.** A guard the symptom disables is not
a guard. Worse, those narrative `@morgan` mentions already violated the Mentions
rule, so the guard would have taken an existing prose non-compliance as input.

Root insight: `turn_is_human_facing` answers *"is a human named?"*, not *"is a
human asking?"* — and those diverge exactly where it matters. It is a fine
reply-anchor heuristic and a wrong safety signal. This also killed the planned
`[Context]` `Triggered by: human|agent` plumbing: correct signal delivery, wrong
signal.

**Fixing the loop in the personas** (`personas.rs`). Rejected: they are
*character* prompts (tone, wordplay), so a conversation-protocol rule is a
layering violation; it would need duplicating across all three and every future
persona; and stored copies are user-editable with modification tracking
(`migrate_retired_personas`, `was_unmodified`) — a user rewording Fizz must not
be able to delete a loop guard.

**Fixing the loop in `welcomeKickoff.ts` copy.** Rejected: treats the trigger,
not the cause. The same rule collision fires for any two agents that mention
each other with nothing to report.

**Available but not chosen: team instructions.** `TeamRecord.instructions` is
plumbed end-to-end (`teams.rs:61` → `PromptContext.team_instructions` →
`[Team Instructions]`, `pool.rs:1114-1127`) and is `None` for the Welcome Team.
It is the natural home for kickoff-specific etiquette and the right place if the
base-prompt fix proves too weak for the intro case specifically — but it only
covers this one team, so it complements rather than replaces §2.

## Related

- **Rate-limiting incident.** One Welcome agent produced a 42KB log of
  "rate-limited: quota exceeded" retries within seconds (2026-07-17, remote
  relay `onboarding.communities.buzz.xyz`). A tight retry loop against a quota
  makes every other send in the session fail too — including the kickoff's, one
  of the §3 silent paths. Worth a separate look at buzz-acp publish backoff.
  Originally suspected to be the §2 loop burning quota; with §2 fixed, if this
  recurs it is an independent retry bug.
- **Why Codex and not Claude Code.** Ruled out: prompt content (identical across
  runtimes — `[Workspace]`+`[Base]`+`[System]`+`[Team Instructions]`+
  `[Agent Memory]`+`[Channel Canvas]`, `pool.rs:742-797`; only *delivery*
  differs and no path omitted rule 1), per-runtime config (args, env, permission
  handling — none adds or removes a loop guard), and persona content. Remaining
  hypothesis: **literal compliance** — Codex read "MUST publish a reply" as
  absolute; Claude Code applied judgment and quietly *violated* rule 1, and that
  violation was the only thing preventing the loop. If so the loop was latent on
  every runtime and Claude Code's good behavior was luck. This is why §2 keeps a
  structural breaker on the backlog rather than trusting prose.
