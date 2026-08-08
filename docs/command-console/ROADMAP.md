# Command Adviser delivery roadmap

Last updated: 9 August 2026

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
| Command Adviser V1 and Buzz v0.5.2 integration | Installed and operational | [PR #14](https://github.com/NavigatorRAN/buzz/pull/14), `codex/upstream-v0.5.2-sync` at `641f5ac4` | Integrate Living Ship, then close the stacked integration line |
| V4 Living Ship | Engineering and installed-app acceptance complete | [PR #15](https://github.com/NavigatorRAN/buzz/pull/15), `codex/living-ship-design` at `6038a17a1` | GitHub still shows the PR as open and draft; merge it into the v0.5.2 integration branch before the next sync |
| Upstream Buzz refresh | Next implementation phase | Latest verified stable tag is `desktop-v0.5.8` at `f3de86057`; re-check at kickoff | Pin one stable tag and complete the controlled merge and regression gate |
| V2 Keeper | Approved product design; not implemented | Canonical design is currently in Memory MCP under `buzz-keeper` | Revalidate against the synchronized base, then start typed-memory MVP |
| V3 native remote access | Design to be frozen; not implemented | Canonical design is currently in the Version 3 peer task and Memory MCP under `native-buzz-remote-access-pilot` | Revalidate after the upstream sync; implementation follows Keeper MVP |

The installed application may contain Living Ship even while its stacked GitHub
PR remains open. Installed acceptance and repository integration are separate
gates; both must be complete before the next upstream merge begins.

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
