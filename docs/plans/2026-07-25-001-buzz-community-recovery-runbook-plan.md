# Buzz Community Recovery And Continuity Runbook Plan
Created: 2026-07-25

## Purpose

Recover the older local Buzz pilot community that was created under `localhost:3000`, then create a durable continuity runbook so future Buzz pilot sessions use a deliberate community, port, and memory pattern instead of accidentally splitting context across local hosts.

## Current Situation

- The Buzz desktop app can remain open even when the local relay is stopped; the app is not the durable memory store.
- Durable local conversation state is in the relay-backed Postgres event log, stored in the Docker named volume `buzz-postgres-data`.
- Local metadata inspection found two useful local communities:
  - `localhost:3000`: 2 channels, 44 events, channels `codex-pilot-smoke` and `pilot-demo`.
  - `localhost:3030`: 4 channels, 176 events, channels `buzz-pilot`, `install-support`, `repo-review`, and `agent-runs`.
- Port `3000` was previously held by a Node process serving another local app; Steve closed that app and a follow-up port check showed `3000`, `3030`, and `8080` are currently free.
- Port switching creates separate Buzz communities because the relay maps request host authority to rows in `communities`; `localhost:3000` and `localhost:3030` are intentionally distinct.

## Goals

- Recover and read the older `localhost:3000` community without mutating its rows or merging it into the newer `localhost:3030` community.
- Preserve both the old community and the newer Day 0 pilot community until we intentionally choose an archive or migration strategy.
- Produce a runbook that explains safe restart, verification, backup, canonical community selection, and agent-memory expectations.
- Make the next pilot step obvious to Steve and future agents.

## Non-Goals

- Do not merge `localhost:3000` and `localhost:3030` in-place.
- Do not run `scripts/dev-reset.sh`, `docker compose down -v`, or any reset path that deletes named volumes.
- Do not inspect or paste private message contents unless Steve explicitly asks for a content export.
- Do not make Slack canonical for memory.
- Do not change Buzz product code as part of this plan.

## Assumptions

- Docker Desktop remains available.
- The `buzz-postgres-data` Docker volume has not been deleted.
- The old community should be recovered first by matching its original authority, not by database surgery.
- If port `3000` becomes occupied again, the safe fallback is to continue on `localhost:3030` and treat `localhost:3000` as an archive until a backup-first migration is planned.

## Safety Rules

- Back up before any operation that changes community identity, host rows, or volumes.
- Prefer metadata checks before content reads.
- Keep `.env`, private keys, auth tags, cookies, tokens, webhook URLs, and raw secrets out of Buzz, Slack, docs, and prompts.
- Treat `docker compose down` as acceptable for stopping services because named volumes are preserved by default.
- Treat `docker compose down -v`, Docker volume deletion, and `scripts/dev-reset.sh` as destructive for this pilot.

---

## Phase 1: Preflight And Preservation Check

### Objective

Confirm the old community is still present and that recovery can proceed without accidental data loss.

### Actions

1. Check the compose service state.
2. Confirm named volumes exist, especially `buzz-postgres-data`.
3. Query community and channel metadata only.
4. Record current port ownership for `3000`, `3030`, and the selected health port.
5. If any destructive command is proposed, stop and require an explicit backup decision first.

### Evidence To Capture

| Check | Expected Signal |
| --- | --- |
| Docker services | `buzz-postgres` exists and is healthy or startable |
| Named volumes | `buzz-postgres-data` exists |
| Old community | `localhost:3000` has channels/events |
| New community | `localhost:3030` has Day 0 channels/events |
| Port ownership | `3000` owner is identified before recovery |

### Exit Criteria

- We know whether `localhost:3000` data still exists.
- We know whether port `3000` is available.
- No content or secret material has been exposed.

---

## Phase 2: Recover The Old Community By Original Authority

### Objective

Make the old `localhost:3000` community readable again by running Buzz against the same host authority that owns those rows.

### Preferred Path

1. Confirm port `3000` is still free immediately before launch.
2. Start the relay on the default local authority; prefer relay-only recovery first if the desktop app is already open, and use full `just dev` only when we want a fresh desktop+relay launch.
3. Confirm relay readiness.
4. Point the desktop app or CLI at `ws://localhost:3000`.
5. List channels and verify `codex-pilot-smoke` and `pilot-demo` appear.
6. Read only thread/channel summaries unless Steve asks for deeper content recovery.

### Fallback Path

If port `3000` cannot be freed quickly:

1. Do not mutate the old community.
2. Continue the active pilot on `localhost:3030`.
3. Mark `localhost:3000` as an archive community in the runbook.
4. Schedule a backup-first export or migration experiment only if the archived conversations are valuable enough.

### Exit Criteria

- The old community is either readable through `localhost:3000` or explicitly preserved as a non-mutated archive.
- We have a short note that distinguishes old-community memory from new-community memory.

---

## Phase 3: Choose The Canonical Pilot Community

### Objective

Avoid split-brain pilot memory by choosing exactly one community for new work.

### Decision Matrix

| Option | Use When | Tradeoff |
| --- | --- | --- |
| Make `localhost:3000` canonical | Port `3000` remains free and old conversations matter most | Conflicts with the other local Node app if that app returns to port `3000` |
| Keep `localhost:3030` canonical | Amancha Wedding or another app needs port `3000` | Old community remains separate/archive |
| Migrate later | Both old and new communities contain useful history | Requires backup-first migration design |

### Recommended Default

Attempt recovery on `localhost:3000` now that port `3000` is free. If the old channels are readable and the app can stay off port `3000`, promote `localhost:3000` to the canonical pilot community; otherwise keep `localhost:3030` canonical and preserve `localhost:3000` as an archive.

### Exit Criteria

- The runbook names one canonical community for new agent writes.
- The runbook names the other community as archive/read-only unless a migration plan supersedes it.

---

## Phase 4: Write The Pilot Continuity Runbook

### Objective

Create a short operator runbook that future Steve/Codex sessions can follow without reconstructing terminal history.

### Target Artifact

- `docs/pilots/buzz-local-continuity-runbook.md`

### Required Sections

- Canonical community and current port set.
- How to stop/start services safely.
- How to tell whether the Mac app, relay, Docker services, and Postgres volume are each alive.
- How to verify community/channel metadata without reading message contents.
- How to back up and restore the local Postgres volume before any migration or host-row edit.
- How to recover the old `localhost:3000` community.
- How to continue the `localhost:3030` pilot.
- What never to run without a backup.
- Where the Day 0 channel IDs and event IDs live.
- How agents should use Buzz as memory: read thread roots and closeouts, cite channel/event IDs, and avoid treating raw setup chatter as canonical truth.
- Slack visibility boundary: Slack may receive summaries and links, but Buzz remains the handoff context and GitHub remains code state.

### Exit Criteria

- A future agent can answer "which Buzz community should I use?" from the runbook.
- A future agent can restart the pilot without deleting data.
- A future agent can explain why old conversations may appear missing when switching between `localhost:3000` and `localhost:3030`.

---

## Phase 5: Verify The Runbook With One Real Readback

### Objective

Prove the runbook can guide a later agent through the actual local state.

### Verification Steps

1. Follow the runbook preflight from a clean terminal.
2. Confirm Docker and the Postgres volume state.
3. Confirm community metadata for `localhost:3000` and `localhost:3030`.
4. Confirm the canonical community choice.
5. If a relay is running, read back one known thread with `buzz --format compact messages thread`.
6. Record whether the readback was enough to resume the pilot in under five minutes.

### Exit Criteria

- The runbook is not just descriptive; it has been exercised against the local machine.
- Any gap found during verification is patched into the runbook immediately.

---

## Open Questions

- If the other local app needs port `3000` again, should Buzz remain canonical on `localhost:3000` or should the app move permanently?
- Should `localhost:3000` remain an archive, or should we later migrate useful old threads into the canonical pilot community?
- Should we create a persistent disposable pilot identity outside the repo for repeated agent writes?
- Should the first runbook verification include message-content reads, or metadata/thread-closeout reads only?

## Risks And Mitigations

| Risk | Mitigation |
| --- | --- |
| Accidental volume deletion | Avoid `down -v`, `dev-reset`, and manual volume removal; back up before destructive work |
| Split-brain pilot memory | Name one canonical community for new writes |
| False belief that the Mac app stores memory | Runbook separates desktop app, relay, Docker services, and Postgres volume |
| Secret leakage in recovery notes | Metadata-first checks; redact logs; never paste key material |
| Over-investing in migration too early | Recover/read first; migrate only if old threads prove valuable |

## Definition Of Done

- Old `localhost:3000` community recovery path is documented and either verified or blocked by a new named port conflict.
- Canonical new-work community is documented.
- `docs/pilots/buzz-local-continuity-runbook.md` exists and covers safe restart, verification, archive handling, and memory rules.
- The runbook has been tested once against local metadata and, if a relay is available, one compact thread readback.
- Remaining decisions are captured as Open Questions rather than hidden in terminal history.
