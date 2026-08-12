Issue #120 — task: treat all pull request content as untrusted data
Stated size: none on the issue → agreed with Serina 2026-08-12  →  cap: 12 steps (plan uses 11)

Scope shape agreed 2026-08-12: **containment first, as a contract.** The four stages
this issue protects (#116 pre-flight, #117 dimensions, #118 adjudication, #119 publish)
do not exist, so the controls here test the containment mechanism, and a written
contract records what each later stage must call. This matches #120's own argument —
containment shapes how every stage receives input, so it cannot be retrofitted.

Revision 4, 2026-08-12, and the last before implementation. Three independent review
rounds ran, each by a different reviewer: round 1 found ten findings, round 2 found ten
more including two blockers *in material round 1 had already passed*, and round 3 found
no blockers — two highs, three mediums and a low, all folded in. Round 3 closed nine of
round 2's ten and caught the tenth as reworded rather than fixed.

Revision 7, 2026-08-12, after `serina:review-final` on the pushed branch: no blockers,
five highs, all fixed before the PR was opened. The one that mattered: **#120's second
criterion names three intents — skip, approve, and suppress a finding — and the detector
covered two**, while the commit said `Closes #120`. A narrow suppression tell was added
and measured at zero false positives across both benign corpora, taking recall from
21/35 to 28/35; only semantic paraphrase remains, and it is the one class with no
unambiguous tell. Also fixed: `CONTAINMENT.md` said "a stage must never be handed raw PR
text" in § Consumer preamble and permitted exactly that in § Contract for later stages,
which is the rule #116 depends on; the aggregate byte-cap path dropped delimiter
findings while withholding content; `review.py`'s incomplete-review banner had no
producer anywhere on the branch; the CI job installed no PyYAML while a control treats
its absence as a failure; and step 3's live criterion accepted exit 2, so it passed with
no network at all — the only weakening on this branch that was never recorded.

Revision 6, 2026-08-12, after `serina:review-code` and `serina:review-tests` ran on the
finished branch. Between them they found five blockers, and one gutted a result reported
as done: **the mutation control tested one removal and the result was generalised.**
Deleting the escaping from `contain()` while leaving the envelope intact passed every
control including the mutation control, so "the controls fail if containment is removed"
was true only of the envelope. Reproduced, then fixed. Consequences:
  - The seam is now one mutant among eight in `check_mutations.py`, which applies each
    regression to a scratch copy and requires a named control to catch it. Six of the
    eight survived on first run — including a constant nonce and a warn-only byte cap.
    All eight are now caught.
  - `review.py` quoted evidence inside a fixed three-backtick fence, so attacker
    backticks closed it early and swallowed the rest of the review. The fence is now
    sized to the evidence, and the control parses the fences rather than counting them.
  - `render()` signalled the aggregate byte cap but still emitted the full document. It
    now withholds. "Refuse, not truncate" had been written down and not implemented.
  - The held-out payload was loaded and never passed to `detect()`. There are now two:
    one in scope that must be caught, one out of scope that must be missed — the second
    pins the documented recall gap so it cannot drift in either direction.
  - `make_nonce(seed=None)`, the production path underneath the 128-bit claim, was never
    called by any control. `check_invariants.py` now exercises it.

Revision 5, 2026-08-12, written **during** step 6 rather than before it, because
building the detector disproved its own acceptance criterion. Step 6 as written passed:
35/35 matrix, zero false positives on the ten upstream records, held-out payload caught.
But the upstream corpus is Rust product PRs, and this agent will be pointed at *this*
repo, whose issues and docs discuss reviews, findings and Blockers constantly. Measured
against that text the same detector produced **10 false positives**, including on issue
#120 itself and on CONTAINMENT.md. The criterion was measuring the wrong corpus, so
steps 6 and 7 are amended below and the reason is recorded rather than quietly fixed.

What changed materially across the revisions, rather than in wording:
  - Revision 2's step 3 ran against `--pr 120`. **No such PR exists** — 120 is an issue
    number, and GitHub shares one counter across issues and PRs. The step carrying the
    first-runs marker could never have run. Step 3 now uses a checked-in captured
    payload for its deterministic criterion, plus one live fetch against a PR that
    exists (#92, merged).
  - Revision 2's benign corpus wanted ten merged PRs from this fork. **Five exist**, one
    of them a 64-commit upstream sync. The corpus now draws from upstream `block/buzz`.
  - Revision 2 set the mutation bar at 20 of 25. That slack is exactly one entry-point
    row, so an unwired seam on one surface would hide inside it. The bar is now total.
  - The three comment surfaces are now three entry points, not one — decided rather than
    left open, because five acceptance criteria were resting on the unresolved count.
  - Revision 2's ALREADY TRUE claimed both launchpad workflows trigger on
    `pull_request`. **That was false and unverified** — `launchpad-issue-check.yml`
    triggers on `issues:` with `issues: write`. Corrected below.
  - Revision 3's step 8 asserted the payload in post-escape form, which is a no-op
    for four of five payloads, since escaping plain English changes nothing. Only the
    delimiter-breakout case can distinguish a contained render from a raw echo, so
    step 8 now turns on it.
  - Revision 3's step 3 counted blocks and labels but never checked that each block
    held its own field. Sentinel routing now proves it.

ALREADY TRUE  (verified against git and gh this session, per file, not generalised)
  This branch `feat/review-agent-untrusted-input` is level with `origin/launchpad` —
    `git diff --stat origin/launchpad...HEAD` is empty. Zero commits of this work exist.
  `feat/review-agent-preflight` (#116) carries one commit, `5e814fe8f`, adding
    `launchpad/plans/2026-08-12-issue-116-pr-review-preflight.md` — a plan, not code,
    and unpushed (`git ls-remote --heads origin` returns nothing for it). No sibling
    stage has been **built**. This line said "empty branch" in revision 3 and was true
    when written; the commit landed mid-session. Re-check it before step 10 finalises
    the contract, because #116's plan is the first consumer that contract must fit.
  **Was true when this plan was written, and is no longer:** "no review-agent code
    exists under `launchpad/`". Steps 1-11 have since built it — `launchpad/review-agent/`
    now holds the implementation, fixtures and controls. Left in place rather than
    deleted, because an ALREADY TRUE section that quietly updates itself teaches nothing
    about how fast these claims decay. Two lines in this section have now gone stale
    mid-session; treat every one as needing re-verification, not re-reading.
  `docs/` is **upstream's** tree — its last commit is upstream's `60ae74b65`
    (`fix(desktop): use WEBKIT_DMABUF_RENDERER_FORCE_SHM`, PR #4505). AGENTS.md §3
    puts cohort files under `launchpad/`, which is why this plan lives there.
  `gh api repos/launchpad-26/buzz/pulls/120` returns **404**. There is no PR #120 and
    there never will be; the shared counter is already past #122.
  This fork has exactly **five** merged PRs: #92, #61, #60, #14, #12. #12 is
    `chore: sync launchpad with upstream block/buzz main (64 commits)`.
  `.github/workflows/launchpad-pr-check.yml` — `on: pull_request`,
    `permissions: contents: read`, logic in an inline `python3` heredoc.
  `.github/workflows/launchpad-issue-check.yml` — `on: issues`,
    `permissions: issues: write` **and** `contents: read`, also an inline heredoc.
    The two differ; only the first is precedent for a PR-triggered check.
  ADR #110 (where the agent runs — Actions or Buzz) is OPEN with its decision section
    deliberately blank. Its outcome does not gate this plan: the containment is a pure
    text transform with no dependency on the execution host.
  This repository is public, so the delimiter chosen in step 1 is known to any attacker.

The seven author-controlled entry points, fixed here and referenced by number below:
  `pr_title`, `pr_body`, `pr_diff`, `pr_issue_comments`, `pr_review_comments`,
  `pr_review_bodies`, `linked_issue`. The three comment surfaces are distinct GitHub
  fields returned by distinct calls, so they are three entry points, not one.

STEP 1  Write the containment spec to `launchpad/review-agent/CONTAINMENT.md` —   [independent]
        envelope structure; the collision rule, covering the literal delimiter,
        repeated occurrences, the escape sequence itself, and near-miss variants
        (whitespace, case, Unicode confusables); the consumer preamble; the
        severity contract (an injection attempt is a `Blocker`, per PRD #109);
        and the degenerate-input rule — distinguishing **absent** (fetch failed),
        **empty** (fetched, genuinely no content), **oversized** (beyond a stated
        byte cap) and **unparseable**, each with its own disposition, so none is
        silently reported as clean.
        done when: `grep -Fx` finds each of these five headings by exact text —
        `## Envelope structure`, `## Delimiter collision`, `## Consumer preamble`,
        `## Severity contract`, `## Degenerate input` — a count of headings is not
        enough, since five arbitrary names would satisfy it; and under the last,
        `grep -F` finds all four words `absent`, `empty`, `oversized`,
        `unparseable`, each with a stated disposition on the same line.

STEP 2  Implement the envelope in `launchpad/review-agent/contain.py`, including  [needs 1]
        the disable seam that step 9 will use — built now, unused for seven steps,
        because retrofitting it means reopening this step. The seam must disable
        containment on **every** entry point, not per-surface.
        done when: over a variant corpus of six — literal delimiter, the delimiter
        twice, the escape sequence itself, a whitespace variant, a **case**
        variant, and a Unicode-confusable variant, one per near-miss class step 1
        names — every case either round-trips exactly through escape/unescape
        **or** is reported as a `delimiter_lookalike`; no case passes through
        silently. The seam is togglable, documented, and single-valued.

STEP 3  CLI over all seven entry points, with two criteria — one deterministic     [needs 2]
        against a checked-in captured payload, one live.       ← RUNS HERE
        done when: (a) `contain.py --payload fixtures/captured-pr.json` exits 0,
        stdout carries seven enveloped blocks — one per entry-point label — and,
        because the captured payload gives each of its seven fields a distinct
        sentinel string, **each sentinel appears in its own block and in no
        other**; a label-and-count check alone would pass a CLI that copied one
        field into all seven blocks or swapped title with body, and every later
        step's per-entry-point matrix rests on that routing being right;
        (b) `contain.py --pr 92` (a PR that exists — see ALREADY TRUE) exits 0 and
        carries the same seven labels; (c) each degenerate case from step 1 is
        forced in turn (`--degrade pr_diff=absent|empty|oversized|unparseable`)
        and each produces its own distinct disposition, with `absent`,
        `oversized` and `unparseable` exiting non-zero and none rendering as a
        clean empty block.

STEP 4  Payload corpus at `launchpad/review-agent/fixtures/payloads.json` —        [needs 3]
        attacks stored as data, bound to no entry point, so one payload can be
        routed through any surface.
        done when: **exactly five** payloads exist — skip-review, approve,
        suppress-a-finding, a paraphrase using none of those three phrasings, and
        a delimiter breakout — and the loader emits a payload × entry-point matrix
        of exactly 35 cases (5 × 7), no payload pre-assigned to any entry point.

STEP 5  Benign and held-out corpora, kept separate from step 4. Benign records     [independent]
        are drawn from upstream `block/buzz`, which has thousands of merged PRs;
        this fork has only five, one of them a 64-commit sync, so it cannot
        supply a representative corpus.
        done when: ten upstream merged-PR bodies and diffs are stored as benign
        records, none exceeding step 1's byte cap, and one attack payload absent
        from step 4's corpus is stored separately as the held-out case.

STEP 6  Deterministic detector — **high-precision tells only** — wired so          [needs 4, 5]
        `contain.py` emits its findings alongside the enveloped data, each
        carrying the step-1 severity. Recall is traded for precision on purpose:
        semantic coverage belongs to #117's model dimensions, and containment
        protects whether or not this layer notices. See CONTAINMENT.md
        § Detection, and the revision note above for why this criterion changed.
        done when: zero findings across **both** benign corpora — the ten
        upstream records **and** this repo's own review-heavy text (issues #109,
        #110, #116–#120, CONTAINMENT.md, this plan), which is the corpus the
        agent will actually be pointed at; every matrix case carrying an
        unambiguous tell is flagged; the caught and missed counts are **stated**
        in CONTAINMENT.md rather than implied; every finding names its entry
        point and carries severity `Blocker`; and `contain.py --json` carries
        them in `containment_findings`.

STEP 7  Control suite over the full matrix. Every control asserts the payload      [needs 6]
        appears only inside a data block, never in instruction position — that
        assertion rests on the envelope and holds for all 35. Cases whose payload
        carries an unambiguous tell additionally assert a finding; the cases that
        do not are **listed by id** as the model layer's responsibility, so the
        gap is named rather than merely absent.
        done when: the runner exits 0 reporting 35 controls, one per (payload ×
        entry point), none skipped, and printing the detected/undetected split
        with the undetected ids enumerated.

STEP 8  Publication control: the attempt must be visible in the rendered review.   [needs 6]
        A review that quotes attacker text verbatim has moved the payload into a
        new position, so the attempt must be visible **and** neutralised there.
        done when: for each payload routed through `pr_diff` that the
        deterministic layer detects — four of five, per step 6's measured
        recall, named rather than sampled — the render contains a single finding
        record that both carries severity `Blocker` and quotes the payload in its
        **post-escape** form, asserted on one record rather than as two
        substrings anywhere in the output; **and** for the delimiter-breakout
        case the quoted text must differ from the raw payload — the only case
        where the two representations diverge, and so the only one that can prove
        the renderer read the contained form rather than echoing the source
        field; **and** the review states the detector's coverage limit, so a run
        over an undetected payload can never read as a clean review.

STEP 9  Mutation control: the suite must collapse when containment is removed.     [needs 7, 8]
        done when: with the step-2 seam off, **all 35** controls fail and the
        runner names them; with it on, all 35 pass. No slack — a seam that
        disables containment everywhere leaves no case standing, so any survivor
        means a surface the seam never reached.

STEP 10 Extend `CONTAINMENT.md` with the contract each later stage adopts —        [needs 3]
        #116, #117, #118 and #119 each named, with the function it must call, the
        seven entry-point labels it must route, and the position its output may
        never occupy.
        done when: `grep` finds all four issue numbers, all seven entry-point
        labels, and for each stage a named function it must call.

STEP 11 CI: `launchpad-review-agent-controls.yml` running the control suite.       [needs 9]
        done when: the file exists, matches `launchpad-*.yml`, parses as YAML,
        triggers on `pull_request` and **not** `pull_request_target`, declares
        `permissions: contents: read` and no write scope, and invokes the step-7
        runner.

PARALLEL  Step 5 is independent and should start first — it is the longest pole,
          since it gathers ten upstream records over the network. Step 1 is also
          independent. Step 4 needs step 3; step 10 needs step 3 too — the
          function later stages call is the entry-point-routed interface, not
          step 2's raw envelope function, so a contract written against step 2
          alone could name the wrong level. It can then run alongside 4, 5, 6, 7
          and 8 as the only remaining step touching `CONTAINMENT.md`.
          Steps 7, 8 and 9 may not run in parallel — 7 and 8 both write the
          control suite, and 9 asserts a property of the suite they produce.
          Steps 1→2→3 are a strict chain: each consumes the previous one's format
          decision.
          This plan does not dispatch anything. Whether to fan out is Serina's call.

GATES     No `verify-*` skills exist in this repo's `.claude/skills/` (only
          `desktop-screenshot` and `sprout-cli`), so the gates are the global
          review agents. `serina:review-plan` has run twice — once against
          revision 1, once against revision 2 with a different reviewer — and both
          rounds are folded in above. A third run against this revision is the
          honest call before step 1, because revision 3 was again written by the
          author who caused the findings; the second round found two blockers in
          material that had already passed one review, so one clean round is not
          evidence of a clean plan. `serina:review-tests` after step 9, because
          the total mutation bar is the claim most likely to be quietly relaxed.
          `serina:review-code` after step 8. `serina:review-final` on the branch
          before the PR, then `serina:review-adjudicate`.
          All of these run before pushing, not after.
          None audits the plan's shape — `check-plan.sh` does that, and a clean
          run there proves only that the sections are present. It passed on
          revision 2, which contained a step that could never execute.

BUDGET    Step 6. The detector now faces 35 matrix cases, a keyword-free
          paraphrase, a held-out payload and ten benign upstream records, which
          requires a detector that generalises rather than a regex over known
          strings. If a deterministic detector cannot clear that bar, the honest
          outcome is to say so and reopen the model question in OPEN — not to
          weaken step 6's criterion.
          Second risk is step 2's seam. If it is not built in step 2 as written,
          and single-valued across all seven surfaces, step 9's total bar means
          reopening the envelope implementation.

OPEN      What the issue did not decide:
          - The script path. #116 says "path decided in the PR" and #120 says the
            same of fixtures. This plan proposes `launchpad/review-agent/` on the
            strength of AGENTS.md §3, but that is an assumption, not a ruling.
          - "Structurally marked as data" has two readings. Delimiter-and-preamble
            inside one prompt string, or a separate structured field the stage
            receives out of band. This plan assumes the first, because it works
            regardless of ADR #110's outcome. The second is stronger and should be
            preferred if #110 lands somewhere that supports it.
          - Unicode confusables can be escaped or flagged, but a model reads text
            visually rather than byte-wise, so a look-alike delimiter may still
            mislead a stage even when step 2 handles it correctly. Step 2 makes
            the attempt visible; it does not claim to make it harmless.
          - The byte cap for `oversized` is not set here. Step 1 must state a
            number; this plan does not pick it, because the right value depends on
            what a stage can actually accept and no stage exists yet.
          - Whether the step-6 detector may use a model. #116 forbids model calls
            in pre-flight; #120 does not say. This plan assumes deterministic,
            which is the weaker detector and the more testable one. See BUDGET.
          - ADR #110 is undecided. It does not block this plan, but step 11's
            workflow will need revisiting if the agent ends up hosted through Buzz.
          - Whether this plan should be tracked at all. AGENTS.md §2 says active
            work becomes an issue and stable knowledge becomes a document — a plan
            is neither. It sits at `launchpad/plans/` because §3 bars cohort files
            from upstream's `docs/` tree, but that settles *where* it goes if
            committed, not *whether* it should be. Serina's call.

LEFT OUT  Deliberately excluded:
          - The four stages themselves (#116–#119). This issue supplies the
            boundary they must use, not the stages.
          - Posting anything to a real PR. #119 owns publication; step 8 tests the
            rendered body, it does not send it.
          - Any model inference call. Nothing here spends a token.
          - Everything in #43's agent execution security model beyond this workflow
            reading untrusted text — #120 puts that out of scope explicitly.
          - Defending against a compromised model or a malicious maintainer, per
            the issue's own out-of-scope section.
          - Accessibility. No step touches UI; there is no keyboard interaction and
            nothing is announced to assistive technology. Excluded, not claimed.
