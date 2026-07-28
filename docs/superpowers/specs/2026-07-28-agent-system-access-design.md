# Agent Access to Internal Systems — Design

**Date:** 2026-07-28
**Status:** Approved for planning

## Problem

Buzz agents can act on the relay (messages, channels, git) via `buzz-dev-mcp`,
but they cannot see any of the systems the business actually runs on: the
Supabase CRM, call transcripts, or operational spreadsheets. An agent asked
"why did this lead go cold?" has no way to answer.

## Goals

- Agents can read live state from Supabase, call transcripts, and Google Sheets.
- Adding this costs a bounded, small amount of permanent context (~600 tokens).
- Credentials never enter the agent's context — only its process environment.
- A persona that doesn't need a capability doesn't carry its tool schemas.

## Non-goals

- **No indexing/ETL pipeline.** All access is live query. Explicitly rejected:
  a pre-indexed knowledge store is a separate project with its own staleness
  and infrastructure costs, and the live sources answer the questions we have.
- **No per-user permission model.** All internal data is fair game to all
  agents (owner decision). Persona scoping below is about context size, not
  access control. If that changes, see "Future: identity delegation".
- **No write access.** Every tool in this design is read-only.

## Decisions and rationale

### One MCP server, four generic tools

A new crate, `buzz-ops-mcp`, exposing exactly four tools:

| Tool | Signature | Backing system |
|------|-----------|----------------|
| `supabase_query` | `(sql: String, limit: Option<u32>)` | Sachi-Prod Postgres, read-only role |
| `calls_find` | `(phone: Option<String>, from: Option<String>, to: Option<String>)` | Mediaserver call API |
| `transcript_get` | `(call_id: String)` | `gs://<bucket>/transcripts/<call-id>.json` |
| `sheet_read` | `(sheet_id: String, range: String)` | Google Sheets API |

**Why one server, not three:** each MCP server is a subprocess with startup
cost and a separate entry in `build_mcp_servers`. Three servers to expose four
tools is overhead with no benefit — they share a credential-loading path and a
result-truncation policy.

**Why generic tools, not per-entity tools:** `supabase_query` taking SQL covers
all nine current tables and every future one in a single ~150-token schema.
The alternative — `get_lead`, `get_sessions`, `get_ad_touches`, … — would be
nine schemas that grow with the database. Tool *count* is the context cost;
keeping it at four is what makes deferred loading unnecessary.

### Deferred tool loading is explicitly rejected

The Claude API supports `tool_search_tool_regex_20251119` /
`tool_search_tool_bm25_20251119` with `defer_loading: true`, which loads tool
schemas on demand. It does not apply here, for two reasons:

1. **We can't reach it.** Agents are ACP subprocesses that receive MCP servers
   from the harness. Whether schemas are deferred is decided by the agent
   process, not by `build_mcp_servers`.
2. **It wouldn't pay for itself.** Four schemas is ~600 tokens of permanent
   context. Building a loading mechanism to reclaim less than a paragraph is
   not a good trade.

### Schema discovery is on-demand, not in the system prompt

Agents don't know what `journey_phase` or `archetype` mean. Putting the DDL of
nine tables into the system prompt is where token burn would actually happen —
far more than the tool schemas.

Instead, `buzz-ops-mcp` exposes the schema as an **MCP resource**
(`schema://supabase`) that the agent fetches when it needs it. Additionally,
`supabase_query` returns a helpful error listing available tables when given
SQL that references an unknown relation. The agent learns the schema in the
session where it needs it, and not otherwise.

### Result size is the real token risk — cap it

Schema size is bounded and small. Result size is not: `SELECT * FROM lead`
returns 164 rows today and more every week.

`supabase_query` therefore:
- Injects `LIMIT 100` when the query has no explicit `LIMIT`.
- Caps the serialized result at **32 KB**.
- On truncation, returns the rows that fit plus an explicit
  `[truncated: N of M rows, add LIMIT or select fewer columns]` marker — never
  silently.

`transcript_get` applies the same 32 KB cap with the same marker. A long call
transcript is the other unbounded payload in this design.

### Read-only enforcement

`supabase_query` connects as a Postgres role with `SELECT`-only grants. This
is the enforcement boundary — not statement parsing, which is bypassable.
Additionally, the tool rejects statements that don't begin with `SELECT` or
`WITH` as a fast fail with a clear message, but the role is what makes it safe.

### Credentials

Injected as environment variables into the MCP subprocess by the harness,
following the existing pattern in `build_mcp_servers`
(`crates/buzz-acp/src/lib.rs:4142`):

| Variable | Scope |
|----------|-------|
| `SUPABASE_READONLY_URL` | Postgres connection string, `SELECT`-only role |
| `MEDIASERVER_API_URL`, `MEDIASERVER_TOKEN` | Minted for this integration |
| `GCS_SERVICE_ACCOUNT_JSON` | `objectViewer` on the `transcripts/` prefix only |
| `GCS_TRANSCRIPT_BUCKET` | Bucket name; path is `<bucket>/transcripts/<call-id>.json` |
| `SHEETS_SERVICE_ACCOUNT_JSON` | Read scope |

They are read by the server process at startup and never appear in a tool
result or schema.

### Per-persona scoping

`PersonaConfig.mcp_servers` (`crates/buzz-persona/src/persona.rs:127`) already
exists and is ignored by `build_mcp_servers`, which only ever attaches
`config.mcp_command`. This design wires it: persona-declared servers are
appended to the harness-provided one.

`McpServerConfig { name, command, args, env }` maps 1:1 onto the ACP
`McpServer` struct, so this is a translation function plus an append.

A persona that never touches calls simply doesn't declare `buzz-ops-mcp`, and
doesn't pay for its schemas.

## Architecture

```
buzz-acp (harness)
  build_mcp_servers(config, persona)
    ├─► buzz-dev-mcp    (existing — shell, files, git, buzz CLI)
    └─► buzz-ops-mcp    (new — declared per persona)
          ├── supabase_query  ──► Postgres (read-only role)
          ├── calls_find      ──► Mediaserver API
          ├── transcript_get  ──► GCS
          ├── sheet_read      ──► Google Sheets API
          └── schema://supabase  (MCP resource, fetched on demand)
```

The join path an agent takes to answer "what did this lead say on their call":

```
supabase_query  →  lead.wa_id (digits-only phone)
calls_find(phone: wa_id)  →  call_id
transcript_get(call_id)  →  transcript text
```

`lead.wa_id` is the join key and already exists, denormalized from
`auth.users.phone` specifically for fast lookup.

## Components

Each is independently testable and has one job.

| Module | Responsibility | Depends on |
|--------|----------------|------------|
| `buzz-ops-mcp/src/lib.rs` | Tool router, server info, stdio transport | `rmcp` |
| `buzz-ops-mcp/src/config.rs` | Read + validate env credentials at startup | — |
| `buzz-ops-mcp/src/supabase.rs` | Query execution, LIMIT injection, schema resource | `config` |
| `buzz-ops-mcp/src/calls.rs` | Mediaserver API client | `config` |
| `buzz-ops-mcp/src/transcript.rs` | GCS object fetch by call ID | `config` |
| `buzz-ops-mcp/src/sheets.rs` | Sheets values fetch | `config` |
| `buzz-ops-mcp/src/truncate.rs` | Shared 32 KB cap + truncation marker | — |
| `buzz-acp` (edit) | Wire `persona.mcp_servers` into `build_mcp_servers` | `buzz-persona` |

Structure mirrors `buzz-dev-mcp`: one module per tool, thin `lib.rs` holding
only the `#[tool_router]` impl and `#[tool]` declarations.

## Error handling

Every tool returns errors as MCP tool errors with actionable text — the agent
should be able to correct itself without a human.

| Condition | Response |
|-----------|----------|
| Missing credential at startup | Fail fast with the variable name; server does not start |
| Non-`SELECT`/`WITH` SQL | `"read-only: only SELECT and WITH are permitted"` |
| Unknown relation | Error listing available tables |
| Transcript not found | `"no transcript for call <id>"` — distinct from a fetch failure |
| Upstream unreachable | Name the system and the underlying error |
| Result over cap | Success + truncation marker (not an error) |

A credential that is present but invalid surfaces on first use, not at startup
— validating every credential at boot would make the server fail when an
unrelated system is down.

## Testing

- **Unit:** LIMIT injection (present, absent, present-in-subquery-only),
  statement gating, truncation boundary, GCS path construction, env parsing.
- **Integration:** each client against a recorded/mock upstream. No live
  Supabase in CI.
- **`buzz-acp`:** extend the existing `build_mcp_servers_tests` module
  (`crates/buzz-acp/src/lib.rs:4952`) with persona-server cases — none
  declared, one declared, name collision with the harness server.

## Open question — does not block

**`calls_find` is designed against an unverified API.** The mediaserver's call
database is confirmed to exist and to be reachable via an API for which a token
can be minted, but its endpoint shape and filter parameters have not been
inspected.

**Assumption:** it can list calls filtered by phone number and date range, and
returns a call ID matching the `<call-id>` in
`gs://<bucket>/transcripts/<call-id>.json`.

If that assumption is wrong, `calls_find`'s signature changes but nothing else
in this design does — the other three tools and all wiring are unaffected.
Implementation should verify the API shape before writing `calls.rs`, and the
plan should sequence `calls.rs` last so it can't block the rest.

## Future: identity delegation

If internal data ever stops being uniformly fair game, the change is to map the
agent's Nostr pubkey to an internal user and pass that identity down to each
tool, replacing the single service identity. No such mapping exists today, and
building one now would be speculative. The tool signatures above do not need to
change to accommodate it.
