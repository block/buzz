# Buzz Nest

Your persistent workspace. Created once by the Buzz desktop app. The static content above the managed-section markers is regenerated on upgrades — add custom notes below the markers or in separate files.

## Directory Layout

| Dir | Purpose |
|-----|---------|
| `GUIDES/` | Actionable runbooks synthesized from research |
| `PLANS/` | Planning documents for work in progress |
| `RESEARCH/` | Findings, notes, and reference material |
| `WORK_LOGS/` | Session logs — what was tried, learned, decided |
| `OUTBOX/` | Shareable docs for external readers (no frontmatter) |
| `REPOS/` | Source checkouts. Work in an existing local checkout when one exists; clone here only when none does |
| `.scratch/` | Temporary working files — treat as disposable between sessions |

Filenames: `ALL_CAPS_WITH_UNDERSCORES.md` (e.g., `OAUTH_FLOW_NOTES.md`).

The bundled CLI is your primary tool interface — run its `--help` command for usage. The CLI skill file has the full reference.

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
- **Stay on task** — only stage files relevant to your current work

## Git Commit Identity

Your commit **author** identity is machine-managed. Every commit is automatically authored and cryptographically signed as your agent identity (`<pubkey>@<relay-host>`) — you do not, and cannot, set it. The managed `git` rejects `user.name`/`user.email` config, `-c user.*`, `--author`, and `--reset-author`. The human operator is credited in the commit message trailers, which the author identity does not replace.

> The operator can turn this off with `BUZZ_GIT_IDENTITY=user` (default `agent`, settable per-agent). In `user` mode your commits carry the operator's own git identity and signing config — no managed `git`, no signing enforcement — and the trailers below are redundant since the commit already *is* the operator's identity. The rest of this section describes the default `agent` mode.

- **Human sign-off (required):** every commit MUST include a `Signed-off-by` trailer for the human operator responsible for the agent's work. Add via `git commit --trailer "Signed-off-by: Human Name <human@email>"`. One blank line must separate trailers from the commit body.
- **Human credit (`Co-authored-by`):** every commit MUST also include a `Co-authored-by` trailer for the same human operator, with identical name and email to the `Signed-off-by` line. GitHub parses `Co-authored-by` for contribution-graph credit; `Signed-off-by` alone does not grant it. Add via `git commit --trailer "Co-authored-by: Human Name <human@email>"`. Place `Co-authored-by` before `Signed-off-by` in the trailer block.
- **Discovering the human's identity:** `git config user.name`/`user.email` now resolve to your machine-managed *agent* identity, NOT the operator's — do not use them for the trailers. Take the operator's name and email from the repository's `AGENTS.md` / contribution docs or from an explicit instruction. Do NOT hardcode or guess. If you cannot determine the operator's email, STOP and ask before committing.
- **Signing:** commits are signed with your agent nostr key automatically (NIP-GS). Do not configure a separate signing key and do not use the human's signing key.
- **Verify before pushing:** `git log -1 --format='%B' | git interpret-trailers --parse` should show the human's `Co-authored-by` and `Signed-off-by` trailers as a contiguous block.

<!-- BEGIN BUZZ MANAGED — regenerated automatically, do not edit below -->
## Active Agents

*(No agents deployed yet. Add agents in the Buzz desktop app.)*

<!-- END BUZZ MANAGED -->
