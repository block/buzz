Issue #119 — task: publish exactly one PR review comment, and re-review on push
Stated size: none given — the task template has no Size field  ->  cap: 12 steps

Sized by asking, not guessing. Answered: more than an hour, so the cap is 12.
Two further answers were asked rather than assumed, because both change what gets
built and neither is a builder's call:

  - **Re-review means update in place.** GitHub cannot delete or dismiss a
    submitted review of type COMMENT (see ALREADY TRUE), so #119's two
    done-criteria — "a new review for that commit" and "does not accumulate one
    stale review per push" — cannot both be literally true. Answered: POST once,
    then PUT the body on every later push, with the head SHA named inside the
    body. Exactly one review object for the life of the pull request.
  - **A fork pull request is skipped loudly.** On `pull_request` a fork gets a
    read-only token and cannot post at all. Answered: detect it, exit 0 having
    posted nothing, and say why in the job log.

Larger than an hour is flagged, not refused. These would each have been an
observable issue on their own — splitting is the reader's call, not this plan's:

  (a) the single-review lifecycle alone — locate, post, update, never duplicate —
      against a stub body, with no findings renderer written
  (b) the body renderer over #117's report envelopes, including the incomplete
      and clean cases, printing to stdout and posting nothing
  (c) the credential and its controls — the permissions block, the static
      assertion, and the live proof that contents write is absent
  (d) the workflow that triggers it

Planned as written below.

ALREADY TRUE  (verified against git, the working trees, the live GitHub API and
the sibling worktrees — not against notes)

  Nothing of #119 is built. Branch feat/review-agent-publish is at d897a06e8.
  `git status` reports a clean tree apart from session scratch under .claude/ and
  .repoql/. `git ls-files | grep -i publish` matches only unrelated upstream
  desktop React files. No launchpad/review-agent/ directory exists on this branch.

  ADR #110 is decided, and it names #119 by name as what it unblocks. The
  decision comment on #110 chooses GitHub Actions for Phase 1 with a committed
  revisit, and fixes the credential as "a GitHub token scoped to
  `launchpad-26/buzz` (pull-request write, contents read)" — no Buzz identity,
  deferred explicitly. #119's own stated blocker ("the GitHub token and its
  scopes — provisioning depends on #110") is therefore settled, and this plan is
  written against that credential.

  The review lifecycle API, checked live rather than recalled. On PR 86 of this
  fork, `gh api repos/launchpad-26/buzz/pulls/86/reviews` returns objects
  carrying `id`, `state` ("COMMENTED"), `commit_id`, `submitted_at` and `user`.
  `commit_id` is set at submission and is NOT updated when the review body is
  later edited — which is precisely why #119's criterion "the comment names the
  commit SHA it reviewed" has to be met by text inside the body, not by the
  review object's own field. The stronger claims — that PUT on a review id
  returns 200, that DELETE returns 422 for a non-pending review, and that
  dismissal is refused for a COMMENT review — are NOT verified here. They are
  what STEP 1 exists to record. This plan's chosen strategy depends on the first
  of the three, and STEP 1 is where it either holds or the strategy changes.

  PR 86 is same-repository: `isCrossRepository` is false and
  `headRepositoryOwner.login` is `launchpad-26`. That matches how the cohort
  works — branches are pushed to the fork, not forked from it — which is what
  makes the fork-skip answer above cheap in practice rather than a real gap.

  #120's work exists, is substantial, and is committed nowhere. The worktree
  /home/serina/Launchpad/buzz__worktrees/feat-review-agent-untrusted-input is at
  d897a06e8 with launchpad/review-agent/ UNTRACKED — 14 Python modules, three
  fixtures, and CONTAINMENT.md. `git log --all --oneline -- launchpad/review-agent/`
  returns nothing. Everything #119 imports currently exists only in another
  worktree's working directory. This is the single largest risk in this plan; see
  BUDGET.

  #119's renderer already has a sibling, and that sibling says #119 owns
  publication. review.py's module docstring opens "Render the review body. Does
  not post it — #119 owns publication." `review.SEVERITY_ORDER` is
  {"Blocker": 0, "High": 1, "Medium": 2, "Low": 3} at review.py:32, and
  `render_review(findings, states) -> str` at review.py:45
  already renders containment findings, an "Incomplete" line naming unreadable
  surfaces, and a COVERAGE_NOTE. #119 composes with that function; it does not
  replace it and does not re-declare the ladder.
  **The signature changed after this plan was committed, and the old one is a
  TypeError.** It was `render_review(findings, states, *, unreadable=None)` at
  review.py:42 when STEP 4 was written. #120 removed the `unreadable` keyword in
  `e072fba55`, because it had no producer anywhere on that branch — so the
  "Incomplete" banner it fed could never render, and a caller cannot forget an
  argument that does not exist. `unreadable` is now DERIVED inside the function
  from `states`, against `UNREADABLE_STATES = ("absent", "oversized",
  "unparseable")` — which now occupies review.py:42, the line this plan cites for
  the signature. STEP 4 is corrected accordingly.
  Those two line numbers were 19 and 29 when this plan was first drafted and
  moved to 32 and 42 within the hour, because #120 was actively writing in that
  worktree. `render_review` has since moved again, to 45, while SEVERITY_ORDER
  stayed at 32. Recorded here as of 2026-08-13 against `c64ff7958`. Cite them as
  orientation, never as evidence — this paragraph's warning has now fired twice,
  and the second time it was the signature rather than the line that moved.

  `render_review` does NOT accept #117's records, and that is a design
  constraint, not a detail. It takes `findings: list[Finding]` and reads
  `.severity`, `.kind`, `.entry_point` and `.evidence` by ATTRIBUTE — the
  `contain.Finding` dataclass, not a JSON object — plus a `states: dict[str,
  str]` mapping entry point to fetch state, which it uses for its "Fetched and
  empty" line. #117's envelope carries neither `kind` nor `evidence`, and no
  stage in the chain emits `states` at all: `contain.render` returns
  (document, findings, all_readable). So containment findings cannot reach #119
  through the `reports` array, and STEP 7's input contract has to carry them
  separately. This was found by review, not by the first draft, which assumed
  composition would just work.

  A workflow already exists for the controls, is untracked, and is read-only.
  .github/workflows/launchpad-review-agent-controls.yml in #120's worktree runs
  on `pull_request` with `permissions: {contents: read, issues: read,
  pull-requests: read}` and a comment explaining, correctly, why it is not
  `pull_request_target`. It cannot host the publish job: publishing needs
  pull-requests **write**, and widening that file would give the containment
  controls a write token they have no use for.

  A control runner exists and is the registration point. run_controls.py holds a
  CONTROLS list of (script, needs_network) pairs, probes `gh api rate_limit` for
  connectivity, and reports SKIP with a reason — never PASS — for a control whose
  input is missing. #119's controls register there rather than inventing a second
  runner.

  #117's output contract is written, and it is PROVISIONAL. It is at
  launchpad/plans/2026-08-12-issue-117-review-dimensions.md in the
  feat-review-agent-dimensions worktree. That file is UNCOMMITTED — `git ls-tree
  -r origin/feat/review-agent-dimensions -- launchpad/` does not carry it — and
  it has NOT been through serina:review-plan. #116's comparable plan took twelve
  findings across two passes, three of them Blockers. This plan is written
  against the contract at that revision, honouring its field names exactly:
  finding fields `dimension`, `severity`, `anchor` (line|file|pr), `file`, `line`
  (new-side, at head_sha), `defect`, `failure`, `finding_id`, `entry_point`; and
  envelope fields `schema_version`, `dimension`, `pr`, `merge_base_sha`,
  `head_sha`, `status` (complete|failed), `outcome` (findings|clean), `error`,
  `findings`, `findings_count`, `completion_marker`. Severity is imported from
  review.SEVERITY_ORDER, not redefined. Which steps break if a field moves is
  named in OPEN, not left implicit.

  #116's pre-flight record is enumerated but not built. origin/feat/review-agent-preflight
  carries only its plan file. Its record has seven top-level keys — pr,
  closing_issue, diff, checks, required_gate, nearest_rules, skips — and its own
  OPEN leaves the schema version undecided. So #119 cannot depend on that record
  today; the stage manifest in STEP 5 is what stands in for it.

  #118 is not started. Adjudication is the stage that would normally sit between
  #117 and #119. Its absence is why STEP 7 takes report envelopes on stdin and is
  agnostic about which stage produced them.

  launchpad/plans/ is the established path, and this plan uses it. AGENTS.md §3
  puts every cohort file under launchpad/ and #116's plan landed there. The
  skill's default docs/plans/ is upstream's tree and is not used; docs/plans/
  does not exist in this checkout. New workflow files go in .github/workflows/
  because GitHub requires it, and §3 requires the `launchpad-*.yml` prefix.

  No verify gate is installed in this checkout. .claude/settings.json and
  .claude/settings.local.json are both absent, so every review skill is a manual
  invocation and none fires on its own.

  Toolchain present: python3 3.12.3, gh 2.93.0.

  #122's corrections are honoured by not repeating the figure. This plan quotes
  no AUROC range. Per #122's verification on #109, the 0.48–0.64 range is one
  judge (JailJudge), one victim model (Llama-3.1-8B), two attacks (GCG and
  GCG-R), and "despite high performance on standard validation sets" is not a
  quotation from the paper. What is confirmed verbatim is that judges perform "on
  average only slightly better than a random coin-flip" against 6,642
  human-verified labels. Nothing in #119 depends on either figure.

STEP 1  Record what the review-lifecycle API actually does.                [independent]
        A throwaway pull request in this fork, and four raw responses captured to
        launchpad/review-agent/fixtures/review-lifecycle.json: POST a COMMENT
        review; PUT a new body on its id; DELETE that id; attempt a dismissal.
        Each entry stores the HTTP status, the response body, and the `gh`
        command that produced it.
        This is first because the whole strategy rests on PUT working and DELETE
        not. If PUT turns out to be refused, the answer recorded above ("update in
        place") is not implementable and the plan changes at STEP 2, not at STEP
        10 with nine steps built on it.
        Run under a human `gh auth` token, which is NOT the workflow credential —
        so this records what the endpoints do, and proves nothing about scopes.
        The scope question is STEP 9's and is not claimed here.
        done when: the fixture exists and contains four entries, each with a
        non-empty `status`, `body` and `command`; the POST entry's response has
        `state` "COMMENTED" and a numeric `id`; the PUT entry's status and the
        DELETE entry's status are both recorded verbatim whatever they are; and
        the file states which of the two strategies in this plan's header the
        recorded statuses support.

STEP 2  launchpad/review-agent/publish.py — the single-review lifecycle.       [needs 1]
        Three functions over an already-rendered body string. No rendering here,
        no findings, no contract.
          MARKER — a hidden HTML comment, `<!-- launchpad-review-agent:v1 -->`,
            emitted as the FIRST line of every body. Identification is by marker,
            not by author login: the workflow token posts as `github-actions[bot]`
            today and #110 commits to revisiting the identity later, so matching
            on the author would break at exactly the moment the credential moves.
          find_existing(pr, repo) -> int | None — lists reviews and returns the id
            of the OLDEST review whose body starts with MARKER, reporting the
            count when there is more than one rather than silently taking the
            first. More than one means a previous run raced or a strategy
            changed; that is a fact to surface, not to paper over.
            It PAGINATES — `gh api --paginate` — and asserts it reached the end
            of the listing. GET /pulls/{n}/reviews returns 30 per page, and a
            pull request accumulates reviews from every human as well as this
            agent. An unpaginated read on a busy pull request returns None for a
            marker sitting on page two, and None means POST, which is a second
            review on exactly the pull requests most likely to have a reader —
            the failure this issue exists to prevent, arriving only under load.
            A listing that cannot be fully read is an ERROR, not an absence: if
            pagination fails part way, find_existing raises rather than returning
            None, because a partial listing is indistinguishable from an empty
            one at the call site.
          post_or_update(pr, repo, body) -> (id, "created"|"updated") — PUT when
            find_existing returns an id, POST otherwise. The event is the literal
            "COMMENT", hardcoded at the single call site, and the function takes
            no event parameter at all. A parameter that could hold "APPROVE" is a
            parameter that one day will.
        Immediately before a POST, find_existing is called a second time. Two
        pushes seconds apart produce two workflow runs, and a check performed at
        the start of a run is stale by the time the run posts.
        done when: `python3 -m py_compile launchpad/review-agent/publish.py`
        succeeds; `grep -nE "APPROVE|REQUEST_CHANGES" publish.py` returns nothing;
        `grep -c "def post_or_update" publish.py` is 1 and its signature has no
        event parameter; against a recorded reviews listing with no marker
        find_existing returns None, with one marker returns that id, and with two
        markers returns the older id AND prints the duplicate count; against a
        recorded TWO-PAGE listing carrying 30 unmarked reviews on page one and the
        marker on page two it returns that id rather than None; and a recorded
        listing whose second page returns an error causes a raise, not a None.

STEP 3  End to end on a throwaway pull request, with a stub body. [needs 2]  <- RUNS HERE
        The lifecycle is demonstrable before a single finding is rendered. Post a
        stub body naming the head SHA; push an empty commit to the same branch;
        run publish.py again; read the reviews back.
        This is behavioural evidence only. It runs under a human token with far
        wider scope than the workflow credential, so it demonstrates that the
        review does not duplicate — it does NOT demonstrate anything about
        pull-requests write or contents read. That is STEP 9's, and the two must
        not be conflated in the PR body.
        done when: after the second run `gh api repos/launchpad-26/buzz/pulls/<n>/reviews
        --jq '[.[] | select(.body | startswith("<!-- launchpad-review-agent:v1 -->"))] | length'`
        equals 1; the surviving review's body names the SECOND commit's SHA and
        not the first; its `id` is unchanged between the two runs; its `state` is
        "COMMENTED"; and the raw before/after listings are saved for the PR body.

STEP 4  launchpad/review-agent/publish_render.py — the findings body.      [independent]
        A pure function
        `render_body(reports, stages, containment, head_sha, merge_base_sha) -> str`.
        No network, no subprocess, no posting.
        `containment` is a SEPARATE argument, not an entry in `reports`, because
        `review.render_review` cannot read #117's records — see ALREADY TRUE. Its
        shape is `{findings: [{severity, kind, entry_point, evidence}], states:
        {entry_point: state}}`, which is `contain.Finding` and
        `fetch.Surface.state` in JSON. render_body reconstructs `contain.Finding`
        objects from that block and passes them, with `states`, straight into
        `review.render_review`, so the post-escape rendering rule in
        CONTAINMENT.md is honoured by the code that already holds it rather than
        reimplemented here.
        The block carries NO `unreadable` key, and render_body passes no
        `unreadable=` argument — that keyword was removed in #120's `e072fba55`
        and passing it now raises TypeError. See ALREADY TRUE. Re-adding it here
        would be a second source of truth for a fact `states` already carries:
        the two could disagree, and `render_review` would silently ignore the one
        this plan sent. Derive, never pass.
        The consequence is that **`states` is now load-bearing for the
        "Incomplete" banner**, not merely for the "Fetched and empty" line. If the
        `containment` block omits `states`, or populates it for only the surfaces
        that succeeded, every unreadable surface reads as absent-from-the-map
        rather than unreadable, and the banner never renders. That is #120's
        original defect — a banner with no producer — relocated one stage up. STEP
        5's trigger for a MISSING block does not cover a PRESENT block with a thin
        `states` map, so the control must assert the map names all seven entry
        points, not merely that it exists.
        This block does not exist upstream yet. #117's runner has the objects —
        `contain.render` returns them — but emits them nowhere. Adding it is a
        one-key change to #117's output and is recorded in OPEN as a dependency,
        not assumed.
        A MISSING `containment` block is INCOMPLETE, never "no containment
        findings". CONTAINMENT.md's own reasoning is that a detected attempt which
        does not reach the review is worse than one never detected, because it
        reads as a clean review; an absent block is exactly that case and must
        never render as the "No containment findings" line, which is a positive
        claim. STEP 5 carries this as a trigger.
        Ordering: findings from every report are merged into one list and sorted
        by `review.SEVERITY_ORDER[finding["severity"]]`, ties broken by
        (dimension, file or "", line or 0, finding_id) so the order is total and
        the body is byte-identical for identical input. SEVERITY_ORDER is
        IMPORTED from review.py; a second copy of a four-value ladder drifts.
        "Most severe first survives an update" is a property of construction, not
        of maintenance: the body is rebuilt from scratch on every run and nothing
        is ever appended to an existing one, so ordering cannot degrade across
        pushes the way an append-only comment would.
        Anchoring follows #117's three rules exactly and renders each accordingly
        — `anchor: line` as `path:line`, `anchor: file` as `path`, `anchor: pr` as
        `(pull request)`. A finding whose anchor and fields disagree is rendered
        under an explicit "malformed finding" heading with its raw record, not
        dropped: a finding silently discarded by the publisher is a finding the
        reviewer believes it reported.
        Every finding renders `defect` and `failure` as separate lines, because
        #119's criterion is "the concrete failure it allows" and a defect with no
        stated consequence is what lets an unfalsifiable finding through.
        done when: given three report envelopes carrying findings of mixed
        severity, the output lists every Blocker before every High, every High
        before every Medium, and every Medium before every Low; two calls with
        the same input produce byte-identical strings; a finding with anchor "pr"
        and a non-null `file` appears under "malformed finding" and is still
        present in the output; the body's first line is publish.py's MARKER;
        `grep -n "SEVERITY_ORDER *=" publish_render.py` returns nothing; a
        containment block carrying one Blocker finding produces a body containing
        that finding's `kind`, its `entry_point`, and its evidence in ESCAPED form
        — asserted by comparing against `contain.escape(evidence)`, not against
        the raw string; and a run with `containment=None` produces the STEP 5
        banner and does NOT contain the string "No containment findings".

STEP 5  The incomplete case — an unfinished stage is never rendered as done. [needs 4]
        A `stages` manifest of {name, status, reason} entries accompanies the
        reports, covering stages that emit no envelope of their own — #116's
        pre-flight and #118's adjudication. A review is INCOMPLETE when any of:
        a stage in the manifest has status other than "complete"; a report has
        `status: failed`; a report's `completion_marker` is absent, not the last
        key, or carries the wrong dimension or nonce; `findings_count` does not
        equal `len(findings)`; a dimension expected by the manifest produced no
        report at all; or STEP 4's `containment` block is absent or unparseable.
        Incomplete renders as a banner at the TOP of the body, above the findings,
        naming every stage that did not finish and its reason. It is at the top
        because a reader who stops after the first finding must still have seen
        it, and #119's criterion is that the comment "never publishes a partial
        review that reads as a complete one".
        The default is incomplete. An input that cannot be classified — a report
        that will not parse, a manifest entry with no status — is incomplete, not
        complete. Absence of a failure signal is not evidence of success.
        done when: for each of the seven conditions above, given an input
        exhibiting only that condition, the body contains the incomplete banner
        and names the offending stage or dimension; for an input exhibiting none
        of them the banner is absent; a report whose `completion_marker` carries
        another dimension's nonce is incomplete; and an unparseable report is
        incomplete rather than raising.

STEP 6  The clean case — no findings still posts, and says so.               [needs 4]
        Every stage complete and every report `outcome: clean` renders an explicit
        body: the SHA reviewed, the dimensions that ran, and a sentence saying no
        confirmed findings were produced. #119's reasoning is that silence is
        indistinguishable from a crashed agent, so this path posts on exactly the
        same code path as the findings path — there is no early return that skips
        publication.
        `outcome: clean` and an empty findings array are not the same input.
        A report with `status: complete` and no `outcome` is incomplete per STEP 5,
        not clean.
        done when: an all-clean input produces a body containing the head SHA, the
        name of every dimension in the manifest, and the no-findings sentence; that
        body still carries the MARKER as its first line; `grep -n "return None"
        publish_render.py` shows no early return on the clean path; and an input
        with `status: complete` and no `outcome` produces the STEP 5 banner rather
        than the clean sentence.

STEP 7  Wire the renderer into the CLI.                                  [needs 2, 4]
        `publish.py` gains a `main` reading one JSON document on stdin —
        `{pr, head_sha, merge_base_sha, stages, reports, containment}`, where each
        entry of `reports` is a #117 envelope verbatim and `containment` is the
        block specified in STEP 4 — rendering it through publish_render, and
        calling post_or_update. A `--dry-run` prints the body and posts nothing.
        The document WRAPS #117's envelopes; it does not restate or rename a
        single field inside them, and `containment` is a sibling key precisely so
        that it does not have to.
        done when: `python3 publish.py --dry-run < fixture.json` exits 0 and
        prints a body whose first line is the MARKER; the same fixture with
        `reports: []` and a manifest naming two dimensions exits 0 and prints the
        incomplete banner; the same fixture with `containment` removed exits 0 and
        prints the incomplete banner; malformed JSON on stdin exits non-zero and
        posts nothing; and `--dry-run` produces no entry in the target PR's review
        list.

STEP 8  .github/workflows/launchpad-review-agent-publish.yml.                [needs 7]
        A separate file from #120's controls workflow, for the reason in ALREADY
        TRUE: this job needs pull-requests write and that one must not have it.
        Named `launchpad-*` per AGENTS.md §3.
          on: pull_request, types [opened, synchronize, reopened] — `synchronize`
            is what makes "re-review on push" happen at all.
          permissions: contents: read, pull-requests: write. Nothing else. Set at
            the workflow level with no job-level override.
          concurrency: group per pull request — the group string INTERPOLATES
            `github.event.pull_request.number` (or `github.ref`), never a fixed
            name — with cancel-in-progress: true. Two pushes in quick succession
            otherwise race, and STEP 2's second find_existing is a backstop for
            that race, not a substitute. A workflow-wide fixed group would be
            worse than none: every pull request would then cancel every other
            one's publish run, and the pull request that lost the race would
            silently keep a review describing a commit it no longer has.
          A first step that exits 0 without posting when
            `github.event.pull_request.head.repo.full_name` differs from
            `github.repository`, printing the reason.
          NOT `pull_request_target`. The suite this job runs lives in this
            repository, so a pull request can modify the code the job executes;
            `pull_request_target` would hand that modified code the base
            repository's token. #120 established this and the comment block in
            that workflow says so — this file carries the same note rather than
            leaving a reader to infer it.
        done when: `python3 -c "import yaml,sys;
        d=yaml.safe_load(open('.github/workflows/launchpad-review-agent-publish.yml'));
        print(d['permissions'])"` prints exactly {'contents': 'read',
        'pull-requests': 'write'}; `grep -c pull_request_target` on the file is 0;
        the `on.pull_request.types` list contains `synchronize`; a `concurrency`
        key is present with `cancel-in-progress: true` AND its `group` value
        contains `${{` and either `pull_request.number` or `github.ref`, so a
        fixed group name fails this check rather than passing it; and no job in
        the file declares its own `permissions`.

STEP 9  launchpad/review-agent/check_publish_scope.py — the credential control. [needs 8]
        Two assertions, because either alone is weak.
          STATIC — parse the workflow YAML and assert the permissions mapping
            equals exactly {contents: read, pull-requests: write}, that no job
            overrides it, and that the file does not mention
            pull_request_target. This runs anywhere, needs no token, and catches
            a later widening in review.
          LIVE — with the workflow's own token, attempt one contents write:
            create the ref `refs/heads/scope-probe-${{ github.run_id }}`. Assert
            HTTP 403. Any other outcome is FAIL, including success, 404, and a
            rate-limit error — a probe that treats "some error happened" as proof
            of absent permission is fail-open, and would report PASS on a network
            blip. If the probe unexpectedly SUCCEEDS the control deletes the ref
            it made and still fails.
        Outside Actions there is no workflow token, so the live half reports SKIP
        with a reason and never PASS — the rule run_controls.py already enforces.
        This is the only step that can demonstrate #119's credential criterion,
        and it can only do so inside a real workflow run. STEP 3's local evidence
        does not substitute for it.
        done when: the static half fails when handed a copy of the workflow with
        `contents: write` and passes on the real one; the live half reports SKIP
        with a stated reason when GITHUB_TOKEN is absent; a recorded 404 response
        fed to the live half yields FAIL rather than PASS; and a real Actions run
        on this pull request shows the live half reporting PASS with the 403
        response body pasted into the PR.

STEP 10 launchpad/review-agent/check_publish_single.py — the behaviour controls. [needs 7]
        Recorded inputs, no network, no model. Five assertions matching #119's
        done-criteria one for one, and EVERY one of them carries a stated mutation
        that must break it. A control never observed failing has not been shown to
        test anything, and the temptation is to prove that only for the assertions
        where it is easy — which leaves the load-bearing ones unproven.
          (i)   the event published is COMMENT and the module contains no other
                event string — the control asserts on the source, since a runtime
                assertion cannot prove an absent branch.
                Mutation: add the literal "APPROVE" to publish.py.
          (ii)  a second run over the same PR with a marker present issues a PUT
                and no POST, using the recorded listing from STEP 1 and an
                injected transport that records calls instead of making them.
                Mutation: make find_existing return None unconditionally. This is
                the assertion the single-review invariant rests on, so it gets the
                mutation proof first, not last.
          (iii) find_existing paginates — the recorded two-page listing from STEP
                2 yields the marked id.
                Mutation: drop `--paginate` from the listing call.
          (iv)  severity order holds in the rendered body after an update whose
                input has a NEW Blocker appended LAST in the reports array — the
                Blocker still renders first.
                Mutation: replace publish_render's sort key with identity.
          (v)   a clean input and an incomplete input both produce a body, and the
                two bodies differ.
                Mutation: remove the incomplete banner.
        done when: all five assertions run offline and pass; each of the five
        stated mutations, applied one at a time, makes exactly its own assertion
        fail and is then reverted; the recorded output of all five mutation runs is
        saved for the PR body; and each assertion prints what it compared rather
        than only PASS.

STEP 11 Register both controls in run_controls.py.                       [needs 9, 10]
        Two entries appended to CONTROLS: ("check_publish_scope.py", True) and
        ("check_publish_single.py", False). The scope control needs network for
        its live half and is expected to SKIP that half locally.
        done when: `python3 run_controls.py` runs both, its summary line counts
        them, and with `gh` unauthenticated the scope control appears in the
        skipped list with a reason rather than in the passed list.

STEP 12 launchpad/review-agent/PUBLISHING.md, and the cross-references.     [needs 11]
        Normative, a sibling to CONTAINMENT.md and in the same voice. States: the
        marker and why identification is by marker and not by author; that exactly
        one review object exists per pull request and is updated in place; that
        the head SHA lives in the body because `commit_id` is frozen at
        submission; the incomplete rule and its six triggers; that the clean case
        posts; the credential and its two controls; and the fork-skip behaviour.
        Cross-referenced from CONTAINMENT.md's "Contract for later stages" table
        and from #117's FINDINGS.md, so the three documents point at each other
        rather than diverging quietly.
        done when: PUBLISHING.md exists under launchpad/review-agent/; it names
        the marker string, the seven incomplete triggers, the `containment` block
        it requires from #117, and both controls by filename; CONTAINMENT.md's contract table has a row pointing at it; and it
        records that #117's contract was honoured at the revision named in this
        plan's ALREADY TRUE rather than implying a settled one.

PARALLEL
  STEP 1 and STEP 4 may run as concurrent subagents. They share no file — STEP 1
  writes only fixtures/review-lifecycle.json, STEP 4 writes only
  publish_render.py — and STEP 4's input is #117's contract, not STEP 1's output.
  STEP 9 and STEP 10 may run concurrently once their dependencies are met. They
  write check_publish_scope.py and check_publish_single.py respectively and touch
  nothing else; STEP 11 is what merges them into run_controls.py.
  Everything else is sequential, and mostly for one boring reason: STEPs 2, 3 and
  7 all edit publish.py, and STEPs 4, 5 and 6 all edit publish_render.py. Two
  steps editing one file are sequential however unrelated they look.
  STEP 3 cannot be parallelised with anything that posts, because two agents
  publishing to the same throwaway pull request would each see the other's
  review and the single-review assertion would be measuring the wrong thing.
  Dispatching is not this plan's decision. Nothing here is dispatched.

GATES
  No verify gate is installed in this checkout, so every one of these is a manual
  invocation and none fires on its own.
  serina:review-plan — on THIS file, before the first implementer is dispatched.
    #116's comparable plan took twelve findings across two passes; assume this one
    has defects that are cheaper to fix now than at STEP 10.
  serina:review-code — after STEP 7, and again after STEP 12. The first pass
    catches the lifecycle and renderer while they are still small; the second sees
    the workflow and the controls.
  serina:review-tests — after STEP 10, on the two control scripts. These controls
    are the only thing standing between "the credential is narrow" and "we said
    the credential is narrow", so a control that cannot fail is the worst defect
    available in this issue.
  serina:review-adjudicate — after the reviewers, before any verdict is read.
  serina:review-final — once, on the whole branch, before merge.
  serina:review-a11y — not applicable and not claimed. See LEFT OUT.
  The plan gate script: `~/.claude/skills/plan-issue/check-plan.sh` on this file.
  It checks form, not substance, and a clean run is not a review.

BUDGET
  STEP 9's live half is the step most likely to overrun. It is the only assertion
  that cannot be made locally: the workflow token exists only inside a real
  Actions run, so every iteration costs a commit, a push and a full run cycle,
  and the failure modes are the slow kind — a permissions block that parses but
  does not apply, a 404 where a 403 was expected because the ref path was wrong,
  a control that reports PASS on the wrong error. Budget several cycles and write
  the static half first so at least one assertion is provable without one.
  Second, and structural rather than per-step: everything #119 imports —
  `review.SEVERITY_ORDER`, `review.render_review`, `run_controls.CONTROLS`,
  CONTAINMENT.md's rendering rule — existed only as untracked files in #120's
  worktree, on no branch and in no commit. **That is no longer true, as of
  2026-08-13.** #120 is three commits on `feat/review-agent-untrusted-input`,
  all pushed, at `c64ff7958`, with its control suite green (11 controls, 0
  failed, 0 skipped). The dependency is now a real ref rather than a working
  directory, so STEPs 4, 5, 6, 11 and 12 can cite commits instead of a path on
  one machine. The risk it replaces is smaller but not gone: the branch is
  unmerged, so a rebase before it lands still moves every line number here.
  Before STEP 4, re-verify render_review's signature and SEVERITY_ORDER's
  location against whatever #120 has actually committed by then, rather than
  trusting the line numbers quoted in ALREADY TRUE. **That instruction has now
  paid for itself**: run on 2026-08-13 it caught the removal of the `unreadable`
  keyword, which would otherwise have been a TypeError on the first call STEP 4
  made — found by reading, not by running, because none of this is built yet.
  The cheapest mitigation is still ordering: let #120 land first. That is a fleet
  sequencing decision, not this plan's.
  This risk is not hypothetical — it fired during planning. Between the first
  draft and its review, `review.py` moved SEVERITY_ORDER from line 19 to 32 and
  render_review from 29 to 42, and two new control scripts appeared in that
  worktree. Nothing broke, because the plan cites those symbols by name as well
  as by line, but a step that had said "review.py:19" and nothing else would
  already have been wrong within the hour.
  Third: STEP 1 depends on a claim this plan has not verified — that PUT on a
  submitted review returns 200. If it does not, STEP 1 is cheap and the plan
  changes there. If instead PUT silently succeeds but GitHub renders the review
  against its original commit in a way reviewers find misleading, that surfaces
  at STEP 3 and is a judgement call, not a bug.

OPEN  Not for a builder to decide.
  The two done-criteria in #119 are in tension and this plan resolved it by
  asking. "Pushing a new commit produces a new review for that commit" is read as
  new review CONTENT for that commit, in the same review object, because GitHub
  offers no way to remove a submitted COMMENT review. A reader who meant a new
  review object per push is asking for accumulation, which the next criterion
  forbids. If STEP 1's recorded responses contradict the assumption that PUT
  works, this is reopened rather than worked around.
  An updated review keeps its original `submitted_at` and `commit_id`. The PR
  timeline will therefore show the review at the time of the FIRST push, with a
  body describing the LATEST commit. The body names the SHA, so nothing is
  ambiguous to a reader who reads it — but whether that is acceptable to human
  reviewers is a call for whoever reviews the first ten (#121).
  Whether the publish workflow is its own file or folds into #116's invocation.
  #110's decision comment names #116 as ".github/workflows/ invocation". #119's
  own "impacted components" names only `launchpad/` and the token, not a
  workflow. STEP 8 adds one anyway, because a criterion about re-review on push
  is untestable without a trigger. If #116 lands an invocation workflow first,
  STEP 8's job should move into it and STEP 9's static assertion should follow
  it. That is a sequencing decision.
  #117's contract is provisional, and these are the steps that change with it.
  A rename or removal of `severity`, `anchor`, `file`, `line`, `defect`,
  `failure` or `dimension` changes STEP 4. A change to `status`, `outcome`,
  `error`, `completion_marker` or `findings_count` changes STEP 5, and `outcome`
  alone also changes STEP 6. Either changes STEP 10's recorded inputs and STEP
  12's prose. STEPs 1, 2, 3, 8, 9 and 11 are unaffected by any field rename,
  because they operate on a body string and a credential and never look inside a
  finding. `finding_id` is not used by #119 at all except as a tie-break in
  STEP 4's sort, so #117's warning that it is unstable across a reworded `defect`
  costs this issue nothing — the body is rebuilt wholesale on every run rather
  than diffed against the previous one.
  A concern with the contract, raised rather than worked around. #117 states that
  #118 re-rates severity and that "the reporting dimension's value must remain
  readable after adjudication rather than being overwritten in place" — but the
  record carries exactly ONE `severity` field, so there is nowhere for the
  re-rated value to live. #119 sorts by `severity` and cannot tell which of the
  two it is holding. Either the contract needs a second field or the sentence
  needs to go; this plan does not choose, and does not silently diverge, because
  #118 will honour the same contract.
  A second concern, and this one is a hard dependency rather than a worry.
  `contain.Finding` carries `kind` and `evidence`; #117's record has a field for
  neither, and `review.render_review` additionally needs a `states` map that no
  stage emits at all. So containment findings CANNOT travel inside `reports`,
  and STEP 4 takes them as a separate `containment` block instead. That block
  does not exist yet: #117's runner holds the objects — `contain.render` returns
  them — and emits them nowhere. **#117 must add one key to its output before
  #119 can publish a containment finding**, and until it does, every real
  injection attempt renders as the STEP 5 incomplete banner rather than as the
  attempt it was. That is the safe failure, not the intended one. Whether the fix
  is a key on #117's output or a `kind`/`evidence` pair absorbed into the finding
  record is a design call for #117 and #118 together; this plan does not choose,
  and does not silently diverge.
  The first draft of this plan assumed `review.render_review` would simply
  compose with #117's records. It does not — the function reads `.severity`,
  `.kind`, `.entry_point` and `.evidence` by attribute off `contain.Finding`.
  That was found by serina:review-plan, rated Blocker, and is recorded here so
  the next reader knows the composition is deliberate rather than inherited.
  Whether the throwaway pull request from STEPs 1 and 3 stays open, and whether
  its recorded responses are committed as fixtures. They contain review ids and
  bodies from this public fork — no credential — but they are permanent once
  committed.
  What happens when the workflow token is present but the review is on a pull
  request the agent has already reviewed at the SAME head SHA — a re-run with no
  new commit. This plan re-renders and PUTs unconditionally, which is idempotent
  in content but produces an edit event each time. Whether to skip when the SHA is
  unchanged is a preference, not a correctness question.

LEFT OUT  Deliberately excluded.
  Approving, requesting changes, merging, and any label that gates a merge.
  #119 puts all four out of scope and AGENTS.md §5 rule 1 forbids the first three
  outright. STEP 2 enforces it by construction rather than by discipline: there
  is no event parameter to pass the wrong value to.
  Inline file:line review comments — the `comments` array that POST /reviews
  accepts. Rejected for a concrete reason, not omitted: PUT updates only a
  review's BODY, so inline comments cannot be re-anchored when the head moves,
  and stale ones would accumulate on lines that no longer exist. #119's criterion
  is satisfied by rendering `file:line` as text in the body, which survives an
  update and survives a force-push.
  `pull_request_target`, per #120 and repeated in STEP 8's own comment block.
  Fork pull requests get a loud skip. No cross-repository publication path is
  built, and none is designed. If an outside contributor ever opens one, the job
  log says why there is no review; the first fork PR is when someone decides
  whether that is good enough.
  Any read of #116's pre-flight record. It is enumerated in a plan and built
  nowhere, and its schema version is its own OPEN question. STEP 5's stage
  manifest is what stands in for it, and swapping the manifest for the real
  record later touches STEP 5 alone.
  Deciding whether a finding is real. #118 owns confirm/refute, re-rated severity
  and dedupe. #119 publishes what it is handed, in severity order, and adds no
  judgement of its own.
  Running the dimensions, and choosing a model. #117 owns the first and puts the
  second out of scope. publish.py takes JSON on stdin and never names a model.
  Measuring whether the reviews are any good. #121 owns the first ten reviews and
  #109's success signals. Nothing here produces a precision or recall figure, and
  nothing here should be read as one.
  Accessibility is out of scope for this issue and is not claimed. The deliverable
  is a CLI and a workflow; the only surface a human reads is markdown rendered by
  GitHub's own interface, which carries its own keyboard behaviour and
  announcements. There is no control to reach, no focus to manage and nothing to
  announce. If a rendered dashboard over these reviews ever follows, it needs its
  own keyboard and announcement specification and does not inherit one from here.
  The AUROC range from #109. Per #122 it is one judge, one victim model and two
  attacks, and the "standard validation sets" phrase is not a quotation. Nothing
  in #119 needs either figure, so neither is repeated.
