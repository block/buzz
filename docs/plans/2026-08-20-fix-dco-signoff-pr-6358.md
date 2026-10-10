---
title: Fix DCO Sign-off on PR 6358 - Plan
type: fix
date: 2026-08-20
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
product_contract_source: ce-plan-bootstrap
execution: code
---

# Fix DCO Sign-off on PR 6358 - Plan

## Goal Capsule

- **Objective:** Make the DCO Check pass on PR block/buzz#6358 by adding the required `Signed-off-by:` trailer to both commits on branch `fix/flaky-passphrase-test`.
- **Authority:** The settled decisions in this plan's KTDs are binding; implementation must not expand scope.
- **Stop conditions:** The DCO Check reports success on the PR head; the branch is force-pushed to the author's fork; no code content changes.
- **Execution profile:** Single-unit, commit-metadata-only fix (no source changes).
- **Tail ownership:** The implementing agent owns the rewrite, push, and verification.

## Product Contract

### Summary

The DCO Check on PR #6358 fails with "All commits are missing DCO sign-off". Both commits (`37b8027`, `50d1833`) carry a `Co-authored-by: CommandCodeBot` trailer but no `Signed-off-by:` trailer. The repo's DCO app requires every commit to carry a `Signed-off-by:` line matching the author (`Som Samantray <som.samantray@gmail.com>`). The fix adds that trailer to both commit messages.

### Problem Frame

DCO (Developer Certificate of Origin) is enforced as a required status check. Without the `Signed-off-by:` trailer on every commit in the PR, the check fails and the PR cannot merge. The commits are on the author's own fork branch with no shared history, so amending them is safe.

### Requirements

- R1. Every commit on branch `fix/flaky-passphrase-test` must carry a `Signed-off-by: Som Samantray <som.samantray@gmail.com>` trailer.
- R2. The existing commit content (test-file changes and plan document) must remain byte-for-byte identical.
- R3. The branch must be pushed to the author's fork (`SomSamantray/buzz`) such that PR #6358's head updates.
- R4. The DCO Check must report success on the new head.
- R5. The `Co-authored-by: CommandCodeBot` trailer on each commit must be preserved.

### Actors

- A1. The DCO app (`https://block.xyz`), which inspects commit trailers.
- A2. The author's fork remote (`fork` = `https://github.com/SomSamantray/buzz.git`).

### Acceptance Examples

- AE1. **Covers R1, R4.** After the fix, `gh pr checks 6358` shows the DCO Check passing on the PR head.
- AE2. **Covers R2, R3.** `git diff origin/main...HEAD --stat` shows the same 2 files and insertion counts as before the fix, and the fork branch head equals the new local HEAD.
- AE3. **Covers R5.** Both commits still carry the `Co-authored-by: CommandCodeBot <noreply@commandcode.ai>` trailer.

### Scope Boundaries

- **In scope:** Adding `Signed-off-by:` trailers to the two commits and pushing to the fork.
- **Out of scope:** Changing any source code, changing commit messages beyond the sign-off trailer, touching the plan document, and fixing any other CI checks.

### Dependencies

- Writable fork remote `fork` (confirmed: `git remote -v` lists it).
- `gh` CLI authenticated as `SomSamantray`.

## Planning Contract

### Key Technical Decisions

- KTD1. Amend both commits in place with `git rebase --exec 'git commit --amend --no-edit --signoff' origin/main`, then force-push to the fork. **(session-settled: user-directed — chosen over closing/re-opening the PR with fresh commits: amending preserves the PR thread, review context, and branch.)**
  - Rationale: `git commit --amend --signoff` appends the `Signed-off-by:` trailer matching the committer. Running it on each commit via `rebase --exec` rewrites only the two branch commits. The branch lives only on the author's fork, so a `--force-with-lease` push is safe and is the standard DCO remediation.
- KTD2. Use `git push --force-with-lease fork HEAD` rather than a bare `--force`.
  - Rationale: `--force-with-lease` refuses to clobber the remote if it moved since the last fetch, protecting against overwriting an unexpected concurrent update. The fork is the author's own, so this is low-risk but still the correct guard.
- KTD3. Verify by checking the DCO status, not just the local commits.
  - Rationale: The DCO app is the source of truth. `gh pr checks 6358` (or the check-runs API) must show success after the push. The DCO app evaluates the PR head, which only updates after the force-push lands.

### Assumptions

- The DCO app recognizes `Signed-off-by: <Author Name> <author email>` where the author email matches the commit author (`som.samantray@gmail.com`). This is the standard DCO contract.
- No other contributor has committed to the fork branch since the last push.

### Sequencing

- Single unit. No cross-unit ordering constraints.

## Implementation Units

### U1. Add DCO sign-off to both commits and push

- **Goal:** Make the DCO Check pass on PR #6358.
- **Requirements:** R1, R2, R3, R4, R5
- **Files:**
  - No source files. Commit metadata only (`desktop/src-tauri/src/key_backup_tests.rs` and `docs/plans/2026-08-19-fix-flaky-passphrase-test.md` contents unchanged; only the commit messages gain the trailer).
- **Approach:**
  - Confirm the current branch and that there are no tracked modifications (`git status --porcelain` shows no tracked changes and no rebase in progress; the untracked plan document `docs/plans/2026-08-20-fix-dco-signoff-pr-6358.md` is expected and out of scope).
  - Assert the rewrite scope: `git rev-list --count origin/main..HEAD` must equal 2, aborting if not (guards against a third commit landing between planning and execution).
  - Run `git rebase --exec 'git commit --amend --no-edit --signoff' origin/main` to append the `Signed-off-by:` trailer to both commits.
  - Verify `git log --format='%h %s%n%b' origin/main..HEAD` shows the trailer on both commits and that the `Co-authored-by` trailer is preserved.
  - Verify the tree content is unchanged: `git diff origin/main...HEAD --stat` matches the pre-fix state (same 2 files, same insertion counts).
  - Push with `git push --force-with-lease fork HEAD`.
  - Verify the DCO check on the PR head via `gh pr checks 6358 --repo block/buzz` (or the check-runs API), polling until the DCO app re-evaluates the new head.
- **Patterns:** Standard DCO remediation (`git commit --amend --signoff`); the repo's CONTRIBUTING/AGENTS conventions already require DCO sign-off.
- **Test Scenarios:**
  - TS1. Both commits in `origin/main..HEAD` contain `Signed-off-by: Som Samantray <som.samantray@gmail.com>`.
  - TS2. The `Co-authored-by: CommandCodeBot <noreply@commandcode.ai>` trailer is still present on both commits.
  - TS3. `git diff origin/main...HEAD --stat` is unchanged from before the fix (2 files: the test file and the plan doc).
  - TS4. The remote fork branch head equals the new local HEAD after the push.
  - TS5. The DCO Check on PR #6358 reports success (may take a moment to re-run after the push).
- **Verification:**
  - `gh pr checks 6358 --repo block/buzz` shows DCO Check passing.
  - `git log --format=fuller origin/main..HEAD` shows the sign-off trailers.

## Verification Contract

- **Primary command:** `gh pr checks 6358 --repo block/buzz`
- **Supporting:** `git log --format='%h %s%n%b' origin/main..HEAD` and `git diff origin/main...HEAD --stat`
- **Quality gates:**
  - DCO Check reports success on the PR head.
  - No source file contents changed (verify with `git diff origin/main...HEAD --stat`).
  - The `Co-authored-by` trailers are preserved.
  - No unrelated commits or changes introduced.

## Definition of Done

### Global

- The DCO Check passes on PR #6358.
- The branch is force-pushed to the author's fork.
- No source code changed.

### Per-Unit

- U1. Done when both commits carry the sign-off trailer, the fork branch is updated, and the DCO Check reports success.
- Cleanup: no stray files, no leftover rebase state, no tracked modifications (`git status --porcelain` shows no tracked changes and no rebase in progress; the untracked plan document is expected and out of scope), no scratch artifacts in the diff.
