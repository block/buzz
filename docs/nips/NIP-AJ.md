NIP-AJ
======

Reliable Agent Jobs
-------------------

`draft` `optional`

This NIP defines correlation, lifecycle receipts, channel-scoped context,
attachment verification, and bounded two-agent debate semantics for Buzz agent
job events (`kind:43001` through `kind:43006`).  It does not define a model,
harness, or workflow engine.

## Motivation

A successful relay publish only proves that an event was accepted.  It does not
prove that an agent received the request, obtained the same context and
attachments as the requester, started execution, or produced a result.  Multi-
agent work adds another ambiguity: "five cycles" can be counted as five messages
instead of five complete exchanges.

Reliable automation needs a fail-closed contract that can answer:

1. which authorized channel context was given to the agent;
2. which exact attachment bytes and derived material were inspected;
3. which logical run, step, and retry emitted each event;
4. whether every lifecycle transition occurred in order; and
5. whether a requested debate completed exactly the promised exchanges.

## Definitions

- **Run**: A correlated execution identified by `run_id`.
- **Step**: One unit within a run, identified by `step_id`.
- **Attempt**: A positive integer retry number for one step.
- **Context pack**: A bounded snapshot of already-authorized channel inputs.
- **Materialization receipt**: Evidence that attachment bytes match the
  publisher's declared digest and size, plus explicit extraction/inspection
  status.
- **Cycle**: One proposer turn followed by one critic turn.  A partial cycle does
  not count.
- **Terminal receipt**: A `responded` or `failed` event with measured latency.

## Job kinds

This NIP uses the existing Buzz job kinds:

| Kind | Meaning under this NIP |
|------|-------------------------|
| 43001 | job request |
| 43002 | agent accepted (`received` or `queued`) |
| 43003 | non-terminal progress (`queued` or `executing`) |
| 43004 | terminal success (`responded`) |
| 43005 | cancellation request |
| 43006 | terminal failure (`failed`) |

No new kind is required.

## Common tags

Every event in a reliable job attempt MUST contain exactly one of each:

```jsonc
["p",       "<agent_pubkey>"],
["h",       "<channel_id>"],
["run",     "<run_id>"],
["step",    "<step_id>"],
["attempt", "<positive_decimal_integer>"]
```

`run_id` and `step_id` are opaque UTF-8 strings of 1 to 128 bytes.  Clients MUST
compare them byte-for-byte.  An attempt is identified by the tuple
`(run_id, step_id, attempt, h, p)`.

Events carrying duplicate common tags, an invalid `attempt`, or a different `h`
or `p` for an existing attempt are invalid and MUST NOT advance its state.

## Lifecycle receipts

The content of kinds 43002, 43003, 43004, and 43006 MUST be a JSON object:

```jsonc
{
  "schema": "buzz-agent-job-receipt/v1",
  "state": "received" | "queued" | "executing" | "responded" | "failed",
  "occurred_at": "<RFC3339 timestamp>",
  "latency_ms": 1234,
  "reason_code": null
}
```

`latency_ms` MUST be absent for non-terminal states and MUST be a non-negative
integer for terminal states.  It measures elapsed time from the request's local
receipt to the terminal state using a monotonic clock.  `reason_code` MUST be
absent except for `failed`, where it is REQUIRED and SHOULD come from a bounded,
documented vocabulary.  Human-readable detail MAY be included in a separate
`message` field and MUST NOT contain credentials.

Allowed transitions are:

```text
start -> received
received -> queued | failed
queued -> executing | failed
executing -> responded | failed
responded -> terminal
failed -> terminal
```

Receipt timestamps MUST be non-decreasing within an attempt.  There MUST be at
most one terminal receipt.  A client MUST NOT display a job as completed from a
relay `OK`, ACP transport acknowledgement, or `received` receipt.

Duplicate delivery of the same signed receipt is idempotent.  Conflicting
receipts for the same transition MUST fail closed and surface an audit error.

## Channel context pack

A 43001 request MAY carry a `context_pack` object:

```jsonc
{
  "schema": "buzz-channel-context-pack/v1",
  "channel_id": "<same value as h>",
  "trigger_event_id": "<event id>",
  "recent_events": [
    {
      "event_id": "<event id>",
      "created_at": 1700000000,
      "author_pubkey": "<pubkey>",
      "kind": 9,
      "content": "<message content>"
    }
  ],
  "summary": "<optional channel summary>",
  "summary_revision": "<optional digest or address>",
  "canvas_revision": "<optional digest or address>",
  "decisions": ["<decision>"],
  "tasks": [{"id": "<task id>", "text": "<task>"}],
  "attachment_receipts": ["<receipt event id or digest>"]
}
```

The pack MUST be built only after authorizing each included source event for the
requesting principal and the channel identified by `h`.  The trigger event MUST
be present.  Implementations MUST apply explicit message-count and byte limits.
They MUST NOT silently pull events from another channel, community, direct
message, search result, or cached session, even when an event identifier is
known.

The request content MUST include `context_sha256`, the lowercase SHA-256 of the
canonical JSON serialization of `context_pack`.  Agents MUST reject a pack whose
digest does not match.

Context is a snapshot, not authority.  Possessing a pack or event ID does not
grant future reads.

## Attachment materialization

Every attachment supplied to an agent MUST have a materialization receipt before
the agent claims to have read it:

```jsonc
{
  "schema": "buzz-attachment-materialization-receipt/v1",
  "source_event_id": "<event id>",
  "channel_id": "<same value as h>",
  "sha256": "<lowercase SHA-256>",
  "declared_mime": "application/pdf",
  "size_bytes": 12345,
  "bytes_verified": true,
  "text_extracted": true,
  "text_sha256": "<lowercase SHA-256 or null>",
  "image_sha256s": ["<lowercase SHA-256>"],
  "images_inspected": false,
  "extractor": "<name/version>",
  "status": "materialized" | "failed",
  "error_code": null
}
```

The downloader MUST authenticate as the job principal, prove that
`source_event_id` is readable in the channel identified by `h`, and obtain the
blob through the relay's authenticated media read path.  It MUST verify both the
declared byte count and SHA-256 before extraction.

`text_extracted` and `images_inspected` are independent.  Extracting embedded
images is not visual inspection.  When an attachment contains images,
`reading_complete` MUST remain false until every listed `image_sha256` has been
inspected by the declared extractor or model.  Unsupported formats MUST produce
an explicit failed or partial receipt; they MUST NOT be treated as empty text.

Hash reuse MUST NOT widen channel access.  Blob storage may be content-addressed,
but authorization grants MUST remain scoped to the referencing channel events.

## Bounded two-agent debates

A reliable debate request contains:

```jsonc
{
  "schema": "buzz-agent-debate-plan/v1",
  "artifact_sha256": "<lowercase SHA-256>",
  "cycles": 5,
  "proposer_pubkey": "<pubkey A>",
  "critic_pubkey": "<pubkey B>",
  "termination_criterion": "publish independent conclusions"
}
```

`cycles` MUST be between 1 and an implementation-defined bounded maximum.  The
participants MUST be distinct and both MUST be authorized for `debate.participate`
in the channel.

Each cycle has exactly two ordered turns:

1. proposer publishes `C<n>/<total> proposer` and hands off to the critic;
2. critic publishes `C<n>/<total> critic`, references the proposer turn, and
   hands off to the proposer (or the finalizer after the last cycle).

Every turn MUST carry:

```jsonc
["artifact", "<artifact_sha256>"],
["cycle", "<n>", "<total>"],
["role", "proposer" | "critic"],
["p", "<next participant pubkey>"]
```

The critic turn MUST additionally reference its proposer turn with an `e` reply
tag.  Both turns MUST use the common `run`, `step`, `attempt`, and `h` tags.

Therefore five cycles require ten valid turns.  A proposer turn without its
critic turn is a half-cycle and does not advance the completed-cycle counter.
Duplicate roles, wrong actors, missing cryptographic handoffs, wrong artifacts,
or cross-channel events invalidate the run.  Plain text such as `@Agent` is not a
cryptographic handoff.

Implementations MUST stop at the requested cycle count.  They MAY stop earlier
only for cancellation, failure, or an explicit termination criterion encoded in
the plan; an early stop MUST emit a terminal failure or result explaining why.

## Channel access control

Reliable jobs are fail-closed on channel boundaries:

- every context, attachment, participant, and result MUST resolve to the same
  community and `h` channel as the request;
- the current principal MUST be a readable member of that channel at read time;
- an agent MUST be explicitly authorized for each required operation, such as
  `context.read`, `attachment.read`, or `debate.participate`;
- event IDs, media hashes, cached context, owner relationships, and membership in
  another channel MUST NOT bypass these checks;
- revocation MUST apply to subsequent reads even when bytes remain cached.

Cross-channel workflows require a separate, explicit delegation protocol and are
out of scope for v1.

## Recovery and retries

Retries increment `attempt` and preserve `run_id` and `step_id`.  Receipts from
different attempts MUST NOT be folded into one lifecycle.  After a crash, a
runtime reconstructs state from valid receipts and may resume only from the last
non-terminal attempt.  It MUST NOT infer successful completion from an agent
message lacking the correlated terminal receipt.

## Security considerations

**Confused deputy.** An owner-agent relationship does not authorize every
channel.  Authorization is evaluated for the concrete principal, operation,
community, and channel.

**Attachment substitution.** URLs and filenames are descriptive.  SHA-256 and
size are authoritative and are verified before parsing.

**Parser risk.** Attachment extraction processes untrusted input.  Extractors
SHOULD run with bounded CPU, memory, output size, file count, path traversal
protection, no network access, and an explicit timeout.

**False completion.** Transport acknowledgements, partial debates, extracted-but-
uninspected images, and uncorrelated messages are not completion evidence.

**Sensitive context.** Implementations SHOULD minimize persisted plaintext packs
and receipts.  Receipts MUST NOT contain private keys, bearer tokens, local paths,
or raw authorization material.

## Relationship to other specifications

- **NIP-29** supplies the group/channel identifier used by `h`.
- **NIP-42** supplies relay authentication.
- **NIP-92** supplies `imeta` attachment metadata.
- **NIP-AO** supplies optional ephemeral telemetry; its frames do not replace
  durable job lifecycle receipts.
- Buzz job kinds 43001-43006 supply the request, progress, result, cancellation,
  and error event families extended by this NIP.
