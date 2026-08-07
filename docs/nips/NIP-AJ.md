<!--
Canonical repository source: docs/nips/NIP-AJ.md
The packaged copy at crates/buzz-core/NIP-AJ.md is a byte-for-byte test fixture.
Edit this canonical source first, then update the packaged copy; the buzz-core
workspace test rejects drift between them.
-->

NIP-AJ
======

MotifOS Agent Job Projections
-----------------------------

`draft` `optional` `projection-only`

This NIP defines strict JSON content for Buzz's reserved kinds 43001 through
43006. Wilson owns canonical conversation and routing identity. MotifOS owns
canonical mission, workstream, attempt, and run state. Buzz may validate and
display a projection; neither the event nor a Buzz interaction grants approval
or execution authority. The codec is not wired to relay ingress, so its
existence alone does not make generic relay storage conformant with this NIP.

## Kinds

| Kind | Meaning | Required transition |
| --- | --- | --- |
| 43001 | request | `null -> requested` |
| 43002 | accepted | `requested -> accepted` |
| 43003 | progress | `accepted|running -> running` |
| 43004 | result | `accepted|running|cancellation_requested -> succeeded` |
| 43005 | cancellation request | `requested|accepted|running -> cancellation_requested` |
| 43006 | error | `requested|accepted|running|cancellation_requested -> failed` |

These kinds are only the task/run spine. Approval decisions, work handoffs,
tool authorization, and private reasoning are outside this NIP. The
`cancellation_requested` state is active and nonterminal; it does not mean that
cancellation completed.

## Content

Every event content is one UTF-8 JSON object with unique member names at every
nesting level and no unknown fields. The maximum encoded content is 16,384 bytes.

```json
{
  "format": "motifos-agent-job-projection",
  "version": 1,
  "authority": {"canonical_system": "motifos", "buzz_role": "projection_only"},
  "conversation_id": "conversation:alpha",
  "mission_id": "mission:alpha",
  "workstream_id": "workstream:contract",
  "attempt_id": "attempt:1",
  "run_id": "run:1",
  "sequence": 1,
  "revision": 1,
  "idempotency_key": "job-request:attempt:1:1",
  "actor": {"seat_id": "wilson", "host_id": "dashboard-local"},
  "transition": {"from": null, "to": "requested"},
  "occurred_at": "2026-08-06T20:00:00Z",
  "expires_at": "2026-08-06T21:00:00Z",
  "sensitivity": "internal",
  "artifacts": [],
  "evidence": []
}
```

### Closed schema

The root object and every nested object are closed. Unknown fields are rejected
at every object depth, and member names must be unique at every depth.
Identifiers are 1-128 bytes, begin with an ASCII alphanumeric byte, and then
contain only ASCII alphanumeric bytes plus `-`, `_`, `.`, or `:`.
The portable idempotency-key predicate applies that grammar and additionally
rejects a value of exactly 64 ASCII hexadecimal characters, which is reserved
as a Nostr event-ID shape.

| Field | JSON type and presence | Values, bounds, defaults, and encoding |
| --- | --- | --- |
| `format` | Required string; no default | Exactly `motifos-agent-job-projection`; always emitted. |
| `version` | Required integer; no default | Exactly `1`; always emitted. |
| `authority` | Required closed object; no default | Contains exactly the required `canonical_system` and `buzz_role` fields; always emitted. |
| `authority.canonical_system` | Required string; no default | Exactly `motifos`; always emitted. |
| `authority.buzz_role` | Required string; no default | Exactly `projection_only`; always emitted. |
| `conversation_id` | Required string; no default | Identifier grammar, 1-128 bytes; always emitted. |
| `mission_id` | Required string; no default | Identifier grammar, 1-128 bytes; always emitted. |
| `workstream_id` | Required string; no default | Identifier grammar, 1-128 bytes; always emitted. |
| `attempt_id` | Required string; no default | Identifier grammar, 1-128 bytes; always emitted. |
| `run_id` | Required string; no default | Identifier grammar, 1-128 bytes; always emitted. |
| `sequence` | Required integer; no default | From `1` through `2^53 - 1`, inclusive; always emitted. |
| `revision` | Required integer; no default | From `1` through `2^53 - 1`, inclusive; always emitted. |
| `idempotency_key` | Required string; no default | Uses the portable idempotency-key predicate: identifier grammar, 1-128 bytes, and rejects exactly 64 ASCII hexadecimal characters as a reserved Nostr event-ID shape; always emitted. |
| `actor` | Required closed object; no default | Contains exactly the required `seat_id` and `host_id` fields; always emitted. |
| `actor.seat_id` | Required string; no default | Exactly one of `wilson`, `scout`, `bambu`, `critic`, or `ledger`; always emitted. |
| `actor.host_id` | Required string; no default | Identifier grammar, 1-128 bytes; always emitted. |
| `transition` | Required closed object; no default | Contains the required `to` field and the optional-on-decode `from` field; always emitted. |
| `transition.from` | Optional string or `null` on decode; absence and `null` both become none | Otherwise, exactly one of `requested`, `accepted`, `running`, `succeeded`, `cancellation_requested`, or `failed`. The encoder always emits this field and writes `null` for none. Only kind 43001 accepts none for `null -> requested`; every other kind rejects a missing or null `from` through the transition matrix. |
| `transition.to` | Required string; no default | Exactly one of `requested`, `accepted`, `running`, `succeeded`, `cancellation_requested`, or `failed`; always emitted. The event-kind matrix above is authoritative for the allowed `from`/`to` tuple. |
| `occurred_at` | Required string; no default | Portable timestamp profile described below; always emitted and decoded as a UTC instant. Successful Rust encoding requires a year from 0 through 9999, inclusive. |
| `expires_at` | Required string; no default | Portable timestamp profile described below; always emitted and decoded as a UTC instant. Successful Rust encoding requires a year from 0 through 9999, inclusive. For active states it must be later than both `occurred_at` and the consumer-supplied validation time. Terminal records retain it and may be emitted or read after it passes. |
| `sensitivity` | Required string; no default | Exactly one of `public`, `internal`, or `restricted`; always emitted. |
| `artifacts` | Optional array; defaults to `[]` when absent | The encoder omits it when empty. Each item is a closed `{ "id": identifier }` object. At most 32 artifact and evidence items combined may appear. |
| `artifacts[].id` | Required string in each artifact object; no default | Opaque portable identifier, 1-128 bytes. It must be unique across both reference arrays. V1 has no URI or filesystem-locator field. |
| `evidence` | Optional array; defaults to `[]` when absent | The encoder omits it when empty. Each item is a closed `{ "id": identifier }` object. At most 32 artifact and evidence items combined may appear. |
| `evidence[].id` | Required string in each evidence object; no default | Opaque portable identifier, 1-128 bytes. It must be unique across both reference arrays. V1 has no URI or filesystem-locator field. |
| `relation` | Optional closed object or `null`; absence or `null` means none | The encoder omits it when none. When present as an object, it contains exactly the required `kind` and `target_idempotency_key` fields. Canonical producers omit it rather than emitting `null`. |
| `relation.kind` | Required string when `relation` is an object; no default | Exactly `corrects` or `supersedes`; emitted whenever a relation object is encoded. |
| `relation.target_idempotency_key` | Required string when `relation` is an object; no default | Uses the same portable idempotency-key predicate as `idempotency_key`, must be distinct from the current record's key, and refers to a prior record. Exactly 64 ASCII hexadecimal characters are rejected as a reserved Nostr event-ID shape. It is never a Nostr event ID. |
| `error_code` | Optional string or `null`; absence or `null` means none | The encoder omits it when none, and canonical producers omit it rather than emitting `null`. Kind 43006 requires a non-null machine-readable identifier of 1-64 bytes. Kind 43004 forbids a non-null value and requires at least one artifact or evidence reference. Every other kind also forbids a non-null value. |

Portable producers MUST encode `occurred_at` and `expires_at` as RFC 3339
timestamps with a four-digit year from `0000` through `9999`, a `T` separator,
and an explicit numeric UTC offset or `Z`. Consumers MUST accept that profile.
Values are normalized and compared as UTC instants. After semantic validation,
the Rust `encode` function enforces years `0..=9999` for both timestamps and
returns `invalid_schema` before serialization when either year is outside that
range. A successful Rust encode therefore emits the portable year profile and
normalized UTC timestamp spelling. The current Chrono-backed Rust `decode`
function accepts a permissive superset, including a space separator and
extended signed years. Those extension spellings may decode today, but they are
noncanonical and nonportable and producers MUST NOT emit them.

Artifact and evidence IDs are source-held, opaque, and portable. A future typed
resolver may map them to authorized content. Relation targets likewise use
portable, preferably namespaced idempotency keys.

Free-form error messages, prompts, transcripts, terminal logs, credentials,
cookies, tokens, chain of thought, and absolute filesystem paths MUST NOT appear
in a conforming record. This is a producer obligation, including for values
that happen to satisfy the identifier grammar.

### Stable error codes

The codec exposes the following stable, public, machine-readable codes:

| Code | Meaning |
| --- | --- |
| `unsupported_kind` | The supplied event kind is not a reserved agent-job kind. |
| `content_too_large` | Encoded content exceeds 16,384 bytes. |
| `invalid_json` | Content is malformed JSON or has trailing data. |
| `duplicate_field` | An object contains a duplicate member name. |
| `invalid_schema` | JSON does not match the closed schema or closed enum values, or an encoded timestamp year is outside `0..=9999`. |
| `unsupported_format` | `format` is not the exact v1 discriminator. |
| `unsupported_version` | `version` is not exactly 1. |
| `invalid_identity` | An identity field violates its identifier grammar or bound, or the primary `idempotency_key` has an invalid or reserved shape. |
| `invalid_sequence` | `sequence` or `revision` is outside the allowed range. |
| `invalid_transition` | The supplied kind and transition tuple do not match the matrix. |
| `invalid_expiry` | An active record violates the occurrence or validation-time expiry rule. |
| `invalid_reference` | A reference ID is invalid or duplicated across the reference arrays. |
| `too_many_references` | More than 32 combined artifact and evidence references are present. |
| `invalid_relation` | A relation target violates the shared portable idempotency-key predicate or self-references the current record. |
| `invalid_outcome` | The conditional reference or `error_code` shape is invalid for the supplied kind. |

These codes are content-free and do not echo input.

## Validation boundary

A successful `decode` or `encode` proves only the applicable record-local
properties implemented by this pure codec:

- closed JSON objects, schema, and enum values;
- wire-size and field bounds;
- identifier grammar;
- the supplied event kind and transition tuple;
- active expiry relative to `occurred_at` and the caller-supplied `now`;
- reference count and uniqueness across both arrays;
- the syntactic validity of both the current and relation idempotency keys;
- the conditional outcome shape for references and `error_code`; and
- for `encode` only, the portable timestamp year range `0..=9999`.

Success does not:

- inspect an identifier-shaped value and prove that it contains no secret,
  token, or private content;
- authenticate the signed author or bind that author to the claimed actor seat
  and host;
- prove that a relation's target prior record exists or resolve a correction or
  supersession;
- prove that sequence or revision values are monotonic across records;
- prove full lifecycle history, ordering, or canonical state;
- prove canonical timestamp spelling from `decode` success alone, because a
  permissive spelling may decode successfully;
- authorize any launch, tool, provider, or external action; or
- encrypt the record or choose its audience.

The content and secret exclusions above are producer MUST-NOT obligations.
Prior-record existence, monotonic ordering, correction resolution,
author-to-seat binding, semantic redaction, and lifecycle reconciliation
require a separate authenticated, stateful Wilson/MotifOS consumer. Buzz
remains projection-only.

## Authority and consumers

`canonical_system` may only be `motifos`, and `buzz_role` may only be
`projection_only`. Consumers reject all other values. A signed event, displayed
approval, channel reaction, or workflow state is never a launch authorization.
Consequential action requires a separate authenticated Wilson/MotifOS approval
receipt verified at the execution boundary.

The only transition-producing actor seats are `wilson`, `scout`, `bambu`,
`critic`, and `ledger`. Provider/model capability slots are not actors. Even a
valid seat/host is only a projection claim until a later authenticated
Wilson/MotifOS adapter binds the signed author to the seat/host.

Consumers reject the complete record on any schema, bound, transition,
reference, correction, authority, or expiry failure. Errors crossing a
process/UI boundary use stable codes and never echo payload.

## Privacy

The schema carries correlation and outcome metadata, not conversation content.
The enclosing event audience and encryption are chosen separately. The codec
does not encrypt, emit, relay, or widen visibility.
