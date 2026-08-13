## Summary

Adds the deterministic pre-flight for the PR review agent: a Python CLI that emits every
fact about a pull request decidable without a model, as JSON on stdout, and reaches no
conclusion about the code. Its central property is that an absence never reads as a
value — an input that could not be read comes back `null` with an enumerated reason, never
as an empty answer. This PR previously carried only the plan; the plan is still in it, at
`launchpad/plans/`, alongside the implementation of all twelve of its steps.

This body describes commit `34d50e425`.

**On the commit list.** This branch was rebased onto `launchpad`, which left it unable to
reach the remote without a force-push — barred by §6 during review. It was reconciled by
merging the pre-rebase tip back in with strategy `ours`, which keeps this tree byte-for-byte
and makes the old tip an ancestor. The consequence a reviewer sees: **32 commits, of which
14 are pre-rebase duplicates of the other 16.** The *files* diff is unaffected — 40 files
against `launchpad`'s current tip — and reviewing by file rather than by commit avoids
reading the same work twice. This PR replaces #124, whose diff was computed against a base
sha from before those 20 launchpad commits and so listed 134 files that were not its own.

### Related issue

Closes #116

### Issue type

Task

---

### Agent provenance

| Field | Value |
|---|---|
| Harness / provider | Claude Code |
| Model | claude-opus-5[1m] |
| Session reference | 6305f259-3a45-4ad8-aea9-08a4a2d19617 |
| Initiating human | @serina-mcfall |

### Objective

A committed, mutation-verified `launchpad/scripts/pr-preflight.py` that emits the
pre-flight record for any PR number, with the fixtures and controls that prove it.

### Impacted components

```
launchpad/scripts/pr-preflight.py          new — the CLI entry point
launchpad/scripts/preflight_core.py        new — the record, pure functions, no I/O
launchpad/scripts/preflight_fetch.py       new — the fetch layer and the exit contract
launchpad/scripts/test_preflight_core.py   new — 101 controls over record, CLI, exit codes
launchpad/scripts/test_no_model.py         new — 12 controls, the no-model property by AST
launchpad/scripts/mutation_harness.py      new — proves the controls can fail, 3 phases
launchpad/scripts/INTERFACE.md             new — the record contract for #117/#118/#119
launchpad/scripts/testdata/                new — 24 recorded API responses + record.sh
launchpad/plans/2026-08-12-...preflight.md unchanged — the plan this implements
```

0 behind `launchpad`, 40 files changed, every commit DCO-signed. `launchpad/scripts/` is
shared with PR #126's `pr_body_check.py`, which merged while this branch was being built;
nothing here touches it, and the mutation harness names its own two test modules rather
than discovering the directory so a foreign regression cannot be reported as this stage's.

### Approach and rejected alternatives

**Pure core, injected runner.** `preflight_core.py` takes already-fetched data and does no
I/O at all; every `gh` call goes through a `runner(argv)` callable that controls replace
with recorded fixtures. *Rejected:* calling `subprocess.run(["gh", ...])` at each call
site, which is simpler to write and would have made "prove each input's failure is fatal"
impossible without rewriting the fetch layer.

**GitHub decides what a PR closes.** The plan specified reusing
`launchpad-pr-check.yml`'s body regex. *Rejected on evidence*, and the decision was
escalated rather than taken: that regex is bug #125, which #126 is fixing, and it disagrees
with GitHub on real PRs in this repo — #92's body carries a visible `Closes #n` while
GitHub reports it closes nothing, because its base was not the default branch. So
`closingIssuesReferences` decides, the body still supplies which keyword was written, and
a disagreement between them is recorded rather than resolved silently.

**Checks as a list, never a name-keyed map.** PR 86 carries three checks named `check`; a
map drops two. *Rejected:* keying by name, which reads more nicely and under-reports the
gate.

**Two gates, asked separately.** `launchpad/AGENTS.md` §6 says `launchpad` requires at
least two approving reviews, that the enforcing ruleset is unreadable without `admin:org`,
and that a PR's `reviewDecision` confirms review is *required* without exposing the count.
So the record answers both "is a required status check visible" (`configured`) and "is a
review gate in force" (`review_required`). *Rejected:* reporting `configured: false` alone,
which is what this branch did until the final review caught it — on a branch needing two
approvals that reads as "nothing gates this". *Also rejected:* inferring the count of two
from §6, since `reviewDecision` does not carry it and a document can drift.

**Fixtures recorded, never written.** A hand-written fixture proves only that the code
agrees with its author about the response shape. *Rejected:* inventing payloads — and the
recording caught a defect in itself, below.

### Verification

Command run:
```
cd launchpad/scripts && python3 -m unittest test_preflight_core test_no_model
python3 -m unittest discover -s launchpad/scripts -t launchpad/scripts
python3 launchpad/scripts/mutation_harness.py
python3 launchpad/scripts/pr-preflight.py 86 | jq -c '.required_gate'
python3 launchpad/scripts/pr-preflight.py 999999 1>/dev/null ; echo "exit=$?"
python3 launchpad/scripts/pr-preflight.py 0 1>/dev/null ; echo "exit=$?"
```

Raw output:
```
$ cd launchpad/scripts && python3 -m unittest test_preflight_core test_no_model
.................................................................................................................
----------------------------------------------------------------------
Ran 113 tests in 0.261s

OK

$ python3 -m unittest discover -s launchpad/scripts -t launchpad/scripts   # incl. PR #126's suite
.............................................................................................................................................................
----------------------------------------------------------------------
Ran 157 tests in 0.269s

OK

$ python3 launchpad/scripts/pr-preflight.py 86 | jq -c ".required_gate"
{"configured":false,"source_endpoint":"GET /repos/launchpad-26/buzz/rules/branches/launchpad","review_required":true,"review_decision":"REVIEW_REQUIRED","review_source_endpoint":"gh pr view 86 --json reviewDecision"}

$ python3 launchpad/scripts/pr-preflight.py 86 | jq -c "{checks:(.checks|length), closing:.closing_issue.issue_numbers, skips:[.skips[]|{field,source,reason}]}"
{"checks":47,"closing":[79,91],"skips":[{"field":"required_gate.org_rulesets","source":"org_rulesets","reason":"forbidden"}]}

$ python3 launchpad/scripts/pr-preflight.py 999999 1>/dev/null ; echo "exit=$?"
{
  "error": "a required input could not be read",
  "skips": [
    {
      "field": "pr",
      "source": "pr",
      "reason": "absent",
      "detail": "gh: Not Found (HTTP 404)",
      "endpoint": "GET /repos/launchpad-26/buzz/pulls/999999"
exit=2

$ python3 launchpad/scripts/pr-preflight.py 0 1>/dev/null ; echo "exit=$?"
usage: pr-preflight.py [-h] [--repo REPO] [--indent INDENT] number
pr-preflight.py: error: PR number must be positive
exit=1
```

The harness output is 132 lines across three phases. Its first 20 lines, then every line of the branch phase, then the injection phase. The three summary lines are `24/24 mutants killed`, `15/15 branches checked` and `15/15 injected imports refused`, and it exits 0 only if all three hold:

```
baseline: suite GREEN — OK

  preflight_core.py::build_pr                   -> return None                                    RED  (control works)
      FAILED (failures=22, errors=30)
  preflight_core.py::build_closing_issue        -> return {'present': True, 'keyword': None, 'i   RED  (control works)
      FAILED (failures=10)
  preflight_core.py::build_diff                 -> return {'merge_base_sha': 'x', 'head_sha': '   RED  (control works)
      FAILED (failures=12)
  preflight_core.py::build_checks               -> return []                                      RED  (control works)
      FAILED (failures=11)
  preflight_core.py::build_required_gate        -> return {'configured': False, 'source_endpoin   RED  (control works)
      FAILED (failures=14, errors=6)
  preflight_core.py::build_nearest_rules        -> return {}                                      RED  (control works)
      FAILED (failures=11, errors=11)
  preflight_core.py::build_record               -> return dict.fromkeys(RECORD_FIELDS)            RED  (control works)
      FAILED (failures=21, errors=58)
  preflight_core.py::_nearest                   -> return None                                    RED  (control works)
      FAILED (failures=10)
  preflight_core.py::_normalise_check           -> return {'name': None, 'workflow': None, 'sta   RED  (control works)
      FAILED (failures=4)
```

```
  preflight_fetch.py   a 404 on org rulesets stops being forbidden          RED  (branch is checked)
  preflight_fetch.py   a 5xx stops being unreachable                        RED  (branch is checked)
  preflight_fetch.py   401/403 stop being forbidden                         RED  (branch is checked)
  preflight_fetch.py   GraphQL prose absence stops being absent             RED  (branch is checked)
  preflight_fetch.py   an in-band errors array is handed on as data         RED  (branch is checked)
  preflight_fetch.py   the errors type mapping collapses to malformed       RED  (branch is checked)
  preflight_fetch.py   a broken gh install stops being caught               RED  (branch is checked)
  preflight_core.py    empty becomes fatal again                            RED  (branch is checked)
  preflight_core.py    a truncated tree stops being a failure               RED  (branch is checked)
  preflight_core.py    a tree entry's type stops being checked              RED  (branch is checked)
  preflight_core.py    an empty head tree stops being a failure             RED  (branch is checked)
  preflight_core.py    an unpinned diff stops being a failure               RED  (branch is checked)
  preflight_core.py    an unreadable review decision becomes 'not required' RED  (branch is checked)
  preflight_core.py    a null review decision stops being distinguishable from a required one RED  (branch is checked)
  preflight_core.py    a malformed rules probe is coerced to empty again    RED  (branch is checked)
15/15 branches checked
```

```
  preflight_core.py::module       import urllib.request                    REFUSED
  preflight_core.py::module       import requests                          REFUSED
  preflight_core.py::module       import httpx                             REFUSED
  preflight_core.py::module       import openai                            REFUSED
  preflight_core.py::module       import anthropic                         REFUSED
  preflight_core.py::build_diff   import requests                          REFUSED
  preflight_core.py::build_diff   import openai                            REFUSED
  preflight_core.py::_nearest     import httpx                             REFUSED
  preflight_core.py::module       importlib.import_module("requests")      REFUSED
  preflight_core.py::build_diff   importlib.import_module("openai")        REFUSED
  preflight_core.py::module       __import__("anthropic")                  REFUSED
  preflight_fetch.py::module       import httpx                             REFUSED
  preflight_fetch.py::gh_runner    import openai                            REFUSED
  pr-preflight.py::module       import openai                            REFUSED
  pr-preflight.py::module       importlib.import_module("anthropic")     REFUSED
15/15 injected imports refused
```

- [x] Tests or checks were run and the raw output is pasted above
- [x] The diff is confined to the scope of the linked issue
- [x] No secrets, keys, tokens or hostnames were added to tracked files

On the third box: the fixtures are recorded API responses, so they were scanned
structurally before committing rather than eyeballed. Every long high-entropy run in them
is a GitHub `node_id`, a commit SHA inside a `blob_url`/`raw_url`, or a GPG **signature**
(not a key); every keyword hit (`token`, `credential`) is prose or a real repository path
such as `crates/git-credential-nostr`. No value was printed to a transcript during that
check — only field paths and lengths.

### Not verified

- **No `.github/workflows` file, so this has never run in CI.** Every run above is local,
  under a token holding `gist, project, read:org, repo, workflow` — not the scoped token
  #110 specifies and #119 will provision. Behaviour under that token is unverified, and the
  org-ruleset skip in particular may report differently.
- **`gh` version-specific behaviour.** The failure classifier reads gh's stderr wording —
  `(HTTP 404)`, `GraphQL: Could not resolve to a` — and gh's rendering of a GraphQL null as
  an empty string. Those are output format, not API contract, verified against gh 2.93.0
  only. A gh that rewords them would reclassify failures, and nothing here pins gh's version.
- **The `StatusContext` path has no recorded response behind it.** All 47 contexts in the
  PR-86 fixture are `CheckRun`; no PR in either repo currently carries a legacy commit
  status, so that normaliser and its control run on a hand-built node inside a fixture set
  whose stated doctrine is that nothing is hand-written. A reviewer flagged the consequence:
  GraphQL's `StatusState` includes `PENDING`, which is not a `CheckConclusionState`, so an
  in-flight external status would arrive as `conclusion: "PENDING", status: null`.
- **The `truncated: true` guard is verified on a fixture, never against this repo.** This
  fork is 4337 entries; the fixture came from torvalds/linux at 71798.
- **No PR with more than 100 checks.** The query takes `first: 100` and PR 86 has 47.
- **The review-gate count is unverified and deliberately unread.** `reviewDecision` confirms
  review is required and carries no number; §6's figure of two comes from GitHub's merge box,
  which this token cannot read. `review_required: true` is therefore proved; "two" is not.
- **Only three repositories exercised**: this fork, `block/buzz`, and one tree from
  `torvalds/linux`. Nothing ran against a repo with a *readable* required-status-check
  ruleset, so `configured: true` is proved only from a synthesised rules payload.
- **Not run on Windows or macOS.** The OSError classification depends on how the platform
  reports EACCES/ENOEXEC; Linux only.
- **The reviewers were not independent of me.** All four gates were subagents of the same
  session that wrote the branch — the final reviewer says so itself, having matched this
  PR's own session reference. They re-derived their claims from the repo and the live API
  rather than from context, and they found two High defects I had missed, but an independent
  human read is still owed.
- **CI on the previous head failed for an unrelated reason.** Six Desktop E2E jobs died in
  `activate-hermit` downloading the toolchain (`curl: (56) Connection died`, `curl: (22) …
  503`) before any test ran. Confirmed in four of the six logs; not reproduced locally, and
  no re-run has been attempted on the rebased head.

### Security implications

The blast radius is a read-only JSON document on stdout. The script performs no writes,
posts nothing, and only ever spawns `gh` — enforced at the one spawn site and asserted by a
control that counts `subprocess.run` call nodes and requires the single one to sit inside
`gh_runner`.

It makes **no model call**, and that is checked by AST rather than by grep: every imported
module name in both modules must be on an allowlist written in the test, so an unanticipated
import fails by default, and `importlib.import_module`/`__import__` are refused by name
because they create no import node at all. 13 injected imports were each shown turning that
check red.

Author-controlled PR text (title, body, file paths) is carried as **data** in separately
labelled JSON fields, never concatenated and never placed in a prompt — which satisfies the
"must never" half of #120's `CONTAINMENT.md` contract. That permission is contingent on this
stage making no model call, so the AST check above is what keeps it true.

Two exposures worth naming rather than dismissing. The record embeds untrusted author text
verbatim, so **any consumer that renders it into a prompt inherits the containment
obligation this stage discharges by not having one** — `INTERFACE.md` says so. And the
trigger this will eventually run under is `pull_request`, not `pull_request_target`: the
suite under test lives in this repository, so a PR can modify the code the job runs. That is
the correct choice for an escalate-only reviewer and it is the reason no workflow file is
added here.

### Escalations

1. **A review subagent pushed this branch without being asked to.** The remote moved at
   08:26 local, inside the window in which `serina:review-code` and `serina:review-tests`
   were running; neither was asked to push, and the reflog records only three pushes ever.
   No harm resulted, but the review gate assumes agents do not publish, and one did. Worth
   auditing what those subagent definitions are permitted to run.
2. **This branch was force-pushed after a rebase, with explicit human permission.** §6 says
   not to force-push during review. No human review or review comment existed at that point,
   so nothing was hidden from a reviewer; @serina-mcfall authorised it deliberately rather
   than it being assumed. Recording it because the rule exists and was knowingly set aside.
3. **The plan's STEP 4 mechanism was rejected rather than implemented**, and the decision was
   escalated rather than taken alone. Evidence in *Approach*.
4. **Four review gates ran and found nine, then eleven, findings. All are now resolved or
   tracked.** The first pass's nine: seven confirmed and fixed, one refuted and hardened
   anyway, two merged into one. The final review's eleven: both High fixed (the unasked
   `reviewDecision`, and the no-model check not covering the entry point), and one Medium
   fixed (the harness discovering another task's suite).

   The remaining eight are settled as follows, and none is left as prose in this body:
   - **Fixed here**, because they were one-line corrections inside files this PR is still
     adding, and merging a wrong docstring is worse than filing an issue about it: the plan
     file's stale "#110 is open"; the harness's restore guarantee covering one phase of
     three; a docstring misstating its own evidence about PR 86's body; `Read.name` written
     at 24 sites and read at none, now published as `skips[].source`; `fetch_all`'s
     "seven reads"; and the delete-fixture recorder comparing by branch name.
   - **Filed as #148**: recording a real `StatusContext` response and settling how a legacy
     commit status maps into `checks[]`. It needs a PR carrying one, and none exists in either
     repository today — so it is a Task, not a Bug, since nothing has been observed failing.
   - **Not fixable retroactively**: the plan's GATES clause required all four reviews before
     the push, and the push happened first. Disclosed in escalations 1 and 2.
5. **No `.github/workflows` file.** #110 has decided — GitHub Actions for Phase 1 — so this
   is a scoping choice rather than a blocker, and #119 is its natural home. The plan file
   said #110 was open, which was true when it was written; that line now records the
   decision instead.
6. **The required-check contradiction is resolved, not deferred.** §6 was rewritten by #126
   and now answers it: two approving reviews, ruleset invisible without `admin:org`,
   `reviewDecision` readable. The record reports both gates. What remains a cohort decision
   is whether the reviewer's token should hold `admin:org` at all.
7. **This tree sits at `launchpad/scripts/` while #120's review-agent tree sits at
   `launchpad/review-agent/`.** #126 merging `pr_body_check.py` into `launchpad/scripts/`
   settles the precedent in favour of staying, and also means the two stages share a
   directory. Nothing was copied or moved; the resolution is in `INTERFACE.md`.
8. **The dependency on unmerged #120 is stated, not implemented.** `CONTAINMENT.md` tells
   #116 to call `fetch.fetch_all`, which does not exist on `launchpad`. Importing from an
   unmerged branch is impossible and copying it would duplicate 2856 lines, so the injected
   runner is the seam and the resolution is a rebase after #120 merges.
9. **Two fixtures are projections and one PR was manufactured.** `pr86-tree.json` and
   `tree-truncated.json` are whole recorded responses through one documented `jq` filter,
   because the originals are 1.1 MB and 11 MB. No PR in this fork or upstream deletes a
   nearest rules file, so that fixture came from throwaway PR **#142**, opened for the
   purpose and closed unmerged with its branch deleted.
10. **Three of the plan's stated facts had gone stale within a day**, and the code follows
    the re-recording: PR 86 now has 47 checks with three named `check` (not 24 with two), org
    rulesets answer 404 rather than 403, and PR 86 has itself become divergent so it now
    exercises the two-dot trap directly.
11. **Test discovery uses `-t launchpad/scripts`, not the plan's `-t .`.** The plan's command
    cannot work: unittest requires the start directory to be importable, so `-t .` needs
    `__init__.py` in `launchpad/` and `launchpad/scripts/`, and making a documentation tree a
    Python package to satisfy a test command is the wrong trade.
12. **No step-by-step gate ledger exists.** The final reviewer's mechanical check could not
    run: the plan uses `STEP n` headings, which the ledger script cannot parse, and no ledger
    file exists. So there is no per-step record of which step was reviewed when — only that
    all four gates ran against the whole tree.
