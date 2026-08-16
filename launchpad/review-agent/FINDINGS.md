# FINDINGS.md — the review agent's output contract

Normative, and a sibling to `CONTAINMENT.md` in the same voice. This is the contract
[#118](https://github.com/launchpad-26/buzz/issues/118) (adjudication) and
[#119](https://github.com/launchpad-26/buzz/issues/119) (publish) are both blocked on.
[#117](https://github.com/launchpad-26/buzz/issues/117)'s own issue says the contract is
"agreed in whichever of the two lands first, then honoured by the other" — #117 is
planning first, so it is settled here in enough detail that #118 and #119 implement
against it without renegotiating.

**Follow-up owed to `CONTAINMENT.md`, not yet paid.** `CONTAINMENT.md`'s own "Contract
for later stages" table lives on `feat/review-agent-untrusted-input`
([#120](https://github.com/launchpad-26/buzz/issues/120)), which is committed but
unmerged and not reachable from this branch — `launchpad/review-agent/` does not exist
here except for this file. This document cites that table's content directly (the
severity ladder, the seven entry-point labels, the `contain.render` signature) because
those facts are already true on #120's pushed commits, but it cannot add the reciprocal
row that table calls for, because editing a file that is not on this branch would create
the second source of truth this project's own convention forbids. The reciprocal edit —
a row or note in `CONTAINMENT.md` pointing back at this document — is owed once #120
merges to `launchpad` and this branch rebases onto it. Until then the two documents point
one way only, named here so it is a tracked debt and not a quiet divergence.

## Severity

Imported, not redefined: `Blocker | High | Medium | Low`, from `review.py`'s
`SEVERITY_ORDER`. A second copy of a four-value ladder drifts, and the containment
findings that share the output stream are already fixed to `Blocker` by
`CONTAINMENT.md` § Severity contract. No dimension, and no stage that consumes this
document, restates the ladder as its own — it is looked up, never copied.

## Anchoring — how a finding carries file:line

The part most likely to be got wrong. Three anchor kinds, one required field naming
which applies, and validation that refuses the mismatched combinations:

| anchor | `file` | `line` | renders as |
|---|---|---|---|
| `line` | required | required | `launchpad/AGENTS.md:42` |
| `file` | required | **must be null** | `launchpad/AGENTS.md` |
| `pr`   | **must be null** | **must be null** | `(pull request)` |

The rule exists because the alternative is a reviewer inventing line 1 for a finding
that is really about a whole file or the PR as a whole. That is false precision, and
#118 would then try to verify a defect at a location where it is not — producing a
REFUTED verdict for a finding that was true. An anchor field makes "this defect has no
line" expressible instead of forcing a lie.

**The anchor is not a free choice.** Structural validity is not appropriateness: anchor
`pr` with both fields null satisfies every rule in the table above for ANY finding, so
it is the cheapest thing an uncertain reviewer can emit, and it is unfalsifiable by
construction — #118 cannot refute a defect at a location it was never given. Left
unconstrained, a dimension may report every finding as anchor `pr`, pass every
structural rule here, and still satisfy none of #117's second done-criterion, which
requires file:line. So:

- a defect visible at a line of the merge-base diff **must** use anchor `line`;
- a defect that is a property of a whole file **must** use anchor `file`;
- anchor `pr` is legitimate **only** where the defect has no file — a missing file, a
  claim in the PR body, a property of the change as a whole.

Each dimension definition states this per its own scope, and the control suite checks it
against planted defect locations, because a rule with no control is guidance and this
one is load-bearing.

`line` is a **new-side** line number in the merge-base diff, i.e. of the file at
`head_sha`. Old-side and new-side numbers differ in every diff that adds or removes
lines above the finding, so the side is stated, not assumed.

## The finding record

Ten fields, and the count is load-bearing — the control suite builds one control per
field, so a field that exists in the output but not in this list gets no control at all.
An earlier draft of this contract mapped an eleventh value (`evidence`) in a conversion
while declaring only nine fields, which would have either dropped the excerpt or grown
an undeclared field that no control exercised. There are exactly ten:

| field | meaning |
|---|---|
| `dimension` | which reviewer produced it — one of the three dimension slugs |
| `severity` | `Blocker \| High \| Medium \| Low` |
| `anchor` | `line \| file \| pr` |
| `file` | repo-relative path, or null when anchor is `pr` |
| `line` | new-side line number, or null unless anchor is `line` |
| `defect` | ONE line: what is wrong |
| `failure` | the concrete failure the defect allows |
| `finding_id` | stable id, see below |
| `entry_point` | **required** on any finding whose defect is an injection attempt — one of `CONTAINMENT.md`'s seven labels, naming the surface the text came from. Null on every other finding. |
| `evidence` | **required** whenever `entry_point` is set, **raw** — the excerpt the finding rests on, exactly as the author wrote it, not escaped. Null elsewhere. |

`entry_point` is required rather than optional because an injection finding that names
no surface is the detected-then-dropped case `CONTAINMENT.md` calls worse than never
detecting it, and a field that is merely "optional" is a field a converter drops without
failing any rule. The cross-cutting injection clause each dimension carries is what
produces these, for the paraphrase cases the deterministic detector in #120 misses.

`evidence` is raw, not post-escape, because escaping belongs to the renderer:
`review.py`'s `render_review` applies `contain.escape` at render time to a raw-evidence
record, and every `contain.Finding` in the system already carries raw evidence. A record
that arrives pre-escaped is escaped twice by that renderer — a `~` publishes as `~~~~` —
so the excerpt stops matching what the author wrote. `evidence` is null everywhere else:
a dimension's finding is already located by file and line, and quoting the diff back
adds bulk without adding evidence.

`defect` and `failure` are two fields, not one, because #117's done-criteria name them
separately, and because a defect statement with no stated consequence is what lets an
unfalsifiable finding through. #118 re-rates severity, so the reporting dimension's
value must remain readable after adjudication rather than being overwritten in place.

### `finding_id`

A truncated hash of `(dimension, anchor, file, line, entry_point, defect, evidence)`. It
is stable across re-runs of an unchanged diff, which is what lets #118 attach exactly
one verdict per finding and makes its dedupe visible rather than silent. It is **not**
stable across a model rewording `defect` — stated plainly here because a reader will
otherwise assume it is, and because dedupe across rewordings is #118's job, not this
id's.

`entry_point` and `evidence` are both hash inputs. Two findings that differ only by
which surface they came from, or only by the excerpt they rest on, are two findings —
`contain._dedupe` keys its own identity on exactly `(kind, entry_point, evidence)` for
that reason. Without them, two injection findings from the same dimension whose
`defect` line happens to match — the same paraphrase caught in `pr_body` and in
`pr_diff` — would hash to one id, and #118 would either attach one verdict to two
attacks or dedupe by id and silently drop one. Dropping one is the detected-then-dropped
case again, arriving through the id rather than through a missing field.

## The report envelope

One per dimension, eleven fields:

| field | meaning |
|---|---|
| `schema_version` | integer, starts at 1 |
| `dimension` | the slug |
| `pr` | number |
| `merge_base_sha` | the commit pair this report read |
| `head_sha` | the commit pair this report read |
| `status` | `complete \| failed` |
| `outcome` | `findings \| clean` — only when `status` is `complete` |
| `error` | `{reason}` — only when `status` is `failed` |
| `findings` | array |
| `findings_count` | integer, must equal `len(findings)` |
| `completion_marker` | **last** key emitted, see below |

`outcome: clean` is how "a dimension that finds nothing says so explicitly" is
distinguished from `status: failed`. Both are legitimate outputs; neither is an empty
`findings` array standing alone, which is exactly the ambiguity the criterion forbids.

`findings_count` must equal `len(findings)`. A report truncated mid-array fails that
equality even if it somehow still parses.

## The merged document

What the concurrent runner prints on stdout — the whole of this contract's outer shape:

| key | meaning |
|---|---|
| `pr` | number |
| `merge_base_sha` | the commit pair every report read |
| `head_sha` | the commit pair every report read |
| `reports` | array of exactly the dimension envelopes above, one per slug the runner lists, and nothing else in it |
| `containment` | the block specified below — findings plus a seven-key `states` map. Present on every run. |
| `nonce` | the run nonce, once, at the top level |

### Where containment findings live

Settled here, normatively. Containment findings do **not** enter the `findings` array of
any dimension report, and they are **not** converted into the ten-field record above.
They travel as a top-level sibling key of the merged document, named `containment`,
carrying `contain.Finding` verbatim in JSON:

- `containment.findings[]` — `severity`, `kind`, `entry_point`, `evidence`: the four
  fields of the `contain.Finding` dataclass, raw and unrenamed. `kind` is one of
  `delimiter_forge`, `delimiter_lookalike`, `injection_attempt`.
- `containment.states` — a map of **all seven** entry points to their `fetch.Surface`
  state (`ok | empty | absent | oversized | unparseable`). All seven, always, including
  the ones that succeeded.

Three reasons, and the first alone decides it:

1. `review.render_review` — the function `CONTAINMENT.md`'s stage table binds #119 to
   call — reads `.severity`, `.kind`, `.entry_point` and `.evidence` **by attribute** off
   a `contain.Finding`, and takes a second argument `states` that no stage currently
   emits. A converted ten-field record has no `kind`, so it cannot reach the published
   review through that function at all, and the conversion would strand exactly the
   findings `CONTAINMENT.md` § Severity contract requires to appear.
2. Escaping: `review.py`'s `render_review` applies `contain.escape` at render, so
   evidence must arrive raw or it is escaped twice.
3. #118 does not need the conversion — `CONTAINMENT.md`'s stage table already routes it
   to `contain.findings_for(surfaces, nonce)` for exactly this data, so converting would
   give #118 two sources for one fact.

`states` is load-bearing and is the easiest thing here to get wrong. It feeds
`render_review`'s "Incomplete" banner, which is **derived** from it against
`review.py`'s `UNREADABLE_STATES` rather than passed in. A `states` map populated only
for the surfaces that succeeded makes every unreadable surface read as
absent-from-the-map, and the banner never renders — a review over three unreadable
surfaces publishing as complete. Hence "all seven, always", and hence a control that
counts the keys rather than merely checking that the key exists.

There is no reserved `"containment"` dimension slug, and no dimension report carries a
containment finding. A sibling key separates the deterministic catches from the model's
judgement more cleanly than a slug inside a shared array, and because a dimension report
whose `findings` array held a containment finding would have to declare
`outcome: "findings"` and so report a dimension as having found something it did not.

### The nonce — why it is a top-level key, and what it does not do

Without it the nonce would exist only inside each report's `completion_marker` string,
so a downstream stage could compare markers to each other and nothing more. #119 needs
to know when a report's marker "carries the wrong nonce" — a condition its own input
cannot support unless the document itself states the nonce it should match against.
This key is what makes that check implementable, and it is the reason it exists.

It is **not** an authentication token for the document: anyone fabricating the whole
document sets this key and the markers consistently, and no value carried in plaintext
beside the thing it authenticates can prevent that. What it catches is the threat the
marker was designed against — a forged marker **copied out of the author's diff** by a
reviewer that echoed it — because that marker carries whatever nonce the author typed
and not this run's. Stated plainly because "the document now carries a nonce" reads
like a stronger claim than it is, and an overclaimed control is worse than an absent
one.

**The trust boundary, named rather than assumed.** #117, #118 and #119 run in one CI
job, so the merged document does not cross an untrusted boundary. The untrusted text is
the model output each dimension returns, and that is where the marker check bites. If a
later phase ever moves a stage to a separate job or a separate machine, this key stops
being sufficient and the boundary needs signing, not a shared plaintext value.

The nonce must **not** be rendered into the published review body. #119 has no use for
it beyond the check, and printing a per-run secret into a public comment teaches a
reader it is not one. Leaking it after the run is harmless — it is fresh per run and
that run has ended — but there is no reason to.

A **committed recording**'s nonce must be seed-derived via `contain.make_nonce(seed)`
rather than copied from a live run, so a fixture value is never mistaken for a
production one.

**One nonce per run, not per dimension** — all three reports embed the same value. So
"carries another dimension's nonce" is not an input this contract can produce, and a
check written against that phrasing tests nothing. The checks that do bite are: a marker
whose nonce differs from the merged document's `nonce`, and markers that disagree with
each other. Both are stated here because a downstream stage phrasing the condition the
other way would build a fixture no run can generate.

### The completion marker

Carries the run nonce. Its value is `BUZZ-DIMENSION-COMPLETE:{dimension}:{nonce}`, using
the same nonce `contain.render` wrapped the surfaces with, and it is the **last** key
emitted in a report.

Two reasons, and the second is not obvious: a marker at the end cannot survive
truncation, so a report cut off mid-`findings` has no marker and is treated as truncated
rather than clean; and a marker with no nonce is a fixed string published in a public
repository, which a PR author can type into their own diff. A reviewer reading a
contained diff containing a forged marker could emit it, and a naive scan of model
output would then read a truncated report as complete. The nonce makes the marker
unforgeable by anyone who has not seen it, on the same reasoning `CONTAINMENT.md` gives
for the envelope delimiter. `contain.make_nonce` is reused, including its refusal to
accept a caller-supplied nonce.

## Contract changes since revision 3

For #118 and #119 to diff against. #119's plan is committed against revision 3 of this
contract and names its field list explicitly, so this section exists so its author can
diff old against new without re-reading a full review. Six changes:

1. Containment findings are a top-level `containment` sibling key carrying raw
   `contain.Finding` (`severity`, `kind`, `entry_point`, `evidence`) plus a seven-key
   `states` map. Revision 3 converted them into ten-field records under a reserved
   `"containment"` dimension slug. **That slug is withdrawn.**
2. `evidence` is **raw**, not post-escape. Revision 3 said post-escape. #119's control
   comparing against `contain.escape(evidence)` is correct against this revision and was
   wrong against revision 3.
3. `entry_point` is **required** on an injection finding, not optional, and `evidence`
   is required with it. Revision 3 made both droppable.
4. `finding_id` hashes `(dimension, anchor, file, line, entry_point, defect, evidence)`.
   Revision 3 hashed `(dimension, anchor, file, line, defect)`. #119 uses `finding_id`
   only as a sort tie-break, so this costs it nothing, but #118 must use the new inputs.
5. There are ten finding fields, and `evidence` is the tenth. #119's plan states "the
   envelope carries neither `kind` nor `evidence`" — the `kind` half stays true (`kind`
   lives on the containment block, not the finding record), the `evidence` half does
   not.
6. The merged document carries a top-level `nonce` key. Revision 3 carried the nonce
   only inside each report's `completion_marker`, so a check for "a report's
   `completion_marker` carries the wrong dimension or nonce" was unimplementable from
   its input. #119 should add `nonce` to its own input document as a sixth key and check
   each marker against it. Two corrections come with it: the nonce is **one per run**,
   so "another dimension's nonce" is not an input this contract can produce and a
   fixture built on that phrasing tests nothing — check a marker against the document's
   `nonce` instead; and the key is not an authentication token for the document, only
   for a marker echoed out of author text, so it must not be described as making the
   document unforgeable.

The ten finding fields and eleven envelope fields are otherwise unchanged, and no field
is renamed.
