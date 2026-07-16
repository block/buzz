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
7. **Closer is templated, gated on resolution (not just success).** The
   closer waits until every mentioned agent has **resolved** — its intro
   arrived, OR it was detected as failed (status `stopped` + `last_error`
   after start). Then Fizz sends the templated closer: *"What can we help
   you build? Bring us something you're working on, or give us a quick
   challenge to see how we work together."* If one agent failed, the closer
   is preceded by one templated Fizz-voiced aside ("Bumble's having trouble
   starting — you can check on it in Agents"). If **both** failed, send a
   Fizz-voiced recovery message that acknowledges the missing teammates and
   itself ends with the CTA (the recovery message *is* the closer) — the
   experience degrades to today's solo-Fizz welcome plus one honest
   sentence. One state machine covers healthy/partial/total failure. No
   generic timeout fallback in v1 — telemetry the "unresolved" stall case.
8. **Canvas is app-seeded at channel creation** (not part of the
   choreography). Light v1 content: what the Welcome channel is for, how to
   work with agents, try-something prompts, links to help/user guides, note
   that the agents can troubleshoot here. Agents may reference it in intros.
9. **No provider configured → placeholder, not kickoff.** If
   `resolveAgentReadiness()` fails, do NOT send the opener. Instead post a
   templated placeholder: *"To get started with agents, connect to an AI
   provider in settings."* (unmarked-by-kickoff-markers, so the kickoff still
   counts as not-run). On every subsequent Welcome focus the gate
   re-evaluates — once a provider exists, the kickoff runs normally.
10. **Existing users get the fresh kickoff.** Users with the old solo-Fizz
    welcome (`buzz-welcome-intro.v1`) still get the new team kickoff — the
    new markers are distinct, so this is automatic. The channel-reuse
    membership check must tolerate adding Honey/Bumble pubkeys.
11. **Starter agents are not renameable/editable during onboarding.** Persona
    IDs and names are stable — no drift concerns for the team record,
    mentions, or templated copy.
12. **No loop-prevention machinery in v1.** The opener is the only message
    carrying agent mention tags; intros aren't asked to mention anyone.
    `ignore_self` covers self-loops. Keep it simple unless real-world runs
    show chatter.

## Sequence

```
onboarding completes (colleague's splash flow adds/edits the 3 agents)
  └─ ensure Welcome channel + Welcome Team + 3 managed agents attached (bots)
  └─ seed canvas (marker/idempotent — canvas set is full-replace, only seed if empty)
user focuses Welcome channel for the first time
  └─ query channel for kickoff markers → none found
  └─ gate: resolveAgentReadiness()
       ├─ NOT ready → send templated placeholder ("connect an AI provider
       │  in settings"); stop. Re-evaluate on next focus.
       └─ ready ↓
  └─ START Honey + Bumble buzz-acp processes (await the start command —
     must be issued BEFORE the opener; buzz-acp's startup watermark replays
     events published after process start, but not before)
  └─ send Fizz opener (synthetic, marker: buzz-welcome-kickoff.opener.v1,
     markerScope: channel, mention p tags for Honey + Bumble) — no need to
     wait for subscription readiness; backfill covers the gap
  └─ Honey + Bumble respond live (real turns, simultaneous)
  └─ orchestrator waits until each mentioned agent RESOLVES:
       intro message from its pubkey arrived, OR failure detected
       (status stopped + last_error after start — poll/refetch, no push event)
  └─ send Fizz closer (synthetic, marker: buzz-welcome-kickoff.closer.v1)
       ├─ both intro'd → plain closer CTA
       ├─ one failed → one templated aside about the missing teammate + CTA
       └─ both failed → recovery message (acknowledges missing teammates,
          ends with CTA) — degrades to solo-Fizz welcome + one honest line
done — markers on relay prevent any re-run, ever
```

### Resume logic (app quit mid-sequence)

On every Welcome focus, evaluate readiness + marker state:

| readiness | opener marker | closer marker | agents resolved | action |
|---|---|---|---|---|
| not ready | absent | — | — | send/keep placeholder message; stop |
| ready | absent | — | — | run full sequence |
| ready | present | absent | 0 or 1 of 2 | (re)start unresolved agents; wait for intro or failure |
| ready | present | absent | 2 of 2 | send closer (variant per failure count) |
| — | present | present | — | do nothing |

"Resolved" = intro message from the agent's pubkey exists after the opener,
OR the agent is `stopped` with a `last_error` after our start attempt. The
placeholder message uses its own marker so it isn't re-sent on every focus,
but does NOT count as the opener.

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
  - Readiness gate first: `resolveAgentReadiness()`
    (`agentReadiness.ts:22`). Not ready → send placeholder (own marker),
    stop.
  - Marker-existence read: **new read-only Tauri command** wrapping the
    existing private helper `find_managed_agent_channel_message_by_marker`
    (Rust `commands/messages.rs:569`). Relay query is NOT viable — `client`
    is a multi-letter tag and Nostr filters only support single-letter tag
    queries (`bridge.rs:202`). ~15 lines; keeps marker/scope semantics in
    one place.
  - START Honey + Bumble via the managed-agent start command and **await
    it before sending the opener** (buzz-acp's `startup_watermark`,
    `lib.rs:1220`, replays events published after process start with
    `since = watermark − 5s`; events published before start are lost). No
    subscription-ready wait needed — backfill covers the gap. `status:
    "running"` = process-alive only; that suffices.
  - Send opener via `sendManagedAgentChannelMessage`
    (`shared/api/tauriManagedAgentMessages.ts`) with mention `p` tags.
    **Small extension required**: the Rust command hardcodes mentions to
    `&[]` when calling `build_message_with_client_tags` (`events.rs:317`);
    add `mention_pubkeys: Option<Vec<String>>` to the command +
    `mentionPubkeys?: string[]` to the TS wrapper. The builder's
    `mention_tags` (`events.rs:63`) already validates/dedupes. No
    `nostr:npub` content formatting needed — buzz-acp's gate checks only
    the `p` tag (`filter.rs:390`) and UI pills resolve `@Name` from `p`
    tags (`remarkMentions.ts` / `resolveMentionNames.ts`).
  - Resolution loop: subscribe to channel events for intros; for failure
    detection, poll/refetch the managed-agents query (no push event exists;
    status model is `running/stopped` — no `error` status; failure =
    `stopped` + `last_error`/`last_exit_code`, `runtime.rs:1152-1188`).
    Reuse `friendlyAgentLastError` for any user-facing failure copy.
  - Closer variants per resolution outcome (§ Product decision 7): plain /
    one-teammate-missing aside / both-failed recovery-as-closer.
  - Stall telemetry: if any agent unresolved after N minutes, log (no
    generic timeout fallback in v1).

### C. Intro-turn quality (buzz-acp / prompts)

- The opener's text is the only instruction Honey/Bumble get — their turn is
  a normal mention-triggered response. Craft opener wording so a generic
  agent produces a short intro (e.g. Fizz explicitly asks: "introduce
  yourself in a sentence or two — what you're good at and when to bring you
  in. Don't start any work yet.").
- Risk: user-edited instructions may produce long/weird intros. Acceptable
  for v1; revisit a one-turn kickoff prompt seam in buzz-acp
  (`prompt_tag` in `filter.rs`) only if needed.
- Loop safety (decided): no machinery in v1 — the opener template is the
  only message with agent `p` tags, intros are asked to mention no one, and
  `ignore_self` covers self-loops. Revisit only if real runs show chatter.

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
- Existing users (decided): everyone with the old Fizz welcome gets the new
  team kickoff — new markers are distinct, so no migration needed; just
  ensure the channel-reuse membership check tolerates the added pubkeys.

### F. Tests

- Unit: marker-state resume table (B), team find-or-create idempotence (A).
- E2E (Playwright, `desktop/tests/e2e/`): mock-bridge spec that simulates
  first Welcome focus → opener appears → inject mock Honey/Bumble messages →
  closer appears; and a resume-state spec. Existing onboarding E2E tests
  hardcode Fizz — update.

## Engineering questions — RESOLVED

(Details in `WELCOME_TEAM_KICKOFF_TECH_MAP.md` § Investigation answers.)

1. **Mentions in synthetic sends** — not supported today; trivial extension
   (thread `mention_pubkeys` through `send_managed_agent_channel_message` →
   `build_message_with_client_tags`; TS wrapper gains `mentionPubkeys`). No
   content formatting needed for gate or UI pills.
2. **Marker read** — new ~15-line read-only Tauri command wrapping the
   existing Rust helper. Relay query not viable (multi-letter `client` tag
   can't be filtered).
3. **Cold start** — start agents (await command), then send opener
   immediately; buzz-acp's startup-watermark backfill replays mentions
   published after process start. No ready-signal wait.
4. **Loops** — decision: no machinery in v1. Opener is the only message with
   agent mention tags; intros mention no one; `ignore_self` covers
   self-loops.
5. **Provider readiness** — gate on `resolveAgentReadiness()`; not ready →
   templated placeholder ("connect an AI provider in settings"), kickoff
   re-evaluates on next focus.
6. **Existing users** — everyone with the old solo-Fizz welcome gets the new
   kickoff (new markers are distinct — automatic). Channel-reuse membership
   check updated to include Honey/Bumble pubkeys (workstream A).
7. **Renaming** — moot: starter agents are not editable during onboarding.
8. **Agent failure handling (new)** — no `error` status; failure =
   `stopped` + `last_error` after start, detected by polling. Closer gates
   on per-agent *resolution* (intro or failure); variants: plain / one
   missing / both failed (recovery-as-closer). Degrades to solo-Fizz
   welcome + one honest sentence.

## Explicitly out of scope (v1)

- Timeout/synthetic fallback for stalled intros (telemetry only).
- Sequenced (gated) intros — simultaneous first.
- Post-kickoff activation state machine (task → specialist → handoff journey
  from the earlier research) — separate workstream.
- Onboarding splash flows for adding/editing the 3 agents (colleague's work).
