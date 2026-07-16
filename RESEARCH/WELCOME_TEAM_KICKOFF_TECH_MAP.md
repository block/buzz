# Welcome Team Live Kickoff — Technical Map

Read-only research pass for the "live multi-agent welcome experience": 3 starter
agents (Fizz, Honey, Bumble) form a Welcome Team; on first landing in the
Welcome channel they introduce themselves LIVE, ending with an invitation to
the user. This document maps what exists on `main` and what needs hooking up.

---

## 1. PR #1925 — Starter Agents (MERGED)

**Title:** "feat(desktop): add Honey and Bumble starter agents" — author
wesbillman, approved by morgmart, **state: MERGED**.

What it does (49 additions / 23 deletions, 4 files):

- `desktop/src-tauri/src/managed_agents/personas.rs` — adds two new
  `BuiltInPersona` entries to the `BUILT_IN_PERSONAS` const array:
  - `builtin:honey` ("Honey", `HONEY_SYSTEM_PROMPT`, `default_active: true`)
  - `builtin:bumble` ("Bumble", `BUMBLE_SYSTEM_PROMPT` — "curious and
    adventurous researcher", `default_active: true`)
  - Both use `model: None`, `runtime: None` (inherit global provider/model).
  - "Bumble" and "Honey" removed from Fizz's `name_pool` so the names are
    reserved for the dedicated personas.
- `desktop/src-tauri/src/managed_agents/personas/tests.rs` — active built-ins
  are now `["builtin:fizz", "builtin:honey", "builtin:bumble"]`.
- `desktop/src/features/agents/lib/useBotRecents.ts:10` —
  `DEFAULT_PERSONA_NAMES = ["Fizz", "Honey", "Bumble"]` seeds quick-add
  surfaces (`pickQuickBotPersonas`).
- Avatar assets bundled for both.

**Key takeaways:**

- Starter agents are **personas** (built-in persona records, Rust const),
  merged into the persona store via `merge_personas_adds_missing_built_ins`
  logic in `personas.rs`. They are *definitions*, not running agents.
- **No Team is created by #1925.** There is no "Welcome Team" anywhere yet —
  the trio only shows up together in quick-add ordering.
- **No managed agents are created** for Honey/Bumble at onboarding; only Fizz
  gets a managed agent (see §2). Creating + attaching Honey/Bumble agents and
  a Team wrapping the three personas is TODO for the welcome experience.

---

## 2. Current Welcome/Onboarding Flow (main)

### Welcome channel — `desktop/src/features/onboarding/welcome.ts`

- `ensureWelcomeChannel()` (welcome.ts:144) finds-or-creates a **private
  stream channel named "Welcome"** (`welcomeChannelInput`, welcome.ts:37).
  Reuse conditions: `isPrivateWelcomeChannel` (welcome.ts:83) — name/type/
  visibility match, not archived, current user is a member, and members are
  only the current user + allowed pubkeys (the welcome-guide agent pubkeys
  passed in as `allowedMemberPubkeys`).
- Idempotence: `markWelcomeChannelEnsured()` writes localStorage key
  `buzz-welcome-channel-ensured.v2:<communityScope>:<pubkey>`
  (welcome.ts:19,163).
- Focus dance: `rememberPendingWelcomeChannel` (sessionStorage, 5-min TTL,
  welcome.ts:255) + `notifyWelcomeChannelReady` custom DOM event
  (`buzz:onboarding-welcome-channel-ready`, welcome.ts:305) + initial-unread
  suppression key.

### Fizz guide — `desktop/src/features/onboarding/welcomeGuide.ts`

- Constants: `WELCOME_GUIDE_PERSONA_ID = "builtin:fizz"` (line 13),
  `WELCOME_GUIDE_INTRO_MARKER = "buzz-welcome-intro.v1"` (line 14), hardcoded
  `WELCOME_GUIDE_INTRO_MESSAGE` (line 18).
- `ensureWelcomeGuideAgent()` (welcomeGuide.ts:106): reuses an existing
  managed agent named "Fizz" with `personaId builtin:fizz` scoped to the relay
  (`pickWelcomeGuideAgentForRelay`, status preference running > deployed >
  first), otherwise activates the persona (`setPersonaActive`) and calls
  `createManagedAgent` with:
  - **`spawnAfterCreate: false`** — Fizz is created *stopped*
  - **`startOnAppLaunch: false`**
  - **`respondTo: "owner-only"`**
- `ensureWelcomeGuideMembership()` (welcomeGuide.ts:127): `addChannelMembers`
  with `role: "bot"`; tolerates "already a member" errors.
- `ensureWelcomeGuideIntro()` (welcomeGuide.ts:152): the intro message is
  **synthetic** — sent *as the agent* by the desktop backend via
  `sendManagedAgentChannelMessage` (`shared/api/tauriManagedAgentMessages.ts:12`),
  not by a live agent turn. Idempotence via `marker: WELCOME_GUIDE_INTRO_MARKER`,
  `markerScope: "channel"`.

### Marker mechanism (Rust) — `desktop/src-tauri/src/commands/messages.rs`

- `event_has_client_marker` (messages.rs:562) — the marker rides in a
  `["client", "<marker>"]` tag on the kind-9 event.
- `find_managed_agent_channel_message_by_marker` (messages.rs:569) queries the
  channel for an existing marked message before sending;
  `marker_author_for_scope` (messages.rs:619) — with `markerScope: "channel"`
  ANY author's marked message counts (survives Fizz being recreated with a new
  pubkey). This is the reusable "send-once as agent" primitive.

### Orchestration — `desktop/src/features/onboarding/hooks.ts`

- `initializeWelcomeChannel()` (hooks.ts:62): gets guide pubkeys →
  `ensureWelcomeChannel` → `ensureWelcomeGuideIntro` → invalidates
  managed/relay agent + channel queries → marks ensured → focuses channel.
- Fizz is left **stopped**; he only spins up later when the user actually
  talks (agent lifecycle handles spawn on demand / user start). The intro
  bubble is fake-authored by the desktop as the agent's key.

---

## 3. Teams Primitive

- **Relay kind:** `KIND_TEAM = 30176` (`crates/buzz-core/src/kind.rs:175`),
  NIP-AP, parameterized-replaceable, owner-authored. Addressed by
  `(pubkey, 30176, d_tag=team-id)`; JSON content projects
  `{name, description, persona_ids}`. Mirrors `KIND_PERSONA = 30175`
  (kind.rs:165) and `KIND_MANAGED_AGENT = 30177` (kind.rs:183).
- **A team groups *personas*, not running agents.** Membership = `persona_ids`
  in the team event content.
- **Desktop UI:** `desktop/src/features/agents/ui/` — `TeamDialog.tsx`
  (create/edit), `TeamDeleteDialog.tsx`, `TeamShareDialog.tsx`,
  `TeamIdentityCard.tsx`, `TeamSnapshotImport/ExportDialog.tsx`,
  `useTeamActions.ts`.
- **Deploy-to-channel:** `useTeamActions.ts:handleTeamDeployed` (~line 142)
  consumes `CreateChannelManagedAgentsResult` from
  `desktop/src/features/agents/channelAgents.ts` — deploying a team to a
  channel means: for each persona in the team, find-or-create a managed agent
  (`findReusableAgent` from `agentReuse.ts`, else `createManagedAgent`), then
  `addChannelMembers` (`attachManagedAgentToChannel`, channelAgents.ts:107).
  So "attach team to channel" ≈ batch (provision agent + add as bot member).
- **No CLI team commands** in `buzz-cli` (checked `crates/buzz-cli/src/lib.rs`
  — only notes mention "team knowledge base"). Team creation is a desktop
  concern (Tauri commands + kind-30176 publish).

**Gap:** nothing creates a "Welcome Team" today. We'd create a team record
with `persona_ids = [builtin:fizz, builtin:honey, builtin:bumble]` and reuse
the existing team-deploy batch path in onboarding.

---

## 4. Agent Runtime & Live-Kickoff Mechanics

### How agents run

- Managed agents are **desktop-spawned subprocesses of `buzz-acp`** (the ACP
  harness). `desktop/src-tauri/src/managed_agents/runtime.rs:30` lists
  `"buzz-acp"` among owned process names; runtime.rs:1547 notes bundled
  sidecars (`buzz`, `buzz-acp`) live in the app bundle
  (`Contents/MacOS/`). Process spawn/reclaim/orphan-sweep is in
  `runtime.rs` + `process_lifecycle.rs` (buzz-acp spawns ~24 workers before
  relay connect, process_lifecycle.rs:49).
- buzz-acp bridges Buzz relay events → an ACP-compliant agent (goose etc.),
  with the persona system prompt layered on `crates/buzz-acp/src/base_prompt.md`.
- So on first launch, compute is **local**: desktop spawns buzz-acp with the
  persona env; the agent runtime (e.g. goose w/ configured provider/model)
  does the inference. Today Fizz doesn't "respond" live at all during
  onboarding — his intro is synthetic (§2). He responds live only once
  spawned and the user messages the channel.

### What makes an agent respond

Two gates in buzz-acp:

1. **Author gate** (`crates/buzz-acp/src/lib.rs:186 author_allowed`):
   `respond_to` mode — `owner-only` (default; managed agents are created with
   this) accepts the owner **and same-owner sibling agents**
   (`is_owner_or_sibling`, lib.rs:200; sibling proof via the author's kind:0
   NIP-OA auth tag, `check_sibling_via_profile`, lib.rs:~210). Also
   `allowlist`, `anyone`, `nobody` (`config.rs:88-110`).
2. **Subscription rules** (`crates/buzz-acp/src/filter.rs`): per-rule
   `require_mention` (filter.rs:93,390) — when true, the event must carry a
   `p` tag equal to the agent's pubkey. Rules map matches to a `prompt_tag`.
   Plus `ignore_self` (lib.rs:1805) so an agent never triggers on its own
   events.

**Agent-to-agent mentions DO work under owner-only**: all three starter
agents share the same owner, so Fizz mentioning Honey passes Honey's author
gate as a sibling; if Honey's subscription requires mention, the `p` tag on
Fizz's message satisfies it. This is the natural relay-native chaining
primitive for the kickoff.

**Loop prevention:** `ignore_self` + the reply-anchor logic
(`turn_is_human_facing`, `crates/buzz-acp/src/queue.rs:1151`) which treats
agent→agent turns specially (anchor = None, tests
`test_anchor_agent_to_agent_*`, queue.rs:3263+). I did **not** find an
explicit agent↔agent turn-count limiter in this pass — worth verifying in
`queue.rs` before designing an open-ended mention chain; the scripted kickoff
should terminate by prompt design (each agent told exactly what to do and
whom to mention, with the last message mentioning no agent).

### Hidden / one-turn instruction (no permanent prompt change)

Options that exist:

- **Synthetic agent-authored messages** —
  `sendManagedAgentChannelMessage(marker, markerScope)`
  (`tauriManagedAgentMessages.ts:12`, Rust in `commands/messages.rs:667`):
  the app posts *as* the agent, idempotently. No LLM involved. This is how
  today's intro works; can script the whole choreography deterministically.
- **Per-rule `prompt_tag`** (filter.rs) lets different triggers select
  different prompt framings inside buzz-acp — a possible seam for a
  "kickoff" instruction, but wiring a dedicated one-shot prompt was not found
  as an existing feature.
- **Owner DM / owner message with instructions**: since respond_to is
  owner-only, the desktop (owner key) can post an instruction message the
  agent will treat as a turn. Posting it in-channel is user-visible; a DM to
  the agent asking it to "go introduce yourself in channel X" is the closest
  existing invisible-nudge path (agents can act cross-context via buzz CLI
  tools per base_prompt.md).
- **buzz-workflow** (`crates/buzz-workflow/src/schema.rs`): triggers
  `MessagePosted` (evalexpr filter), `ReactionAdded`, `DiffPosted`,
  `Schedule` (cron/interval); actions `SendMessage`, `SendDm`,
  `SetChannelTopic`, `AddReaction`, `CallWebhook`, `RequestApproval`
  (schema.rs:38-140). **No "prompt an agent" action** — workflows send static
  templated text, they don't invoke LLM turns. Could sequence static
  messages, but agents mentioning each other already does live sequencing.

### Sequencing options (lead → 2 → 3 → lead)

Nothing exists as a "conversation script" engine. Viable seams:

1. **Prompt-chained mentions** (fully live): desktop posts one owner-authored
   kickoff instruction (or DM) to Fizz → Fizz's turn output mentions Honey →
   Honey's sibling-gated mention-triggered turn mentions Bumble → Bumble
   mentions Fizz → Fizz closes with the question. Ordering emerges from the
   mention chain; each agent's persona/kickoff prompt encodes its one step.
   Risk: LLM non-compliance, cold-start latency of 3 spawns.
2. **Desktop-orchestrated state machine** (semi-live): a TS orchestrator in
   onboarding watches channel events; after each agent's message it nudges
   the next (marker-idempotent per step, e.g. `buzz-welcome-kickoff.step2.v1`).
   Deterministic ordering, resumable across restarts.
3. **Fully synthetic** (today's pattern × 5 messages): scripted
   `sendManagedAgentChannelMessage` calls with staggered timing — zero
   compute, zero latency risk, but not "live" (fails the brief unless used
   as fallback).

---

## 5. Channel Canvas

- **Kind:** `KIND_CANVAS = 40100` (`crates/buzz-core/src/kind.rs:359`) —
  "Canvas (shared document) for a channel". Registered in the kind table
  (kind.rs:563) and ≤ u16 assert (kind.rs:728).
- **CLI:** `buzz canvas get|set` (`crates/buzz-cli/src/lib.rs:173,606-612`;
  impl `crates/buzz-cli/src/commands/channels.rs:266 cmd_get_canvas`,
  `:564 cmd_set_canvas` using `buzz_sdk::build_set_canvas`, content from arg
  or stdin). **Agents can populate the canvas programmatically** — the CLI is
  in every agent's toolbox, and the desktop could sign/submit the same event.
- **Desktop UI:** `desktop/src/features/channels/ui/ChannelCanvas.tsx` +
  `channels/hooks.ts`. Set-canvas is a full replace (whole-document event).
- For the welcome guide doc: the desktop (owner key) can seed a canvas at
  channel-ensure time, or the kickoff instruction can tell Fizz to write it
  via `buzz canvas set`.

---

## 6. Hiding vs. Deleting Fizz / Personas

- **Personas** have `is_active` (`personas.rs:243,291,355,370`;
  `setPersonaActive` TS API in `shared/api/tauriPersonas.ts`, used by
  `welcomeGuide.ts:102`). Deactivating hides a persona from pickers without
  deleting; `merge_personas` re-adds missing built-ins but preserves the
  user's active flag. `PersonaActionsMenu.tsx` exposes this in UI.
- **Managed agents** have lifecycle status (running/deployed/stopped) and can
  be deleted; no dedicated "hidden" flag found on managed agents — hiding a
  starter agent ≈ deactivate its persona + stop/remove the managed agent, or
  simply remove it from the channel. **Channels** have `archivedAt`
  (welcome.ts:79) but agents don't have an archived flag in what I inspected.
- Recommendation seam: persona `is_active=false` is the existing "hide"
  primitive; anything richer (e.g. "hidden but attached") would be new.

---

## Gaps Summary (what must be built)

1. **Welcome Team creation** — nothing creates a kind-30176 team for the trio;
   need to create it at onboarding and reuse team-deploy batch provisioning
   for Honey + Bumble managed agents (only Fizz is provisioned today).
2. **Live spawn at onboarding** — Fizz is created with
   `spawnAfterCreate: false`; a live kickoff requires spawning all three
   buzz-acp processes (cold-start latency, provider config must exist —
   check `agentReadiness.ts` gating in onboarding/ui/).
3. **Kickoff instruction channel** — no one-shot/hidden prompt mechanism;
   need either an owner-authored trigger message/DM, a new "kickoff prompt"
   seam in buzz-acp, or a desktop orchestrator using the existing marker-
   idempotent send path per step.
4. **Sequencing/termination guarantees** — mention-chaining works via the
   sibling gate, but there's no script engine or explicit agent↔agent turn
   limit; ordering + a guaranteed ending needs design (verify loop behavior in
   queue.rs before shipping).
5. **Canvas seeding** — mechanically trivial (kind 40100, `buzz canvas set`),
   but no onboarding hook writes it today; decide desktop-seeded vs.
   Fizz-written.

## Investigation answers (Q1–Q4)

### Q1. Mentions in synthetic sends — NOT supported today; small, safe extension

**Current path:** `sendManagedAgentChannelMessage`
(`desktop/src/shared/api/tauriManagedAgentMessages.ts:12`) passes only
`agentPubkey, channelId, content, marker, markerScope` to the Rust command
`send_managed_agent_channel_message` (`desktop/src-tauri/src/commands/messages.rs:667`).
The Rust side builds the event via `events::build_message_with_client_tags(channel_uuid,
trimmed, None, &[], &[], &[], &[], &client_tags)` (messages.rs:~745) — **the
mentions slot (4th arg) and mention-ref slot are hardcoded to `&[]`**. Tags on
the sent kind-9 event today: `["h", channel]` + optional `["client", marker]`.
No `p` tags → buzz-acp's `require_mention` gate (`crates/buzz-acp/src/filter.rs:390`,
checks for a `p` tag equal to the agent pubkey) would never fire off a synthetic opener.

**The builder already supports mentions.** `build_message_with_client_tags`
(`desktop/src-tauri/src/events.rs:317`) calls `mention_tags(mentions)`
(events.rs:63), which validates pubkeys, dedupes, enforces `MAX_MENTIONS`, and
emits lowercase `["p", <hex>]` tags. The user-authored send path
(`send_channel_message`, messages.rs:478,491) already threads
`mention_pubkeys: Option<Vec<String>>` through this exact function.

**Smallest extension:**
1. Rust: add `mention_pubkeys: Option<Vec<String>>` to
   `send_managed_agent_channel_message` (messages.rs:667), mirror the
   conversion at messages.rs:491 (`let mention_refs: Vec<&str> = ...`), and pass
   `&mention_refs` as the 4th arg to `build_message_with_client_tags`.
2. TS: add optional `mentionPubkeys?: string[]` to the input of
   `sendManagedAgentChannelMessage` and forward it in the `invokeTauri` payload.

**Content formatting:** buzz-acp's gate needs ONLY the `p` tag — no
`nostr:npub` in content required. The desktop UI mention pill also doesn't use
npub URIs: `remarkMentions` (`desktop/src/shared/lib/remarkMentions.ts`) matches
plain `@Name` text against `mentionNames` derived from the event's `p`/`mention`
tags via `resolveMentionProps` (`desktop/src/shared/lib/resolveMentionNames.ts:65`
— aliases from display name / kind-0 name / NIP-05 local part). So the opener
template just needs literal `@Honey @Bumble` in the content plus matching `p`
tags, and pills render + resolve automatically (provided the agents' kind-0
profiles carry those names).

### Q2. Marker-existence read from frontend — no existing read path; add a read-only Tauri command

- `find_managed_agent_channel_message_by_marker` (messages.rs:569) is a private
  helper used only inside the send command's idempotency check. **No Tauri
  command exposes it read-only.**
- No frontend relay-query path exists for `client` tags either. Nostr filters
  only support single-letter tag queries (`#h`, `#p`, `#e` …); `client` is a
  multi-letter tag. The relay's filter matcher (`crates/buzz-core/src/filter.rs:68`)
  iterates `filter.generic_tags`, and `nostr::Filter`'s generic_tags are keyed by
  `SingleLetterTag` (see usages in `crates/buzz-relay/src/api/bridge.rs:202,835`) —
  a `#client` filter is not expressible. This is exactly why the Rust helper
  itself queries by `kinds + #h + limit 500` and scans tags client-side
  (messages.rs:583-600, with pagination via `until`).
- The frontend relay client (`relayClientShared.ts:32 RelaySubscriptionFilter`)
  could replicate that query-and-scan in TS, but it would duplicate the
  pagination/scan logic and the trust boundary.

**Recommendation:** add a thin read-only Tauri command, e.g.
`find_managed_agent_channel_message_marker(channel_id, marker, agent_pubkey?) ->
Option<{event_id, pubkey, created_at}>`, wrapping the existing helper (with
`marker_author_for_scope` semantics, messages.rs:619). ~15 lines of Rust, zero
new logic, single source of truth for marker semantics, and the orchestrator's
resume table (opener/closer present?) becomes two awaits.

### Q3. Cold start & durability — opener can be sent immediately; backfill exists but sequence carefully

**Startup steps** (crates/buzz-acp/src/lib.rs main flow): spawn ~24 ACP agent
workers (before relay connect — `agent_pool_ready`, lib.rs:1216; noted in
process_lifecycle.rs:49) → capture `startup_watermark` (lib.rs:1220-1230) →
connect to relay → `set_startup_watermark` (lib.rs:1248) → subscribe membership
notifications (lib.rs:1255) → resolve channel filters → `subscribe_channel` per
channel (lib.rs:1382). Worker spawn + model/harness init dominates; expect
seconds, not ms.

**Backfill exists (Finding #22):** the watermark is captured BEFORE relay
connect, and the relay task issues the first REQ with `since = watermark - 5s`
instead of `since = now` (lib.rs:1244-1249; `crates/buzz-acp/src/relay.rs:924-947`,
`subscribe_channel_from` relay.rs:664, `last_seen` tracking relay.rs:896,962).
So any mention published **after the buzz-acp process starts** (even before its
subscription lands) is replayed. Reconnects use `min(last_seen,
channel_dropped_since)` (relay.rs:914,974). The relay itself stores events in
Postgres, so replay is durable, not best-effort.

**But**: a mention published *before the process starts* is before the
watermark and is NOT replayed (minus the 5s grace). Practical guidance:

- **Safe pattern:** issue the start command for Honey + Bumble first, then send
  the opener immediately — no need to await "subscribed". Once the process is
  up (watermark captured pre-connect), the opener is inside the replay window
  even if the subscription takes seconds more. The only hard ordering is
  "spawn call before send", plus a small margin (the ≥5s grace covers the
  spawn-syscall → watermark-capture gap in practice, but don't rely on it —
  fire spawn, await the Tauri start command's return, then send).
- **Ready signal:** there is no per-agent "subscribed" event surfaced to TS.
  The status model (Q4) flips to `"running"` when the child process is alive
  (runtime.rs:1291) — that's process-alive, not subscription-ready. buzz-acp
  logs `agent_pool_ready` / "subscribed to channel" but these aren't bridged as
  Tauri events. Given the watermark backfill, we don't need one for v1.

### Q4. Provider readiness & spawn failure

**Readiness:** `resolveAgentReadiness(runtimes, globalConfig)`
(`desktop/src/features/onboarding/ui/agentReadiness.ts:22`) returns
`{ready: true, reason: "cli"|"buzz-agent"}` or `{ready: false}`. CLI path = any
non-buzz-agent runtime with `availability === "available"` and authStatus
`logged_in`/`not_applicable`; buzz-agent path = provider + model set and all
`requiredCredentialEnvKeys` present in `globalConfig.env_vars`. Used by
`SetupStep.tsx:118` (readiness badge, `agent-readiness-badge` /
`agent-readiness-recheck` testids). Types: `AcpAvailabilityStatus` / `AuthStatus`
(`desktop/src-tauri/src/managed_agents/types.rs:534,549` — tagged union so TS
can exhaustively switch). **Kickoff gate:** call `resolveAgentReadiness` in the
orchestrator before doing anything; if not ready, no-op entirely (don't send the
opener — resume logic re-evaluates on next focus, so nothing half-runs).

**Status model:** managed agents carry a `status: String` (types.rs:185,502):
local backend = `"running"` (child alive, runtime.rs:1291) / `"stopped"`
(runtime.rs:1305); remote backends use a two-axis model where `"deployed"`
means provider-invoked (runtime.rs:1266-1282). **There is no `"error"` status**
— failures surface as `"stopped"` + populated `last_exit_code`, `last_error`,
`last_error_code` on the record (runtime.rs:1152-1188: `try_wait` detects
exit, harvests a log-derived error like "harness exited with status …";
snapshot at runtime.rs:1384-1392). Also relevant: spawn falls back to a
**setup-listener mode** if the agent isn't ready at spawn time
(runtime.rs:1591,1693 "agent … not ready — spawning in setup-listener mode").

**Existing UI error surfaces to reuse:**
`friendlyAgentLastError(lastError, lastErrorCode)`
(`desktop/src/features/agents/lib/friendlyAgentLastError.ts`) maps raw errors to
user copy, consumed by `ManagedAgentRow.tsx:93` and
`UnifiedAgentsSection.tsx:308,387`. The kickoff orchestrator can detect
"provider configured but agents failed" by polling/observing the managed-agent
query for `status === "stopped" && lastError != null` after issuing start, and
reuse `friendlyAgentLastError` for any inline messaging. Note there is no push
event stream for status transitions (`grep emit(` in managed_agents/*.rs is
empty) — status is read via the managed-agents list query (React Query
invalidation), so the orchestrator should poll/refetch rather than expect an
event.
