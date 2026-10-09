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

## Git Commit Attribution

When Buzz runs you through its `buzz-acp` harness, the runtime sets your git identity and signing: you commit as yourself (your display name and `<pubkey>@<relay-host>`), signed with your agent key. Don't override it with `user.*` config, `-c user.*`, `--author` or another signing key. The operator can switch an agent to their own git identity with `BUZZ_GIT_IDENTITY=user`.

Credit the other party with a `Co-authored-by` trailer: when you commit as yourself, credit the human you work for (`git config --global user.name` and `user.email` give their identity); when you commit as them, credit yourself. Follow each repository's own `AGENTS.md` for sign-off (DCO) and other attribution rules.

<!-- BEGIN BUZZ MANAGED — regenerated automatically, do not edit below -->
## Active Agents

*(No agents deployed yet. Add agents in the Buzz desktop app.)*

<!-- END BUZZ MANAGED -->
