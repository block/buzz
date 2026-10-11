NIP-AJ
======

Agent Job Result Handoffs
-------------------------

`draft` `optional`

This NIP defines a versioned, inspectable payload for Buzz agent job result events. It keeps the existing signed, channel-scoped agent job lifecycle (`kind:43001` through `kind:43006`) and gives `kind:43004` a stable result contract that clients can render without reconstructing the full source thread.

## Motivation

A plaintext final message such as "done" does not tell a requester which outcome was attempted, what was produced, what verification ran, or whether work is complete, partial, blocked, failed, or intentionally has no durable artifact. These distinctions matter when work is delegated between agents or reviewed on a device that does not have the entire source thread in view.

The result event is the handoff. It carries explicit references and evidence; producers do not scan or upload local workspaces implicitly.

## Event

`kind:43004` is a regular, signed event. It is stored, append-only, and scoped to the same channel as the originating job request.

```json
{
  "kind": 43004,
  "pubkey": "<agent_pubkey>",
  "created_at": 1785048000,
  "content": "<job-result-json>",
  "tags": [
    ["h", "<channel_uuid>"],
    ["e", "<kind-43001-event-id>", "", "reply"],
    ["schema", "buzz.job-result", "1"],
    ["disposition", "completed"]
  ],
  "sig": "..."
}
```

The event MUST contain exactly one channel `h` tag and MUST reference the originating job request with an `e` reply tag. The event content's `jobRequest` value MUST match that `e` tag. The `schema` and `disposition` tags are query and summary hints; clients MUST treat the signed content as authoritative and MUST reject a structured rendering when required content validation fails.

The event uses the existing relay event path. No additional HTTP endpoint, scheduler, or worker loop is required.

## Version 1 Payload

The UTF-8 event content is a JSON object:

```json
{
  "schemaVersion": 1,
  "jobRequest": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  "requestedOutcome": "Make the completed work inspectable.",
  "outcome": "The structured handoff is ready for review.",
  "lastProgress": "The full repository gate passed.",
  "disposition": "completed",
  "artifacts": [
    {
      "kind": "pull_request",
      "label": "Implementation pull request",
      "reference": "https://github.com/block/buzz/pull/1234",
      "sourceState": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
    }
  ],
  "verification": [
    {
      "label": "just ci",
      "status": "passed",
      "evidence": "Exit status 0 at commit bbbbbbb."
    }
  ]
}
```

Required fields:

- `schemaVersion`: integer `1`.
- `jobRequest`: the 64-character hexadecimal event id of the originating `kind:43001`.
- `requestedOutcome`: the outcome the requester asked for.
- `outcome`: a concise final result.
- `disposition`: one of `completed`, `partial`, `blocked`, `failed`, or `no_artifact`.

Optional fields:

- `lastProgress`: the last meaningful phase or progress reached.
- `artifacts`: zero or more artifact references.
- `verification`: zero or more verification results.
- `blocker`: the concrete condition preventing completion.

Consumers MUST ignore unknown object fields when `schemaVersion` is supported. Producers MUST reject unsupported schema versions rather than publishing a payload they cannot validate.

### Disposition invariants

- `completed` requires at least one artifact reference.
- `no_artifact` requires an empty artifact list and represents completed analytical or advisory work that intentionally produced no durable artifact.
- `blocked` requires a non-empty `blocker`.
- `partial` and `failed` MAY include artifacts, verification, and a blocker when those fields help a reviewer understand the usable result and remaining work.

## Artifact References

Each artifact contains:

- `kind`: one of `file`, `media`, `branch`, `commit`, `pull_request`, `canvas`, `workflow_output`, `build`, `deployment`, `link`, or `other`.
- `label`: a reader-facing name.
- `reference`: an uploaded URL, Buzz reference, repository-relative path, branch/ref, object id, workflow/build id, or equivalent stable reference.
- `sourceState`: optional commit, branch, workflow run, build id, deployment revision, or equivalent provenance.

URL-bearing references use `http`, `https`, `buzz`, or `nostr`. HTTP(S) references require a host, and URLs containing embedded credentials are invalid. File references MUST be uploaded URLs or repository-relative paths; absolute local paths, upward traversal, and `file:` URLs are invalid. Commit references MUST be a full 40- or 64-character hexadecimal object id or a supported URL.

Artifacts are references, not implicit uploads. A producer MUST NOT read, attach, or publish local file contents merely because a path appears in a handoff manifest.

## Verification

Each verification item contains:

- `label`: the check or review performed.
- `status`: `passed`, `failed`, or `not_run`.
- `evidence`: optional concise output, source revision, or result URL.

`not_run` is distinct from `passed`. Producers MUST NOT infer verification from the existence of an artifact, successful event publication, a merged pull request, or an unrelated CI run.

## Limits

- Serialized content MUST NOT exceed 65,536 bytes.
- `artifacts` and `verification` are each limited to 50 items.
- `requestedOutcome` and `outcome` are each limited to 8 KiB.
- `lastProgress` and `blocker` are each limited to 4 KiB.
- Labels are limited to 512 bytes.
- Artifact references and verification evidence are limited to 2 KiB.
- Artifact source state is limited to 512 bytes.

Required text MUST contain a non-whitespace character. Labels, artifact references, and source state MUST be single-line and free of control characters.

## CLI

Agents can publish a validated handoff from a file:

```bash
buzz jobs handoff \
  --channel <channel-uuid> \
  --job <kind-43001-event-id> \
  --manifest result.json
```

Or through standard input:

```bash
generate-result-manifest |
  buzz jobs handoff \
    --channel <channel-uuid> \
    --job <kind-43001-event-id> \
    --manifest -
```

The CLI validates JSON, schema, cross-field invariants, references, item limits, size, and the match between `jobRequest` and `--job` before it signs or submits an event. A successful write uses the CLI's standard `{event_id, accepted, message}` response.

## Client Behavior and Compatibility

Clients that recognize a valid version 1 payload SHOULD render the disposition, requested outcome, outcome, progress, artifacts, verification, and blocker as distinct fields. Home or notification summaries SHOULD use the outcome and disposition rather than showing raw JSON.

Legacy or malformed `kind:43004` content remains valid plaintext job-result content. A client that cannot validate the structured payload MUST fall back to its existing plaintext or Markdown rendering. It MUST NOT render partially parsed fields as trusted handoff metadata.

## Security and Privacy

Job result content is visible to channel members under the channel's existing access policy. Producers MUST NOT include credentials, environment variables, private local paths, system prompts, tool inputs/results, or copied local file contents unless the requester explicitly chose to publish that material and the channel is appropriate.

Artifact labels and references are untrusted event content. Clients MUST escape text and restrict clickable links to schemes they can open safely. A signed event proves authorship; it does not prove that an external URL is safe, that an artifact still exists, or that a reported verification result is honest.

The schema improves accountability by making claims explicit. It is not a remote-attestation or reputation protocol.
