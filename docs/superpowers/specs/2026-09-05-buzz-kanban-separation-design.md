# Buzz Kanban Separation Design

Date: 2026-09-05

## Goal

Give Buzz its own Kanban AI project on the self-hosted Scaleway instance instead
of mixing Buzz delivery work into the `linza` project. Preserve Linza-owned
discovery work in `linza`, recreate the missing realtime voice roadmap in the
new Buzz project, and document the operating boundary in the Buzz repository.

## Current State

The self-hosted Kanban AI instance runs on SSH host `sd-105293` in container
`kanban-ai-self-hosted`. Its active SQLite database contains one project named
`linza`.

The database does not contain task `bdd8e725-dce7-4024-abcd-f1f383cd8b72` or
any task whose title or description contains `BUZZ REALTIME`. Five existing
Buzz-native tasks are currently assigned to `linza`:

- `228ca147-4522-43b1-8222-7864db1fe9fa` — BUZZ AGENTS
- `8a0959da-6031-435a-92af-dedf3e9771b1` — BUZZ CLOUD 0
- `ab90c07d-3b8b-4b91-8b5c-6ebf8c233674` — BUZZ CLOUD 1
- `c855f629-5eef-4efe-85e7-f028428536f2` — BUZZ CLOUD 2
- `7b6bdbaa-c207-4684-80c0-851ea4911e74` — BUZZ CLOUD 3

Eleven `[DISCOVERY][BUZZ→LINZA]` tasks also mention Buzz, but their delivery
target and acceptance authority are Linza.

## Board Boundary

Create one project named `Buzz` for work whose delivery target is the Buzz
repository, clients, relay, or Buzz deployment. Move the five existing
Buzz-native tasks to it without changing their IDs, descriptions, statuses,
priorities, sprint values, or timestamps.

Keep every `[DISCOVERY][BUZZ→LINZA]` task in `linza`. The source of an idea does
not determine board ownership; the system receiving the change does.

This routing rule applies to future tasks:

- Buzz implementation, operations, release, and Buzz-only discovery → `Buzz`
- Linza implementation or discovery inspired by Buzz → `linza`
- Cross-project work → one owning board, with links to related task IDs rather
  than duplicate cards

## Realtime Voice Roadmap

Create these nine tasks in `Buzz`:

1. `VOICE 0 — общий epic realtime-общения в Huddle`
2. `VOICE 1 — аудит текущего audio flow и выбор provider boundary`
3. `VOICE 2 — OpenAI Realtime через существующий Huddle Opus room`
4. `VOICE 3 — ElevenLabs adapter после подтверждения общего seam`
5. `VOICE 4 — полный разрешённый tool/effect path агента`
6. `VOICE 5 — barge-in, floor control и echo protection`
7. `VOICE 6 — lifecycle, reconnect и cleanup`
8. `VOICE 7 — privacy, credentials, spend limits и observability`
9. `VOICE 8 — физический Desktop + неизменённый Mobile E2E и release gate`

Preserve the requested epic ID
`bdd8e725-dce7-4024-abcd-f1f383cd8b72`; generate stable UUIDs for VOICE 1–8.
Set VOICE 0 and VOICE 1 to `in-progress`; set VOICE 2–8 to `todo`. Encode the
dependency order and the epic relationship in task descriptions because the
current Kanban schema has no separate dependency table.

Every roadmap description must preserve these decisions:

- Use the existing Huddle relay and attach the agent as an authorized audio
  peer.
- Keep OpenAI and ElevenLabs behind a media/dialogue provider boundary; they do
  not own Buzz identity, authorization, or effects.
- Route tool calls through existing managed-agent boundaries.
- Do not build a new SFU, gateway, or plugin framework without evidence that it
  is necessary.
- Do not duplicate PR #7217, which concerns provider-hosted A/V sessions rather
  than the Huddle runtime.
- Treat PR #7232 as an STT backend, not as realtime speech-to-speech.

## Migration Safety

Before any write, create a consistent backup of the live SQLite database using
SQLite's online backup mechanism. Apply project creation, task moves, and task
creation in one transaction. Abort the transaction if the source task set is
not exactly the five approved Buzz-native tasks or if the requested epic ID has
appeared since discovery.

After the transaction, verify:

- `Buzz` exists exactly once.
- The five existing Buzz-native IDs now belong to `Buzz` and retain all other
  fields.
- All nine realtime voice tasks exist in `Buzz` with the intended statuses.
- No `[DISCOVERY][BUZZ→LINZA]` task moved out of `linza`.
- The `linza` project and its unrelated task count are otherwise unchanged.

The backup path and verification counts form the rollback receipt. No secrets
or raw credentials are printed or committed.

## Repository Documentation

Add `docs/kanban-ai.md` to Buzz and link it from `AGENTS.md`. The runbook will
record:

- the canonical SSH alias, container name, loopback port, and non-secret data
  locations;
- the `Buzz` versus `linza` routing rule;
- read-only discovery and verification commands;
- the authenticated write path and backup requirement, without embedding the
  MCP key or other credentials;
- failure handling: stop on ambiguous ownership, duplicate project names,
  unexpected task counts, or missing backup receipts.

## Acceptance Criteria

The change is complete when the live board separation passes the verification
queries, the runbook is present and linked from agent guidance, repository
documentation checks pass, and the exact migration receipt is recorded without
secrets.
