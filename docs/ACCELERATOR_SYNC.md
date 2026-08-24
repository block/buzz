# Accelerator fork: syncing with block/buzz

This fork (`mfethe1/buzz`) runs a **vendor-branch pattern** so it can move
faster than upstream review while still absorbing upstream's own hardening
and cleanup work over time. Background: see the "Upstream Ledger" and
"Acceleration Plan" write-ups referenced from project memory
(`project_buzz_accelerated_fork_strategy`).

## Branches

- **`upstream-main`** — a pure mirror of `block/buzz`'s `main`. Fast-forward
  only, never diverges, never receives direct commits. It exists only as a
  clean sync point.
- **`main`** — this fork's active development branch. All feature work and
  outside-contributor PRs land here.

## Syncing upstream into `main`

Run this periodically (weekly, or whenever a security-relevant fix lands
upstream):

```bash
git fetch origin main            # origin = block/buzz
git push fork origin/main:refs/heads/upstream-main   # fast-forward the mirror

git checkout main
git pull fork main
git merge upstream-main          # merge, not rebase — preserves fork history
# resolve conflicts if any, then:
git push fork main
```

Use **merge**, not rebase, for this step — `main` carries commits that
aren't upstream's to rewrite, and a merge keeps both histories intact and
bisectable.

## Restacking downstream feature branches after a sync

If a feature branch was started before a sync and now conflicts, use
[git-spice](https://abhinav.github.io/git-spice/) rather than hand-rebasing:

```bash
gs repo sync        # syncs local main with the fork, retargets merged branches
gs stack restack     # rebases every tracked branch in the stack onto the new main
```

## Upstreaming a change back to block/buzz

block/buzz does not accept stacked PRs from forks (cross-repo PR bases must
live in the upstream repo). When a downstream branch is self-contained and
ready to send upstream:

1. Rebase it onto current `origin/main` (block/buzz's real main, not this
   fork's `upstream-main` mirror) in a scratch branch.
2. Open a normal (non-stacked) PR from that scratch branch against
   `block/buzz:main`.
3. Keep the original branch alive on this fork's `main` regardless of the
   upstream PR's outcome — this fork does not block on upstream review.

## Security posture

Every PR into this fork's `main` — including from first-time or outside
contributors — must pass the **Security Gate** required check
(`.github/workflows/security-gate.yml`: secret scanning + `cargo-deny check`)
and a CODEOWNERS review. Neither is optional for any contributor, regardless
of trust tier. See `.github/CODEOWNERS`.
