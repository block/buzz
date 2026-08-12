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

  Nothing of #119 is built. `git ls-files | grep -i publish` matches only
  unrelated upstream desktop React files, and no launchpad/review-agent/ directory
  exists on this branch.
  THIS BRANCH MOVES UNDER THIS FILE, so the anchor is dated rather than asserted.
  It read "at d897a06e8" until an independent review pass checked it: HEAD was
  eb2bf09d0 when this plan was committed, and 8a405a9f5 ("match #119's plan to
  render_review's current signature") landed while it was being revised — a commit
  from another session working in this same worktree. As of 2026-08-13 the branch
  is at 8a405a9f5 with this plan file itself carrying further staged edits. The
  substantive claim survives every one of those moves; the SHA did not. This is the
  pinned-fact rot this plan warns about for review.py's line numbers, applied to
  its own anchor commit — which is why it is now a dated observation and why a
  reader should re-run `git log -1` rather than trust the number above.

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

  `render_review` does NOT accept #117's ten-field records, and that is a design
  constraint, not a detail. It takes `findings: list[Finding]` and reads
  `.severity`, `.kind`, `.entry_point` and `.evidence` by ATTRIBUTE — the
  `contain.Finding` dataclass, not a JSON object — plus a `states: dict[str,
  str]` mapping entry point to fetch state, which it uses for its "Fetched and
  empty" line AND, since `e072fba55`, for the "Incomplete" banner. So containment
  findings cannot reach #119 through the `reports` array, and STEP 7's input
  contract carries them separately. This was found by review, not by the first
  draft, which assumed composition would just work.
  Two claims an earlier revision of this paragraph got WRONG, corrected here
  rather than quietly dropped:
    — "#117's envelope carries neither `kind` nor `evidence`." The `kind` half is
      true: `kind` exists only on a containment finding, never on the ten-field
      record. The `evidence` half is false. `evidence` is the TENTH field of that
      record, RAW, and REQUIRED whenever `entry_point` is set. STEP 4 renders it,
      and finding it absent from this plan's own field list is what STEP 4's
      escaping rule exists to answer.
    — "no stage in the chain emits `states` at all." #117 now emits
      `containment.states` for ALL SEVEN entry points on every run. That is the
      producer whose absence removed the `unreadable` keyword in the first place.
  Both errors came from reading revision 3 of #117's contract; the contract is
  now settled and both are fixed against it. Recorded because a verified section
  that carries a false claim is worse than one that carries none — a later reader
  trusts it by construction.

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

  #117's output contract is now SETTLED, and this plan is rewritten against it.
  It is at launchpad/plans/2026-08-12-issue-117-review-dimensions.md in the
  feat-review-agent-dimensions worktree, it has been through serina:review-plan
  three times, and it carries a "CONTRACT CHANGES SINCE REVISION 3" block written
  expressly so this plan's author could diff. It is still UNCOMMITTED, so it is a
  settled decision rather than a settled ref.
  TEN finding fields, per report envelope entry: `dimension`, `severity`,
  `anchor` (line|file|pr), `file`, `line` (new-side, at head_sha), `defect`,
  `failure`, `finding_id`, `entry_point`, `evidence`. ELEVEN envelope fields:
  `schema_version`, `dimension`, `pr`, `merge_base_sha`, `head_sha`, `status`
  (complete|failed), `outcome` (findings|clean), `error`, `findings`,
  `findings_count`, `completion_marker`. And FIVE merged-document keys, which is
  what #119 actually reads: `pr`, `merge_base_sha`, `head_sha`, `reports`,
  `containment`.
  WHERE CONTAINMENT FINDINGS LIVE is the settled part that decides STEP 4, and it
  settled the way this plan already assumed. Quoting the contract: containment
  findings "do NOT enter the `findings` array of any dimension report, and they
  are NOT converted into the ten-field record. They travel as a TOP-LEVEL SIBLING
  KEY of the merged document, named `containment`, carrying contain.Finding
  verbatim in JSON" — `findings[]` of {severity, kind, entry_point, evidence} and
  a `states` map of all seven entry points. The reserved "containment" dimension
  slug an earlier revision proposed is WITHDRAWN, and no dimension may claim that
  slug. So STEP 4's separate block is correct and stays.
  Five changes from revision 3, and only two cost this plan anything:
    1. the `containment` sibling key exists and has the shape STEP 4 specified —
       this is the key OPEN said #117 must add, and it is added.
    2. `evidence` is RAW, not post-escape. This one costs STEP 4 a rule: raw
       attacker text now arrives in the document, on BOTH the containment block
       and any ten-field finding carrying an `entry_point`.
    3. `entry_point` is REQUIRED on an injection finding, with `evidence`
       required alongside it — not optional as revision 3 had them. STEP 5's
       cross-cutting clause in #117 is what produces those, so #119 will receive
       them on ordinary dimension findings, not only in the containment block.
    4. `finding_id` hashes seven inputs rather than five. #119 uses it only as a
       sort tie-break, so this costs nothing.
    5. the finding-field count is ten, not nine. Corrected above.
  NOTHING IS RENAMED. This plan's OPEN correctly predicted that a rename would
  cost STEPs 4, 5, 6, 10 and 12; no rename happened. What it did NOT predict is
  an ADDITION, which is what actually broke it — see OPEN, which now registers
  additions as well as renames and removals.
  Severity is imported from review.SEVERITY_ORDER, not redefined, and STEP 4 no
  longer subscripts it bare — see its ordering rule.

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
        A throwaway pull request in this fork, and FIVE raw responses captured to
        launchpad/review-agent/fixtures/review-lifecycle.json: POST a COMMENT
        review; PUT a new body on its id; DELETE that id; attempt a dismissal; and
        attempt a PUT on a review THIS TOKEN DID NOT WRITE. Each entry stores the
        HTTP status, the response body, and the `gh` command that produced it.
        THE FIFTH IS THE ONE THREE REVIEW PASSES COULD NOT SETTLE, and it is nearly
        free here. STEP 2's author filter exists because a marked review written by
        someone else must never become a PUT target, and the consequence of getting
        that wrong — the agent publishes nothing on that pull request, permanently —
        depends on GitHub refusing a cross-author PUT. No pass verified that, because
        verifying it means writing to a public fork, which is exactly what this step
        already does. This step runs under a HUMAN token, a different identity from
        the workflow's, so a review already on the pull request under any other login
        is a ready-made subject: attempt the PUT, record the status verbatim.
        If GitHub instead ALLOWS it, that is a bigger finding than the one the filter
        was written for — this agent would be able to overwrite other people's review
        bodies — and it reopens STEP 2 rather than being worked around. Either
        outcome is worth a single call.
        This is first because the whole strategy rests on PUT working and DELETE
        not. If PUT turns out to be refused, the answer recorded above ("update in
        place") is not implementable and the plan changes at STEP 2, not at STEP
        10 with nine steps built on it.
        Run under a human `gh auth` token, which is NOT the workflow credential —
        so this records what the endpoints do, and proves nothing about scopes.
        The scope question is STEP 9's and is not claimed here.
        TWO MORE THINGS ARE RECORDED HERE, because this is the only step that
        touches the live API and both are otherwise never captured at all:
          fixtures/reviews-listing.json — the raw `GET /pulls/{n}/reviews`
            response, saved as the UNMERGED per-page bodies with their `Link`
            headers, not as one merged array. STEP 2 and STEP 10 both assert
            against a recorded listing and neither can produce one.
            THE TWO-PAGE CASE IS SYNTHETIC BY NECESSITY, and the fixture says so in
            its own header. Measured on this fork: PRs 86, 124 and 126 carry 1, 0
            and 1 reviews. Nothing here has thirty, and growing a second page on a
            throwaway pull request would take thirty separate submissions. So the
            fixture holds TWO artefacts, labelled differently and never conflated:
            the genuinely RECORDED single-page response, which is the authority on
            what a review object's fields actually are; and a CONSTRUCTED two-page
            listing, built by replicating that recorded object with distinct `id`s
            and a real `Link: rel="next"` header copied from a genuine paginated
            response. An earlier revision said page one was "padded", which is the
            same construction described in a word that invites hand-authoring — and
            hand-authoring a listing is the defect the fixture exists to remove.
            Calling it constructed is honest; calling it recorded is not.
          whether an EDIT is observable to a human. After the PUT, capture the
            pull request's timeline (`GET /issues/{n}/timeline`) and record
            whether the edit appears in it at all. The whole strategy turns
            "re-review on push" into a body edit, so if an edit produces no
            timeline entry and no notification, every re-review after the first
            is silent — the body is current and no reader is told. That is a
            judgement call for #121, and it cannot be made without this
            observation. Recording it costs one call; inferring it costs the
            feature's entire value.
        done when: the fixture exists and contains FIVE entries, each with a
        non-empty `status`, `body` and `command`; the POST entry's response has
        `state` "COMMENTED" and a numeric `id`; the PUT entry's status and the
        DELETE entry's status are both recorded verbatim whatever they are; the
        cross-author PUT entry records its status verbatim and the fixture states in
        one line whether GitHub refused it, with STEP 2's author filter cited as what
        depends on the answer;
        fixtures/reviews-listing.json exists, holds the recorded single-page
        response AND a constructed two-page listing whose page one holds 30 entries,
        each artefact labelled recorded or constructed in the file itself, and no
        assertion anywhere cites the constructed one as evidence about GitHub;
        the timeline response after the PUT is saved
        and the fixture states in one line whether the edit is visible in it; and
        the file states which of the two strategies in this plan's header the
        recorded statuses support.

STEP 2  launchpad/review-agent/publish.py — the single-review lifecycle.       [needs 1]
        Three functions over an already-rendered body string. No rendering here,
        no findings, no contract.
          MARKER — a hidden HTML comment, `<!-- launchpad-review-agent:v1 -->`,
            emitted as the FIRST line of every body. Identification is by marker
            AND by author, and neither alone is sufficient — the marker says "this
            is our kind of review", the author says "this one is ours".
            Not by a HARDCODED author login: the workflow token posts as
            `github-actions[bot]` today and #110 commits to revisiting the identity
            later, so a literal login in the source would break at exactly the
            moment the credential moves. The login is resolved at runtime instead,
            per find_existing below, which keeps that portability while closing the
            hole the marker alone leaves open. An earlier revision of this bullet
            read "identification is by marker, NOT by author login" and drew the
            wrong conclusion from the right premise: what must not be hardcoded is
            the name, not the comparison.
            publish.py OWNS the constant and publish_render RECEIVES it as
            render_body's first argument. ONE DIRECTION IS BANNED AND THE OTHER IS
            NORMAL: publish_render must import nothing from publish, and publish.py
            imports publish_render at module level in the ordinary way.
            A cycle needs both edges. STEP 7 puts `main` in publish.py, so if
            publish_render also imported publish for the marker, `python3
            publish.py` would die with "cannot import name ... from partially
            initialized module" — which is why the marker is passed in rather than
            imported. With that edge gone there is no cycle left to prevent, and an
            earlier revision nevertheless banned BOTH directions, which forbade the
            normal import and left `main`'s import site unstated. Assert the edge
            that matters, not both. Passing the marker in also makes STEP 4 testable
            on its own with a sentinel value.
          find_existing(pr, repo) -> int | None — lists reviews and returns the id
            of the NEWEST review that carries MARKER **and was written by the
            agent's own identity**. Both conditions, and the author one is checked
            FIRST. Reviews carrying the marker under any other author are counted
            and reported as foreign, never treated as candidates.
            THE MARKER IS ATTACKER-WRITABLE AND THE AUTHOR FIELD IS NOT. A review
            body is not agent-controlled territory: any GitHub user with read access
            can submit a COMMENT review on a pull request in this public fork and
            write its body, marker and all. Matching on the marker alone therefore
            lets an outside party choose which review object this agent tries to
            update. The listing already carries what a filter needs — a review
            object's fields are `_links, author_association, body, commit_id,
            html_url, id, node_id, pull_request_url, state, submitted_at, user` —
            so the fix costs one comparison against data already fetched.
            Without it, the failure is a denial of publication that an outsider can
            trigger deliberately: they post a marked review, find_existing returns
            its id, the PUT is refused because the token does not own it, and the
            hard-failure rule below means the agent then publishes NOTHING on that
            pull request, permanently, presenting as an HTTP error rather than as an
            attack. #119's own criterion calls silence "indistinguishable from a
            crashed agent"; marker-only matching puts that silence on demand.
            THE IDENTITY IS RESOLVED AT RUNTIME, NOT HARDCODED — once at startup
            from `GET /user`, compared against each review's `user.login`. That
            preserves the whole point of identifying by marker rather than by a
            literal author name: #110 commits to revisiting the credential, and a
            hardcoded `github-actions[bot]` would break on the day it moves, which
            is the failure the marker-over-author choice was made to avoid. Marker
            for "this is our kind of review", author for "this one is ours".
            NEWEST AMONG OUR OWN, not oldest, and the two rules are not in tension
            once the filter exists. A submitted COMMENT review cannot be deleted, so
            once two of the agent's own markers exist neither can be retired and one
            stays stale forever. The one a human reaching the bottom of the timeline
            reads LAST is the newest, so that is the one that must carry the current
            body. An earlier revision changed oldest to newest WITHOUT adding the
            author filter, which is the one combination worse than either concern
            alone: taking the oldest at least required an attacker to plant before
            the agent's first run, and taking the newest removes even that timing
            requirement, so a review planted at any moment wins every run
            thereafter. Filter, then take the newest.
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
            THE PAGINATION SEAM, because otherwise the control for it cannot fail.
            `gh` merges pages itself: measured on this fork, `gh api
            "repos/launchpad-26/buzz/pulls?state=all&per_page=2" --paginate` over
            five requests returns ONE flat JSON array that `json.load` accepts,
            length 9. So a fixture holding gh's merged output is
            indistinguishable from a single page of 31 entries, and an assertion
            fed that fixture passes whether or not `--paginate` was ever sent.
            find_existing therefore takes an injected `list_reviews(argv) ->
            list[dict]` transport, defaulting to the real gh call. The stub in
            STEP 10 receives the actual argv and serves STEP 1's page two ONLY
            when `--paginate` is present in it. That is what makes "drop
            --paginate" a mutation that fails rather than one that passes.
          post_or_update(pr, repo, body) -> (id, "created"|"updated") — PUT when
            find_existing returns an id, POST otherwise. The event is the literal
            "COMMENT", hardcoded at the single call site, and the function takes
            no event parameter at all. A parameter that could hold "APPROVE" is a
            parameter that one day will.
            A FAILED PUT IS A HARD FAILURE AND NEVER FALLS BACK TO POST. If the
            PUT returns any non-2xx — 403, 404, 410 — post_or_update raises with
            the review id and the status in the message, and the workflow fails
            loudly with no review updated. A fallback POST would create the second
            review this entire issue exists to prevent, and it would do so on
            exactly the run where something is already wrong. This is not
            hypothetical: PUT on a review requires being its author, and #110
            commits to revisiting the credential, so the identity move the
            marker-over-author choice was designed to survive is precisely when
            find_existing matches a review the new identity cannot edit. A human
            retires the stale review by hand at that point; the tool does not
            paper over it by posting another.
        Immediately before a POST, find_existing is called a second time. Two
        pushes seconds apart produce two workflow runs, and a check performed at
        the start of a run is stale by the time the run posts.
        done when: `python3 -c "import publish"` run from launchpad/review-agent/
        succeeds — not `py_compile`, which compiles without resolving imports and
        so would pass on the circular import the MARKER rule above exists to
        prevent; `grep -nE "APPROVE|REQUEST_CHANGES" publish.py` returns nothing;
        `grep -c "def post_or_update" publish.py` is 1 and its signature has no
        event parameter; MARKER is a module-level constant in publish.py, asserted by
        reading `publish.MARKER` after import — the matching assertion, that
        publish_render imports nothing from publish, belongs to STEP 4's done-when
        and NOT here, because publish_render.py does not exist until STEP 4 and this
        step does not depend on it, so a grep against that file here would pass by
        the file's absence rather than by its contents;
        against STEP 1's recorded listing with no marker find_existing returns
        None, with one marker under the agent's own login returns that id, and with
        two of its own markers returns the NEWER id AND prints the duplicate count;
        a listing whose ONLY marked review carries a different `user.login` returns
        None and reports one FOREIGN marked review — the load-bearing case, since
        returning that id is the denial-of-publication vector; a listing carrying a
        foreign marked review submitted AFTER the agent's own returns the agent's
        own id, not the newer foreign one, so the author filter is proven to run
        before the newest-wins rule rather than after it; the identity is read from
        `GET /user` through the injected transport and `grep -n
        "github-actions\[bot\]" publish.py` returns nothing, so no login is
        hardcoded; with the injected transport
        serving page two only on `--paginate`, find_existing returns the marked id
        from page two, and the same transport with `--paginate` absent from the
        argv returns None — so the two cases differ and the assertion can fail;
        `--paginate` is present in the default transport's argv, asserted on the
        argv itself; a recorded listing whose second page returns an error causes a
        raise, not a None; and a stubbed PUT returning 403 makes post_or_update
        raise with the id and status in the message and issue NO POST, asserted on
        the recorded transport calls.

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
        `render_body(marker, reports, stages, containment, head_sha, merge_base_sha) -> str`.
        No network, no subprocess, no posting. `marker` is passed in rather than
        imported, per STEP 2 — this module must not import publish.py.
        `containment` is a SEPARATE argument, not an entry in `reports`, and #117's
        settled contract is now what says so rather than this plan's inference:
        containment findings "do NOT enter the `findings` array of any dimension
        report" and travel as a top-level sibling key. Its shape is
        `{findings: [{severity, kind, entry_point, evidence}], states:
        {entry_point: state}}` — the four raw `contain.Finding` fields, unrenamed,
        and `fetch.Surface.state` for all seven entry points. render_body
        reconstructs `contain.Finding` objects from that block and passes them,
        with `states`, straight into `review.render_review`, which fences and
        escapes each excerpt itself at review.py:71-73. Evidence is handed over
        RAW and exactly once: #117 now guarantees raw, and pre-escaping it here
        would escape it twice — a `~` publishing as `~~~~`, per #117's own
        reasoning — so the excerpt would stop matching what the author wrote.
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
        This block NOW EXISTS upstream, and that is the change that unblocked this
        step. #117's contract settled it normatively — a top-level `containment`
        key, present on EVERY run, carrying an empty `findings` array and a full
        seven-key `states` map when nothing was found, "never a missing key". So a
        missing block is reserved for the case where #117 genuinely could not
        produce one, which is exactly the reading the incomplete rule below needs.
        The dependency OPEN recorded is discharged, not assumed.
        A MISSING `containment` block is INCOMPLETE, never "no containment
        findings". CONTAINMENT.md's own reasoning is that a detected attempt which
        does not reach the review is worse than one never detected, because it
        reads as a clean review; an absent block is exactly that case and must
        never render as the "No containment findings" line, which is a positive
        claim. STEP 5 carries this as a trigger.
        Ordering: findings from every report are merged into one list and sorted
        by `review.SEVERITY_ORDER.get(finding["severity"], 9)`, ties broken by
        (dimension, file or "", line or 0, finding_id) so the order is total and
        the body is byte-identical for identical input. SEVERITY_ORDER is
        IMPORTED from review.py; a second copy of a four-value ladder drifts.
        `.get` WITH A DEFAULT, never a bare subscript, and review.py:62 already
        does exactly this — `SEVERITY_ORDER.get(f.severity, 9)`. One finding
        carrying a severity outside the ladder ("Info", or a lowercase "blocker"
        from a model-authored report) raises KeyError inside the sort key, and a
        KeyError here means publish.py exits non-zero having posted NOTHING. That
        is total silence on a pull request that had findings — the failure #119's
        clean-case criterion exists to prevent, arriving through the sort. #117's
        validator refuses an out-of-ladder severity upstream, but STEP 7 is
        deliberately agnostic about which stage produced the reports, so #119 must
        not depend on that refusal. An unrecognised severity sorts last, renders
        under the "malformed finding" heading below, AND triggers the incomplete
        banner: the review cannot claim to be ordered by a ladder it could not
        read.
        "Most severe first survives an update" is a property of construction, not
        of maintenance: the body is rebuilt from scratch on every run and nothing
        is ever appended to an existing one, so ordering cannot degrade across
        pushes the way an append-only comment would.
        BOTH SHAs ARE RENDERED, in a header line above everything else: the head
        SHA the review read and the merge base it was diffed against. An earlier
        revision threaded `merge_base_sha` through this signature and through STEP
        7's stdin document and then never rendered, compared or tested it — a value
        carried the whole length of the interface and dropped at the end, which no
        type checker sees and no done-when here caught. It was found by an
        independent review pass counting its occurrences.
        Rendering it is the right resolution rather than removing it, because
        #119's criterion is that "a review is valid only for the commit it read" and
        the commit it read is a PAIR: findings are anchored to new-side line numbers
        in the merge-base diff, per #117, so a reader given only the head SHA cannot
        reconstruct which diff a `path:line` refers to. A force-push that rewrites
        the base makes the same head SHA mean a different diff, and the merge base
        is what distinguishes them.
        Anchoring follows #117's three rules exactly and renders each accordingly
        — `anchor: line` as `path:line`, `anchor: file` as `path`, `anchor: pr` as
        `(pull request)`. A finding whose anchor and fields disagree is rendered
        under an explicit "malformed finding" heading with its raw record, not
        dropped: a finding silently discarded by the publisher is a finding the
        reviewer believes it reported.
        THE MALFORMED RECORD IS THE MOST UNTRUSTED THING THIS RENDERER PRINTS, and
        it gets the same treatment as evidence: serialised, passed through
        `contain.escape`, and wrapped in a `review.fence_for`-sized fence. It is
        the path a record takes precisely BECAUSE its fields did not match the
        contract, so it is the last place to assume any field is well-formed — an
        attacker-shaped record reaches it by construction, and a `defect` value
        carrying a long backtick run would otherwise close a fixed fence and spill
        the rest of the review out of the code block. The fence is sized off the
        serialised record, not off any single field.
        Every finding renders `defect` and `failure` as separate lines, because
        #119's criterion is "the concrete failure it allows" and a defect with no
        stated consequence is what lets an unfalsifiable finding through.
        A DIMENSION FINDING CAN ALSO CARRY RAW ATTACKER TEXT, and this is the part
        the containment block does not cover. #117's settled contract makes
        `entry_point` REQUIRED on any finding whose defect is an injection attempt,
        with `evidence` required alongside it and RAW — and #117's STEP 5 puts that
        clause in all three dimensions, for the paraphrase cases detect.py misses.
        So `reports[].findings[].evidence` will hold verbatim author-supplied text,
        arriving through the ordinary findings path, not through the block that
        `review.render_review` handles.
        Two rules follow, and both are load-bearing:
          RENDER IT. A finding with an `entry_point` renders that entry point and
            its evidence. Dropping the excerpt is the detected-then-dropped case
            CONTAINMENT.md calls worse than never detecting, and it would land on
            exactly the 14 attack classes #117 was handed because detection misses
            them.
          FENCE AND ESCAPE IT WITH THE FUNCTIONS THAT ALREADY EXIST.
            `review.fence_for(evidence)` sizes the fence longer than the longest
            backtick run in the excerpt, and `contain.escape(evidence)`
            neutralises the envelope delimiter. Both are IMPORTED, never
            reimplemented — review.py:21-29 is explicit that attacker text
            containing ``` "would therefore break out of a fixed three-backtick
            fence and corrupt every following section of the review", and a fixed
            fence here would do it in the one place a human is reading.
        `defect` and `failure` are NOT passed through `contain.escape`, and an
        earlier revision of this step said they were, on the strength of a claim
        that measurement falsifies. That revision asserted "the transform only
        touches the escape character and the envelope token, so ordinary prose
        survives it unchanged". `contain.ESC` is a single TILDE. Measured:
          '~/.claude/settings.json is read at startup'
            -> '~~/.claude/settings.json is read at startup'
          'the ~ operator inverts bits'  ->  'the ~~ operator inverts bits'
        `~~text~~` is STRIKETHROUGH in GitHub-flavoured markdown, so a defect line
        naming two home-relative paths publishes with the passage between them
        struck through, and a single mention publishes a visible doubled tilde.
        This fork's own subject matter is dotfile paths — this plan cites
        `~/.claude/skills/plan-issue/check-plan.sh` — so the corruption would land
        on the most likely findings, in the one surface a human reads, and no
        assertion in STEP 10 looks at `defect` rendering at all.
        `contain.escape` is a boundary guard for text re-entering a delimited
        envelope. It is not a markdown sanitiser and this body is not fed back to a
        model in Phase 1. What IS worth neutralising in model-authored prose is the
        envelope token itself, so `defect` and `failure` have `contain.TOKEN`
        replaced with `contain.ESC_TOKEN` and nothing else touched. That is the
        narrow half of the escape, and it leaves a tilde alone.
        Evidence is a different case and IS shown escaped, because `render_review`
        escapes containment evidence at review.py:72 and a dimension finding's
        excerpt must not render by a different rule than a containment finding's.
        Inside a `fence_for` fence `~~` is literal rather than strikethrough, so
        the cost is a visible doubled tilde and not a corrupted body — but it does
        mean the excerpt is not byte-for-byte what the author wrote, so STEP 12
        says so rather than letting a reader assume otherwise.
        done when: given three report envelopes carrying findings of mixed
        severity, the output lists every Blocker before every High, every High
        before every Medium, and every Medium before every Low; two calls with
        the same input produce byte-identical strings; the body contains BOTH the
        head SHA and the merge-base SHA, and a call with two different SHA values
        produces a body containing both of them rather than one twice — the second
        half is the assertion, because rendering head_sha into both slots satisfies
        "contains the head SHA" on its own; a finding with anchor "pr"
        and a non-null `file` appears under "malformed finding" and is still
        present in the output; a finding whose `severity` is "Info" appears under
        "malformed finding", sorts last and does NOT raise — its banner is STEP 5's
        assertion, not this step's, because STEP 4 is [independent] and the banner
        does not exist until STEP 5 builds it; the body's first line is the `marker`
        argument, asserted
        with a sentinel value rather than with publish.py's constant, so this step
        needs no import from publish.py; `grep -n "^from publish import\|^import
        publish$" publish_render.py` returns nothing; `grep -n "SEVERITY_ORDER *="
        publish_render.py` returns nothing; a containment block carrying one
        Blocker finding produces a body containing that finding's `kind`, its
        `entry_point`, and its evidence in ESCAPED form — asserted by comparing
        against `contain.escape(evidence)`, not against the raw string; a
        DIMENSION finding carrying an `entry_point` and an evidence string
        containing a four-backtick run renders that evidence escaped and inside a
        fence of at least five backticks, and the text following it is still
        outside any code block — the assertion is on the fence length and on what
        comes after, because a body that merely contains the excerpt is what a
        broken fence also produces; a malformed record whose `defect` carries a
        four-backtick run is likewise fenced longer than that run and escaped, with
        the text after it still outside any code block — same assertion, applied to
        the other untrusted path; `python3 -c "import pathlib,sys;
        sys.exit(0 if chr(96)*3 not in
        pathlib.Path('publish_render.py').read_text() else 1)"` exits 0, which bans
        a FIXED fence rather than fencing itself — `review.fence_for` builds its
        fence from `chr(96)` at runtime and so satisfies this check, and it is the
        only permitted source of a fence in the module; and a run with
        `containment=None`
        does NOT contain the string "No containment findings" — the positive claim
        is what this step can assert without the banner, and the banner itself is
        STEP 5's.
        WHY TWO CLAUSES MOVED OUT OF THIS DONE-WHEN. An earlier revision asserted
        the incomplete banner here, in a step tagged [independent], while STEP 5
        [needs 4] is what builds it. That is a done-when observing something its own
        tag cannot produce: STEP 4 could not close until STEP 5 existed, STEP 5
        could not start until STEP 4 closed, and the way out of a deadlock like that
        is always to declare the step done on the clauses that do pass. #117's
        second review pass found this same defect in four of its steps. Both clauses
        now live in STEP 5's done-when, which is the step whose tag supports them.

STEP 5  The incomplete case — an unfinished stage is never rendered as done. [needs 4]
        A `stages` manifest of {name, status, reason} entries accompanies the
        reports and names EVERY stage the review depended on — #116's pre-flight,
        #117's three dimensions by slug, and #118's adjudication. Not only the
        stages that emit no envelope of their own, which is what an earlier revision
        said and which contradicted this step's own condition (7) and STEP 6's
        done-when three lines apart: both require dimension names to be IN the
        manifest, and the definition excluded them.
        The dimensions are in it because A REPORT CANNOT TESTIFY TO ITS OWN
        ABSENCE. Every other condition here reads a report that arrived; (7) is the
        only one that catches a dimension crashing so completely that #117 emits no
        envelope for it at all, and it has nothing to compare `reports` against
        unless the manifest says which dimensions were expected. Built to the old
        definition, the manifest held two entries, neither a dimension, so (7) could
        never fire: a three-dimension run that produced two reports rendered as
        COMPLETE. That is the partial review reading as a complete one that #119's
        done-criteria forbid by name, reached through the one condition written to
        prevent it. Found by an independent review pass, not by this plan's author.
        A review is INCOMPLETE when any of
        these TEN conditions holds. The count is stated because an earlier revision
        listed six, called them seven in its own done-when and six again in STEP
        12, and a builder told to test "each of the seven" over a six-item list
        invents one or drops one:
           (1) a stage in the manifest has status other than "complete"
           (2) a report has `status: failed`
           (3) a report's `completion_marker` is absent, or is not the last key
           (4) a report's marker names a dimension other than that report's own
               `dimension` field
           (5) two reports' markers carry DIFFERENT nonces
           (6) `findings_count` does not equal `len(findings)`
           (7) a dimension named by the manifest produced no report at all
           (8) a report has `status: complete` and no `outcome` — moved here from
               STEP 6, which is where it was stated and where STEP 5 could not see
               it
           (9) the `containment` block is absent or unparseable
          (10) `set(containment.states)` does not EQUAL
               `set(contain.ENTRY_POINTS)` — a set comparison against the imported
               tuple, never a count. "All seven entry points" is satisfied by
               counting to seven, and the realistic bug is six real keys plus one
               typo, which counts to seven and passes. The banner names the
               difference in both directions.
        On (5), and on what #119 CANNOT check. #117's marker is
        BUZZ-DIMENSION-COMPLETE:{dimension}:{nonce} and the nonce is what makes it
        unforgeable — but the nonce appears in NO field of the merged document and
        in no field of the envelope, so a stage downstream has nothing to compare
        it against. An earlier revision of this step said the marker is checked for
        "the wrong dimension or nonce"; the dimension half is checkable against the
        envelope's own `dimension`, and the nonce half is NOT, from this input.
        What IS checkable is agreement: every report in one run carries the same
        run nonce, so a single forged marker differs from its siblings and (5)
        catches it. A run in which EVERY marker is forged is not detectable here at
        all, and pretending otherwise would be the fail-open. That residual is
        named in OPEN and flagged upstream rather than worked around.
        (5) IS VACUOUS BELOW TWO REPORTS, and this is stated rather than left for a
        reader to discover. Agreement needs siblings to disagree with, so a
        single-dimension run — or a three-dimension run where two came back
        `status: failed` — has NO nonce checking whatever, while (2) and (7) still
        fire on the failures. The check's strength is proportional to how many
        reports survived, and it is zero exactly when the run was already degraded.
        That is not a reason to drop it; it is the reason the upstream fix matters.
        A `nonce` key on #117's merged document would make the check absolute and
        independent of report count, which is why OPEN raises it there rather than
        treating (5) as sufficient.
        On (10), because a present-but-thin map is the dangerous shape. STEP 4
        derives nothing and `review.render_review` derives its "Incomplete" banner
        from `states` against `UNREADABLE_STATES`. A map populated only for the
        surfaces that succeeded makes every unreadable surface read as
        absent-from-the-map rather than unreadable, and the banner never renders —
        a review over three unreadable surfaces publishing as complete. Counting
        the keys is the check; asserting the key exists is not.
        Incomplete renders as a banner at the TOP of the body, above the findings,
        naming every stage that did not finish and its reason. It is at the top
        because a reader who stops after the first finding must still have seen
        it, and #119's criterion is that the comment "never publishes a partial
        review that reads as a complete one".
        The default is incomplete. An input that cannot be classified — a report
        that will not parse, a manifest entry with no status — is incomplete, not
        complete. Absence of a failure signal is not evidence of success.
        done when: for each of the TEN conditions above, given an input exhibiting
        ONLY that condition, the body contains the incomplete banner and names the
        offending stage, dimension or entry point — ten inputs, ten assertions,
        counted against the numbered list above rather than against the word
        "every"; for an input exhibiting none of them the banner is absent, which
        is the negative control that stops a banner that always fires from passing
        all ten; a two-report input whose markers carry different nonces is
        incomplete while the same input with matching nonces is not; a
        `containment.states` map naming six of the seven entry points is incomplete,
        AND so is one naming seven keys of which one is not in
        `contain.ENTRY_POINTS` — two inputs, because the second is the one a count
        lets through; a manifest naming three dimensions against `reports` carrying
        two is incomplete and the banner names the missing slug; and an unparseable
        report is incomplete rather than raising.
        TWO CLAUSES INHERITED FROM STEP 4, which asserted them while tagged
        [independent] and could not produce them: a finding whose `severity` is
        outside the ladder triggers the banner, and a `containment=None` input
        triggers the banner. Both are behavioural assertions about this step's own
        code, so they belong here.

STEP 6  The clean case — no findings still posts, and says so.               [needs 4]
        Every stage complete and every report `outcome: clean` renders an explicit
        body: the SHA reviewed, the dimensions that ran, and a sentence saying no
        confirmed findings were produced. #119's reasoning is that silence is
        indistinguishable from a crashed agent, so this path posts on exactly the
        same code path as the findings path — there is no early return that skips
        publication.
        `outcome: clean` and an empty findings array are not the same input.
        A report with `status: complete` and no `outcome` is incomplete — it is
        condition (8) of STEP 5's list, which is where it now lives. It was stated
        only here in an earlier revision, so STEP 5 enumerated six conditions while
        two of its own done-whens counted seven, and the seventh was this sentence.
        THE CLEAN PATH MUST BE PROVEN TO POST, not merely to render, and that proof
        lives in STEP 10 (viii) because the decision to post is in `publish.py`'s
        `main` and not in the renderer at all. An earlier revision offered `grep -n
        "return None" publish_render.py` as evidence of "no early return on the clean
        path". That check cannot fail: it greps the PURE RENDERER, whose contract is
        `-> str`, so no correct implementation ever returns None from it, and it
        passes for every implementation. Meanwhile the actual risk — `if not findings
        and not incomplete: return 0` in `main`, which looks like a reasonable
        optimisation against the edit-event noise OPEN itself raises — lives in a file
        that grep never reads. The grep is withdrawn.
        done when: an all-clean input produces a body containing the head SHA, the
        name of every dimension in the manifest, and the no-findings sentence; that
        body still carries the marker as its first line; and an input with `status:
        complete` and no `outcome` produces the STEP 5 banner rather than the clean
        sentence. That a clean run POSTS is asserted in STEP 10 (viii), against
        `main` and through the recorded transport, and is not claimed here.

STEP 7  Wire the renderer into the CLI.                                  [needs 2, 4]
        `publish.py` gains a `main` reading one JSON document on stdin —
        `{pr, head_sha, merge_base_sha, stages, reports, containment}`, where each
        entry of `reports` is a #117 envelope verbatim and `containment` is the
        block specified in STEP 4 — rendering it through publish_render, and
        calling post_or_update. A `--dry-run` prints the body and posts nothing.
        The document WRAPS #117's envelopes; it does not restate or rename a
        single field inside them, and `containment` is a sibling key precisely so
        that it does not have to. #117's contract quotes this shape back — "`reports`
        and `containment` mean here exactly what they mean there" — so the six keys
        are fixed by agreement between two plans and are not #119's to extend.
        `repo` IS NOT ONE OF THEM, and it has to come from somewhere. `find_existing`
        and `post_or_update` both take it, #117's merged document does not carry it,
        and adding it to the stdin document would break the shape #117 documents. So
        it comes from a `--repo owner/name` flag, defaulting to `GITHUB_REPOSITORY`,
        and publish.py EXITS NON-ZERO when neither is present. Not a hardcoded
        `launchpad-26/buzz`: that is wrong the day the fork is renamed and unusable
        from any other checkout. Not a silent `None` either — that builds the path
        `repos/None/pulls/...`, which 404s in a way that reads like a missing pull
        request. STEP 10's controls inject a transport, so neither mistake would be
        caught by any control in this plan; the guard has to be in the code.
        done when: `python3 publish.py --dry-run --repo launchpad-26/buzz <
        fixture.json` exits 0 and prints a body whose first line is the MARKER;
        the same command with `--repo` omitted and `GITHUB_REPOSITORY` unset exits
        non-zero and names the missing repository, posting nothing; the same
        fixture with
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
          NO `--repo` FLAG IS NEEDED HERE. STEP 7 requires either `--repo` or
            `GITHUB_REPOSITORY` and exits non-zero without both; Actions sets
            `GITHUB_REPOSITORY` for every run, so the default covers this workflow
            and the flag is for local invocation. Stated because a reader arriving
            from STEP 7's hard failure will otherwise look for a flag that should
            not be here — and hardcoding one into the workflow would reintroduce
            exactly the wrong-the-day-the-fork-is-renamed problem STEP 7 rejects.
          A STEP that runs `check_publish_scope.py`, in THIS workflow. The live
            half of that control must execute under the credential it claims to
            measure, and this is the only workflow whose token is that credential.
            The script does not exist until STEP 9 writes it; declaring the step
            here is what makes STEP 9's live half reachable at all, and this file's
            done-when checks the step's presence rather than the script's behaviour.
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
        fixed group name fails this check rather than passing it; no job in
        the file declares its own `permissions`; and the file contains a step
        invoking `check_publish_scope.py`, asserted on the parsed YAML rather than by
        eye, because a live credential control that no workflow runs is a control
        that never executes.

STEP 9  launchpad/review-agent/check_publish_scope.py — the credential control. [needs 8]
        Two assertions, because either alone is weak.
          STATIC — parse the workflow YAML and assert the permissions mapping
            equals exactly {contents: read, pull-requests: write}, that no job
            overrides it, and that the file does not mention
            pull_request_target. This runs anywhere, needs no token, and catches
            a later widening in review.
          LIVE — with the workflow's own token, attempt one contents write:
            create the ref `refs/heads/scope-probe-<run id>`, where the run id is
            read from `os.environ["GITHUB_RUN_ID"]` and the control FAILS with a
            stated reason when that variable is absent. NOT the literal string
            `${{ github.run_id }}`: that is an Actions expression, interpolated by
            the workflow YAML and never by Python, so transcribing it into this
            module produces a ref name containing spaces and braces. GitHub then
            answers on ref-name validity before it evaluates permissions, the
            control fails for a reason that has nothing to do with scope, and it
            does so in the step BUDGET already names as the most expensive to
            iterate on. Assert
            HTTP 403. Any other outcome is FAIL, including success, 404, and a
            rate-limit error — a probe that treats "some error happened" as proof
            of absent permission is fail-open, and would report PASS on a network
            blip. If the probe unexpectedly SUCCEEDS the control deletes the ref
            it made and still fails.
        THE LIVE HALF MUST ASSERT WHICH WORKFLOW IT IS RUNNING IN, and this is the
        difference between measuring the credential and measuring a different one.
        It reads `GITHUB_WORKFLOW` and reports SKIP with a reason — never PASS —
        unless it is the publish workflow. Absent that guard the control is
        worse than useless: STEP 11 registers it in `run_controls.py`, and that
        runner is invoked by #120's controls workflow, whose permissions block on
        `feat/review-agent-untrusted-input` at `c64ff7958` reads
        `{contents: read, issues: read, pull-requests: read}` under the comment
        "Read-only, and no write scope of any kind". A ref-create under THAT token
        returns 403 too — both tokens carry `contents: read` — so the control would
        report PASS, and the 403 body would be pasted into the pull request as
        evidence about the publish credential having measured a read-only one. The
        criterion #119 states — "a control or documented check demonstrates the
        absence of contents write" — would read as satisfied while nothing had tested
        the token in question. A control that passes under the wrong token is not a
        weak control; it is a false one.
        The "no token outside Actions" SKIP does not cover this. Inside #120's
        controls workflow there IS a workflow token — it is simply the wrong one — so
        a guard keyed on the token's existence never fires. The guard has to be keyed
        on WHICH workflow is running.
        Outside Actions there is no workflow token at all, so the live half also
        reports SKIP with a reason and never PASS — the rule run_controls.py already
        enforces. This is the only step that can demonstrate #119's credential
        criterion, it can only do so inside a real run of the PUBLISH workflow, and
        STEP 3's local evidence does not substitute for it.
        done when: the static half fails when handed a copy of the workflow with
        `contents: write` and passes on the real one; the live half reports SKIP
        with a stated reason when GITHUB_TOKEN is absent; the live half reports SKIP
        with a stated reason when `GITHUB_WORKFLOW` names any workflow other than the
        publish one — asserted by setting it to the controls workflow's name, which is
        the exact wrong-token case, and the result must be SKIP and never PASS; a
        recorded 404 response fed to the live half yields FAIL rather than PASS; and a
        real Actions run of the PUBLISH workflow on this pull request shows the live
        half reporting PASS with the 403 response body AND the value of
        `GITHUB_WORKFLOW` pasted into the PR together, so a reader can see which
        credential was measured rather than taking it on trust.

STEP 10 launchpad/review-agent/check_publish_single.py — the behaviour controls. [needs 7]
        Recorded inputs, no network, no model. SEVEN assertions covering #119's
        done-criteria, and EVERY one of them carries a stated mutation that must
        break it. A control never observed failing has not been shown to test
        anything, and the temptation is to prove that only for the assertions
        where it is easy — which leaves the load-bearing ones unproven.
        Every recorded input here is STEP 1's, by path, and none is authored for
        this step. An earlier revision cited "the recorded listing from STEP 1"
        when STEP 1 produced only four operation responses, and "the recorded
        two-page listing from STEP 2" when STEP 2 saved nothing — so both fixtures
        would have been hand-written, and a hand-written listing tests this plan's
        belief about gh rather than the code. STEP 1 now names both as
        deliverables.
          (i)   the event published is COMMENT and the module contains no other
                event string — the control asserts on the source, since a runtime
                assertion cannot prove an absent branch.
                Mutation: add the literal "APPROVE" to publish.py.
          (ii)  a second run over the same PR with a marker present issues a PUT
                and no POST, over fixtures/reviews-listing.json from STEP 1 and an
                injected transport that records calls instead of making them.
                Mutation: make find_existing return None unconditionally. This is
                the assertion the single-review invariant rests on, so it gets the
                mutation proof first, not last.
          (iii) find_existing paginates — the transport serves page two of STEP 1's
                recorded listing ONLY when `--paginate` appears in the argv it is
                handed, so the marked id on page two is reachable with the flag and
                unreachable without it. The fixture is the UNMERGED page bodies:
                `gh --paginate` merges pages into one array, measured, so a merged
                fixture makes this assertion pass either way.
                Mutation: drop `--paginate` from the listing call. With the
                flag-aware transport the assertion then fails, which is the whole
                point of building the seam.
          (iv)  severity order holds in the rendered body after an update whose
                input has a NEW Blocker appended LAST in the reports array — the
                Blocker still renders first.
                Mutation: replace publish_render's sort key with identity.
          (v)   a clean input and an incomplete input both produce a body, and the
                two bodies differ.
                Mutation: remove the incomplete banner.
          (vi)  a dimension finding whose `entry_point` is set and whose `evidence`
                contains a four-backtick run renders inside a fence of at least
                five backticks, escaped, with the following section still outside
                any code block.
                Mutation: replace `review.fence_for` with a fixed three-backtick
                fence. The assertion must fail on the text AFTER the excerpt, not
                on the excerpt itself — a broken fence still contains the excerpt,
                so an assertion that only looks for it passes under the mutation.
          (vii) a PUT that returns 403 raises and issues no POST.
                Mutation: make post_or_update fall back to POST on a failed PUT.
                This is the mutation that produces a second review, so it is the
                one whose absence would be least visible: nothing else in this
                suite would notice.
          (viii) AN ALL-CLEAN INPUT POSTS. `main` is driven end to end over a
                fixture whose every report is `outcome: clean`, through the injected
                transport, and the transport records EXACTLY ONE write call. This
                asserts against `publish.py`, not against the renderer, because the
                decision to post lives in `main` and nowhere else.
                Mutation: add `if not findings and not incomplete: return 0` to
                `main`. That is not a strawman — it looks like a reasonable
                optimisation against the edit-event noise OPEN itself raises — and
                before this assertion existed the whole suite passed with it applied:
                STEP 6 offered only a grep of the pure renderer, whose `-> str`
                contract means it never returns None in any correct implementation,
                and STEP 7's done-when exercises `--dry-run`, which posts nothing by
                definition. The agent would have gone silent on exactly the pull
                requests where it found nothing, which is #119's criterion verbatim:
                "A run that produced no confirmed findings still posts... Silence is
                indistinguishable from a crashed agent."
          (ix)  A FOREIGN MARKED REVIEW IS NOT A CANDIDATE. Over a listing whose
                only marked review carries another `user.login`, `main` POSTs a new
                review and issues no PUT against the foreign id.
                Mutation: drop the author comparison from find_existing. The
                assertion then sees a PUT against a review the agent does not own,
                which is the denial-of-publication vector an outside party can
                trigger deliberately, so it gets a control rather than only prose.
        done when: all NINE assertions run offline and pass; each of the nine
        stated mutations, applied one at a time, makes exactly its own assertion
        fail and is then reverted; the recorded output of all nine mutation runs is
        saved for the PR body; and each assertion prints what it compared rather
        than only PASS.

STEP 11 Register both controls in run_controls.py.                       [needs 9, 10]
        Two entries appended to CONTROLS: ("check_publish_scope.py", True) and
        ("check_publish_single.py", False). The scope control needs network for
        its live half and is expected to SKIP that half locally.
        REGISTERING IT HERE DOES NOT MAKE THIS RUNNER ITS HOME. `run_controls.py` is
        invoked by #120's read-only controls workflow, so the scope control's live
        half must SKIP there on STEP 9's `GITHUB_WORKFLOW` guard, and its PASS can
        only ever come from the publish workflow that STEP 8 declares. It is
        registered here so the static half travels with every other control and so
        the suite has one entry point, not so that a run of this runner can satisfy
        #119's credential criterion. A summary line counting it as passed inside the
        controls workflow would be the false PASS this arrangement exists to prevent.
        done when: `python3 run_controls.py` runs both and its summary line counts
        them; with `gh` unauthenticated the scope control appears in the skipped list
        with a reason rather than in the passed list; and with `GITHUB_WORKFLOW` set
        to the controls workflow's name the scope control's live half still appears
        as skipped, so the runner cannot report a live PASS for a token it is not
        holding.

STEP 12 launchpad/review-agent/PUBLISHING.md, and the cross-references.     [needs 11]
        Normative, a sibling to CONTAINMENT.md and in the same voice. States: that
        a review is identified by MARKER AND AUTHOR — the marker says "this is our
        kind of review", the author says "this one is ours" — that the author login
        is resolved at runtime from `GET /user` and never hardcoded, and that a
        marked review under any other login is counted as foreign and never
        updated, because a review body is attacker-writable and marker-only
        matching hands an outsider a way to silence the agent on a pull request of
        their choosing; that the live credential control PASSes only from the
        publish workflow and SKIPs everywhere else, so a PASS from the read-only
        controls runner is not evidence about the publish token; and that
        publish.py owns it while publish_render receives it; that exactly one
        review object exists per pull request and is updated in place, and that a
        failed PUT raises rather than posting a second review; that when two
        markers do exist it is the NEWEST that is kept current, because the oldest
        cannot be deleted and the newest is what a reader sees last; that the head
        SHA lives in the body because `commit_id` is frozen at submission; the
        incomplete rule and its TEN triggers; that the clean case posts; that the
        body names both the head SHA and the merge base, and why the pair rather
        than the head alone; that raw evidence is fenced with `review.fence_for`
        and escaped with `contain.escape`, never with a fixed fence, and that this
        means a published excerpt shows a doubled tilde where the author wrote one,
        so a reader does not mistake the renderer's escape for the author's text;
        that `defect` and `failure` are NOT escaped, with the measured reason —
        `contain.ESC` is a tilde and `~~` is markdown strikethrough; the credential
        and its two controls; and the fork-skip behaviour.
        Cross-referenced from CONTAINMENT.md's "Contract for later stages" table
        and from #117's FINDINGS.md, so the three documents point at each other
        rather than diverging quietly.
        done when: PUBLISHING.md exists under launchpad/review-agent/; it names
        the marker string, the TEN incomplete triggers, the two keys of the
        `containment` block it consumes from #117 — `findings` and a seven-entry
        `states` map, and no `unreadable` key — and both controls by filename;
        CONTAINMENT.md's contract table has a row pointing at it; and it records
        that #117's contract is SETTLED and names which revision, together with the
        one thing #119 cannot verify from it (the marker nonce, see OPEN), so a
        reader is not left to infer that everything upstream is checkable here.

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
  serina:review-plan — HAS RUN ONCE on this file, and this revision is the result
    of it plus #117's contract settling. Fourteen findings: one Blocker, six High,
    six Medium, one Low. The Blocker was WITHDRAWN — it concluded #117 had moved
    containment inside `reports`, and #117 then settled it the other way, so the
    separate block was right all along. Its residue was real, though: the plan's
    ALREADY TRUE was asserting a nine-field record and no `evidence`, both wrong.
    Of the other thirteen, eleven are fixed in this revision, one (finding 3, the
    nonce) is fixed as far as this stage can and flagged upstream in OPEN, and one
    (finding 13, whether an edit is visible to a human) is converted from an
    unanswerable question into a STEP 1 observation. A second pass is available and
    has not run; on both #116 and #117 the second pass found most of its findings
    in the first pass's own fixes, so assume the same of this revision.
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
  #117's contract is SETTLED, and these are the steps that change if it moves
  again. A rename or removal of `severity`, `anchor`, `file`, `line`, `defect`,
  `failure`, `dimension`, `entry_point` or `evidence` changes STEP 4. A change to
  `status`, `outcome`, `error`, `completion_marker` or `findings_count` changes
  STEP 5, and `outcome` alone also changes STEP 6. A change to the `containment`
  block's two keys changes STEPs 4, 5 and 12. Either changes STEP 10's recorded
  inputs and STEP 12's prose. STEPs 1, 2, 3, 8, 9 and 11 are unaffected by any
  field rename, because they operate on a body string and a credential and never
  look inside a finding. `finding_id` is not used by #119 at all except as a
  tie-break in STEP 4's sort, so #117's warning that it is unstable across a
  reworded `defect` costs this issue nothing — the body is rebuilt wholesale on
  every run rather than diffed against the previous one.
  AN ADDITION BELONGS IN THIS REGISTER TOO, and its absence is what actually cost
  this plan a revision. The list above tracked renames and removals. #117 changed
  by ADDING `evidence` as a tenth field and by ADDING the `containment` sibling
  key — no rename, no removal — and this plan's ALREADY TRUE went on asserting
  that neither existed, because nothing prompted a re-read. A field that appears
  is as consequential as one that disappears: `evidence` arriving raw is the
  reason STEP 4 now carries a fencing rule at all, and a plan that only watches
  for subtraction will publish an unescaped payload while its register stays
  green. So: a NEW field on the finding record, a NEW key on the envelope, or a
  NEW key on the merged document changes STEP 4, and any of the three obliges a
  re-read of the contract before STEP 4 is built rather than after.
  A concern with the contract, raised rather than worked around. #117 states that
  #118 re-rates severity and that "the reporting dimension's value must remain
  readable after adjudication rather than being overwritten in place" — but the
  record carries exactly ONE `severity` field, so there is nowhere for the
  re-rated value to live. #119 sorts by `severity` and cannot tell which of the
  two it is holding. Either the contract needs a second field or the sentence
  needs to go; this plan does not choose, and does not silently diverge, because
  #118 will honour the same contract.
  The second concern is DISCHARGED, and recorded as discharged rather than
  deleted. An earlier revision said "**#117 must add one key to its output before
  #119 can publish a containment finding**", and that every real injection attempt
  would render as the incomplete banner until it did. #117 has added it: a
  top-level `containment` key carrying raw `contain.Finding` plus a seven-key
  `states` map, present on every run. The design call this plan declined to make —
  a sibling key versus a `kind`/`evidence` pair absorbed into the finding record —
  went to the sibling key, which is what STEP 4 already assumed. Nothing here had
  to change to accommodate it; what had to change was ALREADY TRUE's claim that
  the key did not exist.

  A NEW concern with the settled contract, flagged rather than worked around: the
  marker nonce is unverifiable by this stage. #117 makes the completion marker
  BUZZ-DIMENSION-COMPLETE:{dimension}:{nonce} and argues, correctly, that the
  nonce is what stops a pull-request author typing a forged marker into their own
  diff and a reviewer echoing it back. But the nonce is in no field of the finding
  record, no field of the envelope, and none of the five merged-document keys — so
  every stage after the runner receives markers it cannot check. #117's own
  `validate(report)` has the same problem from the other side: it is specified to
  reject a marker with the wrong nonce while taking one argument and reading a
  document that does not carry one.
  #119 does what it can: STEP 5 condition (5) rejects a run whose reports disagree
  about the nonce, which catches any single forged marker, and this plan does NOT
  claim to catch a run in which every marker is forged. The clean fix is upstream
  and cheap — a `nonce` key on the merged document, which publishes nothing secret
  because the markers already contain it — and it is #117's or #118's call, not a
  builder's. Recorded here so that "the marker is unforgeable" is not read as a
  property this stage enforces.

  Whether `defect` and `failure` should be escaped at all is #119's call and this
  plan has made it: yes, through `contain.escape`. They are model-authored prose
  rather than author-supplied text, so no upstream contract requires it, and a
  reader may reasonably think it is belt-and-braces. The argument for it is that a
  model quoting an attacker's delimiter into its own defect line is the one path by
  which the payload re-enters the document at full authority after containment has
  done its job, and the transform touches only two characters. If a reviewer
  disagrees, the place to change it is STEP 4's rule and STEP 12's prose together.
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
