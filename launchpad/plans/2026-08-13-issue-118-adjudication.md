Issue #118 — task: adjudication pass over every reported finding
Stated size: none given — the task template has no Size field  ->  cap: 12 steps

Sized by asking, not guessing. Answered: more than an hour, so the cap is 12.
Two further answers shape this plan and were also asked rather than assumed:
the re-rated severity occupies the `severity` field while `reported_severity`
preserves what the dimension said, so #119 ranks by the verdict without a change
on its side; and a REFUTED finding is still published with its verdict beside it,
never suppressed, so nothing this stage does can remove a finding a human would
otherwise have seen.

Larger than an hour is flagged, not refused. These would have been better issues,
each observable on its own — splitting is the reader's call, not this plan's:

  (a) the verdict record and its validator alone — the artefact #119 would read
  (b) the harness with a stub judge, proving pass-through fidelity, the nonce
      hand-off and the stages manifest with no prompt written yet
  (c) the adjudicator definition and the escalate-only guard
  (d) the fixtures, recorded outputs and the control suite

Planned as written below.

ALREADY TRUE  (verified against git, the working trees and the GitHub API, not notes)
  Nothing of #118 is built. Branch feat/review-agent-adjudication is at e14f5fafb
  and `git rev-list --left-right --count origin/launchpad...HEAD` reports 0 0.
  `git ls-files | grep -iE 'adjudic|review-agent'` matches nothing.
  `launchpad/plans/` does not exist on this branch — it is created by this plan.
  Toolchain present: python3 3.12.3, gh 2.93.0.
  THIS ISSUE SITS AT THE END OF A THREE-LINK CHAIN, AND NONE OF IT IS MERGED.
  #120's containment tree — contain.py, fetch.py, review.py, run_controls.py and
  CONTAINMENT.md — is three pushed commits on `feat/review-agent-untrusted-input`
  (618789584, e072fba55, c64ff7958) and `git rev-list --left-right --count
  origin/launchpad...origin/feat/review-agent-untrusted-input` reports 0 3.
  `ls launchpad/review-agent/` fails in this worktree: no such directory.
  #117's tree is WORSE than unmerged — it does not exist as code at all.
  `feat/review-agent-dimensions` adds exactly two files to launchpad/ — the plan
  2026-08-12-issue-117-review-dimensions.md and its review record
  plans/reviews/2026-08-13-117-plan-review.md. `git ls-tree -r
  origin/feat/review-agent-dimensions | grep -E 'findings\.py|run_dimensions|
  dimensions/'` returns nothing: there is no findings.py, no run_dimensions.py and
  no dimension definition anywhere on it. So the
  contract this stage consumes exists ONLY as a normative plan document, and no
  producer of it has ever run. See BUDGET — this is the defining risk of #118 and
  it is different in kind from the one #117 carried.
  #117's contract is settled at its fourth revision and this plan honours it
  without renegotiating. Ten finding fields — dimension, severity, anchor, file,
  line, defect, failure, finding_id, entry_point, evidence. Eleven envelope
  fields — schema_version, dimension, pr, merge_base_sha, head_sha, status,
  outcome, error, findings, findings_count, completion_marker. SIX merged-document
  keys — pr, merge_base_sha, head_sha, reports, containment, nonce.
  THE SIXTH KEY IS `nonce`, AND #117 EMITS IT AS OF 733f48088. Verified with
  `git show 733f48088:launchpad/plans/2026-08-12-issue-117-review-dimensions.md`
  on `feat/review-agent-dimensions`, committed 2026-08-13 09:19:51, "docs(process):
  carry the run nonce on #117's merged document": the merged document carries "the
  run nonce, once, at the top level — the same value contain.render wrapped every
  surface with and every report's completion_marker embeds", and #117's own
  validator rejects a document with no top-level `nonce` and one where a marker
  disagrees with it.
  CITE WHAT `git show <sha>:<path>` RETURNS, NEVER WHAT THE WORKING TREE SHOWS.
  This plan has now got the same class of claim wrong three times, each in a
  different way, and the third is the one that produced this rule. First: a claim
  about #119 that was TRUE when written and falsified 40 minutes later by a sibling
  revising its plan. Second: a claim that #117 did NOT carry a nonce key, written
  while it already did — checkable at the time, and not checked. Third: the
  correction to the second, which cited 9d9dc9065 for content that commit does not
  contain. `git show 9d9dc9065:…| grep -c 'top-level \`nonce\`'` returns 0; the
  working tree returned 6. The content was real and STAGED BUT UNCOMMITTED, and two
  true observations — a grep of the working tree, and `git log --oneline -1` — were
  stitched into a provenance that never existed. It has since landed, at a
  different commit, an hour later.
  So the standing rule for every cross-issue claim in this plan: the evidence is
  what `git show` returns from a named commit, and the citation carries that commit.
  A grep of a sibling worktree proves only that someone has typed something
  somewhere. BUDGET already required re-verifying #117's field names at build time;
  this adds HOW, because the instruction was followed in spirit twice and still
  produced a wrong citation.
  What survives all three rounds is narrower and still worth doing — #118 VERIFIES
  the nonce key it is handed and passes it through, because it is agnostic about its
  producer for the same reason #119 is agnostic about its own. That behaviour is
  correct whether #117's key landed at one commit or another, which is why the
  design did not move when the citation did.
  One Blocker and one High from #117's third review pass bind this stage directly,
  and both are load-bearing here rather than incidental. Blocker 2 is the
  finding_id collision; anchor `pr` is High 6, not a Blocker — the distinction is
  kept because a plan that inflates another review's severities is doing the thing
  this stage exists to catch.
  `finding_id` now hashes (dimension, anchor, file, line, entry_point, defect,
  evidence). Before that fix, two findings differing only by surface or excerpt
  hashed identically — and this stage attaches exactly one verdict per id, so a
  colliding id would have merged two findings into one verdict or dropped one.
  The dedupe in STEP 7 is keyed on ids that are now genuinely distinct;
  `contain._dedupe` keys its own identity on (kind, entry_point, evidence) for the
  same reason.
  Anchor `pr` is STRUCTURALLY VALID FOR ANY FINDING, so this stage may not assume
  a file:line. #117's third review pass records that anchor `pr` with file and
  line null satisfies every validation rule for any finding at all, and #117 fixed
  it with a rule about appropriateness rather than structure. An adjudicator that
  reads `file` and `line` unguarded crashes on a legitimate `pr` finding; one that
  treats a missing location as unverifiable-therefore-refuted refutes true
  findings for free. STEP 6's default answers the second; STEP 2's validator and
  STEP 8's fixtures answer the first.
  THIS STAGE IS THE ONE THAT CAN CREATE A SEVERITY OUTSIDE THE LADDER, AND ITS
  GUARD IS DEFENCE IN DEPTH RATHER THAN #119'S ONLY DEFENCE. #119's plan now sorts
  with `review.SEVERITY_ORDER.get(finding["severity"], 9)` and renders an
  out-of-ladder severity under its "malformed finding" heading, sorting it last and
  triggering the incomplete banner — its review's finding 2 raised the bare
  subscript and its Outcomes table records the fix. `review.py:62` uses `.get(
  f.severity, 9)` for the same reason.
  This plan cited the bare subscript as live, and that citation was true when
  written and false forty minutes later: `feat/review-agent-publish` committed
  47482549e at 2026-08-13 08:48:13 revising the plan under this plan's feet. The
  correction is kept visible rather than quietly swapped, because it is the second
  time in this issue chain that a claim about a sibling's UNMERGED PLAN FILE rotted
  between reading and writing, and the lesson is the citation discipline, not the
  fact.
  The guarantee stands on its own merits. Re-rating is exactly how an out-of-ladder
  value gets created, this stage is its producer, and a producer that relies on its
  consumer's default has moved the failure rather than removed it. STEP 6 refuses
  to emit one; STEP 1 states the guarantee in the contract #119 reads.
  The severity ladder is {"Blocker": 0, "High": 1, "Medium": 2, "Low": 3} at
  review.py:32, and it is IMPORTED here, never re-declared.
  #119's stdin document is `{pr, head_sha, merge_base_sha, stages, reports,
  containment}` and #119 is "agnostic about which stage produced them". #117 emits
  every key of that except `stages`. So `stages` is this stage's to produce —
  #119's STEP 5 says the manifest covers "stages that emit no envelope of their own
  — #116's pre-flight and #118's adjudication", and its ALREADY TRUE records "#118
  is not started" as the reason its input is shaped that way. Cited by section and
  quoted text, not by line: #119's plan is an unmerged file under active revision
  and its line numbers moved twice while this plan was being written.
  #119's REQUEST FOR A NONCE IS ALREADY ANSWERED, UPSTREAM. #119's review finding 3
  is resolved "partly fixed, flagged upstream" — its STEP 5 rejects reports that
  DISAGREE on the nonce, which catches a single forged marker, while an all-forged
  run is undetectable there, and its OPEN asks for "a `nonce` key on #117's merged
  document". #117 added that key at 733f48088. So this stage neither creates the
  nonce nor negotiates for it: it checks the key it received against every marker
  and passes it on, which is what makes #119's condition satisfiable end to end.
  #119 TREATS ANY STAGE STATUS OTHER THAN "complete" AS INCOMPLETE and banners it
  at the top of the published body. That existing machinery is what STEP 6's
  total-refutation flag uses, rather than inventing a second signal #119 would
  have to learn.
  NO NONCE REACHES #119 TODAY, and this stage is the natural place to fix it.
  #119's review finding 3 records that STEP 5's nonce check "cannot be
  implemented, because no nonce reaches publish.py". #117 embeds ONE run nonce in
  every report's `completion_marker` (BUZZ-DIMENSION-COMPLETE:{dimension}:{nonce}),
  and this stage parses every report. STEP 4 extracts it, checks the reports agree
  on it, and emits it — which is a fix #119 can consume rather than a new demand.
  CONTAINMENT.md binds #118 by name, and the binding does not fit the input.
  Its "Contract for later stages" table says #118 must call
  `contain.findings_for(surfaces, nonce)` and must never "re-read raw PR text to
  'check for itself'". `findings_for` needs the Surface dict and the nonce; this
  stage receives a JSON document on stdin and has neither. What #117 already
  places in `containment.findings` IS what `findings_for` returns — the same
  `render()` second value. So the prohibition is honoured and the prescription is
  not literally satisfiable from stdin. STEP 3 passes the block through untouched
  and OPEN names who owns the amendment.
  contain.Finding is (kind, entry_point, evidence, severity="Blocker") and every
  call site takes the default, so every containment finding is a Blocker in
  practice as well as by contract.
  run_controls.py's CONTROLS list is (script, needs_network) pairs and reports SKIP
  with a reason — never PASS — for a control whose input is missing. STEP 10
  appends one row; no second workflow is added.
  ADR #110 is decided and constrains this stage only lightly: GitHub Actions for
  Phase 1, `pull_request` and not `pull_request_target`. Where the adjudication
  job sits relative to #117's and #119's is NOT decided by it — see OPEN.
  launchpad/plans/ is the established path. launchpad/AGENTS.md §3 puts all cohort
  files under launchpad/ and bars root docs/ and root scripts/ as upstream's trees.
  docs/plans/ does not exist in this checkout and is not used.
  No verify gate is installed in this checkout — .claude/settings.json and
  .claude/settings.local.json are both absent — so every review skill is a manual
  invocation and none will fire on its own.
  THE AUROC WORDING IN #118'S OWN ISSUE BODY IS WRONG AND IS NOT REPEATED HERE.
  #122's verification 1 of 3 on #109 establishes, against arXiv:2603.06594 v2:
  the 6,642 human-verified labels are confirmed; "on average only slightly better
  than a random coin-flip" is confirmed verbatim and is the claim those labels
  support; the 0.48–0.64 AUROC range is ONE judge (JailJudge) on ONE victim model
  (Llama-3.1-8B) under TWO attacks (GCG and GCG-R), not a range across judges; and
  "despite high performance on standard validation sets" is NOT a quotation from
  the paper. #109 has been amended in place. #118's body has not. This plan states
  the corrected version wherever the reasoning is used, and OPEN records that
  amending the issue belongs to whoever owns it.

STEP 1  launchpad/review-agent/ADJUDICATION.md — the verdict contract,   [independent]
        normative. A sibling to CONTAINMENT.md and to #117's FINDINGS.md, in the
        same voice, and the document #119 reads to know what arrives.
        It settles, in order:
        THE VERDICT. Exactly one per finding, from CONFIRMED | REFUTED | UNPROVEN.
        "Exactly one" is the whole criterion: a finding present on input and absent
        from output is a defect in this stage, not a tidy-up, so the contract states
        that input and output finding_id sets are EQUAL — not a subset.
        THE DEFAULT IS UNPROVEN, NEVER REFUTED. Absence of confirmation is not
        refutation. An adjudicator that cannot reach the location, cannot parse the
        finding, times out, or returns unusable output yields UNPROVEN with a reason.
        This is the fail-closed direction: a wrongly-UNPROVEN finding still reaches a
        human, a wrongly-REFUTED one reaches them wearing a dismissal.
        THE ADDED FIELDS, on top of #117's ten. Names are final here:
          verdict            CONFIRMED | REFUTED | UNPROVEN
          verdict_evidence   what the ADJUDICATOR established, in its own words —
                             required and non-empty on all three verdicts, including
                             REFUTED and UNPROVEN. An UNPROVEN with no reason is
                             indistinguishable from a stage that skipped the finding.
          reported_severity  the reporting dimension's value, preserved verbatim
          severity           THE RE-RATED value — #117's field, overwritten
          severity_reason    required whenever severity != reported_severity
          duplicate_of       finding_id of the survivor, or null — see STEP 7
        `severity` carries the re-rating and `reported_severity` preserves the
        original because #119 ranks by `finding["severity"]`, so the field name
        decides what the published review leads with. The dimension's value stays
        readable beside it, which is what #118's issue requires — "readable" rather
        than "in place", and the two are only the same field if #119 changes.
        SEVERITY IS RE-RATED ON EVERY FINDING, INCLUDING REFUTED ONES. The rating
        answers "how bad if true", which is a separate question from "is it true".
        Rating only the confirmed ones leaves REFUTED findings carrying an
        unexamined severity into #119's sort.
        `severity` AND `reported_severity` MUST BOTH BE IN review.SEVERITY_ORDER.
        Stated as a guarantee this stage makes to #119, and as DEFENCE IN DEPTH
        rather than #119's only defence: #119 sorts with `.get(severity, 9)` and
        routes an unrecognised severity to its malformed-finding heading, so a bad
        value is survivable there — but this stage is the one that CREATES bad
        values by re-rating, and a producer that relies on its consumer's default
        has moved the failure rather than removed it.
        The guarantee is on the EFFECTIVE severity, which is the re-rating where
        there is one and `reported_severity` where there is not. Both must be
        checked, because a finding arriving with an out-of-ladder
        `reported_severity` that the judge agrees with is never re-rated at all, so
        a guard watching only re-ratings never fires and the bad value is copied
        into `severity` untouched.
        BUT THAT CANNOT HAPPEN IF THE INPUT DOCUMENT IS VALIDATED FIRST, and this
        stage does so: `run_adjudication.py`'s `main` runs #117's own
        `findings.validate` against the input document BEFORE any adjudication
        logic touches it, and exits non-zero, adjudicating nothing, when it
        fails — the same input `reported_severity: "Info"` fails #117's
        validator on its own severity check, so it never reaches this stage's
        re-rating logic at all. This is stronger than "this stage is agnostic
        about its producer": #119 is agnostic about ITS producer because #119
        cannot re-validate a document it did not build from parts; this stage
        CAN, because the input document is exactly #117's own output shape, and
        the validator that checks it already exists. Refusing outright is also
        the only answer that does not need one: there is no legal value to
        preserve `reported_severity` AS once it already arrived broken, and
        inventing one would be this stage deciding, silently, what the
        dimension actually meant.
        So an out-of-ladder EFFECTIVE severity this stage can still reach — one
        this stage's OWN re-rating produced from a legal `reported_severity` — is
        an adjudication FAILURE for that finding: UNPROVEN, `severity` set to the
        nearest legal value with the refusal stated in `severity_reason`, and
        `reported_severity` unchanged, since input validation already guarantees
        it was legal to begin with. This stage may not silently decide that an
        unrateable finding is a small one, and it is never asked to invent a
        value for a field that arrived already broken.
        ESCALATE, NEVER APPROVE — stated as three concrete prohibitions rather than
        a slogan, because a slogan is not checkable:
          1. No field in this contract can carry an approval, a merge
             recommendation, or a pass. There is no `approved`, no `mergeable`, no
             `verdict: OK`. A judge cannot emit what the record cannot hold.
             THIS BINDS FIELD NAMES AND ENUMERATED VALUES, NOT FREE TEXT, and the
             limit is stated because it is otherwise invisible. `verdict_evidence`,
             `severity_reason` and `notes` are free strings; nothing mechanical stops
             a judge writing "this looks fine to me, recommend merge" into one, and a
             control that grepped them for approving phrasing would be a keyword
             filter — the kind of narrow guard the dimensions exist to find in other
             people's code. Today the exposure is nil: #119 renders none of the
             three. It stops being nil the moment #119 renders `verdict_evidence`,
             which OPEN proposes. So the mitigation is placed where it can work — the
             prompt in STEP 5 forbids it, STEP 9's mutation pair measures whether the
             clause makes a difference, and OPEN names this as a condition on
             #119 rendering the field rather than a solved problem.
          2. A REFUTED finding is still published, with its verdict beside it. This
             stage removes nothing from `reports[].findings`, and `findings_count`
             is unchanged by adjudication.
          3. A downgrade is allowed and must be visible. `severity` may fall below
             `reported_severity` — an overstated true finding is its own error — but
             it carries `severity_reason` and the finding is additionally listed in
             `adjudication.downgrades`, so the movement is legible in the document
             even where a consumer renders only `severity`.
        The reasoning is #109's, in #122's corrected wording: judges ruling on
        adversarial safety claims perform "on average only slightly better than a
        random coin-flip" against 6,642 human-verified labels, and the AUROC
        0.48–0.64 range is one judge on one victim model under two attacks. Not
        quoted here as anything broader, and the phrase "despite high performance on
        standard validation sets" is not used, because it is not in the paper.
        THE TOP-LEVEL `adjudication` BLOCK, a sibling of `reports` and `containment`.
        NINE keys, and the count is load-bearing for the same reason #117's ten
        finding fields are: STEP 10 builds one control per key, so a key in the
        output but not in this list gets no control at all. Withdrawing
        `adjudication.nonce` took the count from ten to nine and the two references
        to it disagreed for one revision, which is the miscount #117's plan names by
        name and fixed once already in its own contract:
          schema_version     integer, starts at 1
          verdict_counts     {CONFIRMED, REFUTED, UNPROVEN} — integers
          findings_in        count of findings received
          findings_out       count of findings emitted; must equal findings_in
          duplicate_groups   array of {survivor, duplicates: [finding_id]}
          downgrades         array of {finding_id, from, to, reason}
          total_refutation   boolean — see STEP 6
          notes              array of free-text notes; see LEFT OUT
          completion_marker  LAST key: BUZZ-ADJUDICATION-COMPLETE:{nonce}, using the
                             document's own top-level `nonce` — #117's sixth key,
                             passed through by STEP 4 and never re-generated here.
                             The block carries NO nonce of its own: a second copy in
                             one document is a second thing that can disagree, and
                             there is no question a copy answers that reading the
                             top-level key does not.
        The marker is last and carries the nonce for #117's two reasons unchanged: a
        marker at the end cannot survive truncation, and a fixed string published in
        a public repository is one a PR author can type into their own diff. This
        stage never accepts a caller-supplied nonce.
        `findings_out == findings_in` is the machine-checkable form of "a finding
        that is silently dropped is a defect in this stage".
        CONTAINMENT FINDINGS ARE PASSED THROUGH AND NOT ADJUDICATED. The
        `containment` block is emitted byte-identically to what arrived. Three
        reasons: they are deterministic catches, not claims needing a judge; an
        adjudicator able to REFUTE one could erase a detected attack, which
        CONTAINMENT.md § Severity contract calls worse than never detecting it; and
        their severity is fixed at Blocker by that same contract, so re-rating them
        would contradict a document this one is a sibling to.
        done when: ADJUDICATION.md exists under launchpad/review-agent/; it names
        all six added finding fields and all NINE `adjudication` block keys by the
        names above; it states that the input and output finding_id sets are equal;
        it states UNPROVEN as the default and REFUTED as never a default; it states
        the three escalate-never-approve prohibitions; it states that `severity` is
        the re-rated value, `reported_severity` the dimension's, and that `severity`
        is guaranteed to be in review.SEVERITY_ORDER as DEFENCE IN DEPTH — #119
        sorts with `.get(severity, 9)`, so this guard is not #119's only
        defence, and the guarantee's own justification is that THIS stage is the
        one that creates an out-of-ladder value by re-rating, and a producer that
        relies on its consumer's default has moved the failure rather than
        removed it; it states that the `containment` block passes through
        unadjudicated and cites CONTAINMENT.md § Severity contract; and it is
        referenced from a new row in CONTAINMENT.md's "Contract for later stages"
        table so the two documents point at each other rather than diverging quietly.

STEP 2  launchpad/review-agent/verdicts.py — the contract in code.           [needs 1]
        Pure functions and dataclasses over an already-parsed document. No
        subprocess, no network, no model call in this module.
        `Verdict` and `Adjudication` dataclasses; `validate(input_document,
        output_document) -> list[str]` returning EVERY violation rather than
        raising on the first, because a validator that stops at the first error
        hides the rest and STEP 10's suite asserts on the full list.
        VALIDATE TAKES BOTH DOCUMENTS, NOT ONE. An earlier revision declared
        `validate(document) -> list[str]` and separately required it to report
        the finding_id SYMMETRIC DIFFERENCE between input and output — but
        `findings_in` is a COUNT (see below), and a count cannot tell a dropped
        id from an invented one, or notice a swap that leaves the count
        unchanged. The check the contract already promises needs the actual
        input id SET, and the only place that set exists is the input document
        itself. `run_adjudication.py`'s `main` holds both already — it read the
        input off stdin before producing the output — so passing both costs
        nothing there; it only looked free on `validate` because the count was
        quietly standing in for a set it could never actually compare.
        `validate` enforces, at minimum: every finding carries exactly one `verdict`
        from the three values; `verdict_evidence` present and non-empty on all three;
        `severity` AND `reported_severity` both present and both IN
        review.SEVERITY_ORDER — a lowercase "blocker" or an "Info" is a violation in
        either field, not a curiosity, and checking only `severity` leaves the
        fallback STEP 6 copies from unchecked; `severity_reason` present whenever
        severity differs from reported_severity; the input and output finding_id SETS
        — extracted from `input_document` and `output_document` respectively, never
        from a count — are equal, reported as the symmetric difference so both a
        drop and an invention are named; `findings_out == findings_in == ` the sum
        of every report's
        `findings_count`; each report's `findings_count` still equals
        `len(findings)` after adjudication; `duplicate_of` names a finding_id
        present in the same document and never itself; every downgrade in
        `adjudication.downgrades` corresponds to a finding whose severity really did
        fall, and every such finding appears there — BOTH directions, since a
        downgrade list that can omit a downgrade is a list that hides one;
        `total_refutation` TRUE IF AND ONLY IF `findings_in > 0` and every verdict is
        REFUTED — both directions again, because a document reporting three REFUTED
        verdicts alongside `total_refutation: false` satisfies every other rule on
        this list and is exactly the "reported as a clean PR" case the issue's fifth
        criterion exists to prevent, and STEP 10's behavioural control tests the
        runner's own computation rather than the validator's completeness against an
        arbitrary document; `duplicate_groups` agreeing with every finding's
        `duplicate_of` in both directions, so neither a group naming a finding that
        does not point back, nor a finding pointing at a group that does not list it,
        validates; the completion marker present, LAST, and matching the nonce; the
        top-level `nonce` present, unchanged from the input, and equal to the nonce
        inside every report's completion marker AND inside this stage's own; and no key anywhere
        in the document named `approved`, `mergeable` or `merge_recommendation`,
        which is the machine-checkable half of prohibition 1.
        It also RE-RUNS #117's `findings.validate` over the output, so a document
        that leaves this stage still satisfies the contract it arrived under. A
        stage that quietly breaks its input's own rules is the same defect as one
        that drops a finding.
        SEVERITY_ORDER is imported from review; the ten-field record's rules come
        from #117's findings module. Neither is re-declared here.
        done when: `python3 -c "import verdicts"` run from launchpad/review-agent/
        succeeds — NOT `py_compile`, which compiles without resolving imports and so
        passes on a module whose `import review` cannot be satisfied, which is the
        exact failure the precondition in BUDGET exists to prevent; `validate`
        returns an empty list for a well-formed adjudicated document and, for one
        carrying four independent violations at once, returns four strings rather
        than one; called as `validate(input_document, output_document)` where
        `output_document` drops one input finding_id, rejected with that id
        named; the SAME call with `output_document` instead carrying an invented
        finding_id absent from `input_document`, rejected — the two cases a mere
        COUNT comparison cannot tell apart, since both leave `findings_out ==
        findings_in` true;
        a finding with `severity: "Info"` is rejected AND one with
        `reported_severity: "Info"` is rejected, the second being the case a guard
        watching only re-ratings never sees; a finding whose severity fell
        with no entry in `downgrades` is rejected, AND a `downgrades` entry with no
        corresponding fall is rejected; a document whose findings are all REFUTED
        with `total_refutation: false` is rejected, AND one with
        `total_refutation: true` over a mixed verdict set is rejected; a
        `duplicate_groups` entry naming a finding whose `duplicate_of` is null is
        rejected, AND a finding whose `duplicate_of` names a survivor listed in no
        group is rejected; a `verdict: REFUTED` with empty
        `verdict_evidence` is rejected; and `verdicts.SEVERITY_ORDER is
        review.SEVERITY_ORDER` is true.

STEP 3  launchpad/review-agent/run_adjudication.py — the CLI,  [needs 2]  <- RUNS HERE
        demonstrable before a single prompt is written. Reads one #117 merged
        document on stdin, adjudicates every finding with an INJECTED JUDGE
        CALLABLE defaulting to a stub that returns UNPROVEN with a stated reason,
        and prints one document on stdout in the shape #119 reads.
        Three things this module must get right, each of which is a way to lose data
        rather than a feature:
          PASS-THROUGH IS BYTE-IDENTICAL WHERE IT IS PASS-THROUGH. `pr`,
          `merge_base_sha`, `head_sha` and the whole `containment` block leave
          exactly as they arrived. The evidence inside a containment finding is RAW
          per #117's contract, and #119 escapes at render — a stage that
          re-serialises through anything lossy publishes an excerpt that no longer
          matches what the author wrote. Asserted by comparison, not by inspection.
          THE ADJUDICATOR NEVER RE-READS RAW PR TEXT. CONTAINMENT.md forbids
          "re-read raw PR text to 'check for itself'". This module makes no
          `fetch_all` call and no `gh` call for surfaces. What it may read is the
          repository at `head_sha` — the file a finding is anchored at — which is
          the change under review as CODE, not the author's prose as INSTRUCTION.
          The distinction is stated in the module docstring because it is the one an
          implementer will get wrong in the helpful direction.
          ANCHOR `pr` IS NORMAL, NOT AN ERROR. A finding with file and line null is
          structurally valid for #117 and this module must adjudicate it without
          reading either field. Every location-using path branches on `anchor`
          first, and a `pr` finding's `verdict_evidence` names what was checked —
          the claim, the missing file, the property of the change — rather than a
          line that does not exist.
          THE INPUT IS VALIDATED BEFORE A SINGLE FINDING IS ADJUDICATED. `main`
          runs #117's own `findings.validate` against the input document first,
          and exits non-zero with no output at all when it fails — never
          proceeding to re-rate or emit anything for a document that already
          broke #117's own contract. This is what keeps STEP 2's severity
          guarantee reachable: a finding whose `reported_severity` arrives
          out-of-ladder (an "Info", say) fails #117's validator on that ground
          alone, so it never reaches the re-rating logic where "there is no
          legal value to preserve it as" would otherwise be a real question with
          no good answer.
        A `--judge stub` default and a `--replay <dir>` mode reading STEP 9's
        recordings are the only two ways the suite ever obtains a verdict, so no
        control makes a model call.
        This is also what keeps "choosing the model" out of scope: the runner never
        names one.
        done when: `python3 launchpad/review-agent/run_adjudication.py < fixture.json`
        exits 0 and prints a document that `verdicts.validate` accepts AND that
        #117's `findings.validate` still accepts; the printed `pr`, `head_sha`,
        `merge_base_sha` and `containment` are byte-identical to the input's, asserted
        by `json.dumps(..., sort_keys=True)` comparison rather than by eye; a fixture
        whose only finding has anchor "pr" with file and line null is adjudicated
        without raising and its verdict carries non-empty `verdict_evidence`; a
        fixture carrying all three containment kinds emits them unchanged with
        severity Blocker and no verdict field added to any of them; malformed JSON on
        stdin exits non-zero and prints no document; a fixture whose one finding
        arrives with `reported_severity: "Info"` exits non-zero and prints no
        document at all — validated and refused before adjudication, not
        adjudicated into a best-effort output; a judge injected to REFUTE that
        same finding is never called, asserted on the injected judge's own call
        count, so the refusal is proven to happen before the judge runs rather
        than after; and a control asserts no `gh` subprocess and no HTTP client
        was invoked during a stub run.

STEP 4  The nonce check and the `stages` manifest.                           [needs 3]
        One output #117 does not produce, and one it does that this stage must not
        take on trust.
        THE NONCE IS CHECKED AND PASSED THROUGH, NEVER GENERATED. #117's merged
        document carries it as a top-level key (733f48088) and #117's own validator
        already rejects a document whose marker disagrees with it. This stage checks
        it AGAIN anyway, for the reason #119 gives for checking its own input: a
        stage agnostic about its producer cannot inherit a guarantee it did not
        watch being made, and this one is downstream of an unmerged plan whose
        validator does not exist yet.
        The check: the top-level `nonce` must be present, and must equal the nonce
        embedded in EVERY report's completion marker. There are THREE refusals, each
        with its OWN reason string, and they are ordered because one input can
        exhibit two of them at once — two reports carrying different nonces, neither
        matching the top-level key, satisfies both descriptions, and a rule that does
        not say which is checked first produces two implementations that disagree
        about what the operator is told:
          1. no top-level `nonce`, or no parseable marker on any report — reason
             names the ABSENT PROVENANCE. Checked first because the other two compare
             against a value that must exist to compare with.
          2. the reports disagree with EACH OTHER — reason names a MIXED DOCUMENT:
             envelopes from two different runs merged into one.
          3. the reports agree with each other but not with the top-level key —
             reason names a MISMATCHED ENVELOPE: one run's reports under another
             run's header, which is the shape a forgery takes when the forger sees
             only one of the two places the nonce lives.
        A document exhibiting both 2 and 3 is reported as 2, because a mixed document
        is the larger fact and the header mismatch is its consequence. Stated rather
        than left to the order the code happens to be written in.
        This stage never picks a winner among disagreeing nonces, and never stamps
        its own marker with a nonce it could not establish: an adjudication with no
        provable provenance must not read as complete. It never accepts a caller-supplied nonce and never
        substitutes one of its own for a missing key, which would manufacture exactly
        the provenance the nonce exists to prove.
        What this does NOT catch is an ALL-FORGED run — every marker and the
        top-level key agreeing on a nonce an attacker chose. Neither #117 nor #119
        catches it either, and it is out of reach from inside a document: the only
        stage that can tell is the one holding the nonce `contain.make_nonce`
        actually returned. Named here rather than left to look covered.
        THE `stages` MANIFEST. #119 reads `{name, status, reason}` entries for
        stages that emit no envelope. This stage emits the array containing every
        entry present on input — #116's pre-flight entry when it exists — plus
        exactly one entry named `adjudication`. It never overwrites an existing
        `adjudication` entry silently: a second one on input is a re-run against an
        already-adjudicated document and exits non-zero.
        The `adjudication` entry's status is `complete` only when every finding
        received a verdict, the nonce was established, and STEP 6's flag is false.
        done when: given a fixture whose top-level `nonce` matches all three reports'
        markers, the output's top-level `nonce` is unchanged from the input and
        `adjudication.completion_marker` is
        BUZZ-ADJUDICATION-COMPLETE:{that nonce} as the LAST key of its block; a
        fixture whose two reports carry DIFFERENT nonces exits non-zero with a reason
        naming the disagreement and prints no document; a fixture whose reports agree
        with each other but NOT with the top-level key exits non-zero with its own
        reason, distinct from the first — the two are different failures and one
        shared message hides which; a fixture with no top-level `nonce` exits
        non-zero and no nonce is invented; a fixture whose reports carry no marker
        exits non-zero and the emitted stage status is never "complete"; the output
        carries a `stages` array containing an `adjudication` entry plus every entry
        that arrived; a fixture already carrying an `adjudication` entry exits
        non-zero; and #119's own incomplete rule is satisfied against the output —
        checked by running #119's stated conditions as assertions here, since #119's
        code does not exist to run against.

STEP 5  launchpad/review-agent/adjudicator.md — the definition.              [needs 1]
        What the judge is told: it receives findings and the contained document, it
        decides each one, and it may not do the four things this stage forbids.
        Its scope, in its own words: for each finding, establish independently
        whether the defect is present, produce the evidence for that answer itself,
        and rate the severity on its own merits.
        Its exclusions, each naming why:
          It must NOT restate the reporting reviewer's claim as its own evidence.
          The issue's second criterion is that evidence is "produced itself, not a
          restatement", and a restatement is the cheapest thing a judge can emit.
          It must NOT hunt for new defects. #118 puts that out of scope; a genuinely
          new one it notices goes to `adjudication.notes` and never into
          `reports[].findings`, so it cannot displace the dimensions' work or be
          counted as one of their findings.
          It must NOT emit an approval, a merge recommendation, or a "looks fine".
          The record cannot hold one, and a judge told this in the prompt as well as
          denied it in the schema fails in the same direction twice.
          It must NOT refute for want of evidence. Not-established is UNPROVEN.
        It states the anchor rule from the consuming side: a finding anchored `pr`
        has no file and no line, and that is a legitimate shape, not a malformed
        finding to be dismissed.
        THIS STEP'S DONE-WHEN IS TEXTUAL ON PURPOSE, and says so. Whether the clause
        WORKS is a property of output, which needs STEP 9's recordings, and STEP 9
        needs this file. Tagging this step against 9 would be circular, so the
        behavioural proof lives in STEP 9's done-when and in STEP 10's controls
        instead. #117's plan recorded four separate steps that claimed behavioural
        acceptance their own tags could not produce; this is that defect refused in
        advance rather than found by a reviewer.
        done when: the file exists; it states all four exclusions with the reason for
        each; it states UNPROVEN as the answer to insufficient evidence in those
        words; it states that evidence must be the adjudicator's own and gives a
        worked contrast between a restatement and an independent check; it states
        that anchor "pr" is legitimate; and it names no model.

STEP 6  Escalate-only, enforced in code, and the total-refutation flag.      [needs 3]
        The three prohibitions from STEP 1 turned into behaviour that can fail.
        NOTHING IS REMOVED. The output's finding_id set equals the input's,
        asserted inside the runner before it prints, not only in the validator — a
        stage that can print a lossy document and rely on a downstream check has
        already lost the document once.
        A DOWNGRADE IS RECORDED. Any finding whose `severity` falls below its
        `reported_severity` is appended to `adjudication.downgrades` with from, to
        and reason, at the moment the re-rating is applied rather than by a later
        sweep — a later sweep is a second place the two can disagree.
        AN OUT-OF-LADDER EFFECTIVE SEVERITY IS REFUSED, not published. The guard
        fires on the value that will actually be emitted — the re-rating where there
        is one, `reported_severity` where the judge agreed and there is none — not
        only on a re-rating that differs. A guard watching only re-ratings never sees
        a finding that ARRIVED at "Info" and was agreed with, and copies it into
        `severity` untouched. The finding becomes UNPROVEN with `severity_reason`
        naming the refusal; `severity` falls back to `reported_severity` when that is
        in the ladder, and to `Blocker` when it is not, because there is then no safe
        value to copy and this stage may not decide that an unrateable finding is a
        small one. This is defence in depth rather than #119's only defence — #119
        sorts with `.get(severity, 9)` and renders an unrecognised severity under its
        malformed-finding heading — but this stage is where bad values are created,
        and a producer relying on its consumer's default has moved the failure
        rather than removed it.
        TOTAL REFUTATION IS FLAGGED. When the input carried at least one finding and
        EVERY one of them is REFUTED, `adjudication.total_refutation` is true and the
        `stages` entry for adjudication carries status `total_refutation` with a
        reason. #119 treats any status other than "complete" as incomplete and
        banners it at the top of the body, so a totally-refuting run cannot publish
        as a clean review, and #119 needs no change to make that true.
        The zero-findings case is NOT total refutation and must not set the flag. A
        run over a document whose dimensions all reported `outcome: clean` had
        nothing to refute; conflating the two would banner every genuinely clean PR
        and teach readers to skip the banner — the failure #119's review names in its
        own Blocker.
        done when: with a judge injected to REFUTE every finding, the output's
        `reports[].findings` arrays are unchanged in membership and length, every
        `findings_count` is unchanged, `total_refutation` is true, and the
        `adjudication` stage entry's status is not "complete"; with the same judge
        against a document carrying zero findings, `total_refutation` is FALSE and
        the stage status is "complete"; with a judge injected to return severity
        "Info" over a finding whose `reported_severity` is legally in-ladder, that
        finding is emitted UNPROVEN at its reported severity with a reason and the
        document still passes `verdicts.validate` — this is the case STEP 3's
        input validation does NOT catch, because the input was legal and only
        this stage's own re-rating produced the bad value; the SIBLING case — a
        fixture whose finding ARRIVES with `reported_severity: "Info"` already —
        is STEP 3's job, not this control's: `run_adjudication.py` exits non-zero
        on it before any judge runs, so it is asserted there and not repeated
        here as a per-finding UNPROVEN case, since there is no legal
        `reported_severity` for such a finding to have been emitted WITH; a BARE
        `review.SEVERITY_ORDER[f["severity"]]` subscript
        succeeds for every finding in every output above, used bare on purpose so
        the control fails where a consumer without #119's `.get` default would; and
        with a judge injected to downgrade a Blocker to Low,
        `adjudication.downgrades` names it with from, to and reason.

STEP 7  Dedupe, visible from both ends.                                      [needs 3]
        Findings describing the same defect in different words are grouped, and the
        grouping is in the output rather than in the stage's head.
        A group is {survivor, duplicates: [finding_id]} in
        `adjudication.duplicate_groups`, and every duplicate ALSO carries
        `duplicate_of` naming its survivor. Both directions, so the grouping is
        discoverable from the finding as well as from the block — a consumer holding
        one finding should not have to scan a top-level array to learn it is a
        duplicate.
        A DUPLICATE STILL RECEIVES ITS OWN VERDICT AND IS STILL EMITTED. Dedupe
        changes presentation, never the count. The issue's first criterion is that
        every finding receives exactly one verdict and a silent drop is a defect; a
        dedupe that removes records would breach it while looking like tidiness.
        `finding_id` does not do this work and cannot. #117's contract states
        plainly that the id is "NOT stable across a model rewording `defect`", and
        dedupe across rewordings is named there as #118's job. Two findings from
        DIFFERENT dimensions describing one defect have different ids by
        construction, since `dimension` is a hash input.
        The survivor is chosen deterministically: highest adjudicated severity, then
        CONFIRMED before UNPROVEN before REFUTED, then lowest finding_id. Stated
        because "the best one" is not a rule and two runs must agree.
        done when: given two findings from two dimensions describing one planted
        defect, both are present in the output, both carry a verdict, exactly one
        carries `duplicate_of` naming the other, and `duplicate_groups` carries one
        group naming both; the survivor is the same across two runs of the same
        input, asserted by byte-comparing the two outputs; a finding whose
        `duplicate_of` names an id absent from the document is rejected by
        `verdicts.validate`; a finding naming itself is rejected; and a run that
        dedupes nothing emits an EMPTY `duplicate_groups` array rather than omitting
        the key.

STEP 8  Fixtures — the input documents, and why they are synthesised.     [needs 2, 3]
        Under launchpad/review-agent/fixtures/adjudication/. Five, each isolating
        one thing this stage must handle:
          a three-report document with one finding per dimension, all anchor "line"
          a document whose only finding is anchor "pr" with file and line null
          a document carrying two findings from two dimensions describing ONE defect
          a document carrying all three containment kinds plus a full seven-key
            `states` map, and zero dimension findings
          a document with one failed report, one clean report and one with findings
        THESE ARE SYNTHESISED, NOT RECORDED, AND THAT IS A KNOWN WEAKNESS. #117's
        producer does not exist — there is no run_dimensions.py anywhere — so no
        real #117 output can be captured today. The mitigation is that a fixture is
        valid only if #117's own `findings.validate` accepts it, so each conforms to
        the contract's validator rather than to this author's reading of the contract
        document. That is weaker than a recording and is recorded as weaker: it
        proves shape, not that any producer ever emits that shape.
        The regeneration is a named deliverable, not an intention: once #117 lands,
        one fixture is REPLACED by the stdout of a real `run_dimensions.py <n>
        --stub` run and the suite re-run against it. Until then the suite's coverage
        of real producer output is zero, and the PR body says so.
        done when: five fixtures exist; each parses as JSON and is accepted by
        #117's `findings.validate`; each is a valid input to run_adjudication.py;
        each names in a header field which behaviour it isolates; the containment
        fixture's `states` map has exactly seven keys matching contain.ENTRY_POINTS;
        and a note in the fixtures directory records that they are synthesised, why,
        and which one is to be regenerated from a real #117 run.

STEP 9  Recorded judge outputs, and the falsifiability pair.              [needs 5, 8]
        For each fixture, a recorded judge output stored as JSON and replayed by
        `--replay`. Recorded from a real run against a real model and NEVER
        hand-written — a hand-written "judge output" tests only that the author can
        write valid JSON, which STEP 2's validator already covers.
        The model id and the date are recorded in each file. That is provenance, not
        a model choice; the runner still never names one.
        THE FALSIFIABILITY EVIDENCE LIVES HERE rather than in its own step, because
        the cap is 12 and this is where the live runs already happen. For each of
        STEP 5's four exclusions, one before-and-after pair: delete that clause from
        the definition, run live against the fixture it targets, record what changes,
        restore and record that it changes back. The most important pair is the
        escalate-only clause — with it removed, does the judge produce an
        approval-shaped answer or downgrade a Blocker without a reason?
        THIS IS A SAMPLE, NOT A RATE, and the PR body must say so in its own words.
        One run of a non-deterministic judge is one observation. A clause shown to
        matter once has not been shown to matter reliably. Stating the sample size
        is the difference between evidence and a claim.
        done when: five recordings exist, one per fixture; each is accepted by
        `verdicts.validate` when replayed; each carries the model id and date; four
        before-and-after pairs exist, one per exclusion, each naming the exact clause
        removed; the escalate-only pair shows the difference the clause makes; a
        control asserts the replay path makes no network call, by injecting a runner
        that raises if the real `gh` binary or any HTTP client is invoked; and the
        recordings show the dedupe fixture's two findings grouped rather than one of
        them dropped.

STEP 10 The control suite — one control per done-criterion.       [needs 4, 6, 7, 9]
        launchpad/review-agent/check_adjudication.py, registered in run_controls.py's
        CONTROLS list as ("check_adjudication.py", False) so #120's single CI entry
        point picks it up and no second workflow is added.
        One control per #118 done-criterion, plus:
          the finding_id sets equal, asserted as a symmetric difference so a DROP
            and an INVENTION are distinguished rather than both reading as "mismatch"
          every one of the six added fields fed empty, missing and malformed in turn,
            asserting `validate` names it — six is a list someone can count, "every
            field" is not, and the count is checked against STEP 1's list rather
            than trusted
          an out-of-ladder EFFECTIVE severity refused rather than published, fed both
            ways — as a judge's re-rating, and as a `reported_severity` on the input
            that the judge agrees with, which is the path a re-rating-only guard
            never sees — AND the positive form: for every finding in every recorded
            output, a BARE `review.SEVERITY_ORDER[f["severity"]]` succeeds. Bare on
            purpose: #119 defends itself with `.get(severity, 9)`, and a control
            borrowing that default would pass on exactly the output #118 must not
            emit
          total refutation flagged, and the zero-findings case NOT flagged — both
            directions, since a flag that always fires and a flag that never fires
            are the same defect wearing opposite signs — and the same pair asserted
            against `verdicts.validate` on a HAND-BUILT document rather than only
            against the runner's own output, since a control that only ever feeds
            the runner tests the runner's computation and not the invariant
          a REFUTED verdict leaving `reports[].findings` and every `findings_count`
            unchanged
          no key named `approved`, `mergeable` or `merge_recommendation` anywhere in
            any output, asserted by walking the document rather than by grepping the
            source
          downgrades recorded in BOTH directions — no unrecorded fall, no recorded
            fall that did not happen
          dedupe visible from both ends, and no duplicate dropped
          the containment block byte-identical in and out, for all three kinds, with
            evidence unchanged — a transport that handles one kind and mangles two
            reads as working
          the nonce checked rather than trusted: report-versus-report disagreement,
            report-versus-top-level disagreement and a missing top-level key each
            refused with their OWN reason, the no-marker case never reporting
            complete, and the top-level `nonce` byte-identical in and out
          dedupe visible from both ends asserted on a HAND-BUILT document too — a
            group naming a finding that does not point back, and a finding pointing
            at a survivor listed in no group, both rejected by `verdicts.validate`
          an anchor "pr" finding adjudicated without reading `file` or `line`
          #117's `findings.validate` still accepting the OUTPUT document
        done when: `python3 launchpad/review-agent/check_adjudication.py` passes; it
        is listed in run_controls.py; `python3 launchpad/review-agent/run_controls.py`
        runs it and reports it in the summary; the suite makes no network call,
        asserted by the injected runner above; and each bullet above has a
        separately named control.

STEP 11 Prove each control can fail, against a targeted mutation.     [needs 10]
        AN EARLIER REVISION NEUTERED EACH CHECK FUNCTION TO A CONSTANT, AND THAT
        PROVES THE WRONG THING. Replacing a check with `return ["FAIL"]`
        demonstrates only that the runner can register a hardcoded failure —
        every check would pass this test identically, including one with its
        actual comparison deleted, because the neutering never exercises the
        comparison at all. `return []` (a neutered check reporting no
        violations) proves even less: the suite stays green, which reads as the
        control passing rather than as the defect this step exists to surface.
        Neither result says whether the check catches the SPECIFIC regression
        it exists for — a control that always fails and one that correctly
        catches one defect are indistinguishable under this method, and so are a
        control that always passes and one that correctly misses nothing. This
        is the same defect class #109's own review chain has already found and
        fixed in a sibling codebase's mutation harness — see
        `feat/review-agent-untrusted-input`, `check_mutations.py` — and the fix
        there is the model for this one.
        Each of STEP 10's roughly fifteen controls instead gets ITS OWN named
        mutation of `verdicts.py` or `run_adjudication.py` — a plausible
        regression to that control's specific target, applied to a scratch copy,
        never the working tree — and the suite must go red under that mutation
        and green once it is reverted. A representative set, named here so an
        implementer inherits the shape rather than inventing it fresh:
          finding_id set equality — mutation: compare `findings_out ==
            findings_in` (the counts) instead of the two id sets. Passes on a
            swapped id, which is exactly the hole STEP 2's own fix closed.
          the six added fields, each checked — mutation: delete the
            `severity_reason` presence check alone. Only the control for THAT
            field may fail; if a different field's control also fails, the
            fixture is entangling fields a done-criterion requires be
            independent.
          out-of-ladder severity refused — mutation: drop the `reported_severity
            in SEVERITY_ORDER` half of the guard, keeping the `severity` half.
            Passes on a finding whose `reported_severity` arrived broken and
            was never re-rated — the exact gap this plan's Codex pass found and
            STEP 3 now closes at the input boundary; this mutation is what
            proves STEP 3's refusal is load-bearing rather than redundant with
            STEP 2's guarantee.
          `total_refutation` correctness — mutation: set the flag whenever
            `findings_out > 0`, dropping the "every verdict is REFUTED"
            condition. Passes on a document with one CONFIRMED finding among
            several REFUTED ones, which must NOT flag total refutation.
          REFUTED findings still published — mutation: filter `reports[].
            findings` to only CONFIRMED and UNPROVEN before emitting. Passes
            silently unless a control specifically counts findings in, not just
            findings whose verdict is not REFUTED.
          no approval-shaped key — mutation: add a literal `"approved": null`
            key to the Adjudication dataclass. A control asserting only that
            approved is never `true` would pass under this mutation; the
            control must reject the KEY's presence, not merely its value.
          downgrades recorded both directions — mutation: append to
            `downgrades` on every re-rating regardless of direction, including
            upgrades. Passes on a control that only checks "every real downgrade
            is recorded" without also checking "every recorded entry is a real
            downgrade".
          dedupe visible both ends — mutation: set `duplicate_of` on the
            survivor rather than the duplicate. Passes on a control that checks
            only that the field is set somewhere, not on which finding it
            names.
          containment passthrough — mutation: run `contain.escape` on
            containment evidence before re-emitting it. Passes on a control that
            checks the finding is present but not that its `evidence` field is
            byte-identical to the input's.
          nonce checked, not trusted — mutation: compare each report's marker
            nonce to the FIRST report's marker instead of to the top-level
            `nonce` key. Passes on any fixture where every report happens to
            agree with itself, which is most of them; only a fixture where
            reports agree with each other but disagree with the top-level key
            catches it — see STEP 4's ordering rule.
        The remaining controls follow the same requirement even where not
        enumerated above: the mutation must be a change to what the PRODUCTION
        CODE does, described precisely enough that a builder could apply it to
        a scratch copy without guessing, and it must be the SMALLEST change
        that defeats that control specifically — a mutation broad enough to
        also break three other controls proves less than a narrow one that
        breaks only its target.
        done when: a recorded run shows, for each of STEP 10's controls, the
        specific named mutation above applied to a scratch copy of the
        production code (never the working tree), the suite failing while it is
        applied, and the suite passing once it is reverted, with the raw output
        of all runs pasted into the PR body; and for at least the finding_id,
        total-refutation and out-of-ladder mutations, a control OTHER than the
        one the mutation targets is confirmed to still pass under that mutation
        — proof the mutations are targeted rather than incidentally breaking
        the whole suite.

STEP 12 Open the PR against launchpad.                                      [needs 11]
        AGENT_PR_TEMPLATE.md filled, `Closes #118`, `by:agent`, all raw output from
        STEPs 9 and 11 pasted, and Escalations naming every deferral in OPEN — above
        all that the fixtures are synthesised because #117's producer does not exist,
        that the verdict word reaches no published review until #119 renders it, and
        that CONTAINMENT.md's row for this stage prescribes a call this stage's input
        cannot support.
        done when: the PR is open against launchpad with the by:agent label; the
        "launchpad — PR body check" check is green; every commit carries a
        Signed-off-by trailer so the DCO check passes; and the body names the #117
        and #120 ordering dependencies, the synthesised-fixture weakness, and the
        #119 rendering gap explicitly.

PARALLEL  Two genuine fan-outs, and two that look available but are not.
  STEP 5's definition and STEP 8's fixtures are disjoint files and can be written
  in parallel with each other. STEP 5 needs only STEP 1; STEP 8 needs STEP 3 to
  close, so dispatch it early if it suits the schedule but do not mark it done on
  the strength of five files having been written.
  STEP 6 and STEP 7 both edit run_adjudication.py. Two steps editing one file are
  sequential regardless of how unrelated escalate-only and dedupe look. They are
  tagged [needs 3] each and must run one after the other, not as two subagents.
  STEP 4 also edits run_adjudication.py and joins that same queue.
  STEPs 1, 2 and 3 cannot be parallelised — each is the input to the next. STEP 1
  is the only step independent as written.
  STEPs 9 through 12 are strictly sequential; each reads the whole surface the
  previous produced. Nothing is dispatched here; the decision to fan out belongs to
  whoever executes this.

GATES  No verify gate is installed in this checkout — .claude/settings.json and
  .claude/settings.local.json are both absent — so every gate below is a manual
  invocation and none will fire on its own.
  serina:review-plan has run ONCE on this file, before anything was built, and this
  revision is the result. Five findings — one Blocker, two High, two Low — all
  applied, one disputed on its reasoning and applied anyway. The verified record is
  at launchpad/plans/reviews/2026-08-13-118-plan-review.md.
  The Blocker: this plan cited #119 as sorting by a bare `SEVERITY_ORDER[...]`
  subscript, which #119 no longer does. The finding is right, and the fix was made
  in four places at the time — a fifth, in STEP 1's own done-when for what
  ADJUDICATION.md must state, was missed by that pass and caught later by an
  independent Codex review (see below); five places, now. Its stated timing is
  wrong, and the correction is recorded rather than quietly absorbed: the report
  says #119's `.get` fix landed "before #118 was written". `git log -1
  --format='%ad' 47482549e` gives 2026-08-13 08:48:13, and this plan was drafted
  from a grep run at ~08:20 that returned the bare subscript at a line number that
  no longer exists. The claim was TRUE when written and was falsified forty
  minutes later by a sibling worktree revising its plan. The lesson
  is the citation discipline #117 already adopted for line numbers, now extended to
  behavioural claims about unmerged sibling plans — cite the section and the quoted
  text, never the line.
  The fix also stands on different grounds than the report gives it. The report
  treats the citation as the defect; the citation is the symptom. The defect is that
  a normative document would state a false fact about another stage's mechanics.
  #118's own guard is still worth having, so the reason is restated as defence in
  depth rather than the guard being dropped.
  Two High, both genuine and both fixed: `validate` never cross-checked
  `total_refutation` or `duplicate_groups` against the data they summarise, so a
  totally-refuted run reporting `total_refutation: false` passed every stated rule —
  the exact case the issue's fifth criterion exists to prevent; and the
  out-of-ladder guard watched only re-ratings, so a finding ARRIVING at "Info" that
  the judge agreed with was copied into `severity` untouched.
  Two Low, both fixed: two #119 line citations had rotted the same way the Blocker's
  did and are now by section; and anchor `pr` was #117's High 6, not one of its
  three Blockers.
  serina:review-plan has now run TWICE, and this revision is the result of both. The
  second pass returned two findings — one High, one Medium — and the High was
  against the FIRST PASS'S OWN FIX, which is the argument for the second pass rather
  than a footnote to it.
  Second pass, one High, fixed. Applying the first pass, this plan added a top-level
  `nonce` key on the reasoning that #119's OPEN asks for one, #117 does not emit
  one, and #118 is better placed to supply it. #117 does emit one — though NOT at
  the commit this record originally named, which is the third pass's Blocker below:
  the content was staged and uncommitted when the second pass ran, and landed at
  733f48088 at 09:19:51. Either way the fix was a re-invention of work already
  done upstream, and it would have put a SECOND nonce in one document — a second
  thing to disagree. STEP 4 is now a CHECK on the key it is handed rather than a
  source of one, `adjudication.nonce` is withdrawn (the block is NINE keys, not
  ten — the count itself was got wrong here and is the third pass's first Medium),
  and ALREADY TRUE records six merged-document keys, not five.
  The correction is worth more than the fix. The SEVERITY_ORDER Blocker was true
  when written and falsified 40 minutes later; this one was NEVER true — the #117
  work existed in that worktree before the sentence claiming it did not. This plan's
  own BUDGET says to re-verify #117's field names against what has been committed rather
  than trusting the citations in ALREADY TRUE. The instruction was written, and then
  not followed while applying a review. That is the lesson: the moment a plan is
  MOST likely to assert a stale cross-issue fact is while confidently fixing
  something else.
  Second pass, one Medium, recorded rather than fixed: #119's `stages` contract is
  being revised in its worktree right now — staged but uncommitted at 09:04 — toward
  requiring per-dimension manifest entries this plan does not produce. It is in OPEN
  as the thing to re-check immediately before STEP 4 is built.
  serina:review-plan has now run THREE times, and this revision is the result of all
  three. The third pass returned four findings — one Blocker, one High, two Medium —
  and the Blocker was against the SECOND pass's fix, exactly as the second pass's
  High was against the first pass's. Three for three.
  Third pass, one Blocker, fixed. The second pass's correction cited 9d9dc9065 as
  the commit adding #117's top-level `nonce`. That commit does not contain it:
  `git show 9d9dc9065:…| grep -c 'top-level \`nonce\`'` returns 0 while the working
  tree returned 6. The content was staged and uncommitted, and a grep of the working
  tree was stitched to `git log --oneline -1` into a provenance that never existed.
  It has since landed at 733f48088, 2026-08-13 09:19:51 — verified with `git show`,
  which is now the standing rule in ALREADY TRUE.
  Three rounds, three different ways to get one class of claim wrong: true then
  falsified, never true, and true-but-misattributed. The design did not move for any
  of them, which is the useful part — #118 verifies the nonce it is handed either
  way. What moved each time was a citation, and the rule that came out of it is
  about HOW to cite, not about the nonce.
  Third pass, one High, fixed: STEP 4's prose gave one shared reason to two
  disagreement shapes while its own done-when required them distinguishable. There
  are now three refusals, each with its own reason, ORDERED, with the both-at-once
  input resolved explicitly rather than by whichever branch an implementer writes
  first — the same defect #117's second pass found in its own 401 handling.
  Two Medium, both fixed: the `adjudication` block was called ten keys and
  enumerated nine, a miscount created by withdrawing `adjudication.nonce` — the
  count is now nine and STEP 10 builds one control per key against that list. And
  prohibition 1 was enforced against field NAMES only, leaving `verdict_evidence`,
  `severity_reason` and `notes` free to carry an approving sentence. Inert today
  since #119 renders none of them; the limit is now stated in the contract and OPEN
  makes it a condition on #119 ever rendering one.
  This revision is once-reviewed at the margin, as all three predecessors were: the
  third pass's own fixes are unreviewed. On #117 the passes were still returning
  findings at three. A fourth is available and was not run.
  Then serina:review-code and serina:review-tests after STEP 11, then
  serina:review-adjudicate, then serina:review-final — all BEFORE the push in
  STEP 12, because a review posted after the push only documents what already
  shipped.
  A clean check-plan.sh run is mechanical only. It verifies the single run-marker,
  the step count, a done-when per step, a dependency tag per step, and the six
  named sections. It judges nothing about whether the steps are right, and it
  cannot tell you a step is already done.
  Codex (`codex review --base origin/launchpad`) — HAS RUN ONCE, independent of
  the three serina:review-plan passes above and of the model that wrote and
  revised this plan through them. Five findings; four applied here, one (that
  committing this plan violates AGENTS.md's "stable knowledge belongs in a
  document, active work becomes a GitHub issue" rule) refuted rather than
  applied — `launchpad/plans/` is this project's established, repeatedly-used
  convention for exactly this artefact, used by #116, #117, #119 and #120's own
  plans, none of which that rule was ever read to forbid.
  The two real Blockers: `validate`'s finding_id equality check needed the input
  and output id SETS to catch a drop or an invention, but its declared signature
  took only the output document and `findings_in` is a bare count — a swapped
  id left the count unchanged and invisible. `validate` now takes both
  documents. And an input finding arriving with an already-invalid
  `reported_severity` had no legal value to preserve it as, while the validator
  requires both severity fields be in the ladder — an unsatisfiable combination
  for that one shape of bad input. STEP 3 now validates the input against
  #117's own `findings.validate` before adjudicating anything, so that shape is
  refused wholesale rather than reaching a per-finding fallback with no good
  answer.
  Two real Mediums: one of the four places the SEVERITY_ORDER Blocker (above)
  was fixed missed a fifth — STEP 1's own done-when for what ADJUDICATION.md
  must say, still naming the bare subscript as the reason for a guarantee whose
  actual reason is defence-in-depth. Fixed, and the "four places" claim above is
  now five. And STEP 11's "neuter each check to a constant" proved only that the
  runner can register a hardcoded failure, never that a control catches its
  specific target defect — the same class of gap #109's own sibling codebase
  (`feat/review-agent-untrusted-input`) found and fixed in its own mutation
  harness. STEP 11 now pairs each control with a named, targeted mutation of
  the production code it will eventually test, on that harness's model.

BUDGET  STEP 9 eats the budget. The thing most likely to derail the issue is
  STEP 8, and it is a different risk from the one #117 carried.
  THE PRODUCER OF THIS STAGE'S INPUT DOES NOT EXIST. #117 depended on #120's tree,
  which was at least written and pushed — signatures could be re-verified against
  real code. #118 depends on #117, whose branch carries a PLAN FILE AND NOTHING
  ELSE: no findings.py to import, no run_dimensions.py to run, no dimension that
  has ever emitted a document. Every fixture in STEP 8 is therefore a reading of a
  normative document rather than a capture of real output, and the failure mode is
  invisible from inside the suite — it passes either way. The named mitigation is
  the regeneration in STEP 8's done-when, and it cannot be discharged until #117
  lands.
  The precondition chain is #120 -> #117 -> #118, and it is a precondition rather
  than a preference. STEPs 2 onward require launchpad/review-agent/ present on this
  branch with review.py, contain.py, run_controls.py AND #117's findings.py,
  obtained by those issues merging to launchpad and this branch rebasing — never by
  copying, which creates a second source of truth and a guaranteed conflict when
  they land. Before STEP 2, re-verify review.SEVERITY_ORDER's location and #117's
  field names against what has actually been committed by then rather than trusting
  the citations in ALREADY TRUE. That instruction paid for itself twice on #117,
  where a stale measurement and a moved symbol both invalidated a step's
  specification between drafts.
  STEP 9 is five recordings plus four before-and-after pairs, each needing a real
  run against a real model, and it is the step where "recorded from a real run"
  quietly becomes "hand-written to look like one". STEP 3 decides its cost: if the
  judge is not an injected callable — if a model call is hardcoded at its call site
  — then STEP 9 stops being recording and becomes a rewrite of STEPs 3, 6 and 7.

OPEN  Not for a builder to decide.
  IF #119 EVER RENDERS `verdict_evidence`, THE ESCALATE-ONLY GUARANTEE NEEDS A
  SECOND MECHANISM. Prohibition 1 binds field names and enumerated values; the free
  strings are held only by the prompt in STEP 5. That is sound while #119 renders
  none of them and unsound the day it renders one, because an approving sentence
  inside `verdict_evidence` publishes as this stage's own words. The two available
  answers are both unattractive — a keyword filter over model prose is the narrow
  guard this agent exists to find elsewhere, and a second model judging the first is
  another near-chance judge. Whoever asks #119 to render the field owns choosing.
  THE VERDICT WORD REACHES NO PUBLISHED REVIEW UNTIL #119 RENDERS IT. #119's
  generic path renders `defect` and `failure` and nothing else — its own review's
  finding 7 records the same loss for containment `entry_point` and `evidence`. So
  `verdict` and `verdict_evidence` are computed here and dropped there. The
  re-rating IS visible, because it occupies `severity` and #119 sorts and headings
  by it, which is why that field placement was chosen. Whether #119 should render
  the verdict beside each finding is a change to #119's STEP 4, not a #118
  deliverable, and it needs asking of whoever owns it.
  CONTAINMENT.MD PRESCRIBES A CALL THIS STAGE CANNOT MAKE. Its stage table binds
  #118 to `contain.findings_for(surfaces, nonce)`, which needs the Surface dict and
  the nonce; this stage receives a JSON document. #117 already places exactly what
  that function returns into `containment.findings`, so calling it again would mean
  re-fetching the surfaces — a second source of truth for one fact, and a second
  reason to touch author text. The proposed amendment is that the row reads
  "consume #117's `containment` block verbatim, or `contain.findings_for` when
  adjudicating without a #117 document". Amending CONTAINMENT.md belongs to whoever
  owns #120.
  WHETHER SEVERITY MAY BE LOWERED AT ALL. This plan allows it with a reason and a
  recorded downgrade, because #118's issue says an overstated true finding is its
  own error. The opposite reading is defensible: a near-chance judge that can push
  a real Blocker down the sort order has made an approval-ward move, and
  "escalate, never approve" could be read as forbidding any fall. The downgrade
  list exists so that choice can be revisited without rebuilding the stage.
  WHERE THE ADJUDICATION JOB RUNS. #110 decided GitHub Actions with `pull_request`,
  and that is settled. What is not: whether #117, #118 and #119 are three jobs
  piping to each other in one workflow, or three workflows exchanging artefacts.
  #119 adds its own workflow because it needs pull-requests write and #120's
  controls workflow must not have it. #118 needs neither write nor, in the piped
  reading, any workflow file of its own. This plan adds none; if the artefact
  reading wins, one is needed and this plan does not specify it.
  #119'S `stages` CONTRACT IS BEING REVISED RIGHT NOW, AND STEP 4 BUILDS AGAINST IT.
  `git status --short` in feat-review-agent-publish shows its plan and its review
  record both STAGED-MODIFIED and a second-pass review file untracked, at
  2026-08-13 09:04. The second pass reports that revision requires per-dimension
  entries in the manifest — entries neither #117 nor this plan produces today.
  Building against unlanded work is not available to this plan and predicting it is
  not either, so STEP 4 is written against #119's committed text (47482549e) and
  this is the item to re-check before STEP 4 is built rather than after. Whoever
  sequences the fleet decides whether #118 waits for #119's revision to land.
  THE ALL-FORGED-NONCE RUN IS UNCOVERED ACROSS ALL THREE STAGES. #117 checks its
  markers against its own top-level key, #118 checks the same relation again, #119
  checks reports against each other — and every one of those passes when the whole
  document agrees on a nonce an attacker chose. Only the stage holding what
  `contain.make_nonce` actually returned can tell, and no stage carries it forward
  out of process. Whether that matters depends on whether a document can reach
  #119 without having come from #117 in the same run, which is a property of the
  Actions topology in OPEN's job question, not of any one stage.
  THE CONTRACT IS CONSUMED UNILATERALLY AND ITS SOURCE IS STILL MOVING. #117's
  contract is at revision 4, was revised three times in two days, and lives in an
  unmerged plan file. If it changes again, this plan's STEP 2 validator, STEP 8
  fixtures and STEP 9 recordings change with it. #117's own OPEN invites #118 to
  object; this plan does not object, and that acceptance is itself the decision
  being recorded here.
  WHETHER THE JUDGE SHOULD BE A DIFFERENT MODEL FROM THE DIMENSIONS' REVIEWER.
  #109's evidence is about judges over adversarial claims; a judge that is the same
  model as the claimant shares its blind spots. Model choice is out of scope for
  this issue and the runner names none, so the question is recorded rather than
  answered — it belongs with whoever settles the model for #117.
  #118'S OWN ISSUE BODY STILL CARRIES THE WORDING #122 CORRECTED. #109 was amended
  in place; #118 was not. This plan states the corrected version and never the
  quoted phrase that is not in the paper. Amending the issue is a one-comment fix
  and belongs to whoever owns #118.
  THE ISOLATED ADJUDICATOR METRIC. #118's last criterion says its output is the
  raw material for #109's isolated-adjudicator signal, and #122's verification 3
  establishes that none of the three published architectures checked reports such a
  metric — while also finding that "no published system reports it" is stronger
  than the three-system probe supports. This plan emits the raw material
  (`verdict_counts`, `downgrades`, `duplicate_groups`) and computes no metric.
  Whether #121 or #109 owns the computation is not decided here.

LEFT OUT  Deliberately excluded.
  FINDING NEW DEFECTS. #118's issue puts it out of scope. A genuinely new defect
  the adjudicator notices goes to `adjudication.notes` and never into
  `reports[].findings`, so it cannot displace the dimensions' work, cannot be
  counted as one of their findings, and carries no severity that enters #119's sort.
  DECIDING WHETHER THE PR MERGES. Phase 1 publishes evidence; humans decide. The
  record cannot express an approval, by construction rather than by convention.
  PUBLISHING. #119 owns it. This stage prints JSON to stdout and posts nothing.
  ADJUDICATING CONTAINMENT FINDINGS. They are deterministic catches at a severity
  CONTAINMENT.md fixes, and a judge able to refute one could erase a detected
  attack. They pass through untouched.
  RE-READING RAW PR TEXT. CONTAINMENT.md forbids it by name. This stage reads the
  findings and the contained document, and the repository at head_sha where a
  finding is anchored — never the author's prose as a fresh surface.
  CHOOSING THE MODEL. Out of scope per the issue's framing and #117's. The runner
  takes an injected judge and names no model; STEP 9's recordings carry a model id
  as provenance for a measurement already taken, which is not the same as choosing.
  MEASURING ADJUDICATOR PRECISION AND RECALL. #121 owns the first ten reviews and
  #109's success signals. STEP 9 produces one sample per exclusion, which is
  provenance for the falsifiability criterion and explicitly not a rate.
  ACCESSIBILITY is out of scope for this issue and is not claimed. The deliverable
  is a definition plus a CLI printing JSON to stdout — no UI, no interactive
  control, nothing to announce, no focus to manage. When #119 renders these
  verdicts into a PR comment, that surface is markdown read by GitHub's own
  interface; a rendered dashboard, if one ever follows, needs its own keyboard and
  announcement specification and does not inherit one from here.
