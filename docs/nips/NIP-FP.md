NIP-FP
======

Work Fingerprints
-----------------

`draft` `optional`

**Depends on**: NIP-01 (events, addressable events), NIP-34 (git statuses and repository state), NIP-OA (owner attestation). Interacts with NIP-32 (labels), NIP-09 (deletion), and NIP-11 (relay information).

## Abstract

A **work fingerprint** is a signed, replaceable index of events that its author *selects* as evidence of their work. This NIP defines the fingerprint event, the outcome vocabulary a verifier reads from ordinary events, how a verifier discovers outcomes the index leaves out, and the report a verifier produces.

The report keeps three things apart: what the subject **selected**, what the viewer **observed** by fetching events itself, and how the viewer **interprets** those events under a named policy. This NIP defines no score, tier, rank, or verdict.

## Motivation

Someone meets a key they have never worked with, often in a community they do not share, and has to make a bounded decision: offer it a small, revocable job or not. The evidence already exists as signed events: patches, merges, issue closures, answers, corrections. Nothing assembles it, and an author-curated list hides whatever it leaves out.

A signature proves who authored an event. It does not prove the work was good, that a merge was authorized, that two keys belong to different people, or that nothing contrary exists. A fingerprint therefore carries no authority of its own. It is a starting point. The verifier fetches the selected events from their origin, checks the authority of every claimed outcome against the rules in force at the time, looks for contrary outcomes by subject and by issuer, and reports what it could not see.

## Non-Goals

This NIP does not define a score, tier, rank, or any scalar trust value. RFC block/buzz#3721 proposes portable tier attestations; that is a separate layer and is not used here.
This NIP does not define completeness. A fingerprint never claims to list all of its subject's work, and a verifier never claims that no contrary event exists.
This NIP does not define ledger inclusion, inclusion receipts, or checkpoints.
This NIP does not grant access. Presenting or verifying a fingerprint changes no membership or permission.
This NIP does not establish that distinct keys belong to distinct people.

## Terminology

This document uses MUST, MUST NOT, SHOULD, SHOULD NOT, and MAY as defined in RFC 2119.

- **subject**: the pubkey that signs the fingerprint.
- **viewer**: whoever runs the verifier, with their own key and their own read access.
- **work event**: an event the subject selected.
- **outcome**: a signed event that references a work event and makes a claim about it (see [Outcome vocabulary](#outcome-vocabulary)).
- **origin**: the relay or community a work event lives in, written as the URL the viewer queries.
- **scope**: an origin plus the filter the verifier sent to it.
- **cutoff**: the `until` bound of every discovery query. Nothing after the cutoff is observed.
- **owner link**: a NIP-OA `auth` tag that verifies on an observed event.

## Kinds

| Kind | Name | Signer | Class | Status |
|------|------|--------|-------|--------|
| `30340` | Work fingerprint | subject | addressable | provisional |

`30340` is provisional. It is unassigned in the NIPs registry, and it MUST NOT be treated as stable until it is registered. Buzz relays built with `KIND_WORK_FINGERPRINT` in `buzz-core/src/kind.rs` accept it as a community-global member write and advertise the `nip-fp` NIP-11 extension. Relays without it reject unregistered kinds, so there a fingerprint travels as a [transport bundle](#transport-bundle-pre-registration).

## Fingerprint event

```json
{
  "kind": 30340,
  "pubkey": "<subject>",
  "tags": [
    ["d", "default"],
    ["coverage", "selected"],
    ["e", "<work-event-id>", "<origin>"]
  ],
  "content": ""
}
```

- The event MUST have exactly one `d` tag. A subject MAY keep several fingerprints under different `d` values, e.g. one per area of work.
- `coverage` declares what the index covers. The only defined value is `selected`, which is also the default when the tag is absent. Verifiers MUST display `selected` whatever the tag says, and MUST NOT present a fingerprint as complete.
- Each `e` tag selects one work event. Its third element is REQUIRED and names the origin to fetch it from. A selection `e` tag without an origin is unresolvable and its item is `unknown`.
- Other tags MUST NOT be read as selections.
- A fingerprint MUST NOT carry more than 256 selection tags. A verifier MUST NOT process more than 256 and MUST report any excess as `unknown`.
- A work event counts toward its subject only when it is signed by the subject or by a key whose owner link names the subject as owner. Any other selected event is reported as `unknown` with reason `not_attributed`.
- `content` MAY hold a human note. Verifiers MUST NOT treat it as evidence.
- The fingerprint MAY carry a NIP-OA `auth` tag. When it verifies, the subject is an agent and the tag names its owner.

The fingerprint is replaceable under NIP-01, so the subject can drop an item at any time. A verifier MUST NOT infer anything from an item's absence, and MUST find contrary outcomes through [discovery](#discovery), never only through the index.

A fingerprint MUST NOT select direct messages or events whose content is visible only to their participants unless every participant authorized it.

## Outcome vocabulary

An outcome is an **implicit attestation**: an ordinary event whose signer makes a claim about a work event. It is only as strong as its signer's authority for that claim at the time it was signed.

| Outcome event | Claim | Polarity | Who has authority |
|---|---|---|---|
| `1631` on a patch or PR | merged / applied | positive | repo authority (below) |
| `1631` on an issue | resolved | positive | issue author or repo authority |
| `1632` | closed | contrary | issue/PR author or repo authority |
| `1630` after a `1631` or `1632` | reopened | contrary | issue/PR author or repo authority |
| `1630` otherwise | open | neutral | issue/PR author or repo authority |
| `1633` | draft | neutral | issue/PR author or repo authority |
| `1985` `accepted` | accepted | positive | the requester (below); anyone else is an opinion |
| `1985` `reused` | reused | positive | opinion |
| `1985` `corrected`, `rejected`, `disputed` | contrary | contrary | the requester; anyone else is an opinion |
| `5` by the work event's author | retracted | contrary | the author |
| replies and mentions (`1`, `9`, `1111`) | attention | none | not an outcome |
| reactions (`7`) | attention | none | not an outcome |

**Repo authority** for a status on repository coordinate `30617:<owner>:<d>` is held by `<owner>` and by each key in the `maintainers` tag of the `30617` version **in force when the status was signed**. A key whose owner link names a repo-authority key as owner holds that authority by delegation, and verifiers MUST report it as delegated.

**The requester** of a work event is the author of the event it replies to (its NIP-10 `reply`, or else `root`, `e` marker). A label from the requester is an authorized acceptance or rejection of the answer. A label from anyone else is an opinion: it is reported, it never raises a rung, and its polarity still counts as contrary when it is contrary.

**Authority at the time.** `30617` is replaceable and relays usually keep only the latest version. If the latest observed version was signed at or before the status, it is the version in force. If it was signed after the status, the maintainer set at the time is unknown, and only `<owner>` (fixed by the coordinate) can be shown to have had authority. Verifiers MUST NOT apply a newer maintainer set to an older status. A relay accepting a status is not authority: relays may accept status events from any writer.

Community push policy that is not expressed in signed events, such as a relay that lets channel members push, cannot be checked from events. A status that only such a policy would authorize is reported `self_asserted`, with that caveat in its basis.

**Ref corroboration.** A `1631` with a `merge-commit` tag `X` is ref-corroborated when an observed `30618` for the same repository, from the same origin, lists `HEAD` as `ref: refs/heads/<b>` and `refs/heads/<b>` as `X`, and the `30618` is signed by the repo owner, a maintainer in force, or the origin's NIP-11 `self` pubkey. The verifier reports which signer and the time between the status and the ref state. A `1631` without such a ref state is reported with `ref: unverified`. A ref state shows that a branch pointed at a commit; it does not show who authorized the merge.

## Trust labels

Explicit outcomes on non-code work use NIP-32 labels (kind `1985`) in the provisional `buzz.wot` namespace shared with RFC block/buzz#5880. That RFC uses the namespace for vouches about people; this NIP adds values about **work events**:

```json
{
  "kind": 1985,
  "tags": [
    ["L", "buzz.wot"],
    ["l", "accepted", "buzz.wot"],
    ["e", "<work-event-id>", "<origin>"],
    ["p", "<work-event-author>"]
  ],
  "content": "Used this in the release checklist."
}
```

The values are `accepted`, `reused`, `corrected`, `rejected`, and `disputed`. A label SHOULD carry a `p` tag naming the work event's author, so discovery by subject finds it even when the work event is not selected. Kind `1985` is not yet accepted by Buzz relays; block/buzz#5127 registers it.

## Discovery

Given a fingerprint from subject `P` and a cutoff `T`, a verifier MUST fetch every event it reports from its origin and MUST NOT rely on copies supplied by the subject. For each origin it MUST run:

1. **Selected events**: `{"ids": [...]}` for selected `e` tags. Then `{"ids": [...]}` for the events those work events reply to, so the requester is known.
2. **Outcomes by subject event**: `{"#e": [...work ids], "kinds": [5, 1630, 1631, 1632, 1633, 1985], "until": T}` and, for attention counts, the same with kinds `[1, 7, 9, 1111]`.
3. **Outcomes by subject key**: `{"#p": [P], "kinds": [1630, 1631, 1632, 1633, 1985], "until": T}`. This finds outcomes on work the subject did not select.
4. **Outcomes by issuer**: for each repository a selected work event belongs to (its `a` tag), `{"#a": [coordinate], "kinds": [1630, 1631, 1632, 1633], "until": T}`. Statuses often tag only the repo owner, so a subject-key query alone misses them.
5. **Authority**: `{"kinds": [30617], "authors": [owner], "#d": [d]}` and `{"kinds": [30618], "#d": [d], "until": T}` per repository of any observed work event, and the origin's NIP-11 document for its `self` pubkey. The verifier checks each `30618` signer itself.

For an outcome found by steps 3 or 4 whose target was not yet observed, the verifier fetches the target with `{"ids": [...]}` to learn its author, and that target's parent as in step 1. A target attributed to the subject is reported as unselected work (`selected: false`), and outcomes on it carry `target_selected: false`; a contrary one is **omitted by the selection**. Outcomes on work not attributed to the subject are not reported; the coverage record still counts them.

A verifier MUST discard any returned event that does not match the filter it sent, and MUST only accept an event for a selection whose id equals the selected id.

Every query produces a **coverage record**: origin, filter, cutoff, number of events returned, and whether the result reached the filter's `limit`. Verifiers MUST word results as follows:

- An empty result is "none found in scope S before cutoff T", never "no dispute" or "clean".
- A result that reached its limit is "truncated at N"; the verifier MUST NOT treat it as exhaustive.
- A refused or failed query is "inaccessible: <reason>", and everything that query would have found is unknown.

Discovery observes only what the origin serves to the viewer. A relay can withhold events, and the viewer's read access bounds what it sees. Signed window bounds (NIP-CW) can later make exhaustion within a window checkable; until then no result is exhaustive.

## Independence

A signature proves which key signed an event, not which person holds it. Verifiers MUST report the relationship between an outcome's signer `S` and the subject `P` as one of:

- `same_signer`: `S` is `P`.
- `same_owner`: an owner link connects them: `S`'s owner is `P`, `P`'s owner is `S`, or both have the same owner.
- `unknown`: anything else. A missing owner link proves nothing.

No signed evidence shows that two keys belong to different people, so this NIP defines no `independent` state. A viewer policy MAY mark signers the viewer already knows as `viewer_known`. That is the viewer's interpretation and MUST be labelled as such.

A verifier learns a key's owner only from an owner link on an event signed by that key. A malformed or invalid `auth` tag is no link.

## Verifier report

A verifier produces a report with three sections. Implementations MAY add fields; they MUST NOT merge these sections or add a scalar value.

**`selection`** describes the fingerprint as signed: its id, subject, `d`, `created_at`, whether its signature is valid, its coverage (always `selected`), and the items exactly as listed.

**`observed`** lists every event the verifier fetched, selected or discovered. Each item carries:

| Field | Meaning |
|---|---|
| `id`, `kind`, `pubkey`, `created_at`, `origin` | the event, as fetched |
| `scope` | which coverage record returned it |
| `selected` | whether the fingerprint lists it (work events) |
| `target`, `target_selected` | the work event an outcome targets, and whether the fingerprint lists it |
| `role` | `work`, `request`, `outcome`, `authority`, or `ref`. A `ref` item is a `30618` whose `HEAD` commit is the `merge-commit` of a reported merge status; other ref states are counted in coverage only. Attention events are counted per work event, not listed |
| `claim`, `polarity` | from the outcome vocabulary |
| `authority` | `authorized`, `delegated`, `self_asserted`, `opinion`, or `unknown`, with the basis, e.g. "owner of `30617:<owner>:<d>`" or "maintainer set at the time unknown" |
| `owner_link` | the verified owner, `none`, or `invalid` |
| `independence` | `same_signer`, `same_owner`, or `unknown` |
| `bucket`, `reason` | see below |

Every reported item lands in exactly one bucket, taking the first that applies:

1. **invalid**: the bytes are not a validly signed event. They are evidence of nothing.
2. **inaccessible**: the verifier could not fetch it. It neither passes nor fails, and nothing about it is inferred.
3. **contrary**: a valid signed outcome with contrary polarity, whatever its authority.
4. **verifiable**: fetched from its origin, signature valid, and its claim checks out under the rules above. For an outcome, that means `authorized` or `delegated`; for a work event, attributed to the subject.
5. **unknown**: fetched, but its claim does not check out or could not be checked, e.g. a `self_asserted` status, an `opinion` label, authority at the time, an unattributed selection, or an unresolvable reference.

**`interpretation`** names the policy and its version, gives each work item a rung, lists contrary outcomes omitted by the selection and other gaps, and states every coverage record in the wording above. Each rung carries its basis: the outcome id that supports rung 2, that signer's independence, and whether the policy marks the signer `viewer_known`.

**Rungs** for a work item, selected or discovered:

- **0 — signed**: fetched from its origin with a valid id and signature, but not attributed to the subject.
- **1 — attributed**: signed by the subject, or by a key whose owner link names the subject.
- **2 — outcome**: at least one authorized or delegated positive outcome by a signer that is neither `same_signer` nor `same_owner`. The rung is reported with that signer's independence, which is usually `unknown`.

An inaccessible or invalid item has no rung. Rungs for ledger inclusion or bounded completeness are outside this NIP, and a conforming verifier MUST NOT report them. A rung is never combined with other rungs into a total.

The default policy is `nip-fp/default` version `1`, which applies exactly the rules in this document with no `viewer_known` keys.

## Transport bundle (pre-registration)

Until `30340` is registered, a subject MAY transport a fingerprint as JSON:

```json
{
  "type": "nip-fp/transport-bundle",
  "version": 1,
  "label": "transport bundle (pre-registration)",
  "fingerprint": { "...": "a complete signed kind:30340 event" }
}
```

The fingerprint inside is an ordinary signed event, and verification is identical. A bundle carries no copies of work events, because the verifier fetches them from their origins. A bundle is not a ledger entry and not a permission grant.

## Security Considerations

- **Sybil rings.** Keys can be created for free and can request, answer, and accept each other's work. Their outcomes are reported with independence `unknown`; nothing in this NIP treats them as distinct people. Viewers decide through policy, for example by counting only `viewer_known` signers.
- **Self-endorsement.** Outcomes from the subject or its owner cluster are `same_signer` or `same_owner` and never raise a rung.
- **Forged statuses.** A `1631` from a key without repo authority is `self_asserted`, whatever the relay accepted.
- **Cherry-picking and replacement.** Discovery by subject key and by issuer finds outcomes on unselected work, and reports contrary ones as omitted by the selection.
- **Inaccessible evidence.** Unreachable events stay inaccessible. A verifier never upgrades or downgrades them.
- **Timestamps.** `created_at` is asserted by the signer. Authority at the time and ordering use it as asserted; a signer can backdate. Relay-signed ref states narrow but do not close this gap.
- **Relay withholding.** A relay can omit events from any query. Coverage records make the scope explicit; they do not make it complete.

## Privacy Considerations

Publishing a fingerprint links its subject to every selected event and, through owner links, to its owner. Discovery queries tell each origin which subject the viewer is checking. Verifiers MUST NOT include the content of direct messages, or identifiers of direct messages, in a report unless every participant authorized it.

## Test Fixtures

[`NIP-FP.fixtures.json`](NIP-FP.fixtures.json) holds the conformance cases: a tampered signature, an inaccessible event, a missing outcome, a self-endorsement, a forged `1631`, a cherry-picked index, and a sybil ring, plus passing controls. Each case lists unsigned event templates, the signer of each, which events the viewer's origin serves, and the expected bucket, rung, authority, and independence for each item. Consumers sign the templates with the fixture keys, so the fixtures stay valid for any conforming signer.

## Prior Art

- block/buzz#3721 (NIP-TR, portable trust attestations, kind `48200`): defines tiers and scores; this NIP deliberately defines neither.
- block/buzz#5880 (Web of Trust RFC): evidence-first, advisory; this NIP shares its `buzz.wot` label namespace.
- block/buzz#5127: registers kind `1985` on Buzz relays.
- NIP-32 (labels), NIP-34 (git), NIP-85 (trusted assertions), NIP-OA (owner attestation).
