# The pre-flight record — what later stages can depend on

Implements [#116](https://github.com/launchpad-26/buzz/issues/116) under PRD
[#109](https://github.com/launchpad-26/buzz/issues/109). This document is the contract
between the pre-flight and the stages that read it.

```bash
python3 launchpad/scripts/pr-preflight.py 86            # JSON on stdout
python3 launchpad/scripts/pr-preflight.py --help        # the SKIP taxonomy and exit codes
python3 launchpad/scripts/mutation_harness.py           # prove the controls can fail
python3 -m unittest discover -s launchpad/scripts -t launchpad/scripts   # 157, incl. #126's
cd launchpad/scripts && python3 -m unittest test_preflight_core test_no_model   # 113, this stage's
```

`launchpad/scripts/` is shared — [#126](https://github.com/launchpad-26/buzz/pull/126)
merged `pr_body_check.py` and its suite into it — so plain discovery there runs both
stages' controls. The mutation harness names this stage's two test modules explicitly
rather than discovering the directory, because a regression in a module it does not own
would otherwise abort it and be reported as the pre-flight's.

## The record

Seven top-level fields, enumerated in `preflight_core`'s module docstring, which is the
list a change must be checked against. Adding a field means adding it there in the same
commit.

| Field | Carries |
|---|---|
| `pr` | number, title, body, labels, base_ref, head_sha |
| `closing_issue` | present, keyword, issue_numbers, source, text_disagrees, text_issue_numbers |
| `diff` | merge_base_sha, head_sha, files[path, added, removed, status] |
| `checks` | [name, workflow, status, conclusion, required, details_url] |
| `required_gate` | configured, source_endpoint, review_required, review_decision, review_source_endpoint |
| `nearest_rules` | per changed path, the resolved AGENTS.md **and** CLAUDE.md |
| `skips` | [field, source, reason, detail, endpoint] |

**Two gates, asked separately.** `launchpad/AGENTS.md` §6 states that the `launchpad`
branch requires at least two approving reviews, that the ruleset enforcing it is
unreadable without `admin:org`, and that a live PR's `reviewDecision` confirms review is
*required* without exposing the count. So `configured` answers "is a required **status
check** gate visible", and `review_required` answers "is a **review** gate in force",
from the one signal this token can read. A consumer must not read `configured: false` as
"nothing gates this branch" — on `launchpad` today it means no required status check is
visible while `review_required` is `true`. The review **count** is not in the record,
because `reviewDecision` does not carry it and §6's figure could drift.

**Three rules a consumer can rely on.**

1. **`null` means "not read". It never means "nothing there".** An unread field is
   `null` with an entry in `skips`; an empty-but-readable one is `[]`, `false` or `{}` as
   appropriate. `checks: null` and `checks: []` are different facts and a consumer that
   treats them alike will publish a review of nothing as a review of something clean.
2. **`skips` is never decoration.** Every `null` field has an entry naming it, with an
   enumerated reason and the endpoint that answered. A clean run on this repository still
   carries one: org-level rulesets are unreadable without `admin:org`.
3. **Exit 0 means the record on stdout is complete enough to use.** Exit 2 means a
   required input was unreadable and **stdout is empty** — there is no half-record to
   mistake for a whole one. Exit 1 is a usage error.

No schema version field is emitted. Whether one belongs here is
[open](https://github.com/launchpad-26/buzz/issues/116) and is a decision for whoever
builds the first consumer, not for this script.

## Containment, and the dependency on #120

[#120](https://github.com/launchpad-26/buzz/issues/120) is the untrusted-input stage. Its
`CONTAINMENT.md` § Contract for later stages names #116 in its table:

> #116 pre-flight — **must call** `fetch.fetch_all(pr, repo)`, emitting one labelled
> field per entry point; **must never** concatenate surfaces into one blob, or build a
> prompt.

**How that dependency resolves, stated rather than assumed.**

- **Nothing was copied.** #120's tree — `contain.py`, `fetch.py`, `detect.py`,
  `review.py`, `suite.py`, `CONTAINMENT.md` — lives on the unmerged branch
  `feat/review-agent-untrusted-input`. It was read, and not duplicated. This branch adds
  no second copy of any of it.
- **The "must never" half is satisfied today.** Every author-controlled surface this
  record carries — title, body, per-file paths — sits in its own labelled JSON string
  field. Nothing is concatenated, and no prompt is built anywhere in the tree. The AST
  controls in `test_no_model.py` are what keep that true: the record module may import
  only `re`, `dataclasses` and `typing`, so it cannot reach a model even by accident.
- **The "must call" half is not satisfied yet, and cannot be.** `fetch.fetch_all` does not
  exist on `launchpad`. Importing a module from an unmerged branch is not something a
  merged script can do, and copying it would create the second copy the task forbids.
- **The seam is already in place.** Every call goes through an injected
  `runner(argv) -> RunResult` (see `preflight_fetch.fetch_all`). Adopting #120's fetcher
  is therefore an adapter at one boundary, not a rewrite: `fetch_all` gains a source for
  the three surfaces it shares with #120 — `pr_title`, `pr_body`, `pr_diff` — and
  `preflight_core` does not change at all, because it never fetches anything.

**Resolution: rebase after #120 merges, then adopt `fetch.fetch_all`.** Until then the
two trees are independent by necessity, and the surfaces overlap in three of #120's seven
entry points. The pre-flight reads none of the four comment surfaces, which is why it
needs no envelope: it makes no model call, and CONTAINMENT.md's own note says that
permission is contingent on exactly that.

**One structural question for a reviewer.** #120 put its tree at
`launchpad/review-agent/`; this plan assumed `launchpad/scripts/`, where PR #126 is also
adding `pr_body_check.py`. Both are defensible and they should not stay split: co-locating
the review agent's stages makes the #120 adapter a local import rather than a path
dance. Moving this tree under `launchpad/review-agent/` is a `git mv` plus one path in
`INTERFACE.md`, and it is the reviewer's call, not this PR's.
