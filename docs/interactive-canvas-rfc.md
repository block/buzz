# RFC: Structured interactive channel canvases

- Status: Proposed
- Target: Buzz Desktop
- Storage: existing channel canvas event (`kind:40100`)
- Compatibility: existing Markdown canvases remain valid

## Summary

Buzz should extend its existing channel-scoped Markdown canvas with a small,
versioned set of structured, read-only components. An agent or human would
continue to write one Markdown document through the existing canvas APIs and
`buzz canvas set`. New desktop clients would render validated `buzz-canvas`
fenced blocks as trusted first-party components; clients that do not understand
the blocks would show readable JSON code blocks.

The first release should include metric cards, tables, task lists, callouts, and
grouped layouts. Charts, diagrams, and diff summaries should be added only if a
separate dependency and security review is satisfactory.

This RFC intentionally does not propose agent-generated JavaScript, HTML, CSS,
or JSX. Canvas content remains data, never executable code.

## Motivation

Agents often return useful information as long messages, wide Markdown tables,
or repeated status updates. That works for a conversation, but it is difficult
to scan and quickly becomes stale. A channel canvas is a better place for a
durable project dashboard, PR review, incident summary, or research comparison.

Buzz already has most of the collaboration foundation:

- `kind:40100` stores a signed canvas revision scoped by the channel `h` tag;
- relay ingestion requires `ChannelsWrite` for canvas updates;
- Desktop reads, edits, and renders the latest revision as Markdown;
- the CLI exposes `buzz canvas get` and `buzz canvas set`;
- ACP sessions receive canvas revision metadata and can fetch the current
  content when needed.

The missing layer is a bounded visual vocabulary and a first-class channel
entry point. The MVP can add those without a storage migration, new event kind,
or new ACP extension.

Cursor's canvases demonstrate the value of letting agents present
data-intensive work as interactive artifacts rather than prose. Buzz can adapt
that idea to a shared channel artifact whose revisions are signed,
attributable, and governed by existing channel permissions.

## Goals

- Make project status, PR review, incident response, and research comparison
  easier to scan than Markdown-only output.
- Preserve the current canvas event, CLI, and permission model.
- Preserve ordinary Markdown as the document shell and compatibility fallback.
- Render only validated data through trusted components bundled with Desktop.
- Make the canvas directly accessible beside the active channel.
- Keep manual Markdown editing available.
- Define deterministic limits and failure behavior before implementation.
- Provide accessible text or table semantics for every visual component.

## Non-goals

- Arbitrary JavaScript, JSX, HTML, CSS, or third-party embeds.
- Canvas-originated network requests.
- Buttons that run prompts, tools, workflows, or agent turns.
- Live queries or background refresh from canvas content.
- Multiple named canvases per channel.
- Public or team snapshot links.
- Direct manipulation, element annotation, or a design mode.
- Revision browsing or collaborative text editing.
- Rich-component parity on mobile in the first release.
- Replacing Markdown messages or the existing canvas editor.

## Existing behavior

Desktop currently exposes Canvas from channel management. It queries the latest
`kind:40100` event for the selected channel, renders its content through the
shared Markdown renderer, and allows authorized non-DM channel managers to edit
the complete document in a textarea. Archived channels are read-only.

A canvas event contains the document in `content` and the channel UUID in an
`h` tag. The event ID, author pubkey, and timestamp already exist in the query
response. The proposed renderer should use that event as-is.

## User experience

### Entry point

Add a persistent Canvas action to the active channel header. Opening it should
use Buzz's auxiliary-panel layout rather than requiring a trip through channel
management.

The panel follows the active channel and shows:

- channel and canvas title;
- author, update time, and abbreviated event ID;
- rendered Markdown and structured blocks;
- loading, empty, offline, stale, and invalid-block states;
- Edit for users who already have canvas-edit permission;
- Ask an agent to update, which creates a normal composer draft.

The agent handoff must not silently launch an agent. A suggested message such as
“Update this channel's canvas with the latest project status” is inserted into
the composer, where the user can choose an agent and send it through the normal
audited path.

### Desktop wireframe

```text
┌──────────────────────── Channel ────────────────────────┬──── Canvas ────────┐
│ #project-alpha                     [Canvas] [Search] … │ Project status      │
│                                                        │ by Fizz · 4m ago    │
│ messages                                               │ event a81c…9f2e     │
│                                                        │                     │
│                                                        │ [On track]  72%     │
│                                                        │                     │
│                                                        │ Blocked     2       │
│                                                        │ Checks      18/20   │
│                                                        │                     │
│                                                        │ Milestones          │
│                                                        │ ┌───────────────┐   │
│                                                        │ │ RFC      Done │   │
│                                                        │ │ Parser   Next │   │
│                                                        │ └───────────────┘   │
│                                                        │                     │
│                                                        │ [Ask agent] [Edit]  │
└────────────────────────────────────────────────────────┴─────────────────────┘
```

The panel should use the existing responsive auxiliary-panel behavior. On
narrow windows it may replace the message pane and provide an explicit Back
action. It should not force the message timeline below the minimum supported
width.

### Editing

The existing Markdown textarea remains the source editor. A first
implementation does not require a visual component editor.

When a structured block is invalid, the editor should:

1. keep the original source intact;
2. identify the block and validation problem;
3. render that block as a code block;
4. continue rendering the rest of the document.

Saving plain Markdown remains valid. Structured-block validation may warn
before save, but it must not make older documents uneditable.

## Document format

Markdown remains the outer document. Structured components use a fenced code
block with the exact language identifier `buzz-canvas`.

````markdown
## Release readiness

```buzz-canvas
{
  "version": 1,
  "id": "release-metrics",
  "type": "metrics",
  "title": "Release readiness",
  "items": [
    {
      "label": "Checks passing",
      "value": "18/20",
      "status": "warning"
    },
    {
      "label": "Open blockers",
      "value": 2,
      "status": "danger"
    }
  ]
}
```
````

The block payload is strict JSON:

- `version` is required and must be the integer `1`;
- `id` is required, unique within the document, and stable across updates;
- `type` is a required discriminator;
- unknown properties fail validation;
- duplicate JSON object keys fail validation;
- numbers must be finite JSON numbers;
- component order follows source order.

Requiring stable IDs prevents list keys and component state from changing when
an agent revises unrelated parts of the canvas.

### Common fields

Every v1 component accepts:

| Field | Type | Required | Notes |
| --- | --- | --- | --- |
| `version` | integer | yes | Must equal `1` |
| `id` | string | yes | `^[A-Za-z][A-Za-z0-9_-]{0,63}$` |
| `type` | string | yes | Allowlisted discriminator |
| `title` | string | no | Human-readable component heading |
| `description` | string | no | Plain text, not nested Markdown or HTML |
| `width` | string | no | `full`, `half`, or `third`; hint only |

The renderer may collapse requested columns when the panel is narrow. Source
order remains the reading and keyboard order.

### Required v1 components

#### Metrics

```json
{
  "version": 1,
  "id": "project-metrics",
  "type": "metrics",
  "title": "Project status",
  "items": [
    {
      "label": "Completion",
      "value": "72%",
      "detail": "Up 8% this week",
      "status": "success"
    }
  ]
}
```

Each item has `label`, scalar `value`, optional `detail`, and optional
`status`. Status is one of `neutral`, `info`, `success`, `warning`, or
`danger`. Status must be communicated with text or an icon as well as color.

#### Table

```json
{
  "version": 1,
  "id": "review-files",
  "type": "table",
  "title": "Files needing review",
  "caption": "Files ordered by review risk",
  "columns": [
    { "key": "file", "label": "File" },
    { "key": "risk", "label": "Risk" },
    { "key": "owner", "label": "Owner" }
  ],
  "rows": [
    {
      "id": "auth-rs",
      "cells": {
        "file": "crates/buzz-auth/src/lib.rs",
        "risk": "High",
        "owner": "Francis"
      }
    }
  ],
  "defaultSort": {
    "column": "risk",
    "direction": "desc"
  }
}
```

Columns define the only allowed cell keys. Cell values are strings, finite
numbers, booleans, or null. Sorting is local and deterministic. Filtering and
pagination can be added after the first renderer if they remain accessible.

#### Tasks

```json
{
  "version": 1,
  "id": "milestones",
  "type": "tasks",
  "title": "Milestones",
  "items": [
    {
      "id": "rfc",
      "label": "Agree on the document contract",
      "status": "done"
    },
    {
      "id": "renderer",
      "label": "Build the safe renderer",
      "status": "next",
      "detail": "Blocked on schema review"
    }
  ]
}
```

Task status is one of `todo`, `next`, `in_progress`, `blocked`, or `done`.
The component is read-only in v1; toggling a task must not mutate the canvas.

#### Callout

```json
{
  "version": 1,
  "id": "release-blocker",
  "type": "callout",
  "title": "Release blocker",
  "tone": "warning",
  "body": "Two authorization tests still fail on Windows."
}
```

Callout tone uses the same vocabulary as metric status. `body` is plain text.

#### Group

```json
{
  "version": 1,
  "id": "release-summary",
  "type": "group",
  "title": "Release summary",
  "columns": 2,
  "children": [
    {
      "id": "release-health",
      "type": "metrics",
      "items": [
        { "label": "Checks", "value": "18/20", "status": "warning" }
      ]
    },
    {
      "id": "release-note",
      "type": "callout",
      "tone": "info",
      "body": "Desktop can ship after the Windows failures are resolved."
    }
  ]
}
```

Nested components inherit version `1` from the group and omit `version`.
Groups may contain metrics, tables, tasks, or callouts. Groups cannot contain
other groups in v1.

### Optional components after dependency review

The following are deliberately outside the required first implementation:

- line and bar charts;
- pie or donut charts;
- Mermaid-style diagrams;
- diff and file-change summaries.

The repository currently has no dedicated chart or Mermaid dependency. Each
candidate needs a bundle-size, maintenance, license, accessibility, CSP, and
untrusted-input review. A rejected optional dependency must not block the
required component set.

## Validation and resource limits

Validation happens before React component creation. The parser should first
bound the source byte length, extract `buzz-canvas` fences, parse strict JSON,
and validate the discriminated schema.

Initial limits:

| Resource | Limit |
| --- | ---: |
| Canvas document | 256 KiB UTF-8 |
| Structured blocks per document | 32 |
| JSON payload per block | 64 KiB UTF-8 |
| Total rendered components, including group children | 64 |
| Group nesting | 1 level |
| Component title | 160 characters |
| Description, detail, or callout body | 4,096 characters |
| Metric items | 24 per component |
| Table columns | 20 per component |
| Table rows | 500 per component |
| Task items | 200 per component |

Limits should be exported from one schema module and covered by boundary tests.
They may be lowered after performance testing. Raising them should require
evidence from representative maximum-size documents.

## Rendering and fallback behavior

The Markdown parser should recognize `buzz-canvas` fences through a dedicated
plugin and replace only valid blocks with a typed syntax node. The React layer
maps that node to an exhaustive allowlist of first-party renderers.

The following behavior is required:

- ordinary Markdown renders exactly as it does today;
- a supported valid block renders as a structured component;
- an invalid block renders as its original code block;
- an unknown `type` or future `version` renders as its original code block;
- one renderer exception is isolated and cannot blank the whole canvas;
- source text remains available to editors;
- no structured value is passed to `dangerouslySetInnerHTML`;
- content cannot choose React components, class names, styles, event handlers,
  imports, image sources, or network endpoints;
- deterministic input produces deterministic output.

An unsupported client naturally displays the fenced JSON. This is less polished
than the rich component but preserves the data and does not require a parallel
storage format.

## Links and file references

Structured components do not render arbitrary Markdown. If a later schema
revision adds links, it should accept labeled values only through a dedicated
link object and reuse the shared Markdown link-opening policy.

For v1:

- callout, metric, and task text is plain text;
- table cells are plain scalar values;
- repository file paths are displayed as text unless the client can resolve
  them through an existing trusted file interaction;
- no remote images, iframes, video, or fetchable data sources are allowed;
- `javascript:`, `data:`, `file:`, and custom unreviewed schemes are rejected.

## Architecture

The change should remain client-side for the first release:

```text
kind:40100 content
        │
        ▼
existing Markdown parse
        │
        ├── ordinary Markdown ───────────────► existing renderers
        │
        └── buzz-canvas fenced block
                    │
                    ▼
            bounded strict JSON parse
                    │
                    ▼
             v1 schema validation
                    │
          ┌─────────┴─────────┐
          │ valid             │ invalid/unknown
          ▼                   ▼
 trusted component      original code block
```

Suggested Desktop boundaries:

- a pure parser/validator module with no React or Tauri dependency;
- typed renderers under the canvas feature rather than the generic message
  renderer;
- a narrow hook from the shared Markdown code-block path for the exact
  `buzz-canvas` language;
- an error boundary per structured block;
- a channel canvas panel that continues using the existing query and mutation
  hooks.

Keeping renderers in the canvas feature prevents structured canvas behavior
from silently becoming executable in ordinary chat messages. A follow-up may
choose to render the same data blocks in messages, but that is not part of this
RFC.

No relay, database, SDK, CLI, event-kind, or ACP protocol change is required.
The existing event ID, pubkey, and timestamp should be surfaced as revision
metadata in the panel.

## Permissions and concurrency

The renderer does not change authorization:

- relay writes continue to require `ChannelsWrite`;
- Desktop continues to expose editing only through its current channel
  moderation capability;
- archived channels remain read-only;
- DMs do not gain canvases;
- agents continue to submit signed updates through `buzz canvas set`;
- readers never gain an action that writes on their behalf.

The canvas remains a last-write-wins whole document. This RFC does not add
optimistic concurrency or merging. The editor should capture the event ID it
loaded and warn if the query observes a newer revision before save. Enforcing a
compare-and-swap contract would require a follow-up protocol decision.

## Security model

Canvas content is untrusted, including when it was authored by an agent.

Threats and required controls:

| Threat | Control |
| --- | --- |
| Script, HTML, CSS, or handler injection | Strict data schema; no raw HTML or evaluated code |
| Component selection outside the allowlist | Exhaustive discriminator mapping |
| Network exfiltration | No data sources, embeds, or component-originated fetch |
| Unsafe links | Shared URL policy and explicit scheme allowlist |
| Resource exhaustion | Byte, block, row, item, and nesting limits before render |
| Renderer crash | Per-block error boundary and code-block fallback |
| Action spoofing | No buttons or automatic actions in v1 |
| Cross-channel data leakage | Existing channel-scoped query key and `h` tag |
| Misleading status conveyed only by color | Visible text/icon status plus accessible name |
| Stale overwrite | Revision metadata and pre-save newer-revision warning |

Prompt-running buttons require a separate RFC covering visible provenance,
confirmation, permission checks at execution time, replay protection, signed
audit events, and offline-agent behavior.

## Accessibility

Every component must remain understandable without color, pointer input, or a
visual chart.

- Metrics use headings and named values in a logical reading order.
- Tables use native table semantics, a caption, column headers, and
  keyboard-operable sorting with announced sort state.
- Tasks expose textual status and list semantics.
- Callouts expose their title and tone without relying on color.
- Responsive grouping preserves DOM and reading order.
- Motion is unnecessary for v1; any later animation respects reduced motion.
- Focus never enters non-interactive cards.
- Zoom and narrow-panel tests use Buzz's rem-based type scale.

If charts are accepted later, each chart must include an equivalent data table
or concise textual summary.

## Representative PR-review canvas

````markdown
# PR #1234 review

The authorization change is small, but it affects two trust boundaries.

```buzz-canvas
{
  "version": 1,
  "id": "pr-health",
  "type": "metrics",
  "items": [
    {
      "label": "Files changed",
      "value": 7,
      "status": "neutral"
    },
    {
      "label": "Checks",
      "value": "18/20",
      "status": "warning"
    },
    {
      "label": "Review risk",
      "value": "High",
      "status": "danger"
    }
  ]
}
```

## Findings

```buzz-canvas
{
  "version": 1,
  "id": "review-findings",
  "type": "table",
  "caption": "Open review findings",
  "columns": [
    { "key": "severity", "label": "Severity" },
    { "key": "file", "label": "File" },
    { "key": "finding", "label": "Finding" }
  ],
  "rows": [
    {
      "id": "finding-1",
      "cells": {
        "severity": "High",
        "file": "crates/buzz-auth/src/lib.rs",
        "finding": "Expired challenges are accepted after relay reconnect."
      }
    },
    {
      "id": "finding-2",
      "cells": {
        "severity": "Medium",
        "file": "desktop/src/features/auth/hooks.ts",
        "finding": "The retry state has no accessible status announcement."
      }
    }
  ]
}
```

```buzz-canvas
{
  "version": 1,
  "id": "review-next-steps",
  "type": "tasks",
  "title": "Before approval",
  "items": [
    {
      "id": "fix-expiry",
      "label": "Reject expired challenges after reconnect",
      "status": "blocked"
    },
    {
      "id": "add-regression",
      "label": "Add a reconnect regression test",
      "status": "next"
    },
    {
      "id": "rerun-ci",
      "label": "Run the repository CI gate",
      "status": "todo"
    }
  ]
}
```
````

## Testing strategy

### Parser and schema

- plain Markdown with no structured blocks;
- one and multiple valid blocks;
- each component at minimum and maximum supported sizes;
- malformed JSON, duplicate keys, unknown fields, and wrong scalar types;
- missing, duplicate, and malformed IDs;
- unknown type and future version;
- excessive bytes, blocks, rows, items, text, and nesting;
- script, handler, HTML, CSS, URL, and prototype-pollution payloads;
- mixed valid and invalid blocks.

### Renderer

- deterministic snapshots for every component;
- renderer exception isolation;
- plain-text escaping;
- responsive one-, two-, and three-column layout;
- dark and light themes;
- keyboard-only and screen-reader semantics;
- reduced motion, zoom, and narrow-window behavior;
- no network activity caused by structured content.

### Channel integration

- canvas panel follows active-channel changes;
- event author, timestamp, and ID are correct;
- loading, empty, offline, and stale states;
- owner/admin/member/read-only permission paths;
- archived channel and DM behavior;
- manual editing and cancel/save error handling;
- agent update through the existing `buzz canvas set` command;
- old plain Markdown canvases render unchanged.

## Rollout

1. Land the schema, parser, required renderers, and tests behind a Desktop
   experiment.
2. Add the channel side-panel entry point and revision metadata.
3. Dogfood PR review and project status in private channels.
4. Complete performance, security, and accessibility review.
5. Enable the required read-only component set for willing testers.
6. Decide independently whether charts, diagrams, and diff summaries meet the
   dependency gate.
7. Measure whether at least two target workflows are materially easier to scan
   before expanding the artifact model.

Production implementation should be split into focused PRs. This RFC does not
authorize a single large renderer-and-UX change.

## Alternatives considered

### Keep Markdown only

This preserves simplicity but does not materially improve dense status,
comparison, or review workflows.

### Store a standalone JSON canvas

This makes the structured contract cleaner, but breaks the readable old-client
fallback and creates an unnecessary migration for the first release.

### Add a new event kind now

A new addressable artifact model is appropriate when Buzz needs multiple named
canvases, first-class revision history, or deep links. The single existing
channel canvas already satisfies the MVP.

### Execute generated React or HTML

This offers maximum flexibility at the cost of a much larger code-execution,
network, dependency, permission, and review surface. It is incompatible with
the safety goals of the MVP.

### Render every fenced block in messages too

That would increase reach, but it changes the trust and performance model of
the message timeline. The first implementation should remain isolated to the
canvas feature.

## Open questions

1. Is a `buzz-canvas` fenced JSON block the preferred compatibility contract,
   or should the payload use another data-only encoding?
2. Should pre-save validation be warning-only, or should it require an explicit
   “save with fallback” confirmation for invalid structured blocks?
3. Should the side panel have a user-resizable width, or rely entirely on the
   existing auxiliary-panel layout?
4. Is a newer-revision warning sufficient for v1, or should optimistic
   concurrency be part of the storage contract?
5. Which optional chart, diagram, or diff dependencies meet Buzz's bundle,
   license, security, and accessibility requirements?
6. Should rich rendering later expand to mobile, or should mobile continue to
   show the Markdown/code fallback until usage is proven?

## References

- Existing canvas UI:
  `desktop/src/features/channels/ui/ChannelCanvas.tsx`
- Existing Desktop canvas commands:
  `desktop/src-tauri/src/commands/canvas.rs`
- Existing event builder:
  `crates/buzz-sdk/src/builders.rs`
- Existing CLI commands:
  `crates/buzz-cli/src/commands/channels.rs`
- Existing relay authorization:
  `crates/buzz-relay/src/handlers/ingest.rs`
- Existing canvas foundation:
  [PR #130](https://github.com/block/buzz/pull/130)
- Existing agent CLI access:
  [PR #510](https://github.com/block/buzz/pull/510)
- Existing ACP canvas context:
  [PR #1755](https://github.com/block/buzz/pull/1755)
- Cursor canvas announcement:
  <https://cursor.com/blog/canvas>
- Cursor canvas changelog:
  <https://cursor.com/changelog/04-15-26>
