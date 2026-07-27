# Command Team Conversations and Briefing Memory

## Outcome

Make the six HMAS Supply command advisers first-class Buzz agents. They must
appear in **My Agents**, support ordinary direct messages and group
conversations, retain compact outcomes from substantive discussions in Buzz's
existing encrypted agent memory, and make those outcomes available as evidence
to later Daily Command Briefs.

This phase connects existing Buzz capabilities. It does not introduce another
chat service, memory database, replication layer, or briefing orchestrator.
Buzz remains the signed discussion history, NIP-AE agent engrams hold durable
discussion outcomes, and the current command-brief source collector consumes
those outcomes alongside its existing sources.

## Existing Substrate

The implementation will reuse:

- built-in persona definitions and the My Agents catalogue;
- managed-agent provisioning, reuse, startup, profile, DM, channel membership,
  and mention flows;
- owner-encrypted NIP-AE agent engrams and the existing `buzz mem` commands;
- the owner-gated desktop engram reader;
- the existing command-brief source ledger and `SourceKind::Memory`;
- the existing model route, RAG, LAN Memory MCP, Apple inputs, citations,
  signed brief persistence, and fail-soft behaviour.

The LAN Memory MCP remains a separate authoritative source. Command-team
discussion outcomes are locally signed Buzz memories and remain distinguishable
in provenance even though both enter the brief as `SourceKind::Memory`.

## Command Team Personas

Add six stable built-in personas:

| Persona | Stable ID | Role |
| --- | --- | --- |
| Chief of Staff | `builtin:command-chief-of-staff` | Consolidation, challenge, priorities, and decisions |
| Operations Adviser | `builtin:command-operations` | Readiness, dependencies, risks, and current activities |
| Navigation Adviser | `builtin:command-navigation` | Navigation evidence, considerations, and source limitations |
| Daily Routine Adviser | `builtin:command-daily-routine` | Calendar, reminders, deadlines, and routine |
| Reporting Adviser | `builtin:command-reporting` | Reports, returns, inputs, and review milestones |
| Plans Adviser | `builtin:command-plans` | Medium- and long-range milestones, assumptions, and contingencies |

The persona prompts reuse the role boundaries already defined for the Daily
Command Brief. Navigation remains advisory and must not generate executable
navigation orders or make navigational decisions.

All six personas are active by default and appear together under a **Command
Team** group in My Agents using the approved symbolic naval identities. Built-in
persona merge remains idempotent, so an upgrade adds missing definitions without
duplicating an existing definition or managed-agent instance.

The personas are definitions until used. The app does not start six model
processes at launch. Selecting **Message**, mentioning an adviser, or adding one
to a discussion uses the existing managed-agent provisioning path, reuses an
existing instance with the same persona ID when possible, starts it when
required, and opens or updates the normal Buzz conversation.

The Command Console team cards refer to these same persona IDs and provide a
direct Message action. The console does not create a second set of agents.

## Discussion Outcome Capture

Buzz messages remain the complete signed transcript. Raw transcripts are not
copied into agent memory.

After a substantive adviser turn, the adviser records a compact outcome when
the discussion establishes at least one of:

- an accepted decision;
- an assigned action;
- a material risk;
- a confirmed assumption;
- an unresolved question that should be carried forward;
- a planning conclusion useful to a future brief.

Greetings, exploratory suggestions that were not accepted, repeated
information, and conversational filler are not recorded.

The adviser writes the outcome during the same turn through the existing
`buzz mem` path. This avoids a second summarisation model call. The response
only states **Recorded for future briefs** after the encrypted engram write
succeeds. If the write fails, the substantive response is still delivered and
the adviser states that the outcome was not recorded and can be retried.

If the user says not to retain an outcome, the adviser tombstones it. A
correction to the same discussion updates the same logical engram. A later
discussion that invalidates an earlier conclusion writes a new outcome that
identifies the earlier outcome as superseded.

## Memory Contract

Each entry uses a valid NIP-AE slug:

```text
mem/command-brief/<adviser>/<yyyy-mm-dd>/<outcome-id>
```

`outcome-id` is the lowercase 64-character hex digest of the stable adviser ID,
Buzz channel ID, and triggering Buzz event ID supplied in the current agent
context. Retrying the same adviser response therefore updates the same logical
entry instead of creating a duplicate. `origin.last_event_id` is that triggering
event ID; `origin.thread_root_event_id` is `null` when the discussion is not a
thread or no root is available.

The engram value is strict JSON:

```json
{
  "schema": "command-discussion-outcome-v1",
  "outcome_id": "lowercase-stable-id",
  "adviser": "operations",
  "recorded_at": "2026-07-27T10:00:00Z",
  "origin": {
    "channel_id": "uuid",
    "thread_root_event_id": null,
    "last_event_id": "hex-event-id"
  },
  "status": "active",
  "summary": "Concise durable outcome.",
  "decisions": [],
  "actions": [
    {
      "description": "Obtain the missing readiness input.",
      "owner": null,
      "due_at": null
    }
  ],
  "risks": [],
  "assumptions": [],
  "unresolved_questions": [],
  "brief_sections": ["operations"],
  "review_at": null,
  "supersedes": []
}
```

The permitted adviser and brief-section values reuse the existing command-brief
enums. `status` is one of `active`, `closed`, or `superseded`. Text and array
sizes are bounded before the write and again when collected. Unknown fields,
invalid timestamps, invalid origin identifiers, inconsistent adviser identity,
and unsupported enum values make an entry ineligible for briefing evidence.

The writing agent's persona ID and pubkey are authoritative. A memory body
cannot claim to be another adviser.

## Briefing Source Integration

Refactor the owner-gated engram read into a reusable native function used by
both the existing Tauri command and the command-brief source collector. The
collector only considers managed agents instantiated from the six stable
command-team persona IDs. Memories belonging to unrelated agents never enter
this source.

For each eligible command-team instance, the collector:

1. reads and decrypts the current NIP-AE heads as the owner;
2. filters to the `mem/command-brief/` namespace;
3. validates the strict outcome contract and persona/adviser match;
4. excludes tombstones and `superseded` outcomes;
5. includes `active` outcomes regardless of age;
6. includes `closed` outcomes for 90 days after `recorded_at`;
7. orders active outcomes first and then newest-first;
8. retains at most six outcomes per adviser and 24 across the team.

Each retained outcome becomes a normal validated source with:

- `source_kind = Memory`;
- `collection = command_team_discussions`;
- source/document identity derived from the engram event ID;
- adviser persona ID and agent pubkey in location metadata;
- originating Buzz channel and event references in location metadata;
- engram event time and outcome time;
- a canonical compact rendering of the structured outcome as the quote.

The existing source ledger, prompt budgets, model routing, citations, and
Chief-of-Staff consolidation then handle the evidence normally. If Cloud first
is selected, only the bounded outcome evidence may enter the current cloud
route, consistent with the existing briefing policy; raw Buzz transcripts are
never transmitted by this feature.

## Failure Behaviour

Command-team discussion memory is optional, additive evidence:

- no adviser instance or no eligible outcomes is a normal empty result;
- one unavailable adviser does not prevent memories from the other advisers;
- a malformed entry is excluded and counted in a bounded source warning;
- a truncated engram listing is used conservatively and reported as possibly
  incomplete;
- a complete source-read failure adds a concise warning but does not block or
  globally degrade the brief;
- external LAN Memory MCP failure behaviour remains unchanged;
- an adviser memory write failure does not suppress its conversational answer.

Superseded, expired closed, malformed, tombstoned, unrelated-agent, and
over-limit entries are never model-visible.

## User Experience

The user can:

- find all six advisers in My Agents under Command Team;
- start a DM from the agent list or Command Console;
- include one or more advisers in a Buzz discussion;
- inspect the signed discussion history normally;
- inspect adviser memory through the existing Agent Memory surface;
- ask an adviser to correct, close, forget, or re-record an outcome;
- see command-team discussion outcomes cited in a later brief under
  **Evidence**.

There is no separate memory-management dashboard in this phase.

## Implementation Boundaries

In scope:

- six built-in command-team personas and symbolic profile identities;
- Command Team grouping and Message actions;
- reuse of normal Buzz DM, mention, channel, and managed-agent flows;
- adviser prompt contract for substantive outcome capture;
- strict discussion-outcome schema and validation;
- reusable owner-gated engram reading;
- optional command-team memory collection into the brief evidence ledger;
- provenance, citations, fail-soft warnings, and focused tests.

Out of scope:

- storing or summarising every raw message;
- a new database, Memory MCP endpoint, replication service, or vector index;
- writing each outcome to both engrams and LAN Memory MCP;
- changing model routing or cloud fallback order;
- changing RAG, Apple inputs, external Memory MCP, or signed brief persistence;
- auto-starting all advisers when the app launches;
- external actions, ship systems, or Phase 5 workspace actions.

## Test and Acceptance Criteria

The phase is accepted when:

1. All six definitions appear once under Command Team in My Agents after a
   fresh install and an upgrade.
2. Each persona uses the approved symbolic identity and its existing advisory
   role boundary.
3. Message from My Agents and the Command Console provisions or reuses the
   correct persona instance, opens a Buzz DM, and starts the agent as needed.
4. The same adviser can be added to a normal group discussion without creating
   an unintended duplicate instance.
5. A substantive controlled discussion produces one valid encrypted
   `command-discussion-outcome-v1` entry and acknowledges it only after write
   success.
6. A trivial controlled exchange does not produce a discussion outcome.
7. Retry is idempotent; correction, close, supersede, and tombstone behaviour
   select the expected current evidence.
8. The collector accepts active outcomes, applies the 90-day closed rule,
   enforces per-adviser and total bounds, and rejects malformed or
   identity-mismatched entries.
9. The next generated Daily Command Brief can use the saved outcome and cites
   the adviser, engram event, and originating Buzz discussion.
10. Missing or broken memory for one adviser yields a partial brief with a
    concise warning, not a blocked run.
11. Unrelated managed-agent memories never enter command-team briefing
    evidence.
12. Existing RAG, Apple inputs, LAN Memory MCP, routing toggle, fallback,
    scheduling, cancellation, signed publication, and naval UI tests remain
    green.
13. Focused Rust/TypeScript tests, desktop E2E, a native controlled DM-to-memory
    acceptance, a subsequent brief-generation acceptance, and `just ci` pass
    before handoff.
