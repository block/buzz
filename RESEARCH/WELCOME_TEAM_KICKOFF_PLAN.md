# Welcome Team Live Kickoff — Implementation Plan

Branch: `morganm/welcome-team-kickoff`
Companion research: `RESEARCH/WELCOME_TEAM_KICKOFF_TECH_MAP.md`

## Product decisions (locked)

1. **Starter trio replaces the solo-Fizz welcome.** Fizz stays as the lead;
   Honey and Bumble (merged in PR #1925 as built-in personas) join him. The
   old solo-Fizz intro flow is retired, not deleted.
2. **Welcome Team**: the three personas are grouped as a kind-30176 Team
   ("Welcome Team") using the existing Teams primitive and the existing
   team-deploy batch path (provision managed agent per persona + attach as
   bot member).
3. **Trigger**: the kickoff runs the **first time the Welcome channel is
   focused post-onboarding** — never before, never if the user doesn't visit.
4. **Persistence is identity-keyed via the relay**: "has the kickoff run?" is
   determined by the presence of marker-tagged kickoff messages in the
   Welcome channel (`["client", ...]` tags), not local storage. Survives
   reinstalls and new machines; per-pubkey by construction.
5. **Opener is templated, sent live.** Fizz's opening message is app-authored
   (synthetic, via `sendManagedAgentChannelMessage`) but sent at the moment
   of first focus so the user watches it arrive. It @mentions Honey and
   Bumble and asks them to introduce themselves.
6. **Intros are real model turns, simultaneous.** Both Honey and Bumble fire
   naturally off the opener's mentions (sibling-gated, mention-triggered) and
   land in whatever order they land. No app-side gating between them (v1).
7. **Closer is templated, gated on both intros.** After messages from both
   Honey's and Bumble's pubkeys arrive, Fizz sends the templated closer:
   *"What can we help you build? Bring us something you're working on, or
   give us a quick challenge to see how we work together."* No timeout
   fallback in v1 — but log/telemetry the stall case so we know if we need one.
8. **Canvas is app-seeded at channel creation** (not part of the
   choreography). Light v1 content: what the Welcome channel is for, how to
   work with agents, try-something prompts, links to help/user guides, note
   that the agents can troubleshoot here. Agents may reference it in intros.

## Sequence

```
onboarding completes (colleague's splash flow adds/edits the 3 agents)
  └─ ensure Welcome channel + Welcome Team + 3 managed agents attached (bots)
  └─ seed canvas (marker/idempotent — canvas set is full-replace, only seed if empty)
user focuses Welcome channel for the first time
  └─ query channel for kickoff markers → none found
  └─ spawn Honey + Bumble buzz-acp processes (background, ASAP)
  └─ send Fizz opener (synthetic, marker: buzz-welcome-kickoff.opener.v1,
     markerScope: channel, mentions @Honey @Bumble)
  └─ Honey + Bumble respond live (real turns, simultaneous)
  └─ orchestrator observes ≥1 post-opener message from each of Honey's and
     Bumble's pubkeys
  └─ send Fizz closer (synthetic, marker: buzz-welcome-kickoff.closer.v1)
done — markers on relay prevent any re-run, ever
```

### Resume logic (app quit mid-sequence)

On every Welcome focus, evaluate marker state:

| opener marker | closer marker | intros present | action |
|---|---|---|---|
| absent | — | — | run full sequence |
| present | absent | 0 or 1 | (re)spawn missing agents; wait for intros |
| present | absent | 2 | send closer |
| present | present | — | do nothing |

## Workstreams / files to touch

### A. Welcome Team + trio provisioning (extends `welcomeGuide.ts` pattern)

- `desktop/src/features/onboarding/welcomeGuide.ts` → generalize from
  single-Fizz to the trio. New constants: persona IDs
  `builtin:fizz|honey|bumble`, kickoff marker names.
- Create "Welcome Team" (kind 30176, `persona_ids` = the trio). Reuse
  team-create Tauri path used by `TeamDialog.tsx` / `useTeamActions.ts`;
  idempotent find-or-create by name+persona set (decide a stable team `d` tag,
  e.g. `welcome-team`, so it's replaceable not duplicated).
- Provision Honey + Bumble managed agents via the existing team-deploy batch
  path (`desktop/src/features/agents/channelAgents.ts` —
  `findReusableAgent` / `createManagedAgent` + `attachManagedAgentToChannel`).
  Same settings as Fizz today: `respondTo: "owner-only"`,
  `startOnAppLaunch: false`. `spawnAfterCreate: false` at provision time;
  spawn happens at kickoff.
- `desktop/src/features/onboarding/welcome.ts` → `allowedMemberPubkeys` must
  include all three agent pubkeys for `isPrivateWelcomeChannel` reuse checks.

### B. Kickoff orchestrator (new)

- New module, e.g. `desktop/src/features/onboarding/welcomeKickoff.ts`:
  - `maybeRunWelcomeKickoff(channelId)` invoked from the channel-focus path
    (find where Welcome focus is detected — the
    `buzz:onboarding-welcome-channel-ready` event + ChannelPane focus are the
    existing seams in `hooks.ts` / `ChannelPane.tsx`).
  - Marker query: reuse `find_managed_agent_channel_message_by_marker`
    (Rust `commands/messages.rs:569`) — likely needs a read-only Tauri
    command exposing "does marker exist in channel" to TS, or query kind-9
    events with `["client", marker]` tags via the relay client.
  - Spawn Honey + Bumble (and Fizz if we want him live for follow-ups) via
    managed-agent start command as the first act.
  - Send opener via `sendManagedAgentChannelMessage`
    (`shared/api/tauriManagedAgentMessages.ts`) with mention `p` tags for
    Honey + Bumble — **verify the synthetic send path supports `p` tags /
    mention formatting**; if not, extend the Rust command
    (`commands/messages.rs:667`) to accept mention pubkeys.
  - Subscribe to channel events; when ≥1 message from each of Honey/Bumble
    pubkeys with `created_at` after the opener exists, send closer.
  - Stall telemetry: if intros incomplete after N minutes, log (no fallback
    behavior in v1).

### C. Intro-turn quality (buzz-acp / prompts)

- The opener's text is the only instruction Honey/Bumble get — their turn is
  a normal mention-triggered response. Craft opener wording so a generic
  agent produces a short intro (e.g. Fizz explicitly asks: "introduce
  yourself in a sentence or two — what you're good at and when to bring you
  in. Don't start any work yet.").
- Risk: user-edited instructions may produce long/weird intros. Acceptable
  for v1; revisit a one-turn kickoff prompt seam in buzz-acp
  (`prompt_tag` in `filter.rs`) only if needed.
- **Verify in `crates/buzz-acp/src/queue.rs`**: no runaway agent↔agent loop
  from the intros (intros shouldn't mention each other; `ignore_self` covers
  self-loops). Opener template should be the only message with agent `p` tags.

### D. Canvas seeding

- Seed at channel-ensure time in `initializeWelcomeChannel()`
  (`onboarding/hooks.ts:62`): publish kind 40100 via the desktop (owner key)
  — reuse whatever `ChannelCanvas.tsx` / `channels/hooks.ts` uses to set
  canvas, or the SDK builder (`buzz_sdk::build_set_canvas`).
- Idempotence: canvas set is full-replace — only seed when no canvas event
  exists for the channel.
- Content (markdown, keep light): what this channel is / how to work with
  agents / try-something prompts (duplicate the closer CTA here as the
  durable copy) / help links.

### E. Retire the solo-Fizz flow (hide, don't delete)

- Remove/bypass `ensureWelcomeGuideIntro()` (the `buzz-welcome-intro.v1`
  synthetic message) for new users; keep the code path or marker constant so
  existing channels aren't disturbed.
- Keep `builtin:fizz` persona active (he's the lead). What's retired is the
  single-agent intro + the Fizz-specific composer coachmark copy
  (`WelcomeComposerBanner.tsx`, ChannelPane coachmark) — update or remove.
- Migration question (below) for users who already have the old Fizz welcome.

### F. Tests

- Unit: marker-state resume table (B), team find-or-create idempotence (A).
- E2E (Playwright, `desktop/tests/e2e/`): mock-bridge spec that simulates
  first Welcome focus → opener appears → inject mock Honey/Bumble messages →
  closer appears; and a resume-state spec. Existing onboarding E2E tests
  hardcode Fizz — update.

## Open engineering questions (Trace-level review)

1. Does `sendManagedAgentChannelMessage` support mention `p` tags today? If
   not, smallest extension to the Rust command.
2. Cleanest TS-side "does marker exist in channel" read — new Tauri command
   vs. relay query from the frontend?
3. Cold-start reality check: measured time from `start agent` → buzz-acp
   ready to receive a mention. Does the opener need to wait for spawn
   confirmation before sending, or is the relay queue durable enough that
   mentions sent pre-ready are still consumed?
4. `queue.rs` agent↔agent loop behavior — confirm no unbounded chain risk
   when two agents are mentioned in one message.
5. Provider readiness gating (`agentReadiness.ts`): what happens if the user
   reaches Welcome with no provider configured? Kickoff should no-op (not
   half-run) until readiness — where's the right gate?
6. Existing users / migration: users with the old `buzz-welcome-intro.v1`
   channel — do they get the new kickoff (probably yes, markers differ), and
   does the channel-reuse membership check break when we add two more agent
   pubkeys?
7. Team `d` tag stability + behavior if the user renamed/edited the starter
   agents during onboarding — team groups persona IDs, so edits should be
   transparent; confirm persona edits don't change IDs.

## Explicitly out of scope (v1)

- Timeout/synthetic fallback for stalled intros (telemetry only).
- Sequenced (gated) intros — simultaneous first.
- Post-kickoff activation state machine (task → specialist → handoff journey
  from the earlier research) — separate workstream.
- Onboarding splash flows for adding/editing the 3 agents (colleague's work).
