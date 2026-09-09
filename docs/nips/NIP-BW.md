NIP-BW
======

MyBuzz issue-to-human-acceptance wire contract
-------------------------------------------

`draft` `optional` `P1 contract only`

## Scope and compatibility

This document and `NIP-BW.fixtures.json` specify inputs and expected projections
for independent Host P2H and MyBuzz P2M implementations. Neither is an
implementation, dispatch authorization, production policy, or deployment approval.
MUST, MUST NOT and MAY are normative. All scopes include the host-derived community.

Repositories remain NIP-34 kind **30617**; issue roots remain kind **1621**.
Workflow records use **46100**, working constant `KIND_BUZZ_WORKFLOW_RECORD`.
It is a regular, non-replaceable event, retained append-only by BW consumers.
There is no `d` tag, replacement, latest-timestamp-wins, or deletion-as-undo.
No production constant or kind registry is changed in P1.

This advances VISION.md and VISION_SOVEREIGN.md: relay code and signed decisions
remain authoritative; GitHub is a build mirror. Intentional tension with
VISION_PROJECTS.md's standard-status interoperability: BW has stricter causal,
assignment and human-verdict semantics than NIP-34 statuses can express. Standard
clients can still discover issues and artifacts, but cannot authoritatively
project BW progress. No new HTTP endpoint or legacy status mirror is introduced.

## Registry and capability evidence (2026-09-09)

Read-only checks of these official sources found no assignment of 46100:

- [Nostr NIP index](https://github.com/nostr-protocol/nips/blob/master/README.md).
- [Nostr kind registry](https://github.com/nostr-protocol/registry-of-kinds/blob/master/schema.yaml),
  retrieved raw SHA-256 `88b471c3d567e1deea4b0365e32480e27e9f137fc1064d85bd4bc5536606044a`.
- [Buzz kind constants](https://github.com/block/buzz/blob/main/crates/buzz-core/src/kind.rs),
  retrieved raw SHA-256 `de54e283391b447fcf6a80c3c252debd3f9cadbb506abe899927fb8f2a748f96`.
- Local `crates/buzz-core/src/kind.rs` at baseline
  `83afea0a1549410a8f7effa12ceed0f7b13234e6`: workflow family 46000–46999,
  existing execution kinds 46001–46012, trigger 46020, approvals 46030/46031;
  46100 absent. Local docs/nips search also found no 46100 contract.

[NIP-01](https://github.com/nostr-protocol/nips/blob/master/01.md) reserves the
replaceable/ephemeral/addressable ranges below 40000; BW requires regular storage
for 46100. Absence from these non-exhaustive registries is not a global reservation.
Recheck before implementation; a future collision requires a contract revision.
[NIP-34](https://github.com/nostr-protocol/nips/blob/master/34.md) and
[NIP-94](https://github.com/nostr-protocol/nips/blob/master/94.md) retain their kinds.

The Windows worktree's origin names the canonical Relay Git repository; its
tracking remote is GitHub and MUST NOT be used as canonical evidence. The local
Git read attempt failed because the configured helper had no Nostr key and Coder
had no matching external-auth provider. No credentials were sought. No authorized
Relay event-query session was available, so 46100 queryability and persistence
are **unverified**. A later read uses explicit `kinds:[46100]` and repository `#a`
filters, with pagination and referenced-ID fetches. An empty response, NIP-11
metadata or source inspection does not prove storage. Persisting probes belong
to P3. P1 does not publish any events or reactivate workflow triggers.

## Bytes, primitive types and validation order

An event has exactly the seven NIP-01 fields: `id,pubkey,created_at,kind,tags,content,sig`.
No duplicate JSON keys at any depth; reject invalid UTF-8, lone surrogates, NaN,
fractional/exponent-form numbers, negative numbers, BOM and trailing JSON data.
Outer whitespace is permitted. Content is a JSON object for 46100, plain text
for 1621/1/1063. Content field order and whitespace are signed bytes, not normalized.

Event ID = SHA-256 of UTF-8 compact JSON
`[0,pubkey,created_at,kind,tags,content]`, using NIP-01 escaping, no ASCII escaping
of Unicode, and no newline. Preserve tag order and content string exactly.
Verify BIP-340 `sig` over the 32 ID bytes using `pubkey`. Verify ID first, then
signature; never treat a signature over an unrelated ID as valid.

Limits are inclusive. IDs/pubkeys/digests are lowercase 64 hex characters;
pubkeys must lift to valid secp256k1 x-only points. Signatures are 128 lowercase
hex. Git object IDs are lowercase 40 hex (SHA-1 repository format in BW v1).
Timestamps are integer Unix UTC seconds 0..4294967295, not milliseconds or strings.
Other integers are 0..2147483647 unless specified. Text is measured in UTF-8 bytes.
Total compact event JSON is at most 65536 bytes, content at most 32768 bytes,
at most 16 tags, each tag string at most 2048 bytes (auth conditions at most 256).
Human text uses `text` (1..2048 bytes), `longtext` (1..8192), `title` (1..256),
or `note` (0..2048). URLs are absolute HTTPS without userinfo or fragment,
1..2048 bytes; URLs are not credentials. Repository coordinates are
`30617:<owner-pubkey>:<slug>`, slug `[a-z0-9][a-z0-9_-]{0,63}`.
Stream is a single branch name `[a-z0-9][a-z0-9._/-]{0,127}`, additionally satisfying
`git check-ref-format --branch`; no `..`, `@{`, trailing slash/dot or `.lock` component.
Platform is `windows|android|ios|macos|acp`; client pipelines exclude `acp`.
Release labels are `[a-z0-9][a-z0-9._+-]{0,63}` and unique per repository/pipeline.
Arrays are ordered on the wire, unique by value unless a different key is stated.

Processing stages, in this precedence, are: `envelope`, `id`, `signature`,
`shape`, `attestation`, `references`, `policy`, `role`, `causality`, `external`,
`projection`. First failing stage is returned. Within a stage choose the
lexicographically smallest error code if multiple checks fail. Fixture codes are
stable contract labels; a consumer may attach human details separately.

Outcomes: `accept` (valid and projected), `reject` (permanent invalidity),
`pending` (missing event/external evidence or future timestamp), `conflict`
(multiple valid causal successors or competing resource claims), `replay`
(already-seen identical ID, no second action), `ignore` (non-BW legacy event).
Semantic expectations in fixtures are an oracle for P2, not executed by the P1
structure checker. Decisions are relative to the complete supplied event set,
external evidence and explicit `now`; replaying in another order converges.

## Exact tags and machine schema

`record` is exactly one two-string tag on every 46100 event. Its value selects
one of the eleven record schemas below. Common required tags, each exactly once:
`["record",type]`, `["a",repo]`, `["policy",policy-event-id]`.
Genesis role-policy omits `policy` to avoid self-reference. Every other role-policy
references its predecessor in `policy`. Other required tags by record:

| Record | Additional required two-string tags | Optional two-string tags |
|---|---|---|
| role-policy | none | previous (absent only at genesis) |
| issue-update | issue | previous (absent only for initial correction) |
| triage-action | issue | previous, delegation |
| issue-state | issue | previous (absent only for enrollment) |
| issue-relation | issue | previous (absent only for first edge operation) |
| release-pipeline | none | none |
| release-set | none | previous (absent only at freeze) |
| build-request | none | previous (absent only for first pipeline request) |
| build-run | none | previous (absent only for first run snapshot per request) |
| test-ready | none | previous (absent only for first handoff per set) |
| member-verdict | issue | none |

There are **eleven** 46100 record types. The separate NIP-94 artifact profile
is specified below and checked explicitly by the validator. The machine schema
list is authoritative for the 46100 enumeration. `issue` is the exact
1621 root ID, never a comment or a repo announcement. `previous`, `policy`, and
`delegation` are exact event IDs. No `e`, `h`, `p`, `d`, `t`, `expiration`, extra
index hints or duplicated tags are allowed on 46100 records.

All profiles additionally allow zero or one `auth` tag of exactly four strings
`["auth",owner,conditions,signature]`, validated using [NIP-OA](NIP-OA.md).
BW fails closed on invalid/duplicate attestations, stricter than OA's generic
client ignore behavior. Keep the original tag in archival/export bytes. NIP-42,
NIP-98, Git NIP-GS and relay membership are independent transport admission
checks. A valid OA credential never changes `event.pubkey` or grants a BW role.
The Host MUST additionally validate its configured roles before side effects;
failure blocks execution even when the event's historical policy accepted it.

`NIP-BW.fixtures.json.schemas` is the normative closed content-shape table.
Each schema has `required` and `optional` maps. No other keys are allowed.
Type names refer to the primitives above or closed named objects in `types`.
`?T` permits JSON null; `[T]` is an array with bounds in `arrays`; `A|B` is an
exact enum. An optional field is absent, not null, unless its type permits null.
These closed schemas are shared contract data, not a general schema platform. Cross-field requirements
below are normative and deliberately outside the P1 structural validator.

## Causality and historical authority

Each mutable object's genesis and `previous` successors form a directed chain.
Chain keys are: repository policy; issue text; issue triage; issue state;
`(issue,relation,target)`; frozen set lifecycle; pipeline requests; request run;
set handoffs. Assignment uses its existing `prior` chain. Immutable pipelines,
freeze payloads, artifacts and verdicts have no editable head.

A reference must match repository, object, expected kind and record, and have
`created_at <= successor.created_at <= now`. Missing references remain pending;
invalid references reject dependents (`invalid-reference`). Cycles reject.
A successor of an ancestor is not silently discarded as stale: if independently
valid at that ancestor it creates a fork, even when delivered late. A descendant
of another object is `wrong-previous`; a duplicate of a processed ID is replay.
Two valid genesis or successors produce visible `fork` containing sorted event
IDs. All descendants and dependent side effects are blocked, including previously
projected descendants (display their history as disputed, never erase signatures).
There is no automatic fork winner or in-band fork repair in BW v1. Owner must
resolve operationally before adopting a separately reviewed protocol revision;
clients must not fabricate a merge or bypass with a new timestamp.

Validity is evaluated against the referenced historical parent snapshot, not
arrival order. Changing a policy does not rewrite accepted history. A policy's
`effective_at` must be >= its own created_at and strictly increase along the
chain. At time t, use the last unconflicted policy with effective_at <= t.
An event must reference that exact policy or reject `policy-binding`. A later
policy arriving late may make a projection pending/disputed until its history
is known. No signed timestamp proves real creation time; real-time side effects
also require current external Host authorization and complete canonical readback.

## role-policy

Trust input is `(community,repo,owner-pubkey)` confirmed outside untrusted event
content. Owner must equal the 30617 coordinate owner; announcement signature and
coordinate must verify. Genesis is signed by that owner, version 1, no previous
or policy tag. Successors are owner-signed with version increment exactly one,
previous=policy=old policy. Owner rotation/new repository trust is outside v1.

Content: version, effective_at, cutover, operators, coordinators, workers,
human_testers and triage_delegations. Cutover is immutable after genesis and >=
genesis effective_at. Role arrays contain 0..32 distinct pubkeys; human_testers
contains 1..32. Workers maps each platform to 0..32 pubkeys; overlap is allowed.
Operators provide Buzz-admin triage/classification powers, not Owner powers.
Coordinators select delegates and manage release logistics. No hardcoded human
or production keys. Display names such as Jari and Ania are not identity proofs.

Each triage delegation is a closed object `{issue,action,delegate,expires_at}`;
action is accept/duplicate/decline; at most 32 unique `(issue,action,delegate)`
entries. A delegated triage event must tag `delegation` with that exact policy ID,
match one entry, and be before expires_at. It authorizes one action at the issue's
initial triage head only; another valid use is a conflicting successor, not an
additional grant. Delegations do not authorize assignments, releases or verdicts.

Only a consciously enrolled new issue participates: root created_at >= cutover,
followed by reporter- or owner-signed issue-state `triage` referencing that root.
Pre-cutover roots reject `cutover` even when updated later. Existing unmarked roots
stay outside BW; old 1630..1633 and `mybuzz-status-v1` events are ignored, with no
migration or status mapping. Enrollment never rewrites a root.

## issue-update and triage-action

Initial correction has no previous; all later corrections reference the prior
issue-update. `patch` is a nonempty subset of title, description,
acceptance_criteria, non_goals, type, priority, platform. Reporter, Owner,
operators or coordinators may edit the first four text fields; only Owner,
operators or coordinators may edit the last three. A mixed patch requires all
field permissions. Criteria and non-goals are 0..25 unique text strings.
Type is bug/feature/task; priority P0/P1/P2/P3. Initial values are root subject and
content, empty criteria/non-goals, task/P2, platform null. Classification changes
are allowed only in triage/backlog; text edits only before in-development.

Triage content always includes action. Conditional fields are exact:
accept: none; need-info: question and recipient; snooze: until;
duplicate: target; decline: reason. Forbid all other conditional fields.
All require an enrolled issue still in triage. Owner may perform any action;
operators may need-info/snooze; accept/duplicate/decline additionally accept the
single-task delegate above. Reporter status alone grants no triage power.
Need-info question is nonempty and recipient is a pubkey. Duplicate target is a
different same-repo 1621 root and adds projected duplicate-of relation.
Decline reason is nonempty. Snooze until is > created_at and <= created_at+2592000.

Accept projects backlog; need-info/snooze retain triage; duplicate/decline project
closed. Issue-state backlog must reference the exact accepting triage event.
Snoozed is a view at `now < until`, with no synthetic timer event. Stale is
`now >= last_valid_activity + 604800`; valid activity is root enrollment,
issue-update, triage-action, issue-state, assignment selection or member-verdict.
Comments, invalid events, OA tags and replays do not reset stale time.

## issue-state and the existing assignment wire

Enrollment content: `{state:"triage"}`. Backlog: state plus `triage` ID.
Ready: state plus `stream`, `assignment` (ID or null), `update` (latest text ID),
and `rework` (rejected verdict ID or null). In-development: state, stream,
assignment (non-null). Implemented adds commit, tests and remote_readback.
No fields from another state are permitted. All post-enrollment records require
previous = issue-state head. Transition sequence is triage → backlog → ready →
in-development → implemented. Owner/coordinator may reset ready from ready,
in-development or implemented, but implemented reset requires rejected verdict
or a terminal failed/aborted set (the latter referenced in optional `terminal_set`).
Resolved issues cannot reset. Ready needs nonempty acceptance criteria, classified
platform, exactly one stream, and a current text snapshot; ACP work may be ready
and implemented but is not client-release eligible.

Existing local SDK grammar (`crates/buzz-sdk/src/builders.rs`,
`build_git_issue_assignment_with_prior` / `build_git_issue_unassignment_with_prior`):
kind 1, content <=65536 bytes, `e` = `["e",issue,"","root"]`, `a` = `["a",repo]`,
1..50 distinct sorted lowercase `p` pubkeys, `t` = assignment or unassignment,
optional `["prior",event-id]`. SDK normalizes/deduplicates supplied assignees;
old consumers trust reporter/owner or sole self-assignment/unassignment.
These are existing semantics, not BW authorization.

BW consumes that same signed operation format (optional OA retained), with the
stricter common 32768-byte content/65536-byte event limit, at most one p tag,
and no other tags. First assignment omits prior; every subsequent operation must
reference the unique prior operation. Assignment adds the sole delegate only
when none exists; unassignment removes exactly the current delegate. Only Owner
or the coordinator active at operation.created_at may sign BW operations.
There is no new record kind, self-assignment shortcut or inherited OA authority.
Operations themselves do not change the active writer: Owner/coordinator must
select the exact operation head in a ready event. Ready with assignment null
requires an empty assignment fold. A non-null selection must equal the final
assignment head and contain exactly one delegate. Changes after selection suspend
writer actions until reselection; a fork blocks selection and all writer actions.

Only that delegate signs in-development and its implemented successor, with the
same stream and assignment head. Switching delegate requires unassignment,
assignment, and Owner/coordinator ready reset. Earlier valid work stays historical;
late old-writer work is rejected `assignment-head` when its claimed current
assignment was superseded at its created_at. Backdating cannot authorize Host
execution: external current selection is an additional mandatory check.

Implemented `remote_readback` is `{repo,stream,head,observed_at}`. Head must equal
commit and externally observed canonical Relay head, observed_at <= created_at,
with age <=300 seconds. Tests is a nonempty summary, including honest limitations.
The signed claim alone proves no remote fact. Implemented means source work and
remote readback only, never a build, installation test or human acceptance.

## issue-relation

Content `{relation,target,operation}`; relation blocks/related/duplicate-of/parent-of,
operation add/remove. Owner/coordinator only; both roots enrolled in the same repo;
no self-edge. First op must add; subsequent ops alternate remove/add via previous.
Inverse views: blocks↔blocked-by, related↔related (canonical endpoint order is
lexicographically ascending root ID), duplicate-of↔duplicates,
parent-of↔child-of. Inverse views are not extra events. Triage duplicates also
contribute an immutable duplicate edge; explicit remove cannot remove that edge.
Reject parent or blocking cycles. An executable leaf is a non-closed, non-resolved
issue with no active child, no duplicate-of edge and no unresolved blocker.
Conflicted relation history blocks eligibility. Active-set members cannot acquire
children, duplicate edges or blocking edges until the set terminates.

## release-pipeline and release-set

Pipeline ID is its immutable event ID, Owner-signed. Content binds platform,
stream, workflow path, workflow_sha256, manifest_schema, manifest_sha256, mirror URL,
runner identity, publisher pubkey and provider. Policy tag pins allowed workers and
roles. Path is 1..256 characters, relative POSIX, no empty/dot/dot-dot components.
Manifest schema is a nonempty versioned contract name. Any change, including
role-policy binding, requires a new pipeline event ID; no mutation of existing sets.
Platform excludes ACP. Publisher must be a worker for this platform in that policy.

Freeze content has action=freeze, release, pipeline, platform, stream, relay_sha,
members, readback. Set ID is freeze event ID. Owner/coordinator under the pipeline's
pinned policy signs it; its policy tag must equal that policy (exception to current
policy selection for the complete pipeline/set lifecycle). Host current-role check
still applies. Every downstream release event uses that pinned policy.
Readback has the remote_readback shape with head=relay_sha and age <=300 seconds.
Member is `{issue,implemented}`; 1..25 distinct issue IDs and implemented IDs,
all same repo/platform/stream, exact current implemented state, executable leaves.
External Git input must show every implemented commit is an ancestor of relay_sha
(including equality), and freeze readback must be the actual Relay stream head at
freeze. A signed claim or a GitHub ref is insufficient.

Only freeze creates a set. `release-set` may later close it with content
`{action:"close",set:freeze-id,outcome:"failed"|"aborted",reason}` and previous
= lifecycle head (initially freeze). Frozen fields are never repeated or replaced.
No close while an external provider run remains nonterminal; cancellation must
first be read back. Completed sets need no close: after verdicts for all members,
the set is completed even with partial rejection. A closed/completed set never
reactivates. Failed run alone yields retryable active set, not terminal failed set.
Active = frozen/requested/building/retryable/test-ready/partially-reviewed.
Terminal = completed/failed/aborted. Active membership excludes other active sets.
Two concurrent valid freezes sharing a member conflict and block both; no ID or
timestamp tie-break. Closing one disputed claim is not an implicit conflict repair.

Terminal status releases membership for new freezes. Accepted issues remain
resolved forever in v1 and cannot be members again. Rejected issues enter rework;
ready reset requires that verdict, a new delegate cycle if needed, and new
in-development/implemented events. Failed/aborted sets allow reuse of an unchanged
implemented event, or explicit ready reset referencing terminal_set. This avoids
membership deadlock without mutating frozen membership or inheriting verdicts.

## build-request and build-run

Set existence dispatches nothing. Owner/coordinator signs explicit build-request
`{set,worker,attempt,retry_of}`. Worker is exactly one platform worker in pinned
policy, attempt starts at 1, retry_of is null. Subsequent attempts increment by one
and reference the prior terminal failed/cancelled run ID. A successful run cannot
retry; a new handoff may refer to it, or a new set/pipeline is needed for rebuilding.

Requests form one previous chain per pipeline, even across sets, establishing an
exclusive building lock. A next request requires its predecessor's final run and
any associated handoff to be settled; different sets may build once previous run
is terminal (human testing need not finish). Retry of the same set additionally
requires failed/cancelled predecessor run. At most one Building set per pipeline.
Concurrent children conflict and block dispatch, even if one was first observed.
Same `(pipeline,set,attempt)` with different IDs conflicts; identical event replay
never dispatches again. Durable dispatch idempotency key is request event ID.
Worker reconciles that key with provider before any retried API call; uncertain
provider outcome remains pending and holds lock. New attempt is never a transport
retry substitute. Offline event history alone cannot provide distributed locking;
P2H must serialize side effects after canonical readback and reconcile provider.

Run content: request, provider, run_id, attempt, url, head, result, observed_at.
Run ID is 1..128 ASCII `[A-Za-z0-9._:-]+`; result queued/running/success/failure/cancelled.
Request worker alone signs; provider equals pipeline.provider; attempt=request.attempt;
head=set.relay_sha. First snapshot may be any result; subsequent snapshots preserve
all identity fields and progress queued→running→terminal or queued→terminal.
Terminal has no successor. Every snapshot requires external provider readback keyed
by provider/run_id, including request idempotency key, URL, attempt, head and result,
with observed_at <= created_at and age <=300 seconds. Wrong facts reject; unavailable
facts remain pending. Never trust arbitrary run URLs or signed success claims alone.

## NIP-94 artifact and test-ready

Kind 1063 uses plain-text description (1..2048 bytes). Exact required singleton
arity-2 tags: a, set, run, platform, release, url, m, x, size; optional auth only.
There is no record or policy tag: policy is inherited via set. `set` and `run`
are exact IDs, `release` equals set label, `platform` matches pipeline, `x` is
SHA-256, `size` is canonical unsigned decimal 1..2147483647. MIME is lowercase
`type/subtype` (ASCII tokens `[a-z0-9.+-]+`). Publisher alone signs. Run must be
successful for the exact set; external downloaded raw bytes must match size and x.
No archive unpacking, decompression, Unicode or newline normalization before hash.

Test-ready content: set, run, artifacts (1..8 distinct 1063 IDs), installation,
limitations, tester. Request worker signs; all artifacts reference same set/run and
are published by pipeline.publisher. Tester is exactly one pinned-policy human.
Windows pilot requires installation.method=`tailnet-download`, unsigned=true,
immutable=true, installation.url equal an artifact URL, and external publisher
readback attesting immutable Tailnet availability. Other platforms use method
`download` with immutable=true. `instructions` is nonempty. Limitations is nonempty,
and Windows must explicitly state unsigned status in human-readable text.
Ephemeral CI artifact URLs alone fail `durability` even if hashes match.

Handoffs form a previous chain per set. A later handoff requires all verdict slots
of the old handoff still empty; after any valid verdict a new handoff in that set
is forbidden (`handoff-locked`). Before verdicts, a new handoff may select another
allowed human or corrected artifacts, but old handoff verdicts reject `old-handoff`.
New artifacts never inherit any verdict, even when their bytes happen to match.

## member-verdict

Content: set, test_ready, artifact, verdict (accepted/rejected), reason (nonempty).
Issue tag must name a member. Artifact must belong to current test-ready and the
exact set; tester signer must equal that handoff's chosen pinned-policy human.
Either authorized Jari or Ania is enough when selected: no quorum and no name
matching. Slot key `(set,test-ready,issue)` allows one immutable verdict. A second
valid differing event for the slot produces `verdict-conflict` and blocks derived
resolution for that issue (even two accepts); exact event replay is idempotent.
There is no verdict overwrite. Human correction requires separately reviewed
conflict resolution outside v1, not another timestamp.

Accepted resolves only this issue, rejected projects only this issue to rework.
Accepted siblings stay resolved across sibling rejection and subsequent rework.
Accept all means N individually signed events, never a batch wildcard verdict.
Wrong member, foreign set/artifact, old handoff, terminal aborted/failed set or
new-set artifact with old verdict all reject. Completed-set existing verdicts
remain authoritative; no new slots or replacements may be added afterward.
A new set after rework requires a new handoff and fresh individual verdicts.

## Corpus and P1 validation

The corpus is standalone deterministic JSON. `events` contains complete signed
Nostr inputs, including deliberately malformed cases; labels are fixture-local,
never wire aliases. `cases` lists ordered event labels and per-step expected
outcome/stage/code plus exact partial projection assertions (unspecified projection
keys are unconstrained). Each case starts from empty event history and its explicit
trust/external inputs. `now` is explicit. `base` is not supported: prerequisites
are listed in full. `crypto` labels distinguish invalid-ID/signature cases from
semantic cases. Each event carries expected cryptographic results in metadata;
semantic negatives are signed correctly so they reach the intended stage.

External inputs are named `git_readbacks`, `git_ancestry`, `provider_readbacks`,
`downloads` and `host_authorization`. They are test-oracle observations, never
inferred from signed claims. Downloads include exact raw bytes as lowercase hex
and explicit durability facts. Fictitious pubkeys derive only from public test
scalars 1..9, using BIP-340 reference code, zero auxiliary randomness. They MUST
NEVER be funded, deployed or treated as production identities.

Run `python3 scripts/check-nip-bw-fixtures.py` and
`python3 scripts/test-nip-bw-fixtures.py`. These check JSON/schema/corpus structure,
ID preimages, references within the corpus, required coverage and checker rejection
of damaged corpora. They do **not** implement BW roles/folds. Optional Schnorr checking uses the
public [BIP-340 reference](https://github.com/bitcoin/bips/blob/master/bip-0340/reference.py):
download that file outside the repository, then pass
`--bip340-reference /path/to/reference.py` to the checker. It pins the reference
SHA-256 from corpus provenance before loading it, and checks every declared
signature expectation. This proves cryptographic vectors only, not BW semantics. Host and MyBuzz must later execute all semantic expectations themselves.
No build, service, relay probe, product parser or human acceptance test is implied.

Hash final file raw bytes with `sha256sum docs/nips/NIP-BW.fixtures.json
 docs/nips/NIP-BW.md`. Digests and exact commit belong in the handoff report,
not either hashed document; no circular/self-referential digest. Any byte change
invalidates its digest and independent review binding. P2 opens only after final
technical checks, exact canonical remote readback and independent PASS at that
commit and fixture digest. Source push with REVIEW AUSSTEHEND does not open P2.
