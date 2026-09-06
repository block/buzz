# Kanban AI Operations

## Canonical Instance

- SSH alias: `sd-105293`
- Container: `kanban-ai-self-hosted`
- Host-loopback endpoint: `http://127.0.0.1:3331`
- Live data: `/data/kanban.sqlite` inside the container
- Backups: `/data/backups` inside the container
- MCP key file: `/run/secrets/kanban_mcp_key` inside the container; never print it,
  copy it into the repository, or include it in logs

The loopback endpoint belongs to the host network namespace. Run authenticated
MCP verification on the host, not from inside the container. A host-side helper
may capture `docker exec ... cat /run/secrets/kanban_mcp_key` directly through a
subprocess pipe into process memory; never use shell substitution, print the
value, or place it in arguments, files, or logs.

## Project Ownership

- `Buzz` (`28fe656f-de10-4df9-9f49-cef6cbfa22c0`) owns changes delivered to
  Buzz clients, relay, agents, operations, or releases.
- `linza` owns Linza changes even when Buzz supplied the architectural idea.
- `[DISCOVERY][BUZZ→LINZA]` stays in `linza`.
- Cross-project work has one owning card and links to related task IDs; do not
  duplicate cards.

Stop and resolve ownership before creating or moving a card when the delivery
target is ambiguous.

## Supported Operations

Prefer authenticated MCP tools: `list_projects`, `get_board`, `create_project`,
`update_project`, `create_task`, `update_task`, and comments. Use read-only
`list_projects` and `get_board` for application-facing discovery and
verification. Treat `delete_project`, `delete_task`, and comment deletion as
destructive operations requiring an exact-ID preflight and a verified online
backup, regardless of whether they run through MCP or direct SQL.

The current `update_task` interface cannot change `project_id`, and
`create_task` cannot accept a caller-selected task ID. A board move or
ID-preserving restore is therefore an exceptional database migration: back up
first, validate exact IDs and ownership, use one transaction, and verify through
both SQLite and MCP afterward. Do not use direct SQL for ordinary board edits.

## Safety and Verification

Use read-only SQLite handles for discovery. Before any direct write, or before
a destructive MCP operation:

1. Confirm the exact target IDs and expected ownership.
2. For creation or moves, confirm the target project name/ID is absent and every
   source task ID exists exactly once on its expected board.
3. Create an online SQLite backup in `/data/backups` and record its SHA-256.
4. Run `PRAGMA integrity_check` against the backup before writing.
5. Apply related direct writes in one transaction with fail-closed
   postconditions.
6. Compare preserved task fields and comments against the backup.
7. Run `PRAGMA integrity_check` on live and backup databases.
8. Verify project and board state through authenticated MCP reads.

Stop on ambiguous ownership, duplicate project names, missing tasks, unexpected
counts, changed preserved fields, or a failed integrity check. Never expose the
MCP key or application credentials in commands, output, commits, task
descriptions, or receipts.

The container root filesystem is read-only. If a reviewed one-time migration
script is required, place it temporarily under writable `/data`, execute it once,
and delete it after final verification. Do not retain migration scripts that
embed task content or operational assumptions.

## Buzz Realtime Voice Roadmap

This is the boundary snapshot recorded at migration time; the cards listed in
the receipt are the status and execution source of truth. VOICE 0 is the epic,
VOICE 1 is active discovery, and VOICE 2–8 follow in dependency order. Use the
existing Huddle relay and connect an agent as an authorized audio peer. Buzz
retains identity, authorization, room membership, and effects; OpenAI or
ElevenLabs sits behind the smallest proven media/dialogue provider boundary.
Tool calls use existing managed-agent boundaries.

Do not add a new SFU, gateway, or plugin framework without measured necessity.
Do not duplicate [PR #7217](https://github.com/block/buzz/pull/7217), which
covers provider-hosted A/V sessions. Treat
[PR #7232](https://github.com/block/buzz/pull/7232) as an STT backend, not
realtime speech-to-speech.

## Migration Receipt

- UTC migration time: `2026-09-06T07:12:21.062Z`
- Backup basename: `kanban-before-buzz-20260906T071221.062Z.sqlite`
- Backup size: `2293760` bytes
- Backup SHA-256: `ecdd2ac054914aa48331803fcfb130ae78a7b57b9525c70e2a66cf598557912f`
- Buzz project ID: `28fe656f-de10-4df9-9f49-cef6cbfa22c0`
- Project visibility: private (`private = 1` verified in live SQLite)
- Owner collaborator: `a8fe8636-e369-41b3-aa94-a4cb0722b0c2`, role `owner`
- Moved task IDs:
  - `228ca147-4522-43b1-8222-7864db1fe9fa`
  - `8a0959da-6031-435a-92af-dedf3e9771b1`
  - `ab90c07d-3b8b-4b91-8b5c-6ebf8c233674`
  - `c855f629-5eef-4efe-85e7-f028428536f2`
  - `7b6bdbaa-c207-4684-80c0-851ea4911e74`
- VOICE task IDs:
  - VOICE 0: `bdd8e725-dce7-4024-abcd-f1f383cd8b72`
  - VOICE 1: `3851de3f-f828-44e9-bfa6-dc271562a848`
  - VOICE 2: `f4c8e194-f108-4942-acc2-b424002f91a8`
  - VOICE 3: `08b2282c-cb67-4b54-9b08-badf82290172`
  - VOICE 4: `1afafb59-e50b-46c8-83bf-d2fe807ef581`
  - VOICE 5: `ac29a94f-a5c1-4ab6-a688-d64e0bc1bff0`
  - VOICE 6: `49489cfb-f605-4bda-8770-d0ae84afa035`
  - VOICE 7: `912ebd57-9df3-4ba8-94e5-05f3efb24502`
  - VOICE 8: `ba307d6a-580f-4e21-bca8-c125f207448c`
- Preserved Linza discovery task IDs:
  - `04aa4a08-a8ae-4294-a1bc-5fd28f55cf9d`
  - `1eabba45-78f2-469e-a848-b47554af6f70`
  - `3df3b24c-730b-44fd-8afd-2f814cae55b9`
  - `44da8efa-0562-4d91-b2bd-3270092f2c85`
  - `4c2982ea-c06e-44d4-91fc-9499bc079917`
  - `5b04e032-9fb5-4485-adc5-ec977a87651b`
  - `a06da5a4-b3ef-40bb-835f-6058401b6100`
  - `aaac7b5b-f591-4b2e-9000-3fe6c0e7cb95`
  - `cb11cec2-8a92-4906-bffb-60df989bcfcb`
  - `e37e507c-0f67-406f-bc52-7afbfad71f35`
  - `fa301550-2b43-4ee3-b39f-ccac9397ea68`
- Before: 433 Linza tasks; 11 listed discovery tasks in Linza
- After: 14 Buzz tasks (5 moved, 9 VOICE); 428 Linza tasks; the same 11
  discovery task IDs still in Linza
- SQLite verification: live `ok`; backup `ok`; moved fields and comments match
- MCP verification: projects `linza` and `Buzz`; Buzz board has 14 tasks with 2
  `in-progress`, 10 `todo`, and 2 previously completed moved tasks

## Rollback

The verified backup is the rollback source for this one-time migration. A restore
always discards every board write made after `2026-09-06T07:12:21.062Z`; stopping
the application prevents concurrent-write corruption but does not preserve
those later writes. Before restoring, stop the application, inventory and
explicitly accept or export post-migration changes, verify the backup SHA-256,
then replace the database and rerun SQLite and MCP verification.
