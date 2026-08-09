# Keeper relationship memory — v0.5.8 compatibility specification

Date: 9 August 2026
Status: frozen for typed-memory MVP implementation

## Outcome

Keeper is a first-party, owner-private Command Adviser for remembering people,
interactions, commitments, and useful context. The first release is deliberately
small: typed post-meeting debriefs and typed pre-meeting/on-demand briefs. Voice
and Calendar automation follow only after the typed loop is useful.

The synchronized Buzz v0.5.8 base already supplies the managed-agent, signed DM,
encrypted NIP-AE engram, structured-completion, notification, EventKit, and
Living Ship foundations. Keeper extends those foundations; it does not create a
second memory server, conversation store, scheduler, or agent runtime.

## User journeys

### Debrief

1. The owner opens a one-to-one DM with Keeper and describes a completed
   interaction in ordinary text.
2. Keeper identifies people, facts explicitly stated by the owner, commitments,
   follow-ups, preferences, and unresolved identity references.
3. Keeper shows a compact save receipt naming what was saved, what was treated
   as an observation, and what was quarantined for clarification.
4. The raw debrief remains in the signed Buzz DM. Keeper stores only compact
   outcomes and source event/thread references in its encrypted engrams.

### Brief

1. The owner asks Keeper for a brief about a person or upcoming interaction.
2. Keeper resolves one canonical person or reports the ambiguity.
3. Keeper returns relevant facts, recent interactions, open commitments,
   uncertainties, and source references without inventing missing context.
4. A later correction, forget, or undo request changes subsequent recall.

## Reused v0.5.8 components

| Need | Existing component | Keeper use |
| --- | --- | --- |
| Adviser identity and lifecycle | built-in managed personas in `desktop/src-tauri/src/managed_agents/personas.rs` | Add `builtin:keeper` with one owner-private instance |
| Conversation source | signed one-to-one Buzz DM events | Retain raw debrief and brief exchanges as the source record |
| Durable private memory | NIP-AE kind `30174`, `buzz-core::engram`, and `buzz mem` | Store Keeper records under `mem/keeper/*`, encrypted to the agent-owner pair |
| Extraction and summarization | `command_services::structured_completion` | Produce schema-validated candidate outcomes and briefs through current model routing |
| Agent visibility | unified managed-agent roster and Living Ship projection | Include Keeper from managed-agent metadata, not an unrelated fixed UI-only actor |
| Later calendar work | existing signed Apple-input helper and command-brief scheduler | Extend only in the later Calendar phase |
| Later notification work | native macOS notification activation routing | Notify only in the later Calendar phase |

## Memory model

All slugs remain within the NIP-AE grammar and the existing encrypted
agent-owner boundary.

- `mem/keeper/index` — compact schema/version marker and opaque person-ID index.
- `mem/keeper/person/<opaque-id>` — canonical identity, aliases, explicit
  durable facts, labelled observations, and current open commitments.
- `mem/keeper/interaction/<capture-id>` — compact interaction outcome with
  timestamps and source DM event/thread IDs.
- `mem/keeper/unresolved/<capture-id>` — names or references that must not be
  attached to a canonical person until resolved.

Records use opaque IDs rather than names in slugs. Display names and aliases are
encrypted values. The MVP may store a bounded JSON document as an engram value;
it must reject a write that exceeds the existing NIP-AE plaintext limit.

## Truth and identity rules

- Explicit owner statements may be stored as facts. Model inferences are stored
  only as labelled observations with their source and confidence.
- Duplicate or ambiguous names never merge automatically. The candidate outcome
  is quarantined in `unresolved` and the save receipt says so.
- A save receipt describes only writes that the relay accepted. Failed or
  partial writes are reported, not presented as saved.
- Corrections create the new authoritative record; forget creates the existing
  NIP-AE tombstone; undo restores the immediately prior value retained in the
  signed correction exchange or the current operation receipt.
- Keeper does not silently persist the whole transcript as memory. The signed DM
  remains the raw source.

## Privacy boundary

NIP-AE engrams are NIP-44 encrypted between Keeper and its owner. The initial
Buzz DM is access-controlled and signed but is not represented as NIP-17
end-to-end encryption; the UI and documentation must not claim otherwise.
Keeper must query and write only its current owner pair and current community.
Community switching must not retain Keeper-derived module-level caches.

## MVP implementation slices

1. Add and provision `builtin:keeper`, owner-private by default, and include it
   in the managed adviser roster.
2. Add pure typed Keeper contracts, validation, identity-resolution, and
   mutation planning with unit tests.
3. Add engram read/write/tombstone orchestration that uses the existing relay
   and NIP-AE primitives.
4. Add typed debrief extraction, truthful receipts, and ambiguity quarantine.
5. Add typed person brief, correction, forget, and immediate undo paths.
6. Add Living Ship presentation from the managed-agent roster.

## Acceptance tests

- A typed debrief about an unambiguous person changes a later Keeper brief.
- Two people sharing a name remain separate; an ambiguous debrief is quarantined.
- A correction changes recall, forget removes the record, and undo restores the
  immediately preceding record.
- Every recalled outcome can identify its source DM event or thread.
- A rejected relay write cannot produce a successful save receipt.
- Owner A cannot read or mutate owner B's Keeper memory, and switching
  communities exposes no Keeper data from the previous community.
- Restarting the desktop app preserves Keeper recall through relay-backed
  engrams.
- Keeper appears as a managed adviser and in Living Ship without a separate
  manually maintained runtime identity.

## Deferred scope

- audio capture, transcription, playback, and retained audio;
- automatic Apple Calendar matching, 15-minute pre-meeting scheduling, and
  notifications;
- face/contact ingestion, email scraping, or autonomous web research about
  people;
- shared/team relationship memory; and
- a replacement Memory MCP service or a new database.
