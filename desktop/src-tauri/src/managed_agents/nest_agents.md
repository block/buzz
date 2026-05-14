# Sprout Nest

Your persistent workspace. Created once by the Sprout desktop app — never overwritten. Edit freely.

## Directory Layout

| Dir | Purpose |
|-----|---------|
| `GUIDES/` | Actionable runbooks synthesized from research |
| `PLANS/` | Planning documents for work in progress |
| `RESEARCH/` | Findings, notes, and reference material |
| `WORK_LOGS/` | Session logs — what was tried, learned, decided |
| `OUTBOX/` | Shareable docs for external readers (no frontmatter) |
| `REPOS/` | Cloned repositories (clone freely here for exploration) |
| `.scratch/` | Temporary working files — treat as disposable between sessions |

Filenames: `ALL_CAPS_WITH_UNDERSCORES.md` (e.g., `OAUTH_FLOW_NOTES.md`).

## Communicating via Sprout

You have MCP tools for channels. Use them.

**Read messages:**
- `get_messages(channel_id, limit=50)` — recent history (max 200)
- `get_thread(channel_id, event_id)` — drill into a thread
- `get_feed()` — personalized: your mentions, needs-action items

**Post messages:**
- `send_message(channel_id, content)` — new message
- `send_message(channel_id, content, parent_event_id)` — threaded reply

**Poll for new messages** (no push — poll with sleep):
- Call `get_messages(channel_id, since=<last_seen_unix_ts>)` where the value is the `created_at` timestamp of the last message you saw
- When `since` is set without `before`, results are **oldest-first** (chronological)
- Sleep 10–30 seconds between polls

**Search:**
- `search(q="your query")` — searches across all channels

## Recovering Context on Startup

1. Call `get_feed()` — surface mentions and items needing your action
2. Call `get_messages` on your assigned channel(s) to read recent history
3. Check `RESEARCH/`, `PLANS/`, `GUIDES/` before researching from scratch

## Knowledge File Conventions

Files in `GUIDES/`, `PLANS/`, `RESEARCH/`, `WORK_LOGS/` should include YAML frontmatter:

```yaml
---
title: "Always Quoted Title"
tags: [lowercase-hyphenated]
status: active
created: 2026-01-15
---
```

**Status values:** `active` | `superseded` | `stale` | `draft`

> ⚠️ Title **must** be quoted — unquoted colons can break YAML parsing.

## Core Guidelines

- **Local first** — check `RESEARCH/`, `GUIDES/`, `PLANS/` before external searches
- **Write findings down** — if you research something, save it to `RESEARCH/`
- **Cite sources** — no claim without a path, link, or reference
- **Don't overwrite** — append or create new files; don't silently clobber existing work
- **`.scratch/` is disposable** — don't rely on it across sessions
- **Never push without approval** — do not `git push` to any remote
- **Stay on task** — only stage files relevant to your current work
- **Tagging or @mentioning others** — you can mention other bots or users by simply @'ing them in your message, but you cannot bold, italicize, or otherwise format the mention text if you want them to actually be alerted

<!-- BEGIN SPROUT MANAGED — regenerated automatically, do not edit below -->
## Active Agents

*(No agents deployed yet. Add agents in the Sprout desktop app.)*

<!-- END SPROUT MANAGED -->
