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

Git authorship, co-authorship, DCO sign-off, and cryptographic signing are separate claims. Follow repository-local rules and the authorizing human's explicit directions; do not infer attribution from repository ownership or from who requested, approved, or reviewed the work.

- **Author:** use the person or agent required by the applicable policy. If no policy specifies an author, use the identity that actually authored the change.
- **Co-authors:** add `Co-authored-by` only for other people or agents who materially authored the change. Request, approval, review, or accountability alone is not co-authorship.
- **DCO:** add `Signed-off-by` only when repository policy requires that identity's DCO certification. A sign-off is not an approval marker.
- **Identity:** resolve required identities from trusted local configuration or explicit verified direction; never hard-code or guess them. A managed runtime may make effective `git config user.*` values identify the agent. Stop and ask if a required identity cannot be established.
- **Signing:** use only the signing key configured for the committing identity. Never use another person's signing key.
- **Verify before pushing:** inspect every outgoing commit against the actual upstream or base and confirm its attribution matches the applicable policy.

A repository may require an accountable human as author and the implementing agent as co-author. An agent-owned repository may use the agent as author and require no human trailer. In both cases, repository-local policy controls.

### Managed runtime default

By default (`BUZZ_GIT_IDENTITY` unset or `agent`), the Buzz runtime makes your agent nostr identity the effective committer: `git config user.email` resolves to `<pubkey>@<relay-host>`, `user.name` to your display name (or npub), and commits are signed with your agent key (NIP-GS). That comes from the runtime's `GIT_CONFIG_*` block, which beats repo and global config. Do not set `user.*` config, `-c user.*`, `--author`, or a separate signing key. The managed `git` also refuses those overrides and verifies pushes, but only when it resolves first on `PATH`: a shell whose startup files reorder `PATH`, or a call to git by absolute path, skips it. This is the identity to treat as the author above; add human `Co-authored-by`/`Signed-off-by` trailers when repository policy requires them.

The operator can opt out per-agent with `BUZZ_GIT_IDENTITY=user`. In `user` mode the runtime installs no attribution machinery and no identity or signing settings — no managed `git`, no injected identity, no signing enforcement — so Git uses the operator's ordinary configuration and fallback environment, which may still supply both. In both modes the spawned environment loses `GIT_AUTHOR_NAME`, `GIT_AUTHOR_EMAIL`, `GIT_COMMITTER_NAME`, `GIT_COMMITTER_EMAIL`, `GIT_CONFIG_PARAMETERS` and `NOSTR_PRIVATE_KEY`. `user` mode also drops inherited `GIT_CONFIG_*` entries for `user.name`, `user.email`, `user.signingkey`, `gpg.format`, `gpg.x509.program`, `commit.gpgSign`, `tag.gpgSign`, `author.name`, `author.email`, `committer.name`, `committer.email`, `include.path` and `includeIf.*.path`, replaces an inherited `nostr.keyfile` with the current relay-auth keyfile, and at launch removes every inherited `PATH` entry whose `.git-identity` marker can be inspected (a Buzz wrapper directory). Relay git authentication works in both modes.

<!-- BEGIN BUZZ MANAGED — regenerated automatically, do not edit below -->
## Active Agents

*(No agents deployed yet. Add agents in the Buzz desktop app.)*

<!-- END BUZZ MANAGED -->
