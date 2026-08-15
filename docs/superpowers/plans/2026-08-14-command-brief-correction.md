# Daily Command Brief correction implementation plan

**Goal:** Repair current-brief persistence and make the generated brief concise,
decision-focused, and honest about sparse evidence.

**Architecture:** Keep the existing orchestration and data services. Align the
desktop and core wire contracts, route RAG retrieval through per-adviser
collection allowlists, separate audit-only sources from model evidence, require
cited proposed actions, permit bounded Chief of Staff synthesis, and make the UI
explicitly distinguish failed runs from prior successful output.

**Tech stack:** Rust/Tauri, TypeScript/React, SQLite audit store, Nostr signed
events, MCP RAG and World Monitor, Node tests, Playwright.

## Task 1: Persistence contract and diagnostics

- [x] Add failing core/desktop tests for `battle_rhythm` and `plans` wire
  round-trips and signed persistence.
- [x] Add the missing core source kinds.
- [x] Preserve the bounded UI error while logging the precise persistence stage.
- [x] Run focused Rust tests.

## Task 2: Failed-run presentation

- [x] Add a failing UI test where a new run fails while an older brief exists.
- [x] Label the older content `Last successful brief` with its generation time.
- [x] Ensure the failed run remains the primary status.
- [x] Run focused hook and UI tests.

## Task 3: Evidence routing and prompt hygiene

- [x] Add failing tests proving adviser queries use only relevant live
  collections and never widen to all collections.
- [x] Exclude catalogue audit entries and internal policy filtering from
  model-visible evidence limitations.
- [x] Replace `untrusted_evidence` with explicit no-instruction-authority
  semantics.
- [x] Tell specialists to omit non-material findings instead of producing filler.
- [x] Run source, provenance, and persona tests.

## Task 4: Decisions and concise synthesis

- [x] Add failing contract tests requiring source IDs on proposed actions.
- [x] Project cited proposals into Decisions and approvals required.
- [x] Permit concise Chief synthesis using only specialist-admitted source IDs,
  while preserving dissent.
- [x] Add bounded output tests and run orchestrator/contract/UI suites.

## Task 5: UI cleanup and end-to-end verification

- [x] Hide the redundant World Monitor Connect action while connected.
- [x] Collapse repetitive adviser gaps into one command-facing watch item and
  retain low-level collection notices in the evidence disclosure.
- [x] Run formatter, focused Rust/desktop tests, command-brief E2E, and `just ci`.
- [x] Build/install the corrected Command Adviser without replacing user data.
- [x] Generate a real brief and confirm persistence, relevant sources, concise
  output, and truthful last-successful behaviour.
- [ ] Commit with signoff, push, open the phase PR, and record the verified result
  in Memory MCP with agent `CODEX`.
