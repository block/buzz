Issue #116 — task: deterministic pre-flight for the PR review agent
Stated size: none given — the task template has no Size field  ->  cap: 12 steps

Sized by asking, not guessing. Six done-criteria, one of which is a control suite
whose controls must each fail when their check is deleted. Answered: more than an
hour, so the cap is 12. Language answered: Python 3.

Larger than an hour is flagged, not refused. These would have been better issues,
each observable on its own — splitting is the reader's call, not this plan's:

  (a) emit the PR facts record — title, body, labels, closing keyword, diff
  (b) resolve required checks, which has an unanswered config question behind it
  (c) resolve the nearest AGENTS.md/CLAUDE.md per changed path — no network needed
  (d) the SKIP taxonomy and the mutation-checked control suite

Planned as written below.

ALREADY TRUE  (verified against git and the GitHub API, not notes)
  Branch feat/review-agent-preflight is at d897a06e8 — `git rev-list --left-right
  --count origin/launchpad...HEAD` reports 0 0, so nothing of #116 is built.
  No pre-flight script exists: `git ls-files | grep -iE 'preflight|pre-flight'`
  returns nothing. launchpad/ holds AGENTS.md, AGENT_PR_TEMPLATE.md, README.md,
  labels.yml, sync-labels.sh, decisions/ — and no scripts/ directory.
  Root scripts/ and root docs/ are upstream's trees. launchpad/AGENTS.md §3 puts
  cohort files under launchpad/, so both are off-limits for this work.
  Toolchain present: python3 3.12.3, gh 2.93.0.
  `gh pr view --json statusCheckRollup` does NOT expose isRequired — verified on
  PR 86, whose keys are only __typename, completedAt, conclusion, detailsUrl,
  name, startedAt, status, workflowName. isRequired comes from GraphQL only, as
  isRequired(pullRequestNumber:).
  PR 86 carries 24 checks and two of them are both named "check" from the same
  workflow "launchpad — PR body check". Check names are NOT unique.
  CORRECTED 2026-08-13, one day later: the live API and the recorded fixture both
  report 47 contexts with THREE named "check" and 23 names duplicated overall.
  Every "24" below is stale in the same way. The property each criterion tests —
  names collide, so the record must carry a list — holds more strongly, not less,
  and the controls assert against the recorded fixture rather than against these
  numbers.
  No branch protection is readable on launchpad: /rulesets returns [],
  /branches/launchpad/protection 404s, and /rules/branches/launchpad returns [].
  Every check on PR 86 reports isRequired false. This contradicts
  launchpad/AGENTS.md §6 ("The launchpad branch is protected"), and the token
  cannot see org-level rulesets — /orgs/launchpad-26/rulesets needs admin:org and
  the token holds gist, project, read:org, repo, workflow.
  No verify gate is installed in this checkout — .claude/settings.json and
  settings.local.json are both absent, so review skills are manual invocations.
  Upstream precedent for a testable script is a core module beside its test:
  scripts/check-file-sizes-core.mjs and scripts/check-file-sizes-core.test.mjs.
  The closing-keyword regex already exists in .github/workflows/launchpad-pr-check.yml
  line 39 — `\b(Closes|Fixes|Resolves)\s+#\d+` — applied only after HTML comments
  are stripped (line 28).
  There is no `mergeBaseCommit` field on the GraphQL PullRequest type. Verified by
  introspecting `__type(name:"PullRequest"){fields{name}}`: the merge- and base-
  named fields are autoMergeRequest, baseRef, baseRefName, baseRefOid,
  baseRepository, canBeRebased, mergeCommit, mergeStateStatus, mergeable, merged,
  mergedAt, mergedBy, potentialMergeCommit and the viewer/queue variants. The
  merge base is REST-only: /repos/{o}/{r}/compare/{base}...{head} ->
  .merge_base_commit.sha.
  PR 86 cannot exercise merge-base correctness: its baseRefOid, the current tip
  of launchpad, and its merge_base_commit.sha are all d897a06e8. Every open PR in
  this fork is in that state. A divergent case exists upstream — block/buzz#5625,
  baseRefOid 4b3570671 against merge base 5bf78671f.
  The git trees API reports partial results as HTTP 200 plus `truncated: true`
  rather than an error. At d897a06e8 this repo returns 4332 entries with
  `truncated: false`, well inside the ~100,000-entry limit.
  PR 14 in this fork adds AGENTS.md (+22) and launchpad/AGENTS.md (+244), so the
  rules-file-ADD fixture has a real source. No PR in this fork's seven-PR history
  deletes a nearest rules file, and none was found upstream, so the DELETE
  fixture has to be manufactured.
  ast.walk finds imports nested in function bodies where tree.body does not, and
  neither sees importlib.import_module — both verified by parsing a sample module
  and comparing the collected name sets.

STEP 1  Record real gh/GraphQL responses as fixtures under              [independent]
        launchpad/scripts/testdata/ — seven of them, recorded from the live API
        and never hand-written:
          (i)   PR 86 — 24 checks, two sharing the name "check"
                [recorded 2026-08-13: 47, three sharing it]
          (ii)  a PR carrying a closing keyword
          (iii) a PR whose only `Closes #n` sits inside an HTML comment
          (iv)  a 404 PR number
          (v)   the empty /rules/branches/launchpad response
          (vi)  a DIVERGENT-base PR, where the base branch tip is ahead of the
                commit the head branch actually forked from. Every open PR in
                this fork currently has base tip == merge base, so this fixture
                must come from upstream: block/buzz#5625 has baseRefOid
                4b3570671 against merge base 5bf78671f. Without it nothing in
                this plan can tell a three-dot diff from a two-dot one.
          (vii) two PRs, not one, because the add and delete directions have very
                different provenance. ADD is real and already in this fork:
                PR 14 adds AGENTS.md (+22) and launchpad/AGENTS.md (+244) among
                18 files. DELETE has no real source — no PR in this fork's
                history, and none found upstream, deletes a nearest rules file.
                Open a throwaway PR in the fork that deletes one, record the
                response, close it unmerged. Say so in the PR body; do not let
                the add case stand in for both by assertion.
        Fixture (vi)'s SHAs are current as of 2026-08-12 on a still-open upstream
        PR. Re-confirm at recording time that baseRefOid and merge_base_commit.sha
        still differ, rather than trusting the SHAs quoted here — if that PR has
        merged, any actively-developed repo will have another PR with the same
        property.
        done when: each fixture file parses under `python3 -c 'import json,sys;
        json.load(open(sys.argv[1]))'`; the PR-86 checks fixture contains 24
        entries of which at least two share the name "check"; fixture (vi)'s
        recorded baseRefOid and merge_base_commit.sha are different strings; and
        (vii) is two recorded PRs, one adding and one deleting a rules file.

STEP 2  launchpad/scripts/preflight_core.py — pure functions over               [needs 1]
        already-fetched data, no subprocess and no network in this module. The
        record's `checks` field is a LIST, never a name-keyed map, because PR 86
        proves names collide and a map would silently drop an entry.
        The record's fields are enumerated HERE, so that later steps have a fixed
        list to be checked against rather than an open-ended "every field":
          pr             number, title, body, labels, base_ref, head_sha
          closing_issue  present, keyword, issue_number
          diff           merge_base_sha, head_sha, files[path, added, removed,
                         status]
          checks         [name, workflow, status, conclusion, required,
                         details_url]
          required_gate  configured, source_endpoint
          nearest_rules  per changed path, the resolved AGENTS.md and CLAUDE.md
          skips          [field, reason]
        Adding a field to the record means adding it to this list in the same
        commit. Whether the record also carries a schema version is OPEN below.
        done when: `python3 -m py_compile launchpad/scripts/preflight_core.py`
        succeeds; building the record from the PR-86 fixture yields exactly as
        many check entries as the fixture holds — 24 when this was written, 47 as
        recorded — and the record's top-level keys equal the seven names above.

STEP 3  launchpad/scripts/pr-preflight.py — the CLI shell. Takes a  [needs 2]  <- RUNS HERE
        PR number, prints the record as JSON on stdout, and takes its command
        runner as an injected argument defaulting to real `gh`, so every later
        control can feed fixtures without a network call.
        done when: `python3 launchpad/scripts/pr-preflight.py 86` prints JSON
        whose title matches `gh pr view 86 --json title -q .title` and whose
        checks array has as many entries as the fixture holds (24 when written,
        47 as recorded), exiting 0; and a nonexistent PR number exits non-zero
        rather than printing a record.

STEP 4  Title, body, labels, and closing-keyword detection — reusing            [needs 3]
        launchpad-pr-check.yml's regex and its comment-stripping order, so a
        `Closes #n` that appears only inside `<!-- -->` does not count as present.
        done when: on the closing-keyword fixture the record names the keyword
        and the issue number; on the commented-out fixture `closing_keyword` is
        absent with a recorded reason, and the run does not report it as present.

STEP 5  The merge-base diff — per-file path with added/removed counts,           [needs 3]
        plus the merge-base and head SHAs, so the record is pinned to the commit
        pair it read. The merge base comes from REST:
        `GET /repos/{o}/{r}/compare/{base}...{head}` -> `.merge_base_commit.sha`.
        There is NO `mergeBaseCommit` field on the GraphQL PullRequest type —
        introspection lists baseRef, baseRefName, baseRefOid, mergeCommit,
        potentialMergeCommit and nothing resembling a merge base. Do not reach
        for GraphQL here.
        The trap this step exists to avoid: `baseRefOid` is the CURRENT TIP of
        the base branch, not the commit the head forked from. Diffing against it
        two-dot attributes every commit landed on launchpad since the fork to
        this PR's author, in reverse.
        done when: on PR 86 the record's sorted file paths equal `gh pr diff 86
        --repo launchpad-26/buzz --name-only | sort`; AND on the divergent
        fixture (vi) the recorded base SHA equals its merge_base_commit.sha and
        is NOT equal to its baseRefOid — which a two-dot implementation fails,
        where on PR 86 alone it would pass, because PR 86's base tip and merge
        base are the same commit (d897a06e8).

STEP 6  Required checks, from both sources. Per-context isRequired via           [needs 3]
        GraphQL, plus a probe of /repos/{o}/{r}/rules/branches/{base} and
        /rulesets, so an empty required set is reported as
        `required_checks_configured: false` with the endpoint that answered —
        never as a silent zero, which is indistinguishable from a scope failure.
        done when: on PR 86 the record shows every check (24 when written, 47 as
        recorded) each with required false,
        names which endpoint supplied that, and — because the token lacks
        admin:org — carries a SKIP for org-level rulesets rather than asserting
        that none exist.

STEP 7  Nearest-rules resolution: for each changed path, walk to the            [needs 5]
        repository root collecting the nearest AGENTS.md AND the nearest
        CLAUDE.md. Both, not first-wins — launchpad/ has only AGENTS.md while
        the root has both, so first-wins would hide the root CLAUDE.md from
        every launchpad/ path. Resolved against the PR's head tree, not the
        local worktree, or the answer is wrong for any PR not checked out.
        done when: a changed `launchpad/AGENTS.md` resolves to launchpad/AGENTS.md
        plus root CLAUDE.md; a changed `desktop/src/main.tsx` resolves to root
        AGENTS.md plus root CLAUDE.md; a control passes with no local checkout of
        the head branch present; and on fixture (vii) — a PR that deletes or adds
        the nearest rules file for a path it also touches — resolution reflects
        the head tree, so a deleted `launchpad/AGENTS.md` falls back to the root
        one and a newly added one is picked up. Rules files under launchpad/ are
        actively edited, so this is a case this repo will actually produce.

STEP 8  The SKIP taxonomy and the exit contract — one enumerated reason  [needs 4, 5, 6, 7]
        per unreadable input, distinguishing absent (404), forbidden (403 or
        missing scope), malformed, and empty-but-readable.
        "Required input" must be defined, because the DoD's exit-code rule is
        meaningless without it and "SKIP everything, always exit 0" would
        otherwise satisfy every other criterion. This plan classifies:
          REQUIRED, and a failure to read exits non-zero — the PR itself, its
          metadata (title, body, labels), the compare/diff response, the check
          list, and the head-tree listing STEP 7 resolves against.
          SKIP-ONLY, absence is a legitimate reportable state and exit stays 0 —
          org-level ruleset visibility (unreadable without admin:org), and a
          changed path having no ancestor AGENTS.md or CLAUDE.md at all.
        One trap makes those two categories bleed into each other, and it must be
        closed explicitly: the git trees API does NOT fail loudly when it cannot
        return a whole tree. It answers HTTP 200 with `truncated: true` and a
        partial list once a tree passes roughly 100,000 entries or 7MB. A path
        whose rules file sits beyond the truncation boundary then looks exactly
        like the SKIP-ONLY "no ancestor file" case while the real cause is an
        incomplete REQUIRED read. This fork is nowhere near the limit today —
        4332 entries at d897a06e8, `truncated: false` — but this is a CLI that
        takes any PR number, so the guard is required, not hypothetical:
        `truncated: true` is a head-tree-listing failure and exits non-zero.
        That classification is a judgement, not a fact, and a reviewer may move
        any line of it.
        done when: the enumerated reasons and the required/SKIP-only split are
        listed in the module's docstring and in `--help`; every reason is
        reachable from at least one fixture; for EACH of the five required
        inputs, a control that makes just that one call fail asserts the process
        exits non-zero — not only the nonexistent-PR case STEP 3 already covers;
        and a control feeding a `truncated: true` trees response asserts a
        non-zero exit rather than an empty nearest_rules result.

STEP 9  launchpad/scripts/test_preflight_core.py — a control per                [needs 8]
        done-criterion plus one per SKIP reason, all fixture-driven with the
        runner injected. This suite, not any grep, is the mechanical proof that
        an absence never reads as a value: for each of the seven record fields
        enumerated in STEP 2, a control feeds an empty, malformed and erroring
        response in turn and asserts the field comes back as a SKIP carrying a
        reason, never as a value and never as PASS. Seven is a list someone can
        count; "every field" was not, which is why STEP 2 now fixes it.
        done when: `python3 -m unittest discover -s launchpad/scripts -t .`
        passes; the suite runs with no network — asserted by injecting a runner
        that raises if invoked with the real `gh` binary; and the empty,
        malformed and erroring variants above each have a named test.

STEP 10 Prove each control can fail. For every check function, neuter it        [needs 9]
        to a constant and confirm the suite goes red — the done-criterion is that
        each control fails if its check is deleted, and a control never observed
        failing has not been shown to test anything.
        done when: a recorded run shows, for each check function, the suite
        failing while that function is neutered and passing when restored, with
        that raw output pasted into the PR body.

STEP 11 Assert the no-model property, by AST and not by grep. A control    [needs 9]
        parses preflight_core.py with `ast`, collects every imported module name,
        and asserts that set is a subset of a written allowlist — so any new
        import fails the control by default rather than only the handful anyone
        thought to name in a regex. A grep cannot do this: it reports every
        occurrence and leaves a human to sort legitimate hits from real ones,
        which is not a check that can fail.
        Two ways an import evades that scan, both closed here:
          Traverse with `ast.walk(tree)`, NOT `tree.body`. Verified: for a module
          whose only top-level import is json and which does `import requests`
          inside a function, tree.body yields {json} while ast.walk yields
          {json, requests}. A top-level-only scan would satisfy a done-when that
          did not say where the import went.
          Collect `Call` nodes too, and forbid `importlib.import_module` and
          `__import__` by name. `importlib.import_module("openai")` is a Call
          node, never an Import node — verified: an import-name scan of a module
          containing that line reports it as absent. One line evades an
          import-only check while breaking the issue's no-model rule outright.
        done when: adding `import urllib.request`, `requests`, `httpx`, `openai`
        or `anthropic` to preflight_core.py each turn the control red, verified
        by doing it and pasting the output; the same holds when the import is
        placed inside a function body rather than at module level; a line reading
        `importlib.import_module("requests")` also turns it red; the allowlist is
        written in the test rather than inferred; and a control asserts `gh` is
        the only binary the injected runner is ever asked to spawn.

STEP 12 Open the PR against launchpad — AGENT_PR_TEMPLATE.md filled,      [needs 10, 11]
        `Closes #116`, `by:agent`, raw output pasted, and Escalations naming
        both deferrals: no workflow file until #110 decides where this runs, and
        the required-check contradiction found in ALREADY TRUE.
        SUPERSEDED 2026-08-13: a workflow job IS added, running this stage's
        controls and its mutation harness on every PR. That is not the invocation
        #110 gates — it needs no token and no network — and the deferral of the
        live pre-flight run to #119 stands.
        done when: the PR is open against launchpad with the by:agent label, the
        "launchpad — PR body check" check is green, and the body names #110 as
        the reason no .github/workflows file INVOKES THE PRE-FLIGHT. A workflow
        job running the controls was added on 2026-08-13; it needs no credential,
        so #110 never gated it.

PARALLEL  Nothing here should be fanned out as written. Steps 4, 5 and 6 are
  logically independent and all tagged [needs 3], but all three edit
  preflight_core.py, and two steps editing one file are sequential regardless of
  how unrelated they look. There is one way to earn the fan-out: if step 2 lands
  one module per check (checks/body.py, checks/diff.py, checks/required.py) then
  4, 5 and 6 touch disjoint files and can run as three parallel subagents, with
  step 7 still waiting on 5. That is a real design choice with a real payoff, and
  it belongs to whoever executes this. Steps 8 through 12 are strictly
  sequential — each reads the whole surface the previous one produced. Step 1 is
  the only step that is independent as written. Nothing is dispatched here.

GATES  No verify gate is installed in this checkout — .claude/settings.json is
  absent — so every gate below is a manual invocation and none of them will fire
  on their own. Run serina:review-code and serina:review-tests after step 10,
  then serina:review-adjudicate, then serina:review-final — all before the push
  in step 12, because a review posted after the push only documents what already
  shipped.
  serina:review-plan has run TWICE on this file, both times before step 1, and
  this revision is the result of both passes. Twelve findings total, all applied,
  none disputed.
  First pass — six findings, three Blockers: a GraphQL merge-base field that does
  not exist (STEP 5), a fixture set that could not tell a three-dot diff from a
  two-dot one (STEP 1, STEP 5), an exit contract titled but never defined or
  tested (STEP 8), a grep that could not fail standing in for a control (STEP 9,
  STEP 11), an untested rules-file-churn case (STEP 7), a prose/command mismatch
  over urllib (STEP 11).
  Second pass — six findings, no Blockers, all against the first pass's own
  fixes: a truncated trees response masquerading as a legitimate no-rules-file
  SKIP (STEP 8), "every field of the record" being unenumerable so STEP 2 now
  fixes the seven field names (STEP 2, STEP 9), ast.walk versus tree.body
  traversal depth (STEP 11), importlib.import_module evading an import-node scan
  entirely (STEP 11), fixture (vii)'s delete half having no real source (STEP 1),
  and fixture (vi)'s SHAs being pinned to a live upstream PR (STEP 1).
  Both passes independently re-verified the API claims in ALREADY TRUE against
  the live API. Everything held except the merge-base field. The second pass has
  not re-reviewed the fixes applied for its own findings, so this revision is
  once-reviewed at the margin. A clean check-plan.sh run remains mechanical only
  and judges nothing about whether the steps are right.

BUDGET  Steps 8 and 9 eat the budget, and step 3 decides how badly.
  The controls need every gh and GraphQL response fakeable. If step 3 hardcodes
  `subprocess.run(["gh", ...])` at each call site instead of taking the runner as
  an injected argument, step 9 stops being test-writing and becomes a rewrite of
  steps 3 through 7. Step 10 is the second risk: proving twelve-odd controls each
  go red under neutering is mechanical but slow, and it is the criterion most
  likely to be quietly downgraded to "the suite passes", which is not what the
  issue asked for.

OPEN  Not for a builder to decide.
  Where this runs was #110's call. RESOLVED after this plan was written: #110
  decided GitHub Actions for Phase 1 at 2026-08-12T07:43Z — 45 minutes after this
  file was committed — and its decision comment names #116's .github/workflows
  invocation among what it unblocks. No workflow file is added by this work even
  so, which is now a scoping choice rather than a wait: the token and its scopes
  belong to #119. That is stated here because this file merges and a PR body does
  not.
  What counts as a "required check" has no readable source of truth in this repo
  today: three endpoints report no protection on launchpad, every check on PR 86
  reports isRequired false, and launchpad/AGENTS.md §6 says the branch is
  protected. Whether the fork should have required checks configured is a cohort
  decision, not this script's, and the script must therefore report the absence
  rather than resolve it. Whether the token this eventually runs under should
  hold admin:org so org-level rulesets are readable at all is part of that same
  decision.
  The script path is stated by the issue to be decided in the PR; this plan
  assumes launchpad/scripts/pr-preflight.py and the reviewer may move it.
  Whether the emitted record carries a schema version, so the later review
  dimensions in #109 can depend on its shape, is undecided.
  Whether plans belong in this repository at all is unresolved: the skill's
  default path is docs/plans/, which is upstream's tree and barred by
  launchpad/AGENTS.md §3, so this file went to launchpad/plans/ instead — and
  §2 ("stable knowledge belongs in a document, active work becomes an issue")
  arguably says a plan should not be tracked at all. One `git rm` either way.

LEFT OUT  Deliberately excluded.
  Every judgement about the code — findings, severities, ranking. The issue puts
  it out of scope and #109 puts it in the later dimension stages.
  Posting anything to the PR. Publication belongs to the task that owns the
  single review comment.
  Fetching the linked issue's body. #109 lists it under Read, but #116's
  done-criteria ask only whether a closing keyword is present, so pulling the
  issue body would be building the next task's scope inside this one.
  Accessibility is out of scope for this issue and is not claimed: the
  deliverable is a script emitting JSON to stdout, with no UI, no interactive
  control and nothing to announce. If the record later gains a rendered surface,
  that surface needs its own keyboard and announcement specification.
  Prompt-injection defence, which #109 requires in Phase 1. Untrusted PR text is
  carried through this script as data and never interpreted, which is a property
  of it making no model call at all — the mitigation lives in the stage that
  does call a model.
