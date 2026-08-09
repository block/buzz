# Usable Command Adviser MVP Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver a macOS Buzz Command Console that completes three consecutive useful briefs with real Apple inputs, trusted-LAN RAG/Memory evidence, five specialist advisers, and a Chief of Staff view.

**Architecture:** Keep the existing native Command Brief pipeline, but make source/adviser boundaries fail-soft and replace a failed Chief model response with deterministic consolidation over validated specialist contributions. Commission the existing signed Apple helper with real local identifiers, then build, sign, launch, and exercise the exact app bundle.

**Tech Stack:** Rust, Tauri 2, React 19, Swift/EventKit helper, SQLite audit store, MCP over LAN HTTP, LM Studio, LiteLLM, OpenAI.

## Global Constraints

- One branch and no subagents.
- No new security, replication, workspace-action, or RAG snapshot features.
- Preserve citations and never invent evidence.
- A single source, specialist, or Chief model failure must not prevent a partial brief.
- Completion requires three consecutive live briefs from the signed macOS app.

---

### Task 1: Fail-soft orchestration

**Files:**
- Modify: `desktop/src-tauri/src/command_brief/orchestrator.rs`
- Modify: `desktop/src-tauri/src/command_brief/orchestrator_tests.rs`

**Interfaces:**
- Consumes: validated `AdviserContribution` values and `FrozenSourceContext`.
- Produces: a completed or degraded `CommandBrief` even when Chief model consolidation fails.

- [ ] **Step 1: Write failing tests**

Add tests that make the Chief provider fail and that make a trusted-LAN
post-collection recheck fail. Assert that each run returns a persisted degraded
brief, preserves all five contribution slots, and lists the exact limitation.

- [ ] **Step 2: Run tests to verify failure**

Run:
`cargo test --manifest-path desktop/src-tauri/Cargo.toml command_brief::orchestrator_tests -- --nocapture`

Expected: the new tests fail because the current orchestrator writes a failed
terminal with no brief.

- [ ] **Step 3: Implement minimum fail-soft behaviour**

Build a deterministic Chief output exclusively from validated specialist
findings, limitations, and dissent when the model call fails or is rejected.
For trusted-LAN contexts, treat later source rechecks as informational and keep
the evidence already collected for the run.

- [ ] **Step 4: Run focused tests**

Run the command from Step 2. Expected: PASS.

### Task 2: Fail-soft trusted source collection

**Files:**
- Modify: `desktop/src-tauri/src/command_brief/sources.rs`
- Modify: `desktop/src-tauri/src/command_brief/sources_tests.rs`
- Modify: `desktop/src-tauri/src/command_brief/orchestrator/providers.rs`

**Interfaces:**
- Consumes: direct Memory/RAG MCP responses and Apple helper responses.
- Produces: `FrozenSourceContext` with cited available evidence plus bounded limitations for unavailable inputs.

- [ ] **Step 1: Write failing tests**

Add a test where one Apple read returns a source error and a test where a
trusted-LAN recheck cannot reload the catalogue. Assert that remaining sources
are retained and collection/run completion continues in degraded state.

- [ ] **Step 2: Run tests to verify failure**

Run:
`cargo test --manifest-path desktop/src-tauri/Cargo.toml command_brief::sources_tests command_brief::orchestrator_tests -- --nocapture`

Expected: the Apple error or reload error aborts collection.

- [ ] **Step 3: Implement minimum source isolation**

Convert non-cancellation Apple read errors into per-source limitations. Avoid
fresh trusted-LAN catalogue admission after evidence has been collected.

- [ ] **Step 4: Run focused tests**

Run each focused test target separately. Expected: PASS.

### Task 3: Real Apple commissioning

**Files:**
- Modify outside Git: `~/Library/Application Support/xyz.block.buzz.app/command-apple-inputs.json`

**Interfaces:**
- Consumes: identifiers returned by the packaged `buzz-apple-inputs` helper.
- Produces: a protected selection containing real Calendar, Reminder, Notes, and file identifiers.

- [ ] **Step 1: Inspect helper and current permissions**

Invoke the exact packaged helper for permission status and bounded discovery.
Request EventKit permission if status is `not_determined`.

- [ ] **Step 2: Resolve real identifiers**

Read a bounded list of Calendar and Reminder collections, Notes folders, and
the selected project file path. Do not use sentinel identifiers.

- [ ] **Step 3: Install and verify the protected selection**

Write the exact identifiers with mode `0600`, invoke each source read, and
verify the response is authorized or an honest empty result.

### Task 4: Build and live acceptance

**Files:**
- Generated: `desktop/src-tauri/target/aarch64-apple-darwin/release/bundle/macos/Buzz.app`

**Interfaces:**
- Consumes: Tasks 1-3 and the existing companion binaries.
- Produces: an ad-hoc signed, launchable macOS app plus three persisted live briefs.

- [ ] **Step 1: Run repository gates**

Run focused Rust tests, Apple helper tests, desktop type/lint checks, and
`just ci`. Expected: all required gates pass.

- [ ] **Step 2: Build and sign exact bundle**

Build the release app, replace any zero-byte generated sidecars with the
previously verified arm64 binaries, sign each executable and the full bundle,
then run `codesign --verify --deep --strict`.

- [ ] **Step 3: Launch and generate three briefs**

Launch the exact bundle path. Generate three consecutive briefs from the
Command Console.

- [ ] **Step 4: Verify every live result**

For every run, inspect lifecycle/result evidence and verify all five adviser
slots, readable consolidation, real Apple-source status, and RAG or Memory
citations. If this cannot be achieved, stop and report the exact blocker rather
than adding new architecture.

- [ ] **Step 5: Publish the bounded result**

Commit and push the branch, update the draft PR with test/live evidence, and
record only the final architecture decision, root causes, and commissioning
result in Memory MCP with agent `CODEX`.

