# Agent context compiler

Buzz compiles every event-driven ACP turn into one deterministic dynamic
envelope. Standing platform, persona, team, core-memory, and channel-canvas
content remains in `session/new`'s system role when an adapter supports it; the
manifest still records those inputs and their session-start freshness.

Standing context uses the same paired-boundary convention so long sections are
easy to delimit:

```xml
<workspace>...</workspace>
<base>...</base>
<system>...</system>
<team-instructions>...</team-instructions>
<core-memory>...</core-memory>
<huddle-instructions>...</huddle-instructions>
<channel-canvas>...</channel-canvas>
```

```xml
<buzz-turn>
  <ambient-context>
    Scope: thread
    Channel: engineering
    <recent-thread-delta included="2" total="2" truncated="false">
      Morgan: The deploy is failing.
    </recent-thread-delta>
  </ambient-context>
  <event type="mention" author="Morgan"
      actor="pubkey" event_id="event-id">
    Content: Fix the deployment failure.
  </event>
  <delivery ... />
</buzz-turn>
```

The grammar deliberately uses tags only at ambient context, history, current
event, and delivery boundaries. Human-readable fields carry ordinary channel and message
content without duplicating every fact in nested attributes. Natural-language
content and attribute values are still escaped deterministically. The format
is XML-like but is not an XML protocol: consumers should recognize the
semantic boundaries and must not require a general XML parser. Raw
Nostr tags are deliberately absent. Event IDs, actors, normalized thread
routing, and mentions remain in the envelope; a signed event can be fetched
when needed with `buzz messages raw --event …`.

Recent ambient messages stay deliberately terse (`Author: content`). Their
signed event IDs, timestamps, and actor keys still feed the content-addressed
layer version, so different relay heads produce different manifests without
repeating forensic metadata in the model's active context.

Every model-visible ambient fragment participates in the manifest. Retrieval
status and interrupted-turn evidence therefore change the manifest even when
the current signed instruction is unchanged. Channel metadata and channel
canvas are separate order-4 entries: metadata is resolver-cached, while canvas
content is frozen when the ACP session starts.

The pure compiler does no I/O and reads no clock, environment variable, or
session identifier. Present layers are ordered canonically, empty layers are
omitted, content versions are SHA-256 hashes unless a signed source event ID is
available, and the manifest hash covers the canonical serialized layer array.
The manifest is emitted to local observer diagnostics and included in encrypted
NIP-AM metrics when the harness reports usage.

An ACP steer accepted as `injected` is appended to the active turn ledger by
the prompt read loop before it acknowledges delivery. The terminal NIP-AM
metric consequently hashes both the original prompt and every accepted update;
failed or indeterminate steers are not added. Local diagnostics emit
`prompt_context_steer` with the resulting manifest. An adapter response of
`startedNewTurn` is reported as a distinct diagnostic and is deliberately not
folded into the already-settled turn's manifest.

Queue dispatch is the authority boundary. Contiguous events are coalesced only
when both their signing actor and destination match. A thread's root event ID is
its destination; each top-level event is its own destination. Different actors,
thread roots, and unrelated top-level instructions therefore produce separate
turns.

## Significant implementation diversions

- There is one Buzz-owned compiler in `buzz-acp`, rather than a duplicate
  compiler inside `buzz-agent`. Native `buzz-agent` is an ACP adapter and
  consumes the same compiled envelope as Goose, Codex, and Claude adapters.
  Keeping two renderers would weaken the determinism requirement.
- Layer 5 (`thread_checkpoint`) is omitted because Buzz does not yet have the
  checkpoint artifact proposed in Proposal 4. The manifest intentionally omits
  absent layers instead of fabricating a checkpoint.
- The proposal's attention caps remain experiments, as requested, rather than
  hard-coded budgets. Existing bounded thread retrieval supplies the current
  truncation signal; the compiler records it without introducing a second,
  unvalidated truncation policy.
- Provider-specific four-slot cache partitioning is not forced through ACP,
  which cannot express cache breakpoints. Native `buzz-agent` retains its
  existing stable system/tool prefix and rolling-tail breakpoints; deterministic
  serialization improves automatic prefix reuse for every adapter. A distinct
  channel-snapshot breakpoint requires an ACP capability or native-only prompt
  interpretation and is deferred rather than creating adapter drift.
- Rollout is direct rather than feature-flagged. The legacy bracketed parser is
  retained in Desktop diagnostics for stored observer history, while all new
  event-driven turns use the semantic envelope.
