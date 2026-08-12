Issue #117 — task: the parallel review dimensions that produce findings
Stated size: none given — the task template has no Size field  ->  cap: 12 steps

Sized by asking, not guessing. Answered: more than an hour, so the cap is 12.
Three further answers shape this plan and were also asked rather than assumed:
three dimensions — secrets-and-access, claim-vs-evidence, and
correctness-and-failure-modes; the semantic-injection cases #120 hands to #117
are a cross-cutting clause in every dimension, not a fourth dimension; and the
falsifiability criterion is met by recorded outputs for the deterministic suite
plus one live prompt-mutation run per dimension, pasted raw into the PR body.

Larger than an hour is flagged, not refused. These would have been better issues,
each observable on its own — splitting is the reader's call, not this plan's:

  (a) the findings contract alone — record shape, severity ladder, anchoring
      rules, completion marker — which is the artefact #118 is blocked on
  (b) the concurrent runner with a stub reviewer, proving isolation and the
      clean-versus-failed distinction with no prompt written yet
  (c) the three dimension definitions and their scopes
  (d) the fixture diffs, recorded outputs, and the mutation evidence

Planned as written below.

ALREADY TRUE  (verified against git, the working trees and the GitHub API, not notes)
  A note on how the containment tree is cited below. It is UNCOMMITTED and being
  actively edited: between this plan's first and second review passes,
  contain.Finding's severity default moved from line 66 to 83, contain.render from
  214 to 233, `_gh`'s docstring from fetch.py:40 to 43, and the Finding call sites
  from four to FIVE. Every substantive claim survived; only the pins rotted. So
  the claims below name SYMBOLS and FILES, not line numbers. A line number pinned
  to a file on no branch is a citation with a half-life, and re-pinning it each
  time it drifts teaches a reader to trust a number that is wrong again by the
  time they read it.
  Branch feat/review-agent-dimensions is at d897a06e8. `git rev-list
  --left-right --count origin/launchpad...HEAD` reports 0 0, so nothing of #117
  is built. `git ls-files | grep -iE 'dimension|reviewer|review-agent'` matches
  one unrelated upstream React file — desktop/src/features/projects/ui/
  PullRequestReviewersRow.tsx — and nothing of this work.
  Toolchain present: python3 3.12.3, gh 2.93.0.
  ADR #110 is decided and #117 is unaffected by it. The decision comment chooses
  GitHub Actions for Phase 1 with a committed revisit, and the credential is a
  GitHub token scoped to launchpad-26/buzz (pull-request write, contents read) —
  no Buzz identity, deferred explicitly. The comment names #116 and #119 as what
  it unblocks and states that "#117, #118 and #120 never depended on this ADR and
  are unaffected". So this plan adds NO workflow file: #120 already added
  .github/workflows/launchpad-review-agent-controls.yml (untracked, see below)
  and the dimensions' controls belong in that runner, not a second one.
  #120's work exists, is substantial, and is committed nowhere. The worktree
  /home/serina/Launchpad/buzz__worktrees/feat-review-agent-untrusted-input is at
  d897a06e8 with launchpad/review-agent/ UNTRACKED — `git log --all --oneline --
  launchpad/review-agent/` returns nothing, so the tree is on no branch and in no
  commit. It is 1,789 lines across 14 Python modules plus CONTAINMENT.md (218
  lines) and three fixtures. Everything #117 imports currently exists only in
  another worktree's working directory. This is the single largest risk in this
  plan; see BUDGET.
  CONTAINMENT.md is normative and binds #117 by name. Its "Contract for later
  stages" table requires #117 to call `contain.render(surfaces, nonce)` "before
  any text reaches a model" and forbids placing "any surface above the preamble or
  after the closing marker". That function exists and its signature is
  `render(surfaces: dict, nonce: str, *, enabled: bool = True) -> tuple[str,
  list, bool]`, returning (document, findings, all_readable) — contain.py:214. It
  already folds in `detect.detect` findings itself, so a caller that also calls
  `detect.detect` double-reports.
  Part of the output contract is already set, and it does not carry file:line.
  `contain.Finding` is a dataclass of kind, entry_point, evidence,
  severity="Blocker" (contain.py), and every one of its five call sites — one
  delimiter_forge and three delimiter_lookalike in contain.py, plus
  injection_attempt in detect.py — takes that default rather than overriding it,
  so every containment finding is a Blocker in practice and not only by contract.
  The severity ladder is {"Blocker": 0, "High": 1, "Medium": 2, "Low": 3} in
  review.py — at line 32 as of this writing, though see the citation note above;
  the first review pass proposed 19 and this plan originally said 20, and both
  were wrong. #117's own done-criteria
  require severity, file:line, a one-line defect statement and the concrete
  failure — three of those four have no field in the existing record. Reconciling
  the two is the central design act of this issue, not a detail.
  #120 has already assigned #117 a coverage gap in writing. CONTAINMENT.md
  § Detection records detect.py catching 21 of 35 attack matrix cases, missing 14
  (semantic paraphrase and finding-suppression) at 0 false positives, and says the
  14 "are #117's responsibility, not an accident" and that the dependency "is
  written down here because it is otherwise invisible from inside #117".
  detect.py's own docstring repeats it.
  Surface is the input shape. `fetch.Surface` is entry_point, state, text, reason
  with state in ok | empty | absent | oversized | unparseable and a `readable`
  property (fetch.py:31). Caps are 512 KiB per entry point and 2 MiB per
  invocation (fetch.py:21), and oversized input is refused, never truncated.
  `fetch.fetch_all` NEVER signals a hard failure, and this shapes STEP 3. Its
  helper `_gh` is documented "Never raises on a failed call" (fetch.py:40) and
  converts every gh failure — 404, missing auth, network outage, timeout,
  non-UTF-8 — into the same `Surface(entry_point, "absent", reason=...)`. A
  nonexistent PR is therefore indistinguishable from an unreachable one at the
  Surface level; only the free-text `reason` differs. `fetch_all` raises only if
  an entry point is missing from its own output (fetch.py:125).
  There is NO merge-base or head-SHA logic in the containment tree. `grep -ni
  'sha|merge_base|head'` across fetch.py and contain.py matches nothing but an
  unrelated hashlib call. `fetch_all` does fetch the full PR JSON but narrows it
  immediately through `lambda d: d["title"]` and `d.get("body")`, so the SHAs on
  that response are discarded, and `contain.render` returns no SHA either. Any
  step needing the commit pair must resolve it itself — see STEP 3.
  `fetch.from_payload(path)` (fetch.py:148) loads a captured PR from disk so a
  control need not touch live GitHub, and `fetch.degrade(surfaces, spec)`
  (fetch.py:163) forces a named surface into a degenerate state. These are the
  offline seams STEPs 7 and 8 depend on.
  A control-runner pattern already exists to extend rather than reinvent.
  run_controls.py holds a CONTROLS list of (script, needs_network) pairs, probes
  `gh api rate_limit` for connectivity, and reports SKIP with a reason — never
  PASS — for a control whose input is missing.
  #116's plan is committed and does not overlap this work.
  origin/feat/review-agent-preflight carries launchpad/plans/
  2026-08-12-issue-116-pr-review-preflight.md (354 lines) and nothing else —
  `git ls-tree -r origin/feat/review-agent-preflight -- launchpad/` lists only the
  pre-existing launchpad files plus that plan. So the pre-flight record #117 would
  ideally consume DOES NOT EXIST as code yet. Its STEP 2 enumerates the record's
  seven top-level keys (pr, closing_issue, diff, checks, required_gate,
  nearest_rules, skips) and its OPEN section leaves "whether the emitted record
  carries a schema version, so the later review dimensions in #109 can depend on
  its shape" undecided.
  launchpad/plans/ is the established path. launchpad/AGENTS.md §3 puts all cohort
  files under launchpad/ and bars root docs/ and root scripts/ as upstream's
  trees; #116's plan landed at launchpad/plans/ for that reason and #120's
  uncommitted plan sits beside it. The skill's default docs/plans/ is not used
  here, and docs/plans/ does not exist in this checkout.
  No verify gate is installed in this checkout — .claude/settings.json and
  .claude/settings.local.json are both absent — so every review skill is a manual
  invocation and none will fire on its own.
  #118's issue body still carries the wording #122 corrected. #109 has been
  amended in place (struck through, with the corrected claim following). #118's
  "Note on why this stage is treated sceptically" has not: it still reads "AUROC
  0.48–0.64 against 6,642 human-verified labels, and did so despite high
  performance on standard validation sets". Per #122's verification 1 of 3, the
  AUROC range is one judge (JailJudge), one victim model (Llama-3.1-8B), two
  attacks (GCG and GCG-R) — not a range across judges — and "despite high
  performance on standard validation sets" is not a quotation from the paper. What
  is confirmed verbatim is that judges perform "on average only slightly better
  than a random coin-flip" against the 6,642 labels. Amending #118 is not this
  issue's work; it is recorded in OPEN so it is not lost.

STEP 1  launchpad/review-agent/FINDINGS.md — the output contract,        [independent]
        normative. The artefact #118 is blocked on. #117's issue says the contract
        is "agreed in whichever of the two lands first, then honoured by the
        other", and #117 is planning first, so it is settled here in enough detail
        that #118 implements against it without renegotiating. Written as a
        sibling to CONTAINMENT.md and in the same normative voice.
        The severity ladder is IMPORTED, not redefined: Blocker | High | Medium |
        Low, from review.py's SEVERITY_ORDER. A second copy of a four-value ladder
        drifts, and the containment findings that share the output stream are
        already fixed to Blocker by CONTAINMENT.md § Severity contract.
        How a finding carries file:line — the part most likely to be got wrong.
        Three anchor kinds, one required field naming which applies, and validation
        that refuses the mismatched combinations:
          anchor "line" — file required, line required, renders launchpad/AGENTS.md:42
          anchor "file" — file required, line MUST BE NULL, renders launchpad/AGENTS.md
          anchor "pr"   — file MUST BE NULL, line MUST BE NULL, renders (pull request)
        The rule exists because the alternative is a reviewer inventing line 1 for
        a finding that is really about a whole file or the PR as a whole. That is
        false precision, and #118 would then try to verify a defect at a location
        where it is not — producing a REFUTED verdict for a finding that was true.
        An anchor field makes "this defect has no line" expressible instead of
        forcing a lie.
        `line` is a NEW-SIDE line number in the merge-base diff, i.e. of the file
        at head_sha. Old-side and new-side numbers differ in every diff that adds
        or removes lines above the finding, so the side is stated, not assumed.
        The finding record. Field names are final here:
          dimension    which reviewer produced it — one of STEP 4's three slugs
          severity     Blocker | High | Medium | Low
          anchor       line | file | pr
          file         repo-relative path, or null when anchor is pr
          line         new-side line number, or null unless anchor is line
          defect       ONE line: what is wrong
          failure      the concrete failure the defect allows
          finding_id   stable id, see below
          entry_point  optional — one of CONTAINMENT.md's seven labels, set only
                       on a finding about author-supplied text, so the containment
                       class survives in the same stream without a second record
                       type
          evidence     optional — the excerpt a finding rests on, rendered
                       post-escape. Required on any finding carrying an
                       entry_point, because a containment finding whose excerpt is
                       dropped is the detected-then-dropped case CONTAINMENT.md
                       calls worse than never detecting it. Optional elsewhere: a
                       dimension's finding is already located by file and line,
                       and quoting the diff back adds bulk without adding evidence.
        Ten fields, and the count is load-bearing — STEP 9 builds one control per
        field, so a field that exists in the conversion but not in this list gets
        no control at all. `evidence` was exactly that: the first revision mapped
        it in the conversion below while declaring only nine fields, which would
        have either dropped the escaped excerpt or grown an undeclared tenth field
        that no control exercised.
        How a `contain.Finding` becomes a #117 Finding, stated here because
        defining `entry_point` without defining the conversion is what leaves the
        deterministic catches stranded. `contain.render` returns its own findings
        as its SECOND return value, and they carry kind, entry_point, evidence and
        severity but NO file and NO line — there is nowhere in that record for
        one. So the mapping is fixed:
          anchor      "pr" — never "line" or "file", because a contain.Finding
                      has no location and inventing one is the false precision the
                      anchor rule exists to prevent
          severity    carried through unchanged, which CONTAINMENT.md § Severity
                      contract fixes at Blocker for all three kinds
          dimension   "containment" — a reserved slug, not one of STEP 4's three,
                      so #118 can tell a deterministic catch from a model's
                      judgement and #121 can score them separately
          defect      derived from `kind` (delimiter_forge, delimiter_lookalike,
                      injection_attempt)
          failure     the fixed consequence for that kind
          evidence    rendered post-escape, per review.py's existing rule that
                      quoting attacker text verbatim moves the payload into a
                      fresh position with fresh authority
        "containment" is reserved in this contract so no dimension may claim it.
        `defect` and `failure` are two fields, not one, because #117's
        done-criteria name them separately and because a defect statement with no
        stated consequence is what lets an unfalsifiable finding through. #118
        re-rates severity, so the reporting dimension's value must remain readable
        after adjudication rather than being overwritten in place.
        `finding_id` is a truncated hash of (dimension, anchor, file, line,
        defect). It is stable across re-runs of an unchanged diff, which is what
        lets #118 attach exactly one verdict per finding and makes its dedupe
        visible rather than silent. It is NOT stable across a model rewording
        `defect` — stated plainly here because a reader will otherwise assume it
        is, and because dedupe across rewordings is #118's job, not this id's.
        The report envelope, one per dimension:
          schema_version    integer, starts at 1
          dimension         the slug
          pr                number
          merge_base_sha    the commit pair this report read
          head_sha
          status            complete | failed
          outcome           findings | clean  — only when status is complete
          error             {reason}          — only when status is failed
          findings          array
          findings_count    integer
          completion_marker LAST key, see below
        `outcome: clean` is how "a dimension that finds nothing says so
        explicitly" is distinguished from `status: failed`. Both are legitimate
        outputs; neither is an empty findings array standing alone, which is
        exactly the ambiguity the criterion forbids.
        The completion marker carries the run nonce. Its value is
        BUZZ-DIMENSION-COMPLETE:{dimension}:{nonce}, using the same nonce
        contain.render wrapped the surfaces with, and it is the LAST key emitted.
        Two reasons, and the second is not obvious: a marker at the end cannot
        survive truncation, so a report cut off mid-findings has no marker and is
        treated as truncated rather than clean; and a marker with NO nonce is a
        fixed string published in a public repository, which a PR author can type
        into their own diff. A reviewer reading a contained diff containing a
        forged marker could emit it, and a naive scan of model output would then
        read a truncated report as complete. The nonce makes the marker
        unforgeable by anyone who has not seen it, on the same reasoning
        CONTAINMENT.md gives for the envelope delimiter. `contain.make_nonce` is
        reused, including its refusal to accept a caller-supplied nonce.
        `findings_count` must equal len(findings). A report truncated mid-array
        fails that equality even if it somehow parses.
        done when: FINDINGS.md exists under launchpad/review-agent/; it states all
        ten finding fields and all eleven envelope fields by the names above, and
        states that `evidence` is required whenever `entry_point` is set; it
        states the contain.Finding conversion including the reserved "containment"
        slug and the anchor "pr" rule; it contains the three anchor rules with
        both "must be null" constraints; it
        names Blocker | High | Medium | Low and cites review.py as their source
        rather than restating the ladder as its own; and it is referenced from a
        new row in CONTAINMENT.md's "Contract for later stages" table so the two
        documents point at each other rather than diverging quietly.

STEP 2  launchpad/review-agent/findings.py — the contract in code.          [needs 1]
        Pure functions and dataclasses over already-fetched data. No subprocess,
        no network, no model call in this module. `Finding` and `Report`
        dataclasses, a `validate(report) -> list[str]` returning EVERY violation
        rather than raising on the first, and `parse(text) -> Report` for reading
        what a reviewer emitted.
        `validate` enforces, at minimum: the anchor/file/line combinations from
        STEP 1; severity in the imported ladder; findings_count == len(findings);
        the completion marker present, last, and matching the expected dimension
        AND nonce; status/outcome/error mutual exclusivity; non-empty `defect` and
        `failure`; `evidence` present whenever `entry_point` is set; and — the rule
        the first revision was missing — `outcome` CROSS-CHECKED AGAINST THE
        FINDINGS ARRAY: "clean" requires findings to be empty, "findings" requires
        it non-empty. Without that rule a report reading status "complete", outcome
        "clean", findings [one real Blocker], findings_count 1 satisfies every
        other rule at once and validates cleanly. That is the exact inverse of the
        ambiguity `outcome` was added to remove, and it fails in the dangerous
        direction: a real finding published under the word "clean".
        A validator that stops at the first error hides the rest, and STEP 9's
        suite asserts on the full list.
        SEVERITY_ORDER is imported from review, not re-declared.
        done when: `python3 -m py_compile launchpad/review-agent/findings.py`
        succeeds; `validate` returns an empty list for a well-formed report and,
        for a report carrying three independent violations at once, returns three
        strings rather than one; a finding with anchor "pr" and a non-null file is
        rejected; a finding with anchor "line" and a null line is rejected; and
        `findings.SEVERITY_ORDER is review.SEVERITY_ORDER` is true.

STEP 3  launchpad/review-agent/run_dimensions.py — the concurrent  [needs 2]  <- RUNS HERE
        runner. The CLI shell, demonstrable before a single prompt is written.
        Takes a PR number, resolves the commit pair, calls `fetch.fetch_all(pr,
        repo)` then `contain.render(surfaces, nonce)`, runs the dimensions
        concurrently, and prints one merged JSON document containing every
        dimension's report on stdout.
        Three things this module must do that the containment tree does NOT do for
        it. Each was found by review of this plan, not by building it:
          THE COMMIT PAIR IS THIS MODULE'S JOB. Nothing in fetch.py or contain.py
          resolves a merge base or retains a head SHA — fetch_all narrows the PR
          JSON to title and body and discards the rest. So this module makes one
          further REST call, `GET /repos/{o}/{r}/compare/{base}...{head}`, and
          reads `.merge_base_commit.sha`, plus head_sha from the PR JSON. #116's
          plan establishes why it must be REST: there is no `mergeBaseCommit`
          field on the GraphQL PullRequest type, verified there by introspection.
          The trap it also names applies here unchanged — `baseRefOid` is the
          CURRENT TIP of the base branch, not the commit the head forked from, and
          diffing against it attributes every commit landed on launchpad since the
          fork to this PR's author.
          A NONEXISTENT PR MUST BE TOLD APART FROM AN UNREACHABLE ONE. `_gh`
          never raises and collapses 404, missing auth, network outage and timeout
          into the same `absent` Surface, so `all_readable == False` cannot carry
          the distinction and treating it as "PR not found" would report an
          outage as a verdict about the PR. This module therefore probes existence
          once, explicitly, before fetching surfaces, and classifies into THREE
          states, not two — because gh returns exit code 1 for all of them and
          only its stderr differs:
            no such PR          the PR endpoint 404s AND `GET /repos/{o}/{r}`
                                succeeds, so the credential can see the repo and
                                the PR genuinely is not in it
            cannot see it       BOTH 404, or the repo call 401s. GitHub returns
                                404, not 403, for a resource the caller lacks read
                                access to — deliberately, so as not to confirm
                                existence to an unauthorised caller. #110's token
                                is scoped to launchpad-26/buzz and its scope can be
                                narrowed or revoked independently of any PR, so a
                                token that loses read access presents EXACTLY as
                                "PR 42 does not exist". Without the repo probe this
                                module would report a credential failure as a fact
                                about someone's pull request.
            infrastructure      an auth failure, a network outage or a timeout on
                                either call
          All three exit non-zero with distinct reason strings, never one shared
          "could not review". The repo probe narrows the ambiguity; it does not
          abolish it — a token that can read the repo but is blocked from one PR
          still reads as "no such PR", and that residual case is named in OPEN
          rather than papered over.
          CONTAINMENT.MD'S OWN FINDINGS MUST BE CARRIED. `contain.render` returns
          (document, findings, all_readable) and the SECOND element is the
          deterministic 21-of-35 catches. This module converts every one into a
          #117 Finding under the reserved "containment" slug per STEP 1, and emits
          them alongside the dimensions' reports. Dropping them would breach
          CONTAINMENT.md § Severity contract, which requires all three kinds to
          appear in the published review and calls a detected-then-dropped finding
          "worse than one never detected, because it reads as a clean review" —
          and the dimensions cannot cover the gap, because LEFT OUT scopes them to
          the 14 that detection misses.
        The reviewer is an INJECTED CALLABLE, defaulting to a stub that returns a
        well-formed clean report. Two payoffs: the harness is provable end to end
        today, and every control in STEP 9 feeds recorded outputs with no model
        call and no network. This is also what keeps "choosing the model" out of
        scope, as #117 requires — the runner never names one.
        A `--payload <path>` mode reads surfaces via `fetch.from_payload` instead
        of the network, and `--degrade <ep>=<state>` forces a degenerate surface
        via `fetch.degrade`. Both already exist in fetch.py. Without the first,
        STEP 7's "valid input to run_dimensions.py" and STEP 8's no-network control
        are both unsatisfiable, since fetch_all always shells to gh.
        A `--list` mode prints the dimension slugs, discovered by reading
        STEP 4's directory rather than from a hardcoded list. It is named here
        because STEP 4's done-when depends on it, and a deliverable that only
        appears in another step's acceptance criterion is one nobody builds.
        The `document` from contain.render is passed through as the reviewer's
        input verbatim. This module never concatenates surfaces itself, never
        places anything after the closing marker, and never re-calls
        `detect.detect` — contain.render already merges those findings and calling
        it again double-reports.
        done when: `python3 launchpad/review-agent/run_dimensions.py 86 --stub`
        exits 0 and prints JSON containing three reports, each with status
        "complete", outcome "clean", and a completion marker that
        `findings.validate` accepts; every report's merge_base_sha and head_sha
        are non-empty and the merge_base_sha equals `gh api
        repos/launchpad-26/buzz/compare/{base}...{head} -q
        .merge_base_commit.sha`; `--payload` on a captured fixture produces the
        same three reports with the injected runner asserting `gh` was never
        spawned; a 404 PR number exits non-zero with a reason naming the PR as
        absent; a simulated auth failure exits non-zero with a DIFFERENT reason
        naming infrastructure, so the two are not one message; and a payload
        carrying a delimiter_forge yields a "containment" finding with anchor "pr"
        in the output rather than only inside contain.render's discarded return.

STEP 4  The three dimension definitions, each naming what it must NOT    [needs 1, 3]
        review. Tagged against 3 as well as 1 because its done-when runs
        `run_dimensions.py --list`: the three files can be WRITTEN as soon as
        STEP 1 lands the contract, but the step cannot CLOSE until the runner
        exists. Both facts matter — see PARALLEL, which still fans the writing out.
        One file per dimension under launchpad/review-agent/dimensions/.
        Each states its scope, its exclusions, the severity guidance for its own
        finding classes, and the output contract it emits. Slugs are final because
        finding_id hashes them:
          secrets-and-access — credentials, tokens, keys and passwords in tracked
            files; permission and scope widening in workflows and configs;
            anything granting an agent or job more access than the change needs.
            Grounded in #109's own evidence: a review of a deployment PR found a
            plaintext shared console password in a tracked file, violating that
            folder's own hard rule, while fifteen CI checks were green. Must NOT
            review correctness, style, or whether a claim is supported.
          claim-vs-evidence — assertions in the PR body, commit messages and
            documentation that the diff does not support: a done-criterion marked
            complete with nothing in the diff doing it, a cited file path or issue
            number that does not exist, a quoted figure attributed to a source
            that does not say it, a test named as proof that cannot fail. This is
            the dimension #109's "evidence layer already exists" points at, and
            #122's own corrections are a worked example of the defect class. Must
            NOT review code behaviour, secrets, or anything the PR does not claim.
          correctness-and-failure-modes — what the changed scripts, workflows and
            configs do at their edges: fail-open defaults, an absence rendered as
            a value, a guard narrower than the thing it guards, an error path that
            reports success. Scoped to what this fork actually writes — Python,
            YAML, markdown and shell — not Rust crates or React. Must NOT review
            secrets, unsupported claims, or style.
        The exclusions are load-bearing rather than tidy: the issue's first
        done-criterion is that "a reviewer that reviews everything reviews nothing
        well", and without exclusions all three dimensions converge on the same
        generic review and #118's dedupe absorbs the cost.
        done when: three files exist, one per slug; each names both what it
        reviews and what it must not, with the exclusions naming the OTHER TWO
        dimensions' subjects explicitly; each states the finding fields from
        STEP 1 rather than inventing its own; and `run_dimensions.py --list`
        prints exactly the three slugs, read from the directory rather than
        hardcoded in two places.

STEP 5  The cross-cutting injection clause, in all three definitions.    [needs 4, 8]
        Tagged against 8, not only 4, because its done-when below observes actual
        reviewer output on the two injection fixtures — which STEP 7 creates and
        STEP 8 records. Tagged [needs 4] alone, this step's verification could only
        have been met by inspecting the clause text, which proves nothing about
        detection. Writing the clause needs only STEP 4; proving it needs STEP 8,
        and the weaker tag is what would have let the proof quietly become an
        eyeball check.
        One identical clause in every dimension: author-controlled text attempting
        to act on the review — instructing it to skip, approve, suppress a
        finding, or treat the review as ended — is itself a finding at Blocker,
        per CONTAINMENT.md § Severity contract, with entry_point set to the
        surface it came from.
        This is #117 discharging a debt #120 recorded, not new scope.
        CONTAINMENT.md states detect.py misses 14 of 35 matrix cases — semantic
        paraphrase and finding-suppression — and that those are #117's
        responsibility. The clause goes in all three definitions rather than one,
        because the issue requires that one dimension failing does not prevent the
        others reporting; if a single dimension owned semantic injection, that
        dimension failing would drop the coverage to zero silently.
        The clause must not be phrased so that a dimension reports the
        DESCRIPTION of an attack. CONTAINMENT.md and this plan both contain
        sentences that look like attacks at the token level — the use-mention
        problem detect.py's docstring documents, and the reason its rule set
        stayed narrow after a broader one produced ten false positives on this
        repository's own issues.
        done when: the clause is byte-identical across all three definition files,
        asserted by a control rather than by eye; a fixture whose diff contains a
        paraphrased suppression instruction (one of the 14 classes, not one of the
        21 detect.py already catches) yields a Blocker finding with the right
        entry_point from each of the three dimensions independently; and a fixture
        containing a DESCRIPTION of such an attack, taken verbatim from
        CONTAINMENT.md itself, yields no injection finding from any of them.

STEP 6  Concurrency and isolation — one dimension failing still reports     [needs 3]
        the others. `concurrent.futures` over the three dimensions, with a
        per-dimension timeout and exception containment: a dimension that raises,
        times out, or returns unparseable output produces status "failed" with a
        reason and does not prevent the other two emitting their reports, nor
        change the process's ability to emit them.
        The exit code is a separate decision from the reports, and this plan fixes
        it: ANY dimension failing exits non-zero while still printing all three
        reports. A run where a dimension crashed is not a clean run, and an exit 0
        is what a CI step reads as "reviewed".
        done when: with a reviewer injected to raise for exactly one dimension,
        stdout carries three reports of which two are complete and one is failed
        with a reason, and the process exits non-zero; with one injected to hang
        past the timeout, the same holds and the run terminates within the timeout
        rather than blocking; and a control asserts the three reviewers were
        actually invoked concurrently rather than serially, by timing a run of
        three deliberately-slow stubs against the single-dimension duration.

STEP 7  Fixture diffs with planted defects — one per dimension.          [needs 1, 3]
        Tagged against 3 for the same reason as STEP 4: its done-when requires each
        fixture be a valid input to run_dimensions.py, which does not exist until
        STEP 3. The fixture FILES can be authored against STEP 1's contract alone.
        Three fixture diffs under launchpad/review-agent/fixtures/dimensions/,
        each carrying one defect its dimension must find and the other two must
        not report, so the fixtures test STEP 4's exclusions and not only
        detection:
          secrets-and-access — a credential in a tracked file. Real precedent
            exists (#109's deployment-PR finding), but the fixture uses an
            OBVIOUSLY FAKE value, never a real or historical credential, and never
            a value copied from any real file. A plausible-looking secret
            committed to a public fork as test data is the defect this dimension
            exists to find.
          claim-vs-evidence — a PR body asserting a done-criterion the diff does
            not satisfy, plus a cited path that does not exist.
          correctness-and-failure-modes — a fail-open default: a check whose
            unreadable input yields a pass. Modelled on the real shape
            run_controls.py guards against, where a missing input must report SKIP
            and never PASS.
        Plus the two injection fixtures STEP 5 requires: one paraphrased attack
        from the 14 missed classes, one description-of-an-attack quoted from
        CONTAINMENT.md.
        done when: five fixtures exist; each parses as a diff or as a Surface
        payload `fetch.from_payload` accepts; each of the three defect fixtures is
        a valid input to run_dimensions.py; and each names, in a header comment,
        the dimension that must find it and the two that must not.

STEP 8  Recorded reviewer outputs for the deterministic suite.        [needs 3, 4, 7]
        Tagged against 3 and 4, not only 7: a recording is the output of a real
        model run, which needs STEP 4's prompt to run and STEP 3's runner to invoke
        it. PARALLEL said as much in prose — "depends on STEP 7's fixtures and on a
        working runner" — while the tag named only the fixtures.
        For each fixture, a recorded reviewer output per dimension, stored as JSON
        and replayed by the injected reviewer. Recorded from a real run against a
        real model and NEVER hand-written — a hand-written "model output" tests
        only that the author can write valid JSON, which STEP 2's validator
        already covers.
        The model used and the date are recorded in each file. That is provenance,
        not a model choice: #117 puts choosing the model out of scope, and the
        runner still never names one.
        done when: fifteen recordings exist (five fixtures x three dimensions);
        each is valid against `findings.validate`; each carries the model id and
        date it was recorded; a control asserts the replay path makes no network
        call, by injecting a runner that raises if the real `gh` binary or any HTTP
        client is invoked; and the recordings show each defect fixture found by
        its own dimension and not reported by the other two.

STEP 9  The control suite — one control per done-criterion.           [needs 5, 6, 8]
        Tagged against 5 because two of its controls below read the injection
        clause STEP 5 writes — one of them names STEP 5 in its own text. The
        earlier [needs 6, 8] tag reached 1, 2, 3, 4 and 7 transitively but never 5.
        launchpad/review-agent/check_dimensions.py, registered in
        run_controls.py's CONTROLS list as (check_dimensions.py, False) so #120's
        single CI entry point picks it up and no second workflow is added. One
        control per #117 done-criterion, plus:
          a truncated report — completion marker removed — read as truncated and
            never as clean, asserted for each of the three dimensions
          a marker present but carrying the WRONG NONCE, rejected
          outcome "clean" distinguished from status "failed" in both directions:
            neither is ever read as the other
          findings_count disagreeing with len(findings), rejected
          for each of the TEN finding fields, a control feeding it empty, missing
            and malformed in turn, asserting `validate` names it — ten is a list
            someone can count, "every field" is not. The count is checked against
            STEP 1's list rather than trusted: the first revision declared nine
            while the conversion mapped a tenth, and a per-field suite silently
            skips a field that is not in the list it iterates
          outcome cross-checked against the findings array in BOTH directions:
            outcome "clean" carrying a finding is rejected, and outcome "findings"
            with an empty array is rejected. The first of the two is the dangerous
            one — a real Blocker published under the word "clean" — and it passed
            every other rule in the first revision
          the byte-identical injection clause across the three definitions
          the injection clause's BEHAVIOUR, not only its text: replaying STEP 8's
            recordings for the paraphrased-attack fixture yields a Blocker finding
            with the right entry_point from each of the three dimensions, and the
            description-of-an-attack fixture yields none from any of them. STEP 5's
            done-when proves this once at build time; without a control here it is
            never re-checked, and byte-identity of the clause text is not evidence
            the clause works
          a contain.Finding conversion control: a payload carrying each of
            delimiter_forge, delimiter_lookalike and injection_attempt produces a
            #117 Finding with dimension "containment", anchor "pr", severity
            Blocker, and evidence rendered post-escape — asserted for all three
            kinds, since a conversion that handles one and drops two reads as
            working
          no dimension may claim the reserved "containment" slug
        done when: `python3 launchpad/review-agent/check_dimensions.py` passes; it
        is listed in run_controls.py; `python3
        launchpad/review-agent/run_controls.py` runs it and reports it in the
        summary; the suite makes no network call, asserted by the injected runner
        above; and each bullet above has a separately named control.

STEP 10 Live prompt-mutation evidence — one run per dimension, pasted.      [needs 9]
        The falsifiability criterion, answered the way it was decided: for each
        dimension, delete the clause of its definition that targets its planted
        defect, run it live against its fixture, and record that the defect is no
        longer found — then restore and record that it is. Raw before-and-after
        output pasted into the PR body.
        This is the half that is NOT deterministic, and the PR body must say so.
        It is a one-off measurement with a named model on a named date, not a
        gate. A single run of a non-deterministic reviewer is one sample; a
        dimension that finds its defect once has not been shown to find it
        reliably. Stating the sample size is the difference between evidence and a
        claim.
        done when: three before/after pairs are pasted raw into the PR body; each
        names the model, the date, and the exact clause removed; each shows the
        planted defect found with the clause and absent without it; and the PR
        body states in its own words that this is a single non-deterministic
        sample per dimension and that the deterministic gate is STEP 9.

STEP 11 Prove each control can fail. For every check function in STEP 9,    [needs 9]
        neuter it to a constant and confirm the suite goes red. A control never
        observed failing has not been shown to test anything, and this is the
        criterion most likely to be quietly downgraded to "the suite passes".
        done when: a recorded run shows, for each check function, the suite
        failing while that function is neutered and passing when restored, with
        the raw output pasted into the PR body.

STEP 12 Open the PR against launchpad — AGENT_PR_TEMPLATE.md filled,   [needs 10, 11]
        `Closes #117`, `by:agent`, all raw output from STEPs 10 and 11 pasted, and
        Escalations naming every deferral in OPEN below — above all the dependency
        on #120's uncommitted tree and the fact that the findings contract is
        settled unilaterally here for #118 to honour.
        done when: the PR is open against launchpad with the by:agent label; the
        "launchpad — PR body check" check is green; every commit carries a
        Signed-off-by trailer so the DCO check passes; and the body names both the
        #120 ordering dependency and the #118 contract hand-off explicitly.

PARALLEL  Two genuine fan-outs, and one that looks available but is not.
  STEP 4's three definitions are three disjoint files and can be WRITTEN as three
  parallel subagents once STEP 1 has landed the contract. That is the one place
  the work is naturally three-way, and it is the reason STEP 4's subagents must
  not each invent their own phrasing of the injection clause — STEP 5 later
  requires all three to be byte-identical.
  Writing is not closing. STEP 4 is tagged [needs 1, 3] because its done-when runs
  `run_dimensions.py --list`, so the fan-out produces three files that sit
  unverified until STEP 3 exists. Dispatch them early if it suits the schedule, but
  do not mark the step done on the strength of three files having been written —
  that is the failure this plan already made once, in STEP 5.
  STEP 7's five fixtures are disjoint files and can be authored in parallel with
  STEP 4, since fixtures and definitions do not touch each other. Same caveat: it
  is tagged [needs 1, 3] and closes only once the runner can accept them.
  STEP 5 does not sit where its number suggests. It edits all three definition
  files, so it cannot join STEP 4's fan-out, and it is tagged [needs 4, 8] because
  its verification observes real reviewer output. In execution order it lands after
  STEP 8, alongside STEP 6 rather than before it. The numbering is left as-is
  because renumbering would break every cross-reference in this plan; the tags,
  not the numbers, are the dependency graph.
  STEPs 2, 3 and 6 cannot be parallelised, though 6 looks independent of 2.
  STEP 6 edits run_dimensions.py, which STEP 3 creates; two steps editing one file
  are sequential regardless of how unrelated they look. STEP 8 depends on STEP 7's
  fixtures and on a working runner, so it cannot start early either.
  STEPs 9 through 12 are strictly sequential — each reads the whole surface the
  previous produced. STEP 1 is the only step independent as written. Nothing is
  dispatched here; the decision to fan out belongs to whoever executes this.

GATES  No verify gate is installed in this checkout — .claude/settings.json is
  absent — so every gate is a manual invocation and none will fire on its own.
  serina:review-plan has run ONCE on this file, before STEP 1, and this revision
  is the result. Five findings, all applied, none disputed — one Blocker: STEP 3's
  done-when required a merge-base SHA, a head SHA and a distinguishable 404, and
  none of the three is obtainable from the two calls it named, because fetch.py
  discards the SHAs and `_gh` collapses every failure into one `absent` state.
  Two High: contain.render's second return value — the deterministic 21-of-35
  catches — was computed and then consumed by nobody, despite STEP 1 defining
  `entry_point` expressly to carry it; and STEP 5's behavioural done-when was
  tagged [needs 4] when it needs STEP 8's recordings, with STEP 9 checking only
  the clause's byte-identity, so the one mechanism CONTAINMENT.md hands #117 by
  name had no durable control. Two Low: an envelope field miscount, and a wrong
  line citation for SEVERITY_ORDER.
  On that last one the reviewer was also wrong — it proposed review.py:19 against
  this plan's review.py:20, and `grep -n` puts it at review.py:32. The plan now
  says 32. Recorded because a correction accepted without checking is how a wrong
  number launders itself into looking verified.
  serina:review-plan has now run TWICE, and this revision is the result of both.
  The second pass returned SEVEN findings and, as on #116, nearly all of them were
  against the first pass's own fixes rather than the original plan — which is the
  argument for the second pass, not a footnote to it.
  Second pass, one Blocker: STEP 1's containment conversion mapped an `evidence`
  value into a record that declared only nine fields and had no such field, so the
  escaped excerpt would either be dropped — the detected-then-dropped case
  CONTAINMENT.md calls worse than never detecting — or become an undeclared tenth
  field that STEP 9's per-field suite would never exercise. `evidence` is now
  declared, required whenever `entry_point` is set, and the count is ten.
  Four High. FOUR MORE STEPS carried the exact defect the first pass fixed in
  STEP 5 — a done-when observing something its own tags cannot produce: STEP 4 and
  STEP 7 both call `run_dimensions.py` while tagged [needs 1], STEP 8 needed the
  runner and the prompts while tagged [needs 7], and STEP 9 named STEP 5 in its own
  control text while tagged [needs 6, 8]. All four retagged. `--list` was also
  promised only inside STEP 4's acceptance criterion and is now a named STEP 3
  deliverable. Separately, `findings.validate` never cross-checked `outcome`
  against the findings array, so a report reading outcome "clean" while carrying a
  real Blocker passed every other rule — a fail-open in the dangerous direction,
  now a validation rule and a two-directional control. And STEP 3's 404 handling
  still conflated a missing PR with one the credential cannot see, because GitHub
  returns 404 rather than 403 for no-access; it now probes the repo and classifies
  three states, with the residual case named in OPEN.
  One Medium and one Low, both accepted: the pr_diff anchor loss is now recorded in
  OPEN rather than left implicit, and the drifting line citations are replaced by
  symbol names — contain.py had moved under this plan's feet between the two
  passes, which is itself the clearest evidence for BUDGET's warning.
  This revision is once-reviewed at the margin: the second pass's own fixes have
  not been reviewed. A third pass is available and was not run.
  Then serina:review-code and serina:review-tests after STEP 11, then
  serina:review-adjudicate, then serina:review-final — all BEFORE the push in
  STEP 12, because a review posted after the push only documents what already
  shipped.
  A clean check-plan.sh run is mechanical only. It verifies the single run-marker,
  the step count, a done-when per step, a dependency tag per step, and the six
  named sections. It judges nothing about whether the steps are right, and it
  cannot tell you a step is already done.

BUDGET  STEP 8 eats the budget, and STEP 3 decides how badly — but the thing most
  likely to derail this issue is neither.
  STEP 3 grew after review and is now the second-largest step, not the thin shell
  it started as. It owns the commit-pair resolution, the 404-versus-outage
  distinction, the contain.Finding conversion and two offline modes, on top of the
  concurrent dispatch. Each was added because a done-when elsewhere depended on it
  and nothing provided it. If STEP 3 is built to its original one-line description
  — two calls and a print — STEPs 6, 7, 8 and 9 all fail their own criteria and the
  cost lands there instead, which is the more expensive place to pay it.
  #120's tree is committed nowhere. Every module #117 imports — contain.render,
  fetch.fetch_all, fetch.Surface, review.SEVERITY_ORDER — exists only in another
  worktree's untracked working directory, on no branch and in no commit. If that
  work is rebased, renamed, abandoned, or lands with a different signature, STEPs
  2, 3, 6 and 8 are all rewriting against a moved target, and nothing in this plan
  detects it early. Before STEP 2, re-verify contain.render's signature and
  Surface's fields against whatever #120 has actually committed by then, rather
  than trusting the line numbers quoted in ALREADY TRUE. The cheapest mitigation
  is ordering: let #120 land first. That is a sequencing decision for whoever runs
  the fleet, not for this plan.
  STEP 8 is fifteen recordings, each needing a real run against a real model, and
  it is the step where "recorded from a real run" quietly becomes "hand-written to
  look like one". A hand-written recording tests nothing STEP 2's validator does
  not already cover, and the failure is invisible from inside the suite, which
  passes either way.
  STEP 3 decides STEP 8's cost. If the reviewer is not an injected callable — if a
  model call is hardcoded at its call site — then STEP 8 stops being recording and
  becomes a rewrite of STEPs 3 and 6.

OPEN  Not for a builder to decide.
  The findings contract is settled here unilaterally. #117's issue says it is
  "agreed in whichever of the two lands first, then honoured by the other", and
  this plan takes that mandate. #118 may still object, and if it does the field
  names in STEP 1 change and STEPs 2, 8 and 9 change with them. Whether the
  contract should instead be agreed in a comment on #118 before STEP 1 is built is
  a sequencing call.
  Whether the pre-flight record is an input to the dimensions at all. #116's plan
  has the record enumerated but NOT BUILT — its branch carries only the plan file.
  This plan therefore has the runner call fetch and contain directly rather than
  consuming #116's record, which means the merge-base SHA pair is resolved twice
  by two stages. Whether #117 should instead wait on #116 and consume its record
  is a real design choice with a real cost either way, and #116's own OPEN section
  leaves the record's schema version undecided — a version the dimensions would
  need in order to depend on its shape.
  #118's issue body still carries the wording #122 corrected, as recorded in
  ALREADY TRUE. #109 was amended; #118 was not. Amending it is a one-comment fix
  and belongs to whoever owns #118, not to this issue.
  Three dimensions and their slugs were answered, not derived. A reviewer may
  judge that this fork's diffs want a different split — for instance that
  claim-vs-evidence and correctness-and-failure-modes overlap on a test that
  cannot fail, which is both an unsupported claim and a fail-open control. The
  slugs are hashed into finding_id, so changing one after STEP 8 invalidates the
  recordings.
  Where the dimensions run is #110's decision and is already made — GitHub
  Actions, with a token scoped to launchpad-26/buzz. No new workflow file is added
  by this work: #120 already added
  .github/workflows/launchpad-review-agent-controls.yml and the dimension controls
  register into its runner. Whether that file should be split per issue is for
  whoever reviews #120.
  The per-dimension timeout in STEP 6 has no principled value yet. No dimension
  has been run against a real diff, so any number is a guess. It is written as a
  flag with a stated default and a note that the default is unmeasured.
  Containment findings from `pr_diff` lose a location that is arguably derivable.
  STEP 1 maps every contain.Finding to anchor "pr" because the record has no file
  or line field. For six of the seven entry points that is simply the truth. For
  `pr_diff` it is a real loss rather than an honest absence: the surface text is a
  unified diff carrying `+++ b/path` headers and hunk ranges, so an injection
  attempt embedded in the code itself does have a location — it is just not
  extracted. Deriving it would mean mapping an excerpt offset back through
  contain.py's escaping to a hunk, which is more than #117 was asked for, and
  guessing it would be the false precision the anchor rule exists to prevent. So
  it is anchor "pr" for now, recorded here as a named loss rather than left to
  look like a considered absence. Whoever wants line-anchored injection findings
  in the diff should file it.
  A PR the credential can see the repo for but not the PR still reads as absent.
  STEP 3's repo probe separates "no such PR" from "cannot see it" for the common
  cases, and GitHub's 404-for-no-access behaviour is why the probe is needed at
  all. It does not cover a token with repo read access that is nonetheless blocked
  from one pull request. Whether that case is worth a further probe is a judgement
  about how the Actions credential in #110 can actually fail, which needs #119's
  provisioning to be settled first.

LEFT OUT  Deliberately excluded.
  Deciding whether a finding is real — confirm/refute, re-rated severity, dedupe,
  the total-refutation flag. #117 puts it out of scope and #118 owns it. These
  dimensions emit candidates and nothing more.
  Publishing to the PR. `review.render_review` exists and #119 owns publication.
  This runner prints JSON to stdout and posts nothing.
  Choosing the model. Out of scope per #117. The runner takes an injected reviewer
  and never names a model; STEP 8's recordings carry a model id as provenance for
  a measurement already taken, which is not the same as choosing one.
  Any approval weight. Phase 1 is escalate-only. #109's reasoning for that
  survives #122's corrections: a judge validated on static held-out material can
  still be near-chance on adversarial material, so a pilot on ordinary PRs would
  not reveal the failure. The confirmed figure is that judges perform "on average
  only slightly better than a random coin-flip" against 6,642 human-verified
  labels; the AUROC 0.48-0.64 range is one judge on one victim model under two
  attacks and is not quoted here as anything broader.
  The 21 of 35 attack classes detect.py already catches. Duplicating deterministic
  detection in a model prompt costs tokens and adds a second source of truth. #117
  takes the 14 it was handed.
  Measuring the dimensions' precision and recall. #121 owns the first ten reviews
  and #109's success signals. STEP 10 produces one sample per dimension, which is
  provenance for the falsifiability criterion and explicitly not a rate.
  Accessibility is out of scope for this issue and is not claimed. The deliverable
  is definitions plus a CLI printing JSON to stdout — no UI, no interactive
  control, nothing to announce, no focus to manage. When #119 renders findings
  into a PR comment, that surface is markdown read by GitHub's own interface; if a
  rendered dashboard ever follows, it needs its own keyboard and announcement
  specification and does not inherit one from here.
