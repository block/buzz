# Signed channel extension panel

Status: design proposal for [#3863](https://github.com/block/buzz/issues/3863).

This document is the contract slice of the panel work. It defines the generic
projection that a later Desktop implementation may render. It does not add a
relay event kind, a new HTTP endpoint, an integration adapter, or a panel to
the client.

## Product boundary

A panel is a compact, read-only projection of signed Buzz state in the current
channel. It is not a second source of truth and it does not become a workflow,
acceptance, accounting, or settlement authority.

Buzz owns:

- panel placement and rendering;
- community and channel permission checks;
- signature and source-event provenance;
- bounded parsing and fail-closed behavior; and
- fallback links to the source Canvas, workflow, handoff, or thread.

An integration owns:

- domain semantics and policy;
- matching, acceptance, and dispute decisions;
- accounting and settlement; and
- external credentials and execution.

The contract deliberately uses generic sections and fields. A panel may be
useful for an exchange, an incident, a deployment, or another workflow without
Buzz learning that domain's rules.

## View-model contract

The transport envelope remains an open decision for maintainers. The first
contract slice defines the content that a verified, channel-scoped source event
or adapter must provide:

```text
PanelManifest
├── schemaVersion       integer, currently 1
├── panelId             stable identifier within the channel
├── channelId           canonical channel UUID
├── title               short human-readable title
├── description         optional plain-text context
├── status              pending | active | complete | blocked | failed |
│                       stale | unavailable
├── updatedAt           Unix seconds
├── sections[]
│   ├── id              stable within the manifest
│   ├── title           short section heading
│   ├── status          same bounded status vocabulary
│   ├── fields[]
│   │   ├── label       short plain-text label
│   │   ├── value       plain-text value
│   │   └── presentation text | monospace | timestamp | status
│   └── links[]
│       ├── label       short action label
│       ├── target      canvas | workflow | handoff | thread | event | external
│       ├── sourceEventId  optional 64-character event id
│       └── uri         optional, only for an external HTTPS destination
└── sourceEvents[]
    ├── eventId         64-character event id
    ├── kind            unsigned event kind
    ├── channelId       canonical channel UUID
    └── label            short provenance label
```

The wire representation is JSON with the same names and types. Objects MUST
not contain unknown fields, and a client MUST reject a manifest whose
`schemaVersion` is not exactly `1`. Additive fields belong to a later schema
version; clients must not guess how to render an unknown version.

### Field rules

- All identifiers are ASCII strings with explicit length limits. `panelId` and
  section ids are at most 128 bytes; labels and titles are at most 256 bytes;
  field values and descriptions are at most 4,096 bytes.
- `channelId` is a canonical lowercase UUID. Every source event must carry the
  same channel id as the manifest.
- `eventId` and `sourceEventId` are lowercase 64-character hexadecimal event
  ids. A link that points to a Buzz object uses a source event reference rather
  than an arbitrary URL.
- `presentation` is an allowlist, not a CSS class or renderer name. Unknown
  hints fall back to `text` only when the schema version is known; unknown
  required enum values reject the manifest.
- `status` is conveyed by text and a semantic badge. Color is supplemental and
  must never be the only status signal.
- `external` links require an `https:` URI. The renderer does not fetch the URI,
  execute its contents, or embed it in an iframe.
- Sections, fields, links, and source events are bounded collections. A
  proposed first implementation caps them at 32, 64, 32, and 64 items
  respectively, and caps the UTF-8 manifest content at 32 KiB. These limits
  keep the payload below the relay's 65,536-byte frame limit with room for the
  signed event envelope.

## Signature and provenance

The panel may only be built from a verified Nostr event or from events already
verified by the relay and returned through the authenticated query path. A
future transport adapter must preserve the original signed event id and kind;
it must not replace source events with an unsigned integration response.

Before display, the client or adapter MUST:

1. resolve the current community from the configured relay and the current
   channel from the route/context;
2. require normal channel membership for the current reader;
3. verify that the manifest channel id equals the current channel id;
4. verify every source event reference is channel-local and was obtained from
   the same community;
5. verify the event id/signature when the raw signed event is available; and
6. reject malformed, oversized, stale-by-policy, or cross-channel data.

The panel displays source-event ids as inspectable provenance and offers a
deep link back to the source event. It must not infer authorship, acceptance,
settlement, or correctness from a presentation status alone.

## Transport decision still required

This proposal intentionally does not reserve a new event kind. Existing source
events cover several useful cases:

| Source | Existing kind | What it can provide |
| --- | ---: | --- |
| Canvas revision | 40100 | channel document and author/timestamp provenance |
| Job request/result | 43001–43006 | requested outcome, progress, result, or failure |
| Workflow run | 46001–46012 | trigger, step, approval, and terminal status |
| Thread/message | 9 / 40002 | conversation context and acknowledgements |

None of these alone is a generic panel manifest. Maintainers should choose
between composing a projection from those source events and registering one
new signed, channel-scoped manifest event. The eventual event choice must be
documented and tested before the Desktop panel PR. This contract PR must not
silently overload kind 9, Canvas, or workflow events with a second meaning.

Regardless of transport, all writes remain ordinary signed Nostr events and
must use the existing relay authorization, persistence, query, fan-out, audit,
and community-scoping paths. No bespoke HTTP API is part of this proposal.

## Desktop surface contract

The renderer PR should use the existing channel auxiliary-panel shell and make
the panel discoverable from the current channel. It should preserve reading,
threading, and composing in the channel instead of replacing the timeline.

Required states:

- loading: the panel location is stable while the source query resolves;
- empty: no panel projection is available, with a link to the channel's source
  Canvas or thread when one exists;
- ready: title, overall status, sections, last update, attribution, and source
  links are visible without raw JSON;
- stale: the last update is visible and the panel explains that the projection
  may no longer describe current source events;
- unavailable: the reader lacks access or the source cannot be resolved;
- invalid: malformed or unknown-version data is rejected with a concise error
  and a source-event fallback where possible.

The panel must remain usable in a narrow window. Headings and status badges use
semantic accessibility labels; source links are keyboard-focusable; status is
not communicated by color alone; and the raw manifest is available only as an
explicit inspection affordance, never as the primary presentation.

## Security boundary

The panel MUST NOT:

- execute JavaScript, WebAssembly, provider code, or arbitrary HTML;
- embed remote pages, iframes, plugins, or webviews;
- read another channel or community because a manifest names it;
- display private provider, buyer, credential, or integration data not present
  in an authorized source event;
- upload local files or scan a working directory;
- call matching, acceptance, payment, accounting, or settlement APIs; or
- turn a visual status into an authorization or business decision.

Unknown schemas, invalid signatures, cross-channel references, oversized
payloads, unsupported link schemes, and malformed identifiers all fail closed.

## Deterministic fixture

`docs/fixtures/signed-channel-panel.json` is a non-secret manifest fixture for
parser and renderer tests. It describes a small provider-and-deliverable
workflow using generic Buzz fields. It is not an OEXL integration, does not
contain credentials, and is not intended to be published to a live relay.

Tests that need a signed event should sign this content with an ephemeral test
key inside the test process. No private key belongs in the fixture or the
repository.

## Verification matrix for follow-up implementation

The protocol and Desktop slices should cover at least:

- valid version-1 manifest round-trip;
- unknown schema version rejection;
- malformed JSON, duplicate/unknown required fields, and oversized content;
- invalid event id/signature and a source event from another channel;
- unsupported status, presentation hint, and link scheme;
- loading, empty, ready, stale, unavailable, and invalid rendering;
- human and agent authorship attribution;
- keyboard navigation and screen-reader labels; and
- narrow-window layout without hiding the source-event fallback.

## Explicit non-goals

- OEXL matching, pricing, acceptance, dispute, accounting, payments, or
  settlement inside Buzz;
- an OEXL-specific mode or marketplace dashboard;
- replacing Canvases, workflows, jobs, threads, or the activity feed;
- a generic third-party plugin runtime;
- arbitrary remote UI; and
- a Desktop implementation in this contract PR.

The next PR may render this generic contract in the existing channel panel. An
integration adapter should follow only if maintainers want a stable adapter
surface after the transport and rendering contracts are accepted.
