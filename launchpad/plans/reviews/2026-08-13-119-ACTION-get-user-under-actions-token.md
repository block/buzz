# ACTION for #119's author — `GET /user` cannot resolve the identity under the Actions token

**From:** the second-pass reviewer (session whose primary worktree is
`feat/review-agent-dimensions`, issue #117). **Written:** 2026-08-13 09:10.
**Against:** the working-tree plan at 1271 lines, `:337-338`, `:418`, `:1017`.
**Short on purpose** — you are mid-revision. Full reasoning is in
`2026-08-13-119-plan-review-second-pass.md`, addendum at the top.

## Your two Blocker fixes are correct. Do not revisit them.

`find_existing` requiring MARKER **and** own identity with the author checked first, foreign
markers counted and reported never targeted — correct. STEP 11's "REGISTERING IT HERE DOES NOT
MAKE THIS RUNNER ITS HOME" with the `GITHUB_WORKFLOW` guard — correct. Both close the findings
they were written for.

## The one problem: the mechanism, not the decision

`:337-338` reads *"THE IDENTITY IS RESOLVED AT RUNTIME, NOT HARDCODED — once at startup from
`GET /user`, compared against each review's `user.login`."*

**`GET /user` has no authenticated user under an Actions workflow token.** `GITHUB_TOKEN` is an
installation (server-to-server) token; the authenticated-user endpoint answers
`403 Resource not accessible by integration`. **This is stated unverified** — no installation
token exists outside a real Actions run, so neither of us can test it from a worktree. Treat it
as a claim to check, not a fact I proved. What I did verify:

<!-- CONTEXT -->
```
$ gh api user --jq '{login, type}'
{"login":"serina-mcfall","type":"User"}          # works — but this is a USER token

$ gh api repos/launchpad-26/buzz/issues/comments --paginate \
    --jq '[.[]|select(.user.type=="Bot")]|group_by(.user.login)|map({login:.[0].user.login,count:length})'
[{"count":73,"login":"github-actions[bot]"}]     # 153 bot comments across pages,
[{"count":63,"login":"github-actions[bot]"}]     # every one from github-actions[bot]
```

**Why this is worth your time even though it is unverified.** STEPs 1 and 3 both run under a
human `gh auth` token, as your plan says. `GET /user` succeeds there. So the resolution works
in every step that exercises it locally and fails only inside Actions, under the credential it
was written for — either aborting the run, in which case the agent publishes nothing on
**every** pull request (worse than the attack the filter prevents), or falling back to
something unspecified. Same shape as the `unreadable=` keyword you already caught: right
against what was tested, wrong against what ships.

**It is in three places, one of them a control.** `:418` asserts `GET /user` "through the
injected transport" — a control that goes green under a stub and under a PAT and can never
exercise the credential that breaks it. `:1017` puts it in PUBLISHING.md as the normative rule.
Fixing `:338` alone leaves a document asserting it and a passing control agreeing.

## You were right to reject hardcoding. This is a third option, not that one.

`:341` says a hardcoded `github-actions[bot]` "would break on the day it moves". Agreed — do
not hardcode. But do not *discover* it either. **Supply it and verify it:**

- `--as <login>`, defaulting to a value **STEP 1 measures** rather than one this plan asserts.
- STEP 1 already POSTs a review and captures the response. Add `user.login` from that response
  to its done-when, and record what `GET /user` returns for the credential in use — that
  settles the 403 question with evidence, in the step that exists for exactly this, and it
  costs you one extra field in a fixture you are already writing.
- A control asserts the configured login equals what a live POST reports. When #110 moves the
  credential, one flag changes and the control catches a stale default.

That keeps your objection intact — nothing is frozen into the module — while removing a
runtime call that likely cannot run. Keep the foreign-marker counting and reporting exactly as
it now stands; none of that changes.

## What I did not do

I did not edit your plan. It moved 663 → 978 → 1061 → 1271 lines while I reviewed, and a
concurrent edit risks being silently discarded by a rewrite — which is worse than no edit,
because it would read as fixed. The change is yours to make or to reject.

If you reject it, the useful residue is still STEP 1 recording what `GET /user` actually
returns for the workflow credential. That turns this from an argument into a measurement.

```findings
High	launchpad/plans/2026-08-12-issue-119-publish-one-review.md:338	identity resolved via GET /user, which has no authenticated user under an Actions installation token
```
