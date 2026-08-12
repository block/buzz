# Pre-flight fixtures

Every file here was **recorded from the live API** by [`record.sh`](record.sh) on
**2026-08-13**, never hand-written. A hand-written fixture proves only that the code
agrees with its author's belief about the response shape; the shapes below are the ones
GitHub actually returns, including the parts nobody would have thought to invent.

Re-record with `launchpad/scripts/testdata/record.sh` and read the diff.

| Fixture | What it is | What it proves |
|---|---|---|
| `pr86-pr.json` | `GET /repos/{o}/{r}/pulls/86` | the PR identity read — base ref, head sha |
| `pr86-meta.json` | `gh pr view 86 --json title,body,labels` | the metadata read |
| `pr86-checks.json` | GraphQL `statusCheckRollup` with `isRequired` | **47 checks, three named `check`** — names collide, so the record carries a list |
| `pr86-compare.json` | `GET /compare/{base.sha}...{head.sha}` | merge base and per-file counts |
| `pr86-tree.json` | `GET /git/trees/{head}?recursive=1` | the head tree nearest-rules resolves against |
| `pr92-meta.json` | a PR whose body carries `Closes #n` in visible text | the keyword-present case |
| `upstream5695-meta.json` | a PR whose **only** `Fixes #n` sits inside `<!-- -->` | an unfilled template placeholder is not a closing reference |
| `pr-notfound.json` | `GET /pulls/999999` | a 404 body; the exit code is in `pr-notfound.exit` |
| `rules-branches-launchpad.json` | `GET /repos/{o}/{r}/rules/branches/launchpad` | `[]` — readable **and** empty, which is a fact, not a failure |
| `orgs-rulesets-forbidden.json` | `GET /orgs/launchpad-26/rulesets` | this token cannot see org rulesets |
| `upstream-divergent-pr.json` | an upstream PR whose base tip is **ahead** of its fork point | `baseRefOid` is not the merge base |
| `upstream-divergent-compare.json` | its compare response | the merge base a three-dot diff needs |
| `pr14-compare.json`, `pr14-tree.json` | PR 14 — **adds** `launchpad/AGENTS.md` | the rules-file ADD direction, real and merged |
| `prdelete-compare.json`, `prdelete-tree.json` | throwaway PR #142 — **deletes** `launchpad/AGENTS.md` | the DELETE direction (see below) |
| `tree-truncated.json` | a tree past the API's size limit | `truncated: true` arrives with **HTTP 200** |

## Four things worth knowing

**Every compare is recorded by SHA, `base.sha...head.sha`, never by branch name.**
A branch name resolves to its tip *today*, and for a merged PR the tip already contains
the head — so `compare/launchpad...{head}` answers `200 OK` with **zero files**: a real
six-file PR rendered as a PR that changed nothing. These fixtures were recorded that way
once, and the empty file list is what caught it. Same class of error as the two-dot trap,
one layer down.

**The check count moved.** The plan for #116 recorded 24 checks on PR 86 with two named
`check`. On 2026-08-13 both `gh pr view --json statusCheckRollup` and GraphQL report
**47**, with **three** named `check` and 23 names duplicated overall. The property the
fixture exists for is stronger than before, not weaker, and the controls assert against
the recorded fixture rather than a number copied out of the plan.

**PR 86 has become divergent, which makes it a better fixture than the plan expected.**
Its recorded `base.sha` is `b3db9afbc` while its merge base is `d897a06e8` — `launchpad`
moved 19 commits under it, so the compare reports `status: diverged`, `behind_by: 6`. The
plan for #116 said PR 86 could not exercise merge-base correctness because its base tip
and merge base were the same commit. That is no longer true, so a two-dot implementation
now fails on PR 86 as well as on the deliberately-divergent upstream fixture.

**The forbidden case is a 404, not a 403.** `GET /orgs/launchpad-26/rulesets` answers
`404 Not Found` for a token without `admin:org` — `launchpad-26` is an Organization
(verified), so the 404 hides access, it does not report absence. A 404 here therefore
cannot be read as "this org has no rulesets", which is exactly why org-level ruleset
visibility is a reported SKIP rather than a silent zero.

**PR #142 was manufactured, and closed unmerged.** No pull request in this fork's
history and none found upstream deletes a nearest rules file, so the DELETE direction
was recorded from a throwaway PR opened by
[`record-delete-fixture.sh`](record-delete-fixture.sh) and closed minutes later with its
branch deleted. It was never merged and never reviewed. The add case is real (PR 14);
the delete case had to be made, and saying so here is the point — a resolver that reads
the local worktree instead of the PR's head tree passes the add case and fails this one.

## Two fixtures are projections

Both are whole recorded responses passed through one documented `jq` filter, because the
whole thing is megabytes of entries nothing reads. Every key of every retained entry is
untouched — only irrelevant entries were dropped, and the filter is in `record.sh`:

- `pr86-tree.json` — 4337 entries → the 120 `*.md` entries (1.1 MB → 30 KB)
- `tree-truncated.json` — 71798 entries → the first 20 (11 MB → 4 KB); it exists for its
  `truncated: true` flag

`endswith(".md")` is deliberately wider than the rules files themselves: it keeps 116
markdown paths that are *not* rules files, so a resolver matching on the wrong suffix has
something to trip over — `VISION_REMOTE_AGENTS.md` ends with `AGENTS.md` and is not one.
