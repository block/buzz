# Welcome Kickoff — Silent Failure Paths

Status: **open** — the *perception* gap is handled (see below); the silent
paths themselves are not. Documented for follow-up work.
Context: the Welcome-channel kickoff choreography
(`desktop/src/features/onboarding/welcomeKickoff.ts`) where Fizz posts an
opener, teammates introduce themselves in-thread, and Fizz posts a closer.

## The problem

Every fallback message in the kickoff assumes Fizz — the lead agent and
sender — is alive and able to post. When Fizz itself fails, or an early step
throws, **nothing is ever posted** and the user stares at an empty Welcome
channel with no explanation.

The client-side kickoff stage (the starter-team characters standing on the
Welcome composer banner) covers the *perception* gap, and that part has landed:
after `WELCOME_KICKOFF_STAGE_TIMEOUT_MS` (90s) with no message, the characters
play their exit and the banner drops back to its normal mention hint. A failed
kickoff degrades to an ordinary, usable empty channel rather than claiming a
team is still being set up.

What it does **not** do is explain anything — the silent paths below still need
real handling. Two things worth knowing before picking this up:

- The stage is driven purely by "is the timeline empty" plus that timer
  (`useWelcomeKickoffStage.ts`). It never reads the real kickoff state, so it
  cannot distinguish "Fizz crashed" from "the relay is slow" — the timeout is a
  perception backstop, not a diagnosis. Surfacing a cause means plumbing one out
  of `useWelcomeKickoff` (step 1 of the Sketch below).
- The empty channel it degrades to invites the user to `@`-mention Fizz — who,
  in exactly these failure cases, is the thing that isn't working. So the quiet
  timeout is honest but still a dead end.

## Message inventory (what the user CAN receive today)

All hard-coded client-side; only teammate intro replies are LLM-generated.

| # | Message | Trigger | Sender |
|---|---------|---------|--------|
| 1 | Provider fallback ("connect to an AI provider in Settings…") | Readiness check fails before kickoff | Fizz (marker: `provider-required.v1`) |
| 2 | Happy-path opener (mentions teammates, asks them to introduce themselves) | Team online | Fizz (marker: `opener.v1`) |
| 3 | Degraded opener ("I'm here with Honey and Bumble…") | Fizz online, zero teammates online within 60s | Fizz (opener + closer markers, self-contained) |
| 4 | Closer variants (clean / failed / slow teammate wording) | 3s beat after intros resolve, or 15s intro timeout | Fizz (marker: `closer.v1`) |
| 5 | Setup-mode nudge ("here's what you still need to configure") | Agent process spawns but its requirements check fails (e.g. missing API key) | The agent process itself (backend, buzz-acp setup-listener mode) |

## Silent paths (what the user CANNOT be told today)

1. **Fizz fails to start.** `startManagedAgent` for the lead rejects (harness
   binary missing, spawn error). The kickoff effect logs
   `Failed to start Welcome agent…` and returns — by design, the opener is
   only sent by Fizz, so nobody speaks.
2. **Any step throws.** The entire kickoff runs in one `try/catch` that logs
   `Failed to start the Welcome team kickoff.` and gives up. Causes seen in
   practice:
   - relay unreachable / websocket down
   - `ensureWelcomeTeam` failure (team record creation)
   - the send itself rejected — e.g. relay rate-limiting
     ("rate-limited: quota exceeded", observed 2026-07-17 with an agent
     publishing in a tight retry loop)
3. **Closer-path failures.** The closer send failing is also caught-and-logged
   only; the thread ends without the CTA. Lower stakes than 1–2 (an opener and
   intros already happened) but still a dangling state.

Navigation away mid-kickoff also cancels silently, but that is intentional
(the kickoff resumes on next visit) — not a failure.

## Constraints for the fix

- **Fizz cannot be the messenger** for these paths: she is the thing that
  failed. Any user-visible fallback must come from the client UI itself
  (banner, intro-block state, or the kickoff stage's `timed-out` phase) — not a
  channel message impersonating an agent.
- A relay-side or system-authored message is possible in principle
  (kind-scoped system event) but heavier; the client already knows locally
  that the kickoff threw, so a local UI state is the cheap, honest option.
- Whatever surfaces must be **idempotent across revisits** — same rule as the
  opener markers: don't re-alarm the user every time they click Welcome.
- Distinguish *retryable* (relay hiccup, rate-limit) from *actionable*
  (harness missing → point at Agents/Settings). The `Requirement` machinery
  in `desktop/src-tauri/src/managed_agents/readiness.rs` already classifies
  the actionable ones.

## Sketch (to validate later)

1. Surface a `kickoffError` phase from `useWelcomeKickoff` when the catch
   block fires or the lead's start rejects, with a coarse cause
   (`lead-start-failed` | `relay` | `unknown`).
2. The kickoff stage's `timed-out` phase renders that cause: quiet copy + a
   pointer to Agents (for start failures) or a retry affordance (for relay
   failures). Retry = re-run the effect (the coordinator already dedupes).
   Note the phase currently exits immediately on timeout — giving it copy to
   show means holding it on screen instead, and the stage is `aria-hidden`
   decoration today, so anything it says needs to reach screen readers too.
3. Consider a bounded auto-retry (once, after a short delay) for the relay
   class before showing anything.
4. Closer-path failure: on send rejection, retry once; otherwise leave the
   thread as-is (intros already delivered the core experience).

## Related

- Rate-limiting incident: one Welcome agent produced a 42KB log of
  "rate-limited: quota exceeded" retries within seconds
  (2026-07-17, remote relay `onboarding.communities.buzz.xyz`). Worth a
  separate look at buzz-acp publish backoff — a tight retry loop against a
  quota makes every other send in the session fail too.
