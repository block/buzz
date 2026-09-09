---
name: buzz-wrapup
description: Close a meaningful Buzz work session when the user explicitly says done, wrap up, or end session. Preserve a durable handover and queue a human-readable session digest without depending on Obsidian.
---

<!-- BEGIN BUZZ MANAGED SKILL -->
# Wrap up a session

Run only after an explicit `done`, `wrap up`, or `end session` request.

1. Skip the wrap-up if the session was trivial: no work shipped, decision made, reusable finding produced, or open follow-up created.
2. Create a uniquely named `WORK_LOGS/YYYY-MM-DD_SESSION_<SLUG>.md` handover. Include valid Buzz knowledge-file frontmatter, outcome, evidence, decisions, and open follow-ups. Never overwrite an existing file.
3. Create a matching plain Markdown digest in `OUTBOX/session-digests/YYYY-MM-DD_SESSION_<SLUG>.md`. Treat it as pending export and never overwrite an existing file.
4. If an owner-provided wrap-up or export integration is available and authorized, let that process export the digest one-way to the owner's vault. Ordinary agents must not write or sync the vault directly.
5. If the vault or exporter is unavailable, leave the digest queued in `OUTBOX/session-digests/`. Wrap-up still succeeds.

Buzz relay events, agent memory, and workspace files remain canonical. The vault is a human-readable archive, never a runtime dependency or bidirectional store.
<!-- END BUZZ MANAGED SKILL -->
