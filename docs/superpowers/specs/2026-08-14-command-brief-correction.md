# Daily Command Brief correction and simplification

Date: 14 August 2026  
Status: approved for implementation

## Outcome

The Daily Command Brief will complete reliably with Battle Rhythm and Plans
evidence, distinguish a failed run from the last successful brief, and present a
short command-focused assessment instead of filling sections with weak RAG
matches or internal source-policy messages.

This is a correction phase. It does not add a new service, database, security
layer, retrieval engine, or adviser. Existing encrypted persistence, signed Buzz
publication, RAG, World Monitor, Apple inputs, planning data, and model routing
remain in place.

## User experience

- A successful run shows the current brief and its generation time.
- A failed run shows the actual failed-run status. If an older brief exists, it
  is visibly labelled `Last successful brief` with its own timestamp; it is never
  presented as the output of the failed run.
- `Today at a glance` is a concise Chief of Staff synthesis of material findings,
  normally five to seven bullets or fewer.
- `Decisions and approvals required` is populated only from evidence-cited
  specialist proposals. If none exist, it says so plainly.
- Empty or weak source coverage results in an empty section or one useful gap,
  not a quotation included merely because retrieval returned it.
- Internal messages such as source permission filtering, catalogue audit rows,
  raw source identifiers, and prompt-injection control language do not appear as
  operational missing information.
- When World Monitor is connected, the redundant Connect button is hidden.

## Persistence correction

The authoritative core wire contract will accept every source kind the desktop
brief contract can persist, including `battle_rhythm` and `plans`. Regression
coverage will serialize, validate, sign, and store a brief containing both source
kinds so contract drift cannot recreate `brief_persistence_failed`.

Persistence keeps the existing bounded user-facing error code. Internal logging
will retain the failing stage—contract validation, signing, local storage, or
publication—without exposing sensitive payloads in the UI.

## Evidence selection

Retrieval remains source-bound, but it becomes adviser-specific:

- doctrine checks use the `ADF Doctrine` collection when available;
- navigation uses maritime-navigation and weather collections;
- operations and plans use doctrine, ship-program, and strategic-planning
  collections;
- logistics uses doctrine, HMAS Supply, Navy publication, and logistics
  collections;
- reporting and daily routine use ship, Navy publication, Battle Rhythm, Apple,
  Memory, and command-team sources rather than generic software collections;
- Maritime N2 receives doctrine and relevant strategic material plus its separate
  World Monitor collection path.

The allowlists are intersected with the live RAG catalogue. Missing optional
collections cause the relevant retrieval to be skipped rather than widened to
every collection.

Catalogue snapshots remain in the audit ledger but are not sent to advisers as
substantive evidence. Retrieved text remains non-authoritative for instructions;
the model envelope describes it as evidence with no instruction authority, not
as factually `untrusted`.

## Adviser and Chief of Staff contracts

Specialists may return no findings when the available material does not support a
useful assessment. Prompts explicitly prohibit filler and require decision
relevance.

Every proposed action includes exact `sourceIds`. Proposals without admitted
sources are rejected. Valid proposals are projected into the Decisions section
and retain adviser provenance.

The Chief of Staff may rewrite and combine validated specialist findings into a
concise executive summary. Every synthesis bullet must cite only source IDs that
appear in the validated specialist material. The Chief cannot add uncited facts,
remove recorded dissent, or execute actions.

## Acceptance

1. A brief containing Battle Rhythm and Plans persists and reloads.
2. A failed run never labels an older brief as current.
3. Broad daily-routine retrieval does not query all collections or return
   product-documentation sources.
4. Internal permission/audit language is absent from model-visible limitations.
5. Decisions are derived from cited specialist proposals.
6. Chief synthesis is concise, source-bound, and preserves dissent.
7. Existing RAG, World Monitor, Apple, planning, mobile, and signed-persistence
   tests remain green.

