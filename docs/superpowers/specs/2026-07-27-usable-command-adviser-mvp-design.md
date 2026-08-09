# Usable Command Adviser MVP Design

## Outcome

Ship a usable macOS Buzz Command Console that generates a practical Daily
Command Brief from real Apple data, the owner's trusted LAN RAG and Memory
services, and five specialist advisers coordinated by a Chief of Staff.

The acceptance test is three consecutive live briefs. Each brief must complete,
show the available Apple inputs, cite retrieved RAG or Memory evidence, retain
the five specialist contributions, and present a consolidated command view.

## Keep

- The existing Buzz macOS app and Command Console UI.
- Operations, Navigation, Daily Routine, Reporting, and Plans advisers.
- Chief of Staff consolidation.
- Direct HTTP access to the configured LAN RAG and Memory MCP services.
- LM Studio first, followed by LiteLLM and OpenAI when local execution fails.
- Real Apple Calendar, Reminders, Notes, and selected-file reads.
- Existing signed Buzz history and audit persistence where it does not block a
  usable brief.

## Simplify

- Trusted LAN sources are ordinary configured inputs. They do not require
  signed snapshots, replication, fingerprint equality, or an OFFICIAL egress
  policy.
- Catalogue observations are informational. A recheck cannot invalidate a
  brief that already collected its cited passages.
- A failed source or specialist creates a visible limitation and a partial
  section. It does not fail the whole brief.
- If model-based Chief of Staff consolidation fails or returns unusable JSON,
  the app constructs a deterministic consolidation from the validated
  specialist contributions. The brief still completes and visibly notes the
  fallback.
- Apple allowlists contain real identifiers selected from this Mac rather than
  sentinel values.

## Exclude

- Memory replication and conflict protocols.
- Signed RAG snapshot export/import.
- RAG fingerprint admission gates.
- Workspace-action execution work from Phase 5.
- Mesh compute and remote coding agents.
- Further security or assurance architecture.

## Failure Behaviour

- Cancellation and local persistence failure remain terminal.
- RAG, Memory, Calendar, Reminders, Notes, files, an individual specialist, or
  Chief model consolidation are fail-soft.
- A completed partial brief lists exactly which input or adviser was
  unavailable and never invents replacement evidence.

## Live Acceptance

1. Launch the exact signed `Buzz.app` produced from this branch.
2. Verify Calendar, Reminders, Notes, and selected files use real local
   identifiers and return current records or an honest empty result.
3. Verify the configured LAN RAG and Memory services return cited evidence.
4. Generate three consecutive briefs.
5. For each run, verify a completed/degraded result exists, all five adviser
   slots are present, consolidation is readable, Apple input status is visible,
   and RAG or Memory citations are retained.

