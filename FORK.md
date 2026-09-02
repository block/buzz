# FORK.md — branch architecture & maintainability contract

`mfethe1/buzz` fork of `block/buzz`. Goal: **pull from upstream cheaply, forever.**
Every rule below exists to keep `git fetch upstream && merge` a boring operation.

## Branch roles — one job each

| Branch | Role | Base | May contain | Push rule |
|---|---|---|---|---|
| `main` | **strict upstream mirror** | — | nothing fork-specific | fast-forward from `origin/main` ONLY |
| `product/main` | tested integration line | `origin/main` | approved fork features | merge only, never rebase (published) |
| `feat/*` | one upstream-portable feature | current `origin/main` | single concern | opened as upstream PR |
| `integration/*` | cleaned composition of community PRs | current `origin/main` | reconciled third-party work | never raw PR heads |
| `infra/*` | fork-only tooling | `origin/main` | CI, CODEOWNERS, fork docs | never merged upstream |
| `rescue/*` | immutable archival refs | — | recovered commits | never developed on |

**The invariant that makes upstream pulls cheap:** `main` has `ahead=0`. If `main`
carries fork commits, every upstream sync becomes a conflict negotiation. Fork-only
work lives in `infra/*` (see `infra/accelerator`, rescued from a `main` violation
on 2026-08-24 — commit `4c251b876`, preserved at `rescue/accelerator-4c251b876`).

## Why `feat/*` and `integration/*` branch from `origin/main`, not `product/main`

A branch based on `product/main` inherits every fork feature, so its diff is no
longer proposable upstream. Basing on `origin/main` keeps each branch a clean
cherry-pick candidate. `product/main` is where they *land*, not where they *start*.

## Landing checklist (a branch is not done without these)

- [ ] rationale + upstream base SHA recorded in the commit message
- [ ] inherited baseline failures identified and NOT silently fixed
- [ ] focused tests for the change
- [ ] **negative control**: revert the core change, prove the test fails
- [ ] gates run **on the merge result**, not just the source branch
- [ ] no unrelated formatting cleanup mixed in
- [ ] linked queue item in `queues/work.yaml`

### Baseline failures are inherited, not owned
`cargo fmt --check` currently fails at `crates/buzz-acp/src/pool.rs:5237` and did so
**before** device identity merged (proven by checking out `cbdb57a8e` and re-running).
Do not reformat unrelated files to make your merge look green — that buries someone
else's problem inside your commit. Record it, route it separately.

## Third-party PR policy

Never merge an unreviewed community PR head directly into `product/main`: if the
author force-pushes or upstream revises it during review, we own the rebase debt
forever. Instead build an `integration/*` branch that takes a **base** and ports
**specific, cited** improvements from the alternative.

## Worktree discipline

`git worktree add` **fails** if the branch is already checked out elsewhere. A
chained `worktree add && cd && merge` then runs the merge in the *current* checkout
— this happened on 2026-08-24 and put a merge on `product/main` unintentionally.

Always:
```bash
set -euo pipefail          # so a failed add cannot fall through
git worktree list          # find who owns the branch first
```

## Sync procedure

```bash
git fetch origin                                   # upstream
git push mine origin/main:refs/heads/main          # mirror must fast-forward
git checkout product/main && git merge origin/main # integrate, then run gates
```
If the mirror push is rejected as non-fast-forward, `main` has drifted: rescue the
commit to `rescue/*`, then restore with an explicit `--force-with-lease=<expected-sha>`.
Never a blind `--force`.
