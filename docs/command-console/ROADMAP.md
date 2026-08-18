# Command Adviser delivery roadmap

Last updated: 18 August 2026

This is the authoritative delivery sequence for Command Adviser. It exists to
keep one implementation line, one owner, and one visible acceptance gate for
each capability.

## Delivery rules

- The main Buzz AI task owns implementation, integration, testing, release,
  and installed-app acceptance.
- Peer-agent tasks may explore or refine bounded designs, but they do not run
  independent implementation streams.
- Only one implementation phase is active at a time. New ideas go into the
  backlog until the active phase is accepted.
- Each implementation phase uses a `codex/phase-*` branch and draft pull
  request. It is not complete until automated checks and the relevant real
  user journey pass.
- Upstream synchronization targets a pinned stable Buzz Desktop tag, never the
  moving `upstream/main` branch.
- Existing Command Adviser data and the last working macOS application are
  preserved before every upstream or migration phase.
- Material decisions, failures, fixes, and acceptance results are recorded in
  Memory MCP with agent `CODEX`, and this file is updated when a phase changes
  state.

## Current control point

| Capability | State | Repository control | Next gate |
| --- | --- | --- | --- |
| Command Adviser V1 and Buzz v0.5.2 integration | Complete and merged | [PR #14](https://github.com/NavigatorRAN/buzz/pull/14), merged as `e9b121154` | Retain as the rollback baseline for the v0.5.8 phase |
| V4 Living Ship | Complete and merged | [PR #15](https://github.com/NavigatorRAN/buzz/pull/15), merged as `865569bb4` after installed-app acceptance | Included in the v0.5.2 integration line; retain its real-user journey in every release regression |
| Upstream Buzz refresh | Complete and accepted | [PR #16](https://github.com/NavigatorRAN/buzz/pull/16), merged into the integration line as `f09c3a1ea`; pinned `desktop-v0.5.8` at `f3de86057` | Preserve as the accepted product baseline |
| Repository baseline promotion | Complete | [PR #17](https://github.com/NavigatorRAN/buzz/pull/17), merged to `main` as `d8dca6d49` | Retain as the authoritative v0.5.8 downstream baseline |
| V2 Keeper | Typed-memory MVP complete and accepted | [PR #19](https://github.com/NavigatorRAN/buzz/pull/19) at `56a4d7045`; repository specification and Memory MCP entity `buzz-keeper` | Merge the accepted phase, then begin the basic V3 private remote-access pilot |
| V3 native remote access | Automated private pilot implemented; physical iPhone acceptance pending | [PR #20](https://github.com/NavigatorRAN/buzz/pull/20); tailnet-only Tailscale Serve, pairing-only desktop advertisement, and mobile diagnostics | Run the outside-LAN iPhone journey; keep push/wake hardening deferred |
| Offline Phase 1 — Gemma runtime | Complete and merged | [PR #23](https://github.com/NavigatorRAN/buzz/pull/23), merged as `2d91168dc` | Retain the exact `gemma4-26b-official` 64K capacity-one contract |
| Offline Phase 2 — Mac-local RAG | Complete and merged | [PR #24](https://github.com/NavigatorRAN/buzz/pull/24), merged as `35dabbe89` | Preserve snapshot identity and semantic canary |
| Offline Phase 3 — adaptive memory | Complete and merged | [PR #25](https://github.com/NavigatorRAN/buzz/pull/25), merged as `4e804b51a` | Preserve append-only history and local active recall |
| Offline Phase 4 — autonomous skills | Complete and merged | [PR #26](https://github.com/NavigatorRAN/buzz/pull/26), merged as `87fe1a26c` | Preserve immutable skills, verified projection, and rollback |
| Offline Phase 5 — disconnected acceptance | Final candidate installed; automated gates and CI pass; owner accepted the disconnected multi-adviser, local RAG, and local Memory journey with interrupted-brief resume recorded as a residual limitation | Draft [PR #27](https://github.com/NavigatorRAN/buzz/pull/27); final manifest `1ac2d4363a02a4ab4b425930c25595bc6af8fbd44cd3d81e64140f4a88168767` | Run the service-restart matrix, cold restart, and eight-hour disconnected soak |

Living Ship, the v0.5.2 integration line, the controlled sync to pinned upstream
tag `desktop-v0.5.8`, and repository baseline promotion are complete. PR #17
placed the accepted product on `main`; PRs #2-13 were closed as superseded and
PR #1 is retained as merged history. The synchronized application passed
installed-app acceptance, including live adviser turns and substantive doctrine
RAG retrieval. PR #18 completed the V2/V3 compatibility freeze. Keeper passed
the full repository gate and installed-app typed debrief, later brief,
correction, scoped forget, immediate undo, and Living Ship journey on PR #19.
The offline programme is now the active implementation line: Phases 1–4 are
merged through PR #26, and Phase 5 is deliberately limited to packaging and
physically proving those existing components.

## Execution sequence

### 0. Close the current integration line

1. Merge Living Ship PR #15 into `codex/upstream-v0.5.2-sync`.
2. Verify that PR #14 contains the accepted Command Adviser v0.5.2 product and
   Living Ship changes.
3. Merge PR #14 into `codex/project-execution-v1` after its existing-data and
   installed-app acceptance evidence remains valid.
4. Tag or otherwise record the resulting downstream baseline commit and retain
   the current working application as rollback material.

Exit gate: one integrated downstream baseline contains the operational Command
Adviser product and Living Ship, with no feature work left on stacked branches.

Completion record: PR #15 merged into PR #14, PR #14 merged into
`codex/project-execution-v1`, and the resulting integrated baseline was retained
through the v0.5.8 sync. The older stacked pull requests remain open only until
the accepted v0.5.8 tree is promoted to `main`.

### 1. Controlled Buzz stable-tag sync

At phase kickoff, fetch upstream tags and pin the latest suitable stable Buzz
Desktop release. `desktop-v0.5.8` is the current candidate, but the selected tag
is recorded in the phase plan so the target cannot move during implementation.

The sync must:

- start from the integrated downstream baseline created in Step 0;
- preserve the naval Command Adviser shell, Command Team, provider routing,
  source connectors, Battle Rhythm, Plans, Living Ship, and signed workspace;
- preserve Command Adviser-specific LM Studio, RAG, Memory MCP, World Monitor,
  and Apple integration paths when upstream generic behavior would remove them;
- review upstream agent-observer batching and per-agent/channel watermarks for
  Living Ship rather than copying it blindly;
- treat upstream voice, transcription, notification routing, managed-agent, and
  mobile changes as components to assess for Keeper and remote access; and
- resolve conflicts deliberately by subsystem, with no wholesale replacement
  of the Command Adviser product layer.

Before changing the live installation, capture a restorable backup of:

- Postgres and signed relay events;
- MinIO objects and local application state;
- managed-agent definitions and harness/provider configuration;
- Command Adviser source and routing configuration;
- Battle Rhythm, Plans, Command Brief, and Living Ship state;
- Apple Calendar publication configuration and Keychain-backed references; and
- the last working `/Applications/Command Adviser.app` bundle.

Exit gate: the synchronized release is installed and the regression matrix below
passes without loss of existing user data.

Completion record: PR #16 merged into `codex/project-execution-v1` as
`f09c3a1ea`. The installed Command Adviser application retained existing data,
all managed advisers completed real turns, Living Ship worked, and the fixed
ADF doctrine semantic canary returned substantive retrieval after the embedding
service fault was repaired.

### 1.5 Promote the accepted downstream baseline

- Merge the accepted `codex/project-execution-v1` tree into `main` through one
  consolidation pull request.
- Change only the delivery control record during promotion; do not reopen the
  accepted product implementation.
- After merge, close stacked PRs #1-13 as superseded by the consolidated
  baseline so GitHub shows one current implementation line.
- Preserve PR #16 and the installed-app acceptance record as the regression
  evidence for the promoted tree.

Exit gate: `main` contains the accepted v0.5.8 Command Adviser product, old
stacked PRs are closed without deleting their history, and Keeper is the only
active implementation programme.

Completion record: PR #17 merged as `d8dca6d49`. PRs #2-13 were closed as
superseded without deleting their branches or review history; PR #1 is part of
the merged ancestry. Keeper is now the only active feature programme.

### 2. Revalidate and freeze V2 and V3 designs

This is a short compatibility checkpoint, not another open-ended design cycle.

- Materialize the approved Keeper design as a repository specification and
  update only seams changed by the upstream sync.
- Materialize the native remote-access design as a repository specification and
  split it into a basic private-LAN/VPN pilot and later push/wake hardening.
- Remove duplicated infrastructure in either design when the synchronized Buzz
  base already provides the required capability.
- Record explicit non-goals and acceptance tests before code begins.

Exit gate: both specifications name their reused components, new components,
data boundaries, test journeys, and deferred scope. No unresolved architecture
choice blocks Keeper Phase 1.

Implementation record: the frozen specifications are
`docs/superpowers/specs/2026-08-09-keeper-relationship-memory-v0.5.8.md` and
`docs/superpowers/specs/2026-08-09-native-private-remote-access-v0.5.8.md`.
Keeper reuses managed personas, signed DMs, NIP-AE engrams, structured model
completion, and Living Ship. The remote pilot reuses native mobile pairing and
relay messaging over Tailscale; APNs, wake, and durable outbox work remain
deferred.

Completion record: PR #18 merged as `473bcfefd`. The Keeper implementation
plan is `docs/superpowers/plans/2026-08-09-keeper-typed-memory-mvp.md`; native
private remote access remains deliberately deferred until after the Keeper MVP.

### 3. Implement V2 Keeper

#### 3.1 Typed relationship-memory MVP

- Provision first-party `builtin:keeper` as an owner-private managed adviser.
- Support typed post-meeting debriefs and on-demand text briefs.
- Store canonical people, compact interaction outcomes, and unresolved identity
  records in the approved encrypted `mem/keeper/*` engram namespace.
- Keep the raw conversation in its signed private Buzz DM and store source
  event/thread references with extracted outcomes.
- Implement truthful save receipts, duplicate-name quarantine, correction,
  forget, and undo behavior.
- Add Keeper to Living Ship through the managed-agent roster rather than another
  fixed sprite list.

Exit gate: a real typed debrief changes a later Keeper brief, ambiguity does not
merge two people, corrections affect recall, and community/owner isolation is
verified.

Implementation record: draft PR #19 adds active owner-private
`builtin:keeper`, an explicit encrypted `mem/keeper/*` operating contract,
on-demand Message provisioning with managed-agent reuse, and a Living Ship
projection in Ship's Office. Keeper remains outside the command-brief adviser
schema and receives no doctrine RAG or World Monitor environment injection.
Focused Rust, desktop agent, Living Ship domain, provisioning, and Living Ship
screen journeys pass. The full repository gate also passed with 4,740 desktop,
2,584 desktop-native, and 1,261 mobile tests. Installed-app acceptance passed
using fictional data: Keeper wrote and read back typed memory, retrieved it in a
later sourced brief, corrected one fact, removed only that fact, immediately
restored the same fact ID and provenance, and appeared in Living Ship. The
pre-upgrade app and application-data backups are retained with suffix
`before-keeper-20260809-130732`.

#### 3.2 Voice capture and playback

- Reuse synchronized Buzz local transcription and playback components where
  they fit the approved design.
- Discard captured audio after transcription.
- Retain text fallback whenever transcription or playback is unavailable.

Exit gate: an installed-app voice debrief produces the same verified memory
outcome as typed capture, without retaining the source audio.

#### 3.3 Apple Calendar meeting briefs

- Extend the existing EventKit helper and durable scheduler rather than creating
  parallel calendar infrastructure.
- Prepare matched briefs 15 minutes before the meeting.
- Deliver a private notification and play audio only after the user taps it.
- Fence scheduling, claims, and delivery by owner and community and prove
  idempotency across restart and sleep/wake.

Exit gate: one real calendar event produces one timely brief and notification;
permission denial, unmatched attendees, restart, and playback failure degrade
cleanly.

### 4. Implement V3 native remote access

#### 4.1 Basic private remote pilot

- Keep the MacBook relay, models, RAG, Memory, and application data
  authoritative.
- Connect the native iPhone Buzz client over the private Tailscale/VPN path.
- Prove authenticated message send, receive, history, and one Command Adviser
  interaction without adding a hosted relay or public ingress.
- Start with the existing model-routing rules; remote access does not move model
  execution or source data to the phone.

Exit gate: from outside the home LAN, the paired phone can reliably exchange a
message with Command Adviser through the private network, and loss of VPN fails
closed and visibly.

Implementation record: draft PR #20 adds a separately validated and persisted
pairing-only `https://*.ts.net` origin, derives private WSS in the trusted Tauri
command, classifies mobile network versus authentication failures, and provides
the repeatable acceptance and rollback runbook. Tailscale Serve proxies the
MacBook's stable tailnet HTTPS name to `127.0.0.1:3000`; status reports
`tailnet only` and Funnel remains disabled. Automated repository and release
gates precede the remaining physical iPhone journey.

#### 4.2 Notification, wake, and resilience hardening

Only after the basic pilot is useful, assess and implement the minimum required
APNs, wake, encrypted outbox, background-resume, and device-attestation work.

Exit gate: notifications resume the private session without duplicate delivery,
silent data loss, or exposing the relay publicly.

## Regression matrix for every integration release

Automated checks are necessary but do not replace these installed-app journeys:

| Area | Required evidence |
| --- | --- |
| Existing data | Signed event counts and key application records match the pre-change baseline; Battle Rhythm, Plans, briefs, channels, users, and managed agents remain visible |
| Command Team | Each adviser is online, can be messaged, and can persist an outcome for future command briefs |
| Doctrine RAG | A fixed semantic canary returns substantive doctrine with document, section/chunk, and `point_id` metadata; collection listing alone is not a pass |
| Memory MCP | Write and recall a disposable test event through the same path used by the app |
| Maritime N2 | World Monitor MCP returns a live result and the adviser distinguishes live OSINT from doctrine and planning assumptions |
| Model routing | Cloud-primary and local-fallback paths both complete a real adviser turn; the UI accurately shows the selected route |
| Daily Command Brief | A manual brief completes with sourced adviser contributions, current conversation outcomes, freshness, and useful failure labels |
| Apple integration | Read-only inputs and one-way dedicated-calendar publication work; permission denial fails softly |
| Battle Rhythm and Plans | Existing entries load, a disposable edit can be saved, multi-day/all-day display remains correct, and project dates remain linked to the calendar |
| Living Ship | Agents show working, collaborating, idle, and unavailable states at the accepted window size, and activity navigation opens the correct context |
| Recovery | The previous app and backed-up data can be restored on a clean test profile or equivalent isolated rehearsal |

## Deferred backlog

The following work does not block the roadmap above:

- optional Phase 6 model refinement, which remains closed until the accepted
  archive meets its evidence threshold;
- RAG 2.0 or a new knowledge replication ecosystem;
- expanded operational-risk and mission-analysis workflows beyond the current
  advisory team;
- Slack, Teams, SMS, or other external agent channels;
- autonomous changes to external Navy, navigation, logistics, personnel, or
  communications systems;
- general security or infrastructure redesign that is not required to make the
  active user journey work; and
- advanced remote push/wake hardening before the basic Tailscale pilot proves
  useful.

## Phase completion checklist

A phase moves to complete only when all applicable items are true:

- [ ] scope and acceptance tests are in the phase specification;
- [ ] work is isolated on its declared branch and draft PR;
- [ ] focused tests and repository quality gates pass, or inherited failures are
      named with evidence and are unrelated to the change;
- [ ] the installed macOS application passes the real user journey;
- [ ] existing Command Adviser data is preserved and recovery remains possible;
- [ ] the PR is merged or intentionally closed, with no hidden implementation
      left on a peer task;
- [ ] the outcome and material gotchas are recorded in Memory MCP by `CODEX`; and
- [ ] this roadmap is updated with the new baseline and next active phase.
