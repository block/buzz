NIP-AD
======

Agent Disposition
------------------

`draft` `optional` `relay`

This NIP defines a durable, plaintext event kind for recording how an AI
agent resolved one human→agent request: `completed`, `refused`, `responded`,
or `errored`. An agent (or its harness) publishes one `kind:44300` event per
disposition, signed by the agent's own key, so any authorized reader of the
channel — not just the requester or the agent's owner — can verify that a
request was answered and read why it was refused, without parsing chat
bodies or trusting unverifiable prose.

**One request, one obligation, one answerable agent.** A disposition is only
meaningful if "which obligation does this discharge, and who was obliged?"
has exactly one answer. This NIP therefore defines an *obligation* as the
unit of accounting — a marked request naming exactly one target agent — and
binds every disposition to the obligation's target by signing key. A marked
request that does not name exactly one reachable target is a malformed
request, reported as a protocol fault rather than as an agent's unanswered
gap.

**Scope of "readable."** Kind 44300 is not specially gated: it carries no
owner-only, author-only, or p-gated restriction, and is plaintext rather
than encrypted (the deliberate inverse of [NIP-AM](NIP-AM.md)). But reads
still inherit ordinary channel access like every other channel-scoped kind.
An unauthenticated process on the open internet cannot audit a private
channel's dispositions; an external auditor must authenticate as a member of
that channel (or the channel must be one it can already read). "Verifiable
by anyone who can see the conversation" is the accurate claim.

## Motivation

Buzz agents answer requests inline, as ordinary chat messages. That answer is
never distinguishable from chat at the protocol layer: a reader must parse
every message body and guess which ones are dispositions, there is no kind
filter and no relay-enforced shape, and a plain "thanks!" is indistinguishable
from an unanswered request. No third party can currently verify that an agent
answered every request directed at it, or recover a refusal's stated reason
without reading transcript prose.

[NIP-AM](NIP-AM.md) (kind 44200) already gives owners durable, harness-independent
accounting for *token usage* — but it is deliberately the opposite of this NIP
in every property that matters here: encrypted rather than plaintext, and
owner-gated rather than readable by the whole channel, because token cost is
private and a disposition's entire purpose is the reverse — verifiability by
anyone who can see the conversation.
Kind 44300 stores only the disposition: which request it answers, what
happened, and (for `refused`) why.

## Definitions

- **Agent**: an AI process with its own Nostr keypair, participating in a
  Buzz channel.
- **Requester**: the principal (human or agent) whose message triggered the
  agent's turn.
- **Request**: a channel message carrying `["t","request"]`, addressing an
  agent and asking it to act.
- **Obligation**: a *valid* request — one naming exactly one target agent
  that is also `p`-mentioned — together with the identity of that agent.
  The obligation, not the raw request, is the unit this NIP accounts for.
  A marked request that is not a valid obligation is an **invalid request**
  (see "Request validity"), which is a fault of the emitting client, not an
  unmet duty of any agent.
- **Disposition**: a single kind 44300 event recording how the agent resolved
  one request — `completed`, `refused`, `responded`, or `errored`.
- **Binding**: the consumer-side check that a disposition actually
  discharges a given obligation. An unbound disposition contributes nothing
  to that obligation's state.
- **Unsupported request**: a *well-formed* marked request expressing an
  intent v1 cannot represent — currently only a request naming several
  agents. Distinct from an invalid one: an invalid request is a bug to fix,
  an unsupported one is a feature to build. Both block a clean claim; only
  one implies anybody did anything wrong.

## Event

`kind:44300` is a regular event by Buzz convention (alongside
44100/44101/44200): stored and never replaced. It is the deliberate inverse
of kind 44200 in visibility: plaintext content, no encryption, readable by
any authorized reader of its channel rather than only the agent's owner — a
disposition that could be silently overwritten would defeat the point of a
verifiable record.

**"Append-only" is a property of the kind, not a retention guarantee.**
Being a regular (non-replaceable) event means no later event can overwrite
a stored disposition in place. It does not mean history is permanent:
[NIP-09](09.md) deletion applies as normal, and relay retention policy can
make older events unavailable. A verifier that needs to prove nothing was
removed needs deletion tombstones or Buzz's hash-chain audit log — neither
is part of this NIP's query contract. Read a clean disposition history as
"nothing contradicts this," not "nothing was ever removed."

```json
{
  "kind": 44300,
  "pubkey": "<agent_pubkey>",
  "created_at": 0,
  "content": "{\"disposition\":\"refused\",\"reason\":\"outside my delegation\"}",
  "tags": [
    ["e", "<request_event_id>"],
    ["h", "<channel_uuid>"],
    ["p", "<requesting_principal_pubkey>"],
    ["disposition", "refused"]
  ],
  "sig": "..."
}
```

Events MUST have exactly one `e` tag (the request event id), exactly one `h`
tag (the channel UUID), exactly one `p` tag (the requesting principal), and
exactly one `disposition` tag whose value is exactly one of `completed`,
`refused`, `responded`, `errored`. The `disposition` tag is lifted out of `content` onto
the tag list specifically so a reader can determine state directly from tags
— without parsing any event body — for every event already fetched by
another filter. It is NOT itself a server-side query filter: see "Not a
query filter" below.

### v1 scope: channel-scoped requests only

The `h` tag is REQUIRED, not optional. Buzz's channel-scoping model is a
strict binary: a kind is either global-only (no `h` tag; channel identity, if
any, stays out of tags entirely) or channel-required (`h` mandatory). Direct
messages already use a distinct kind family (`KIND_DM_OPEN` and friends)
rather than an optional `h` tag on a shared kind, so there is no existing
"conditionally channel-scoped" precedent for this NIP to extend. Dispositions
for DM-scoped requests are out of scope for this version; a future revision
may add a distinct mechanism if real usage demands it.

Exactly one `h` tag is enforced by kind 44300's own ingest validation, not
merely by the shared channel-scope check. That shared check only asks "is
there a usable `h` tag?" and silently takes the first — so a second,
different `h` would scope storage and authorization to one channel while
the other still rode along on the stored event for any generic tag
matching. For a kind whose entire purpose is unambiguous attribution, that
cross-channel ambiguity is worth rejecting outright, even though it makes
44300 stricter than other channel-scoped kinds.

### Not a query filter: `#disposition`

`#e` and `#h` are ordinary NIP-01 single-letter tag filters and work exactly
as expected in REQ subscriptions and the HTTP query bridge. `disposition` is
NOT: NIP-01's `#<tag>` filter grammar is defined only for single-character
tag names, and the `nostr` crate Buzz builds on types `Filter.generic_tags`
as a map keyed by exactly one character — its deserializer silently drops
any `"#<word>"` key that isn't a single character rather than rejecting it.
A client that sends `{"kinds":[44300],"#h":[...],"#disposition":["refused"]}`
expecting the relay to filter by state gets back **every** disposition in
the channel with no error — a silent over-return, not a filtered result.
Consumers MUST treat `disposition` as a tag to read after fetching (state is
always visible on every returned event) and MUST NOT send `#disposition` as
a filter key expecting server-side narrowing.

## Content

`content` is a plaintext (never encrypted) UTF-8 JSON object:

```jsonc
{
  "disposition": "refused",              // REQUIRED: must equal the `disposition` tag
  "reason": "outside my delegation",      // REQUIRED string; MAY be empty for any state
  "request_id": "<request_event_id>"      // OPTIONAL; if present, MUST equal the `e` tag
}
```

`disposition` and `reason` are REQUIRED. `reason` MUST be present as a
string, but MAY be an empty string for ANY disposition state, not only
`completed` — the relay's schema guard accepts `""` uniformly across all
four states. In practice a meaningful `reason` is expected for `refused`
(a refusal record with no stated reason defeats much of its own purpose),
but this is a convention for emitters to follow, not a protocol-enforced
requirement. Omitting the `reason` field entirely is invalid, not
equivalent to an empty string. `request_id`, when present, is a redundant
self-check: consumers MUST reject a disposition whose `content.request_id`
disagrees with its own `e` tag, since agreement is exactly the invariant a
verifier depends on.

Consumers MUST ignore unknown fields (forward compatibility).

## Lifecycle

The four states divide into **terminal** and **non-terminal**:

| State | Terminal? | Means |
|---|---|---|
| `completed` | yes | The agent asserts it did what was asked. |
| `refused` | yes | The agent declined, and says why. |
| `responded` | **no** | The agent produced an answer but does not claim the request is settled. |
| `errored` | **no** | The turn failed; the request may still be retried and resolved. |

An obligation is **resolved** only in a terminal state. `responded` exists
because most agent turns genuinely end this way: the agent replied, and
whether that reply actually satisfied the request is not something the agent
can honestly assert on its own behalf. A harness that mapped every
successful turn to `completed` would be publishing signed claims of success
it has no basis for — the ledger's value depends on `completed` meaning
something, so an emitter MUST NOT emit `completed` merely because a turn
ended without error. Emitting `responded` is the honest default; `completed`
is reserved for cases where completion is actually established (for example,
a requester or a workflow confirms it).

**Terminal claims are absorbing, and an obligation's outcome is distinct
from its latest observation.** Consumers MUST derive two separate things:

- **Effective outcome** — `unanswered`, `open(state)`, `settled(state)`, or
  `disputed`. This is the only basis for any display or accounting decision.
- **Latest observation** — the last bound state by `(created_at, id)`,
  retained for audit. It answers "what arrived last", which is a different
  question from "what is true", and MUST NOT be used to decide whether an
  obligation is done.

The rules, in order:

1. Both terminal states bound → `disputed`. There is no settled answer.
2. Otherwise, a terminal claim → `settled` with that state, **regardless of
   anything ordered after it**.
3. Otherwise, anything bound → `open` with the latest state.
4. Otherwise → `unanswered`.

Rule 2 is what makes "terminal" mean terminal. Deriving state as "latest
wins" — as an earlier version of this NIP did — let a late `errored` silently
reopen a settled obligation, which contradicted this document's own claim
that `completed` and `refused` are terminal.

Unbound dispositions are not part of the history at all (see "Target-agent
binding").

### Known limitation: terminal claims cannot be corrected in v1

Absorption makes `completed` and `refused` **irrevocable**, not merely
"not reopened by a weaker observation." Nothing in v1 can express "the earlier
claim was wrong; this is the corrected result." Three consequences, stated
plainly because a reader will otherwise discover them the hard way:

- **Premature completion.** An agent emits `completed`, then finds the
  operation rolled back or never committed. A later `errored` is absorbed; the
  obligation stays settled as completed. This bites hardest precisely because
  a disposition is self-reported (see Security Considerations) — the protocol
  already admits terminal claims may be mistaken, and then gives no way to fix
  one.
- **A wrong or missing refusal reason.** Re-emitting `refused` with better
  text is only a `duplicate_terminal` warning; the settled reason comes from
  whichever terminal sorts first. And because `(created_at, id)` is
  publisher-supplied rather than causal, a later-published correction can even
  be backdated into that position — which is a way to *replace* a reason, not
  a way to correct one honestly.
- **Switching between terminals.** The only expression available is the
  opposing state, which produces a permanent `disputed` outcome rather than a
  correction.

[NIP-09](09.md) deletion is not an adequate substitute: this NIP already
states that deletion and retention can make history unavailable and that the
query contract carries no tombstone-completeness guarantee.

**Why v1 ships without a correction protocol.** A correction relation — for
example a `["correction", <prior-event-id>]` tag, same-signer, same-obligation,
acyclic, with only unsuperseded terminal leaves determining the outcome — is a
genuine protocol extension with its own authority and ordering questions. It
is the right shape for v2, and inventing it inside a fix pass is how the
earlier defects in this document got here. v1 therefore states the limitation
and makes terminal claims expensive to emit: the agent-facing instruction says
`completed` and `refused` are final, and only the target agent can emit either.

Consumers MUST NOT present a settled obligation as correctable, and clients
SHOULD make emitting a terminal state a deliberate act rather than a default.

**Warnings are diagnostic and never change an outcome.** Two exist:

| Warning | History | Meaning |
|---|---|---|
| `duplicate_terminal` | the **same** terminal state bound more than once | Redundant, usually a retry that re-published. The outcome stands. |
| `ordered_after_terminal` | a **non-terminal** disposition sorts after a terminal one | A stale or late weak observation. The settled result stands. |

They are deliberately non-overlapping: a terminal after a terminal is already
fully described by `duplicate_terminal` (same state) or by a `disputed`
outcome (opposing states).

`ordered_after_terminal` is named for what it can actually establish. The
ordering is deterministic but publisher-supplied, so no consumer can show
that anything was *written after* settlement — only that it sorts later. An
earlier name (`post_terminal_write`) asserted a causal fact the algorithm
cannot observe; a real causal claim needs a trusted receive sequence or an
attempt ordinal.

An earlier version made every warning force the obligation out of its settled
state and render as "disputed", which defeated the point of categorizing
them: a duplicate delivery and a genuine contradiction produced identical,
equally destructive results.

**Same-second ties.** `created_at` is whole-second Nostr precision. Two
dispositions for the same request published within the same second are
tied — "latest" is then undefined by timestamp alone. Consumers MUST break
such a tie by event `id` (lexicographically greatest), so current-state
derivation is a deterministic function of the stored events rather than of
arrival or query-result order. This is the same class of ambiguity
[NIP-AM](NIP-AM.md) resolves with an explicit `(sessionId, turnSeq)`
ordinal; that mechanism is disproportionate here — a real turn retry takes
materially longer than one second, so the tie case is a synthetic/adversarial
edge rather than a normal-operation one, and a tiebreaker is enough to make
behavior well-defined without new required fields.

The complete transition table. **This table is generated** from the same
declarative rules that produce the conformance corpus
(`scripts/gen-nip-ad-corpus.mjs`), and CI fails if it drifts. An earlier
hand-written version claimed a terminal history carrying a warning was
resolved while every implementation said the opposite, and nothing caught
it — the corpus pinned the two implementations to each other, never to this
document.

Rows cover every distinct (outcome, warning-set) pair the rules can produce.

<!-- BEGIN GENERATED: transition-table -->
| History (ordered) | Effective outcome | Resolved? | Warnings |
|---|---|---|---|
| (nothing bound) | unanswered | no | — |
| completed | **settled** (`completed`) | yes | — |
| refused | **settled** (`refused`) | yes | — |
| responded | open (`responded`) | no | — |
| errored | open (`errored`) | no | — |
| completed → completed | **settled** (`completed`) | yes | `duplicate_terminal` |
| completed → refused | **disputed** | no | — |
| completed → responded | **settled** (`completed`) | yes | `ordered_after_terminal` |
| refused → refused | **settled** (`refused`) | yes | `duplicate_terminal` |
| refused → responded | **settled** (`refused`) | yes | `ordered_after_terminal` |
| completed → completed → refused | **disputed** | no | `duplicate_terminal` |
| completed → completed → responded | **settled** (`completed`) | yes | `duplicate_terminal`, `ordered_after_terminal` |
| completed → refused → responded | **disputed** | no | `ordered_after_terminal` |
| refused → refused → responded | **settled** (`refused`) | yes | `duplicate_terminal`, `ordered_after_terminal` |
<!-- END GENERATED: transition-table -->

- **errored → completed** and **responded → completed** are normal repair:
  the one class of legal post-hoc transition, legal precisely because
  `errored` and `responded` are non-terminal. Readers report the obligation as
  settled, not as ever having been a gap.
- Repeated `errored` or `responded` with no eventual terminal state is an
  open, unsettled obligation — visible as such, not a warning.
- A warning never changes an outcome. A duplicated `completed` is still
  settled; only two *opposing* terminal claims produce `disputed`.

Note what this ordering does and does not give you. `(created_at, id)` makes
current-state derivation a deterministic function of the stored events, but
it is not *causal* ordering — event ids carry no turn semantics, and a
disposition published later can carry an earlier `created_at`. A protocol
needing true attempt ordering would need an explicit ordinal (as
[NIP-AM](NIP-AM.md) uses `(sessionId, turnSeq)`); this NIP deliberately does
not. Terminal absorption is what keeps the weaker ordering from silently
producing a wrong settled answer: no ordering accident can unsettle an
obligation, because nothing ordered after a terminal claim replaces it.

## Relay Behavior

On receiving a kind 44300 event, a relay MUST:

1. Validate the event signature per NIP-01.
2. Validate the tag envelope: exactly one each of `e`, `h`, `p`, and
   `disposition`; `e` and `p` are 64 lowercase hex characters; `disposition`
   is one of the four valid values.
3. Validate `content`: valid JSON object; `disposition` field present and
   equal to the tag; `reason` field present and a string; if `request_id` is
   present, it equals the `e` tag.
4. Reject the event (do not store it) if any of the above fail.
5. Enforce the same channel-membership/access check applied to other
   channel-scoped writes (the publisher must have write access to the `h`
   channel).
6. Store the event durably, scoped to the channel, with the `disposition`
   tag preserved on every read so consumers can determine state without
   parsing `content` (see "Not a query filter" above — this is a per-event
   tag read, not a server-side filter).
7. Never gate reads beyond ordinary channel membership — kind 44300 is
   deliberately absent from every read-gating list (result-gated, author-only,
   p-gated). Any member of the channel — not only the requester or the
   agent's owner — can read it.

**Structural validity is a single shared contract, and consumers MUST apply
exactly the same one.** Every rule in "Event" and "Content" above is checked
by one validator that relay ingest itself calls; no event may contribute to
any obligation without passing it. This is normative because the alternative
already happened: the relay enforced kind, exact tag cardinality, and a
required `reason` while consumer-side binding checked none of them — it read
the first matching tag and ignored the rest — so an event with two `e` tags,
or no `reason`, or not even of kind 44300 was rejected at ingest and
simultaneously counted as a settling disposition by an auditor. A consumer
that verifies signatures but not structure is not protected: a valid
signature can cover a perfectly well-signed non-disposition.

The relay does NOT verify that the referenced `e` tag actually names a real
request that mentioned this agent, and does NOT verify that the `p` tag
matches the request event's actual author. Those checks are left to
consumers (see Security Considerations) — the relay's job is structural
validity and storage, not judging whether a disposition is honest.

## Client Behavior

Any client recovers a channel's dispositions with:

```json
{"kinds": [44300], "#h": ["<channel_uuid>"]}
```

or narrow to one request's history with `#e` (both valid server-side
filters). To isolate one state (e.g. only refusals), fetch the `#h`- or
`#e`-scoped set and filter locally on each event's `disposition` tag — do
not send `#disposition` as a filter key (see "Not a query filter" above).
Clients pair a request to its disposition(s) by matching the disposition's
`e` tag to the request's own event id, and derive current state as described
in Lifecycle.

### Target-agent binding

The relay validates a disposition's tag *shape* (Relay Behavior above) but
does NOT verify that its signer was actually asked — that check is left
entirely to consumers (see Security Considerations). Without it, any
channel member with ordinary write access can publish an honestly-signed
kind:44300 event naming someone else's request by `e` tag, and a consumer
that groups purely by `e` tag would render it as that request's
resolution — a real cross-principal spoof, not merely the "an agent can lie
about its own work" limitation the rest of this NIP already accepts.

**Requests name their target.** A marked request carries exactly one
`["agent", <pubkey>]` tag naming the agent it is addressed to, alongside its
`["t","request"]` marker, and `p`-mentions that same agent:

```json
{
  "kind": 9,
  "content": "@bugbot triage this crash",
  "tags": [
    ["h", "<channel_uuid>"],
    ["t", "request"],
    ["agent", "<bugbot_pubkey>"],
    ["p", "<bugbot_pubkey>"],
    ["p", "<a_cc_d_human_pubkey>"]
  ]
}
```

Marker and target come from the same value at the composer, so a marked
request can never lack a target and a targeted message is never unmarked.

### Request validity

A marked request is a valid **obligation** only if all of the following
hold. A request failing any of them is an **invalid request** with the
stated reason:

Classification is **total**: every event lands in exactly one of
`NotRequest`, `Invalid(reason)`, `Unsupported(reason)`, or `Valid(obligation)`.
Consumers MUST NOT pre-check the marker themselves and classify second — an
earlier version required that, and Buzz's own agent harness grew a looser
predicate instead, acting on requests every reader called invalid.

**Invalid** (malformed — a client bug):

| Reason | Condition |
|---|---|
| `UnsupportedKind` | The event's kind is not in the v1 request-kind set. |
| `MissingChannel` | No `h` tag. |
| `MultipleChannels` | More than one `h` tag, so the scope is ambiguous. |
| `MissingAgentTarget` | No `agent` tag — nothing was addressed for action. |
| `DuplicateAgentTarget` | The same `agent` target repeated. |
| `MalformedAgentTarget` | The `agent` value is not 64 lowercase hex characters. |
| `TargetNotMentioned` | The target is not also `p`-mentioned. |

**Unsupported** (well-formed, unrepresentable in v1):

| Reason | Condition |
|---|---|
| `MultipleAgentTargets` | Two or more *distinct* `agent` tags, **each of them canonical and `p`-mentioned**. |

**Precedence: malformed beats unsupported.** Every target is validated for
canonical form and `p`-mention *before* cardinality is judged. A request
naming one real agent and one garbage value is a malformed event, not a
feature this version cannot represent — classifying it as unsupported would
file a client bug under "nobody's fault" and tell the sender their perfectly
reasonable request needs a future protocol version. Where several faults
coexist, the first in the table order above is reported.

`TargetNotMentioned` matters because `p` is what actually routes a message
to a principal: an `agent` tag without the matching `p` names a target that
will never be asked, creating an obligation nobody could ever discharge.

`MultipleChannels` mirrors the same rule kind 44300 applies to itself: one
channel would govern authorization while the other still rode along for
generic tag matching.

`DuplicateAgentTarget` is rejected rather than folded into one target. This
document promises exactly one `agent` tag, and an implementation that
silently accepted two made that promise false — canonical events keep
independent implementations and audit output simple.

**The v1 request-kind set.** A marker is only meaningful on kinds this
version accepts requests on (currently kind 9 alone). Without a fixed set,
two consumers querying different kinds produce different accounting for the
same channel and both look correct — the CLI queried only kind 9 while the
desktop timeline carried several message and job kinds. Every query builder
and every classifier MUST derive its candidate universe from the same set.

**Invalid requests are protocol faults, not agent gaps.** Consumers MUST
report them in a separate class from unanswered obligations, and MUST NOT
count them against any agent. Filing a malformed request as "the agent
failed to answer" blames an agent for a client bug and manufactures a gap
that no agent could ever close — the accounting would never come clean, and
the one number a reader cares about would be permanently wrong.

**Why exactly one target in v1.** A request naming two agents has no single
answer to "who was obliged?" Two natural readings — either target may
discharge it, or both must — cannot be distinguished from the event alone,
and an implementation that picks one silently gets the other case wrong. An
earlier draft of this NIP gave such a request one request-wide state, which
produced a concrete defect: two agents each correctly reporting `completed`
were read as contradictory terminal claims, so *two correct answers*
rendered as a conflict. Rather than encode a guess, v1 classifies
multi-target requests as invalid. A future revision may key obligations
`(request_id, agent_pubkey)` to support them properly.

**The binding rule.** Consumers MUST NOT treat a disposition as discharging
an obligation unless all of the following hold:

- the request is a valid obligation (table above);
- the disposition's `e` tag equals the request's `id`;
- the disposition's `h` tag equals the request's channel;
- the disposition's `p` tag equals the request's author;
- the disposition's `pubkey` equals the obligation's target agent.

A disposition failing any of these MUST be excluded from that obligation's
derived state entirely — not rendered as a distinguishable "foreign" or
"unverified" state, simply treated as if it did not exist, so an unbound
disposition can never make an unanswered obligation look answered.

Binding is against the `agent` tag, deliberately **not** `p` tags. The
target is also `p`-mentioned, but `p` additionally contains humans CC'd on
the message; binding to the mention set would let any of them close the
agent's obligation with a signed `completed`. That is a cross-principal
spoof, categorically worse than the "an agent can lie about its own work"
limitation this NIP already accepts — an agent vouching for itself is at
least the party that was asked.

### Gap detection

A disposition alone cannot tell a reader whether *every* request got
answered — only whether the requests it already knows about did. To make gap
detection exact rather than inferred, requests self-identify with
`["t", "request"]` plus their `agent` targets (set by the composer when a
message mentions an agent). A verifier then computes:

- requests = `{"kinds": [<message kind>], "#h": ["<channel>"], "#t": ["request"]}`
- answers = `{"kinds": [44300], "#h": ["<channel>"]}`, paired by `e` tag
  **and filtered by the binding rule above** — a disposition whose signer
  isn't one of its request's `agent` targets does not close the gap

and sorts every marked request into exactly one of six buckets:

| Bucket | Meaning |
|---|---|
| `settled` | Valid obligation with a bound terminal claim. |
| `open` | Valid obligation, answered non-terminally (`responded`/`errored`). |
| `unanswered` | Valid obligation with no bound disposition at all. |
| `disputed` | Valid obligation with both terminal states bound. |
| `invalid_requests` | Malformed marked request, with its reason. |
| `unsupported_requests` | Well-formed marked request v1 cannot represent, with its reason. |

Consumers SHOULD additionally report `rejected_claims`: stored dispositions
that named an obligation but did not bind. They contribute nothing to any
outcome — that is the whole point of binding — but hiding them entirely
denies an auditor sight of spoof attempts.

Without the marker, gap detection could only fall back to noisy inference
(e.g. "any message that mentions an agent"), which miscounts plain
acknowledgments as unanswered requests. Without the binding filter, anyone
with channel write access could close a gap they were never asked about.
Without the `invalid_requests` bucket, client bugs would be laundered into
agent failures.

**Coverage must be explicit, and it has two sides.** "Zero unanswered
requests" is bounded by the queries that produced it. Neither this NIP nor
the relay provides a completeness token, so a consumer MUST carry a coverage
record alongside its accounting:

| Field | True only when |
|---|---|
| `requests_complete` | Every marked request in scope was fetched. |
| `dispositions_complete` | Every disposition for those requests was fetched. |

**Both are required.** A single flag over the request set is not enough: a
consumer could paginate every request, fetch one page of dispositions, see a
`completed`, miss the later `refused` on page two, and truthfully set a
one-sided flag while reporting a disputed obligation as settled.

A consumer may report "everything is settled" only when both coverage bits
are true **and** `open`, `unanswered`, `disputed`, `invalid_requests`, and
`unsupported_requests` are all empty. Omitting the last two from that
conjunction is the subtle error: a channel whose only problem is a malformed
request would otherwise claim a clean bill of health while carrying a
request no agent can ever discharge.

Combined with the retention caveat under "Event", the honest reading of a
clean result is "nothing unanswered among the requests I could see," not
"nothing unanswered ever happened here."

### Conformance

The classification, binding, and lifecycle rules above are normative and
executable: `docs/nips/nip-ad-conformance.json` holds the shared case
corpus, and both Buzz implementations (Rust in `buzz-core`, TypeScript in
the desktop client) run it as a test. A change that makes one implementation
disagree with this document fails that suite rather than drifting silently.
Independent implementations are encouraged to run the same corpus.

## Relationship to Other NIPs

- **The 43xxx agent job protocol** (`KIND_JOB_REQUEST` 43001 through
  `KIND_JOB_ERROR` 43006, defined in `buzz-core/src/kind.rs`) is a distinct,
  explicit contract that a caller opts into for one-shot structured jobs; as
  of this writing it has no publishers or consumers in the Buzz tree and is
  undocumented in `docs/nips/`. This NIP does not use it and does not require
  it. A disposition annotates an *ordinary conversational request* — the
  request itself stays whatever kind it already was (typically a channel
  message); kind 44300 is layered on top, not a replacement for the job
  protocol's request/result exchange. A future NIP may formalize 43xxx and
  its own relationship to dispositions; until then, treat them as unrelated.
- [NIP-AM](NIP-AM.md): the closest sibling kind by number and by "one event
  per agent action" shape, but opposite on every visibility property —
  encrypted vs. plaintext, owner-gated vs. channel-readable, usage accounting
  vs. accountability record.
- [NIP-09](09.md): deletion semantics apply as normal; an agent or relay
  policy may request removal of a disposition it published.

## Migration from kind:9 dispositions

Before this NIP, tooling recorded dispositions as ad hoc JSON bodies inside
ordinary `kind:9` chat messages — functional, but unfilterable and
unenforced. Existing kind:9 dispositions remain valid history and are NOT
retroactively converted, and this implementation does NOT read them: v1
ships a **relay-first** rollout contract, not a dual-write/fallback one.

- A relay MUST enumerate kind 44300 (accept and serve it) before any
  emitter (harness or CLI) is enabled against it. A relay that does not yet
  enumerate 44300 rejects it at ingest with `restricted: unknown event
  kind` (the relay's default-deny for unlisted kinds); emitters do not
  detect this and fall back to kind:9 — they simply fail to publish until
  the relay is upgraded. Operators MUST upgrade the relay before enabling
  NIP-AD emission.
- `buzz dispositions list` and the desktop accountability surface read only
  kind 44300 — they do NOT union in kind:9 JSON-body history. A channel
  with dispositions recorded before this NIP shipped will show that older
  history as missing from these views, not merged in.
- A compatible dual-write/union-read migration (emit both kinds, union-read
  both, define precedence when they disagree) is a real, larger protocol
  change, and is deliberately deferred rather than half-implemented: an
  earlier draft of this section described a union-read migration that was
  never actually built, which an external design review correctly flagged
  as a documentation/implementation mismatch. If live upstream relay-upgrade
  lag becomes a real operational problem, implement the dual-write contract
  properly as a follow-up rather than reintroducing a partial one here.

## Security Considerations

**Self-reported, not independently verified.** A disposition is signed by the
agent that emitted it, which proves *an answer with this content was
authored by this key* — it does not prove the judgment was correct, that a
`completed` disposition reflects real completed work, or that a `refused`
reason is the agent's true reason. A compromised or dishonest agent can
publish a false disposition exactly as easily as it could post a false chat
reply. Stronger guarantees require an independent reviewer or an
orchestration layer that checks the agent's actual output, not just its
disposition claim.

**A harness cannot honestly emit `completed`, and Buzz's does not.** An ACP
harness observes only that a turn ended without technical failure (an
`EndTurn` stop reason). That covers an agent asking a clarifying question,
reporting it couldn't finish, or answering only one message of a batched
turn, exactly as readily as it covers a task genuinely done — so mapping
`EndTurn` to `completed` would publish a signed success claim with nothing
behind it, in a ledger whose whole value is that `completed` means
something. Buzz's harness therefore emits `responded` for a clean turn,
`refused` only for an explicit ACP `Refusal`, and `errored` for failure,
cancellation, and timeout. It never emits `completed`.

**`completed` is the target agent's own signed assertion — nobody else's.**
The binding rule requires a disposition's signer to be the obligation's
target, so a requester cannot publish a bound confirmation and neither can an
independent workflow. v1 therefore defines `completed` as exactly one thing:
the agent that was asked, asserting it did the work. Third-party
confirmation ("the requester agrees this is done") is a genuinely different
claim with different authority, and would need its own event kind and its own
binding rules; an earlier draft of this section blurred the two, describing a
confirmation path that no rule here permits.

In practice this means a managed agent reaches `completed` by publishing one
itself — in Buzz, `buzz dispositions emit --request <id> --disposition completed`,
which refuses to sign unless the running identity is the obligation's target.
A turn that ends cleanly and settles nothing leaves an `open` obligation,
which is the honest record of what happened.

**A batched turn cannot attribute any per-obligation outcome.** When one turn
carries several obligations, nothing it observes can say which obligation a
statement applies to, so Buzz's harness emits nothing at all for such turns.

An earlier version carved out `errored`, reasoning that a failed turn means no
obligation in the batch received an answer. That holds only if a turn's output
is atomic — if the agent fully answers A and then B's tool call fails,
`errored` on A is exactly the overclaim already removed from `responded`, one
state further down. No such atomicity guarantee is established, and asserting
an unverified premise is how the `responded` projection arrived in the first
place. Those obligations remain `unanswered`, which is the honest record:
nothing observed knows what became of them. The agent settles them itself.

**A refusal MUST carry a reason where one exists.** `reason` may be empty
for any state (see Content), but a refusal that explains nothing defeats
much of the point of recording it — this NIP's headline promise is that a
reader can recover *why*. The ACP runtime's own `Refusal` stop reason
carries no text, so Buzz's harness records the stable marker
`acp-runtime-refusal` rather than an empty string: it says exactly as much
as the harness knows — the runtime refused and did not say why — instead of
producing a refusal that looks unexplained by choice.

**No binding between `p` tag and request authorship.** The relay validates
that `p` is a well-formed pubkey, not that it matches the actual author of
the `e`-tagged request. A verifier that needs this guarantee MUST fetch the
request event itself and compare authors — this NIP does not do it for you.

**Verify signatures before auditing, or say that you did not.** A consumer
that applies every rule in this document to events it has not
cryptographically verified is a semantic verifier trusting its relay, not an
independent auditor — and the distinction only shows up in the case the
audit exists for. Buzz's `buzz dispositions list` verifies each event's id
and signature before adaptation and reports the number it rejected. A
consumer that cannot or does not verify MUST state that trust boundary
rather than presenting relay-supplied events as audited.

**Accounting is a public-input computation.** Any channel member can store
structurally valid but unbound dispositions, so a consumer that re-scans
every disposition for every request gives a cheap writer a quadratic cost.
Group dispositions by `e` tag once; accounting should be O(requests +
dispositions).

**Cross-principal spoofing is possible at the relay layer; consumers MUST
bind to the target agent.** The relay does not verify that a disposition's
signer was actually addressed by the request it references (Relay Behavior
above) — any channel member with ordinary write access can publish an
honestly-signed kind:44300 event naming someone else's request by `e` tag.
See "Target-agent binding" under Client Behavior for the required
consumer-side check (the disposition's `pubkey` must equal the obligation's
single `agent` target — NOT merely a `p` mention, which would let any CC'd
human close the agent's obligation). A reader that skips this check is not
merely missing a nice-to-have — it can be made to display a request as
resolved when it was never even addressed to whoever signed the
disposition. Every first-party consumer in this tree (`buzz-cli`, the
desktop app) implements this check against one shared semantic contract with
two tested implementations — a Rust verifier in `buzz-core` and a TypeScript
mirror, pinned to each other and to this document by the conformance corpora.
That is deliberately a weaker claim than "one verifier": they are two
implementations, and only the cases in the corpora are proven to agree. A
third-party reader (e.g. an external auditor) MUST implement the check too,
or its accountability claims are not trustworthy.

**The binding check is authorization to *count* a disposition, not proof of
honesty.** It establishes that the signer is the agent that was asked. It
does not establish that the signer did the work, or that its `completed` is
truthful. Those remain the self-reporting limits described above.

**Metadata leakage is the point.** Unlike NIP-AM, there is no metadata to
leak here beyond what is already implied by the conversation: dispositions
are plaintext and readable by the channel by design, so this is a feature,
not a consideration to mitigate. Note this is channel-scoped, not public:
the conversation's existing audience, no wider.

**Availability, not correctness, is what a gap check proves.** "Zero
unanswered requests" (see Gap detection) proves every valid obligation
received a bound signed response — it does not prove the responses were
good, honest, or complete. Treat it as a liveness/accountability signal,
not a quality signal. Note also that a `responded`-only channel has zero
*unanswered* obligations while having zero *resolved* ones: "answered" and
"settled" are different questions, and this NIP deliberately keeps them
apart rather than letting a reply pass for a result.
