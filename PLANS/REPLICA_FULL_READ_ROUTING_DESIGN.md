---
title: "Replica Full-Read Routing — Caller Classification"
tags: [relay, replica-routing, consistency]
status: active
created: 2026-08-28
---

# Replica Full-Read Routing — Caller Classification

`Db::query_events` always reads the **writer** pool. `Db::query_events_routed`
(and its `_bounded` / `count` / feed siblings) opt a read into **replica
routing**: the read may be served from a read replica when
`BUZZ_REPLICA_READ_MAX_AGE_MS` is set, subject to the soundness predicate
`RoutePredicate::for_query` derives from the query shape. The seam fails closed
to the writer at every error and is a genuine no-op until the budget is
configured (`crates/buzz-db/src/lib.rs`).

## Routing rule

**If a read's result influences a write or a permission decision, it reads from
the writer.** A replica can lag behind the caller's own just-committed write; a
read that gates the next write against that lag would make a wrong decision
(e.g. validate a save against a stale head, or miss the caller's own accepted
event during post-write verification and report a false conflict). Every such
read stays on `query_events` / writer.

Display reads — a page the user scrolls, a count on a badge, a history list —
tolerate bounded staleness and take the routed path.

Adding, removing, or reclassifying a caller **requires updating the table
below**; the `query_events_routed` doc-comment points here.

### Client-carried intent: the `consistency` extension field

Some write-influencing reads are issued by clients (Desktop, CLI) through the
HTTP `/query` bridge, and are **indistinguishable by query shape** from display
reads — a kind-40100 `limit:1` read serves both `get_canvas` (display) and a
canvas save's head precondition. The client therefore signals intent with the
`consistency` extension field on the raw filter:

- `"consistency": "strong"` → the bridge serves that filter from the writer
  (`query_events`), never a replica.
- absent → the default routed path.
- any other value → `400 Bad Request` (fails loud, never silently degrades).

The field only ever forces the **writer**, which is always the sound direction;
there is deliberately **no** inverse "force replica" value, so it cannot be
sprayed to bypass the replica-staleness guard on reads that should not. Parsed
in `crates/buzz-relay/src/api/bridge.rs` (`extract_consistency`).

## Caller classification table

Rows below are every `query_events_routed` / `query_events_routed_bounded`
call site at head, plus the writer-pinned canvas row this change adds. (The
`count` and feed routed families — `count_events_routed`,
`get_events_by_ids_routed`, `query_feed_*_routed`, and the `get_channel_window`
cursor/head reads — are all display or bounded-count surfaces on the routed
path; they carry their own soundness notes at their definitions in
`crates/buzz-db/src/lib.rs` and are out of scope for this table.)

| Caller / path label | Pool | Justification |
|---|---|---|
| `bridge_query` (default `/query` filter) | routed | Display reads over the HTTP bridge; bounded staleness acceptable. |
| `bridge_query` + `consistency: strong` | **writer** | Client-declared write-influencing read (canvas save precondition, restore precondition, post-write ancestry verification). |
| `req_historical` (WS REQ historical page) | routed | Display backfill of a subscription; per-row re-filter absorbs a briefly-stale row. |
| `bridge_thread_aux` (`AuxReader::Routed`, thread aux page) | routed | Thread reply hydration; display, post-verified against the fence wall. |
| `bridge_count_fallback` (`query_events_routed_bounded`) | routed (bounded arm) | COUNT fallback that materializes rows; bounded arm only, never covered. |
| `count_req_fallback` (`query_events_routed_bounded`) | routed (bounded arm) | WS COUNT fallback that materializes rows; bounded arm only. |

The **writer** row is the only write-influencing entry; every other caller is a
display or count surface that tolerates bounded staleness. Client canvas reads
that gate a write set `consistency: strong` (Desktop
`desktop/src-tauri/src/commands/canvas.rs`, CLI
`crates/buzz-cli/src/commands/channels.rs`) so they land on the writer row.
