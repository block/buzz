# serina:review-plan — #119, second pass (independent)

**Plan reviewed:** `launchpad/plans/2026-08-12-issue-119-publish-one-review.md` **as committed
at `8a405a9f5`** — 663 lines, the revision whose last commit message is *"match #119's plan to
render_review's current signature"*. Line citations in the findings below are against THAT
text. Current-revision line numbers are given separately for the findings that survive.

**Who ran this:** a session whose primary worktree is `feat/review-agent-dimensions`
(issue #117). Second pass, run after and independently of
`2026-08-13-119-plan-review.md`, which was deliberately **not read** until the findings below
were finalised. Reviewed: 2026-08-13.

**Independence — declared, because it is partial.** I am not independent of the *contract* this
plan consumes. Earlier in the same session I reviewed #117's plan and then **fixed** it,
including settling where containment findings live. The first pass's Blocker 1 was withdrawn
*because of the settlement I wrote*. Treat my reading of anything touching the #117 interface
as interested. Findings 1, 2 and 6 — the three that survive — are independent of that
interface and do not depend on it.

**Format gate:** `check-plan.sh` on the reviewed revision →
`Mechanical checks: clean. Your call on the substance.`

**Severities are proposals.** `serina:review-adjudicate` re-rates.

**LINE PINS IN THIS FILE ROT, and did so while it was being written.** The plan went 663 →
978 → 1061 lines during this pass, across commit `47482549e` plus further uncommitted edits.
The three surviving findings were re-verified against the 1061-line text and their
current-revision pins are given as of that. Every finding below therefore names the RULE and
the SYMBOL it lands on, not only a number — cite those, and re-locate the number. Both this
plan and #117's record the same lesson about pinned citations; this file is a demonstration
of it rather than an exception to it.

---

## URGENT — read this first

The plan was being revised against the FIRST PASS ONLY while this pass ran. Of my seven
findings, **three survive that revision and two of them are Blockers.** They survive because
they were not in the first pass, so nothing prompted the reviser to look at them.

Worse: **the first pass's fix for its own finding 10 made my Blocker 1 strictly easier to
exploit.** That is the single most important line in this document.

| Finding | Severity | In first pass? | Status in the in-flight revision |
|---|---|---|---|
| 1. `find_existing` matches attacker-writable text | Blocker | **No** | **SURVIVES — and WORSENED** |
| 2. Live scope control probes the wrong token | Blocker | **No** | **SURVIVES** |
| 3. `SEVERITY_ORDER[...]` subscript | High | Yes (#2) | Fixed |
| 4. `states`-names-all-seven in no done-when | High | **No** | Fixed anyway (now condition 10) |
| 5. Pagination control cannot fail | High | Yes (#5) | Fixed, better than I proposed |
| 6. No control asserts a clean run POSTS | High | **No** | **SURVIVES** |
| 7. Incomplete-trigger count contradicts itself | Medium | Yes (#8) | Fixed |

---

## Blocker 1 — `find_existing` matches attacker-writable text, with no author filter

**Reviewed revision:** `:195-199`. **Current revision (1061 lines):** `:287` — the
`find_existing` NEWEST rule; the marker-not-author justification in STEP 12; the
failed-PUT-raises rule in STEP 2. Re-verified: no `user.login`, `author_association` or
`GET /user` check appears anywhere in the plan.

**Defect.** Identification is `body.startswith(MARKER)` with no author filter, chosen
deliberately so the lookup survives the credential's identity changing. But a review body is
not agent-controlled territory: any GitHub user with read access can submit a `COMMENT`
review on an open pull request in this public fork, and its body is theirs to write.

Verified — the listing already carries everything a filter needs:

<!-- CONTEXT -->
```
gh api repos/launchpad-26/buzz/pulls/86/reviews --jq '.[0] | keys'
["_links","author_association","body","commit_id","html_url","id","node_id",
 "pull_request_url","state","submitted_at","user"]
```

**Concrete failure.** Anyone submits a COMMENT review on the PR whose body's first line is
`<!-- launchpad-review-agent:v1 -->`. `find_existing` returns that review's id.
`post_or_update` issues a PUT against a review the token does not own, GitHub refuses it, and
the current revision makes the consequence deterministic at `:303-308`: *"A FAILED PUT IS A
HARD FAILURE AND NEVER FALLS BACK TO POST… post_or_update raises… and the workflow fails
loudly with no review updated."* So the agent publishes **nothing**, permanently, on any pull
request an attacker chooses — and the failure presents as an HTTP error, not as an attack.
#119's own criterion calls silence "indistinguishable from a crashed agent"; this makes that
silence available on demand.

**Why the revision made it worse.** The reviewed revision took the **OLDEST** marked review,
which at least required the attacker to plant *before* the agent's first run. The in-flight
revision takes the **NEWEST** (`:265`, `:269-274`) to fix the first pass's finding 10. With
NEWEST there is no timing requirement at all: a review planted at any moment is immediately
the newest, so it wins every subsequent run, forever. The first pass's diagnosis for its
finding 10 was correct — keeping the oldest current leaves an undeletable stale newest — but
its fix optimised one failure mode into another, because it was not weighing an adversary.

**Fix, satisfying both concerns at once.** Filter by author FIRST, then take the newest among
the agent's own: match on marker **and** `user.login` equal to the identity the token
authenticates as, resolved once at startup from `GET /user` rather than hardcoded — which
preserves exactly the credential-portability property the marker-over-author choice was made
for. Report the count of *foreign* marked reviews rather than treating them as candidates.

---

## Blocker 2 — the live credential control probes the wrong token, and passes either way

**Reviewed revision:** `:406`, `:465`. **Current revision (1061 lines):** `:809` — STEP 11's
`("check_publish_scope.py", True)` entry; STEP 9's live half, its "outside Actions" SKIP guard
and its PASS-with-403 done-when. Re-verified: no `GITHUB_WORKFLOW` check appears anywhere in
the plan, so nothing distinguishes which workflow executes the live half.

**Defect.** STEP 9's live half attempts a contents write "with the workflow's own token" and
asserts 403. STEP 11 registers it as `("check_publish_scope.py", True)` in
`run_controls.py` — which is invoked by **#120's controls workflow**, not the publish
workflow. Verified:

<!-- CONTEXT -->
```
$ grep -nE 'permissions|contents|pull-requests|issues|run_controls' \
    .github/workflows/launchpad-review-agent-controls.yml
23:permissions:
24:  contents: read
25:  issues: read
26:  pull-requests: read
48:        run: python3 run_controls.py
```

The token under test is therefore `{contents: read, issues: read, pull-requests: read}` — a
read-only token whose lack of contents-write was never in question. Both it and the publish
token carry `contents: read`, so the ref-create returns 403 and the control reports PASS
regardless of what the publish workflow's permissions block says.

The SKIP guard does not save it. `:655` reads *"Outside Actions there is no workflow token, so
the live half reports SKIP"* — but inside #120's controls workflow there IS a workflow token.
It is simply the wrong one, so the guard never fires and the control PASSes.

**Concrete failure.** `:664` asks for *"a real Actions run on this pull request shows the live
half reporting PASS with the 403 response body pasted into the PR"*. That PASS and that 403
body get pasted into the pull request as evidence about the publish credential, having
measured a different credential entirely. #119's criterion — *"A control or documented check
demonstrates the absence of contents write"* — is reported satisfied and is not. The static
half does catch a later widening of the publish workflow's YAML, so this is not a total blind
spot; it means the live half, which STEP 9 calls "the only step that can demonstrate #119's
credential criterion", demonstrates nothing.

**Fix.** Run the live half as a step **inside** the publish workflow, where `GITHUB_TOKEN` is
the credential in question. Additionally have it assert *which* workflow it is running in
(`GITHUB_WORKFLOW`) and SKIP with a reason — never PASS — anywhere else, which is the rule
`run_controls.py` already enforces for missing inputs. Registering it in the shared runner
without that guard guarantees a PASS on the wrong token.

---

## High 6 — no control asserts that a clean run actually posts

**Reviewed revision:** `:349`. **Current revision (1061 lines):** `:787` — STEP 10's
assertion (v), still "two bodies differ"; STEP 6's `grep -n "return None" publish_render.py`
done-when. Re-verified: no assertion anywhere drives `post_or_update` on a clean input.

**Defect.** #119's criterion is *"A run that produced no confirmed findings still posts,
saying so explicitly. Silence is indistinguishable from a crashed agent."* The plan offers
`grep -n "return None" publish_render.py` as proof of "no early return on the clean path".
That check fails twice over: it greps the **pure renderer**, whereas the decision to call
`post_or_update` lives in `publish.py`'s `main`; and it cannot fail for the right reason,
since a function whose contract is `-> str` never returns `None` in any correct
implementation. It passes for every implementation, correct or not.

STEP 10's assertion (v) — still unchanged at `:704` in the current revision — asserts only
that "a clean input and an incomplete input both produce a body, and the two bodies differ".
That is a rendering assertion. None of the now-seven assertions drives `post_or_update` on a
clean input. STEP 7's done-when exercises `--dry-run`, which by definition posts nothing.
STEP 3 does post, but with a stub body, before any findings logic exists.

**Concrete failure.** An implementer adds `if not findings and not incomplete: return 0` to
`publish.py`'s `main` — a reasonable-looking optimisation against the edit-event noise the
plan itself raises in OPEN. Every control in the plan still passes. The agent goes silent on
exactly the pull requests where it found nothing, and a silent agent is indistinguishable
from a crashed one, which is the criterion.

**Fix.** Add an eighth STEP 10 assertion with its own mutation: an all-clean input drives
`post_or_update` through the injected transport, and the transport records exactly one call.
Mutation: early-return on the clean path in `publish.py`'s `main`. That puts the assertion in
the file where the risk lives, and STEP 10 already has the transport seam to do it.

---

## Findings 3, 4, 5 and 7 — fixed in the in-flight revision

Recorded so the diff is complete, not because action is needed.

- **3 (High) — `review.SEVERITY_ORDER[finding["severity"]]` bare subscript** raising `KeyError`
  and posting nothing, where `review.py:62` deliberately uses `.get(f.severity, 9)`. Fixed:
  STEP 4's done-when now asserts a finding with `severity: "Info"` renders under "malformed
  finding", sorts last, does not raise, and triggers the banner. Corroborates first pass #2.
- **4 (High) — `states` must name all seven entry points, prescribed in prose at `:270` and in
  no done-when.** The plan diagnosed the hazard itself and then did not gate it: a thin
  `states` map means `unreadable` derives empty and a review over unreadable surfaces
  publishes as complete — #120's banner-with-no-producer defect relocated one stage up. Fixed:
  now condition (10) of STEP 5's enumerated list. The first pass did not report this; the
  reviser reached it while restructuring, so it is fixed by luck of adjacency rather than by
  either review.
- **5 (High) — the pagination control cannot fail under its own stated mutation.** Verified
  independently that `gh api --paginate` merges pages into one array:

<!-- CONTEXT -->
```
$ gh api "repos/launchpad-26/buzz/pulls/86/commits?per_page=1" --paginate
first 3 chars  : [{"
count of '][' : 0
json.loads OK — single value, type list len 12
```

  So a "recorded two-page listing" is indistinguishable from a one-page listing of the same
  length, and "drop `--paginate`" changes nothing a data-only fixture observes. Fixed, and the
  fix is better than the one I proposed: the transport serves page two **only when
  `--paginate` appears in the argv**, and the fixture is the unmerged page bodies. Corroborates
  first pass #5, which reached the same measurement by a different route.
- **7 (Medium) — the incomplete-trigger count said six, seven, six, seven** across four places,
  with the genuinely missing condition ("a report with `status: complete` and no `outcome`")
  living only in STEP 6's prose. Fixed: ten numbered conditions, counted identically in
  STEPs 5, 6 and 12. Corroborates first pass #8.

---

## What I looked for and did not find

- **A silent #117/#119 disagreement about containment transport.** This was my going-in
  hypothesis and it is **refuted**. OPEN at `:599-611` states it as a hard dependency, names
  the block shape needed, records that *"#117 must add one key to its output before #119 can
  publish a containment finding"*, and names the safe failure until then. That is a plan doing
  its job, and it is why #117's contract could be settled without guessing.
- **`gh api --paginate` emitting concatenated per-page arrays.** Expected; false. It merges.
- **The `8a405a9f5` amendment.** Verified correct against `c64ff7958`:
  `render_review(findings, states) -> str` at `review.py:45`, `SEVERITY_ORDER` at `:32`,
  `UNREADABLE_STATES` at `:42`, attribute reads at `:62-73`.
- **A path to `APPROVE`.** `post_or_update` has no event parameter. Enforced by construction.
- **STEP 8's `permissions`, `concurrency` and `pull_request_target` reasoning.** All correct.
  The group-interpolation check genuinely FAILS a fixed group name rather than passing it, and
  `github.ref` on a `pull_request` event is `refs/pull/N/merge`, so both permitted forms are
  per-PR.
- **A fail-open classifier.** The incomplete default is correct; the scope probe rejects
  success, 404 and rate-limit errors rather than treating any error as proof.

**Two claims left unverified, deliberately.** Whether GitHub refuses a PUT on another user's
review, and whether a PR author may COMMENT-review their own pull request. Both require a
write to a public fork to settle. Blocker 1 does not depend on either: the current revision
specifies that any non-2xx PUT raises, so both branches end in no review being published, and
the attack needs only *some* user, not the author specifically.

**Tools.** Full main-session pool. Used `Read` and `Bash` (`gh`, `curl`, `git`, `grep`, `sed`,
`python3`), plus `Write` for this file. Every behavioural claim above has command output
beside it.

```findings
Blocker	launchpad/plans/2026-08-12-issue-119-publish-one-review.md:287	find_existing matches attacker-writable body text with no author filter, and NEWEST removes the timing requirement
Blocker	launchpad/plans/2026-08-12-issue-119-publish-one-review.md:809	live credential control is registered in the read-only controls runner, so it PASSes against the wrong token
High	launchpad/plans/2026-08-12-issue-119-publish-one-review.md:787	no control asserts a clean run actually posts; the offered grep targets the wrong file and cannot fail
```

REVIEW COMPLETE

---

# ADDENDUM 2026-08-13 09:05 — both Blockers fixed, and the Blocker 1 fix has a defect

Checked against the live text at 1244 lines (HEAD `47482549e` plus uncommitted edits, file
modified seconds before this check). **Both Blockers above are fixed**, independently and
well — `find_existing` now requires MARKER **and** the agent's own identity with the author
checked first, foreign markers counted and reported never targeted; and STEP 11 now states
that registering the scope control in `run_controls.py` "DOES NOT MAKE THIS RUNNER ITS HOME",
with the live half SKIPping there on a `GITHUB_WORKFLOW` guard and its PASS only ever coming
from the publish workflow. Nothing in this file's Blocker 1 or Blocker 2 needs further action.

**One new finding, on the Blocker 1 fix itself.** It reads: *"THE IDENTITY IS RESOLVED AT
RUNTIME, NOT HARDCODED — once at startup from `GET /user`, compared against each review's
`user.login`."*

`GET /user` has no authenticated user under an Actions workflow token. `GITHUB_TOKEN` is an
installation (server-to-server) token, and GitHub answers `403 Resource not accessible by
integration` for the authenticated-user endpoint. **I could not verify that directly — no
installation token is available outside a real Actions run — and it is stated here as
unverified.** What IS verified:

<!-- CONTEXT -->
```
$ gh api user --jq '{login, type}'                      # human PAT
{"login":"serina-mcfall","type":"User"}                  # works, and is a USER token

$ gh api repos/launchpad-26/buzz/issues/comments --paginate \
    --jq '[.[]|select(.user.type=="Bot")]|group_by(.user.login)|map({login:.[0].user.login,count:length})'
[{"count":73,"login":"github-actions[bot]"}]             # 153 comments across pages,
[{"count":63,"login":"github-actions[bot]"}]             # every one of them from
[{"count":17,"login":"github-actions[bot]"}]             # github-actions[bot]
```

So the login the workflow credential posts as is `github-actions[bot]`, confirmed in this
fork. The mechanism prescribed to discover it is the one that most likely cannot run.

**The concrete failure.** STEP 1 and STEP 3 both run under a human `gh auth` token, which the
plan says explicitly. `GET /user` succeeds there, so identity resolution works in every step
that exercises it locally. It fails only inside Actions, under the credential it was written
for — either aborting the run, in which case the agent publishes nothing on **every** pull
request (worse than the attack the filter was added to prevent), or falling back to something
the plan does not specify. A fix that passes every local control and breaks only under the
real credential is the same shape as this plan's own `unreadable=` keyword defect: correct
against the thing that was tested, wrong against the thing that ships.

**It is now in three places, one of them a control.** `:337-338` prescribes the mechanism;
`:418` asserts it — *"`GET /user` through the injected transport"* — which is a control that
passes under an injected transport and under a human PAT and can never exercise the credential
that breaks it; and `:1017` puts it in STEP 12's normative PUBLISHING.md, *"resolved at runtime
from `GET /user` and never hardcoded"*, which is where a future reader would learn it as the
rule. Fixing the prescription without fixing those two leaves a document asserting it and a
green control agreeing.

**Change.** Do not discover the login; supply it and verify it. An `--as <login>` argument
defaulting to a value STEP 1 **measures** — STEP 1 already POSTs a review and captures the
response, so add `user.login` from that response to its done-when, and record what
`GET /user` returns for the credential in use so the next reader does not have to guess.
That answers the objection the marker-over-author choice was originally made for: when #110
moves the credential, one flag changes, and a control asserts the configured login equals what
a live POST reports. Keep the foreign-marker reporting exactly as it now stands.

```findings
High	launchpad/plans/2026-08-12-issue-119-publish-one-review.md:338	identity resolved via GET /user, which has no authenticated user under an Actions installation token
```

---

# Diff — this pass against `2026-08-13-119-plan-review.md`

The first pass reported 14 findings (1 Blocker, 6 High, 6 Medium, 1 Low) against `eb2bf09d0`.
This pass reported 7 against `8a405a9f5`. Both gates clean.

## Corroborated by both passes

Independent agreement, reached by different routes. These were the strongest signals and all
three are already fixed.

| Both found | First pass | This pass |
|---|---|---|
| Bare `SEVERITY_ORDER[...]` subscript posts nothing on an out-of-ladder severity | #2 High | #3 High |
| Pagination assertion cannot fail — `gh --paginate` merges pages | #5 High | #5 High |
| Incomplete-trigger count contradicts itself; the seventh lives only in STEP 6 | #8 Medium | #7 Medium |

Worth noting: both passes measured the `--paginate` merge independently — the first with
`per_page=2` across 9 pull requests, this one with `per_page=1` across 12 commits. Same
conclusion. That is the kind of claim a single pass could have got wrong from memory, and two
passes measuring it is the reason to trust it.

## Found only by this pass — the actionable residue

All three survive the in-flight revision, because nothing prompted the reviser to look.

1. **Blocker — `find_existing` matches attacker-writable text.** The first pass touched this
   code twice (#10 on oldest-versus-newest, #11 on unspecified PUT failure) without treating
   the body as adversarial. Both of its fixes have now landed and **both increase the
   exposure**: NEWEST removes the timing requirement, and raise-on-failed-PUT makes the denial
   deterministic.
2. **Blocker — the live scope control probes the wrong token.** Neither pass's territory
   overlapped here; the first pass reviewed STEP 9's internals (#9, the `${{ }}` literal) but
   not which workflow ends up executing it.
3. **High — no control asserts a clean run posts.** The first pass accepted STEP 6's
   `grep "return None"` at face value.

## Found only by the first pass

Ten findings, all already applied. Three deserve calling out because a reader of this file
should not conclude the first pass was the weaker one — it was not, it was broader:

- **#3 (High) — the nonce half of the completion-marker check is unimplementable**, because no
  nonce reaches `publish.py`. I missed this entirely, and it is the best finding in either
  pass: it identifies a check that *cannot* be built from the input the contract supplies.
  **It also lands on #117, and see the cross-issue item below.**
- **#13 (Medium) — STEP 1 records four API responses and never records whether a body edit is
  visible to a human.** The plan's entire re-review strategy is a body edit, and this is the
  one behavioural assumption STEP 1 does not capture. I did not look for an absent
  observation; I was looking for wrong ones.
- **#6 (High) — the stdin document supplies no `repo`** while both lifecycle functions require
  one. A plain interface mismatch I walked straight past.

The rest: #1 (withdrawn, below), #4 (`unreadable` TypeError — already fixed by `8a405a9f5`
before my pass began, which is why I saw a correct signature), #7 (containment evidence
dropped and unfenced), #9 (`${{ github.run_id }}` in Python), #10 (oldest duplicate stale),
#11 (unspecified PUT failure), #12 (STEP 10's fixtures produced by no step), #14 (circular
import).

## Where the two passes disagree

Four disagreements. Stated as judgements, not averaged.

### 1. First pass #10 — oldest versus newest. **I think its fix is wrong.**

Its diagnosis is right: keeping the oldest marked review current leaves an undeletable newest
one permanently stale, and a reader reaching the bottom of the timeline sees the stale one.
Its fix — keep the newest current — is wrong on its own, because it hands an attacker a
lookup with no timing requirement (Blocker 1). The two concerns are not in tension once the
author filter exists: **filter to the agent's own reviews, then take the newest among them.**
The first pass changed the ordering without adding the filter, which is the one combination
that is worse than either concern alone.

### 2. First pass #9 — `${{ github.run_id }}` inside a Python module. **It is right; I was wrong to dismiss it.**

I saw it, judged it already documented, and did not report it, on the grounds that BUDGET
already names *"a 404 where a 403 was expected because the ref path was wrong"*. That was a
bad call. BUDGET documents the risk **class**; the plan then prescribes a specific literal
that instantiates it. A documented hazard is not a documented instance, and the skill's own
rule — a limitation the plan already documents is not a finding — does not stretch to cover a
concrete defect that happens to fall inside a named risk area. Concede.

### 3. First pass #1 — containment inside `reports`. **Correctly withdrawn, and I am not a neutral judge of that.**

Its premise was that #117 had moved containment inside `reports`. #117 settled the opposite
way, normatively: a top-level `containment` sibling key carrying raw `contain.Finding` plus a
seven-key `states` map, never inside a dimension's findings array. The plan's separate block
was right all along.

**Disclosure: I wrote that settlement.** Its withdrawal is a consequence of my own work on
#117 earlier in this session, so my agreement that it was correctly withdrawn is not
independent evidence. What *is* independent: the withdrawal's residue was real and the first
pass caught it — ALREADY TRUE claimed a nine-field record with no `evidence`, and the record
has ten with `evidence` as the tenth. That half of the finding held regardless of which way
the transport settled.

### 4. Severity of first pass #7 versus its post-settlement form. **Its re-derivation is right and mine would have been too narrow.**

The first pass rated it High for containment evidence dropped by the generic render path. After
the settlement dissolved the containment half, it **re-derived it larger** rather than
withdrawing it: `evidence` is raw on the ten-field record too, `entry_point` is required on
injection findings, and #117's cross-cutting clause makes all three dimensions emit them — so
raw attacker text arrives via the ordinary findings path and needs `review.fence_for`. That is
the correct move, and it is a better piece of reasoning than anything in my pass. I did not
find this at all.

## The union, ranked

Everything outstanding after the in-flight revision, most urgent first. Items 1–3 are mine and
unaddressed; item 4 is the first pass's, partly addressed and flagged upstream.

1. **Blocker — `find_existing` has no author filter** (`:287` at 1061 lines). Fix before
   anything else: it is the only defect here an outside party can trigger deliberately, it
   costs one comparison against data already fetched, and the revision's NEWEST change made it
   cheaper to exploit. Fixing it also completes first pass #10 properly.
2. **Blocker — the live scope control probes the read-only controls token** (`:809`). The
   credential criterion is currently reported satisfied by a measurement of the wrong
   credential.
3. **High — no control asserts a clean run posts** (`:787`). One STEP 10 assertion plus its
   mutation.
4. **High, cross-issue — the nonce is verifiable by no downstream stage** (first pass #3,
   marked "partly fixed, flagged upstream"). See below; the fix is in #117, not here.

Items already fixed and needing nothing: my 3, 4, 5, 7 and the first pass's 2, 4, 5, 6, 8, 9,
10, 11, 12, 14, with 1 withdrawn and 13 converted to an observation.

## Cross-issue action for #117 — raised against my own work

The first pass's finding 3 is a real gap in the contract I settled this session, and it should
not be left sitting in #119's OPEN.

#117's merged document is `{pr, merge_base_sha, head_sha, reports, containment}` — **no
`nonce` key.** The run nonce exists only embedded inside each report's
`completion_marker` (`BUZZ-DIMENSION-COMPLETE:{dimension}:{nonce}`, #117 STEP 1). So a
downstream stage can check that every marker *agrees* on a nonce — which catches one forged
marker among several — but it cannot check that the agreed nonce is the one actually generated
for the run. An all-forged run is undetectable downstream, which is precisely the forgery the
nonce was introduced to defeat.

#119's revision handles this as well as it can from where it sits, and says so. The clean fix
is a `nonce` key on #117's merged document, checked against each marker. **That is a change to
#117's plan, which this pass did not make** — writing into another issue's plan on the strength
of my own review is exactly the loop this second pass exists to break.
