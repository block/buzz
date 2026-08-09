# Command Model Routing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a persistent Cloud-first/Local-first Command Adviser toggle and make the existing LiteLLM ChatGPT route work through bounded SSE streaming.

**Architecture:** Extend the protected trusted-LAN configuration with a closed routing preference, include that configuration in runtime identity, and construct each adviser runtime with one fixed provider order. Add a strict bounded LiteLLM SSE parser while preserving the direct OpenAI Responses path and all existing evidence validation.

**Tech Stack:** Rust, Tauri 2, reqwest, serde, React 19, TypeScript, Vitest/Node tests, macOS Keychain, LiteLLM.

## Global Constraints

- Keep this phase limited to one global persistent toggle and provider commissioning.
- `Cloud first` is LiteLLM, direct OpenAI, then LM Studio.
- `Local first` is LM Studio, LiteLLM, then direct OpenAI.
- Manual and scheduled briefs share the same preference.
- Existing files without a preference default to `local_first`.
- Active runs retain their captured provider order.
- Secrets remain Keychain-only and must never appear in logs, fixtures, commits, or command output.
- LiteLLM responses remain bounded to 4 MiB and must terminate with `[DONE]`.
- No source, citation, signing, persistence, or workspace-action contract changes.

---

### Task 1: Protected routing preference contract

**Files:**
- Modify: `desktop/src-tauri/src/command_services/trusted_lan.rs`
- Modify: `desktop/src-tauri/src/command_services/trusted_lan_tests.rs`
- Modify: `desktop/src-tauri/trusted-lan-sources.example.json`

**Interfaces:**
- Produces: `ModelRoutingPreference::{CloudFirst, LocalFirst}`
- Produces: `TrustedLanConfig::routing_preference()`
- Produces: `TrustedLanConfig::configuration_identity()`
- Produces: `save_routing_preference(path, preference)`

- [ ] **Step 1: Write failing contract tests**

Add tests proving:

```rust
assert_eq!(
    TrustedLanConfig::parse(legacy_fixture)?.routing_preference(),
    ModelRoutingPreference::LocalFirst
);
assert_eq!(
    TrustedLanConfig::parse(cloud_first_fixture)?.routing_preference(),
    ModelRoutingPreference::CloudFirst
);
```

Add a protected-temp-file test that saves `CloudFirst`, reloads the file,
asserts mode `0600`, and proves all endpoints/models/key names are unchanged.
Add an identity test proving changing only the preference changes
`configuration_identity()`.

- [ ] **Step 2: Run the focused tests and verify RED**

Run:

```bash
. ./bin/activate-hermit
cargo test --manifest-path desktop/src-tauri/Cargo.toml command_services::trusted_lan_tests
```

Expected: compile failures for the missing preference APIs.

- [ ] **Step 3: Implement the minimal protected contract**

Add the closed enum, legacy default, strict serialized names, configuration
identity hash, and atomic restricted writer. Update the example to the verified
web-01 endpoint, `chatgpt-5.4`, and `cloud_first`.

- [ ] **Step 4: Run focused tests and verify GREEN**

Run the Task 1 command and require all trusted-LAN tests to pass.

- [ ] **Step 5: Commit**

```bash
git add desktop/src-tauri/src/command_services/trusted_lan.rs \
  desktop/src-tauri/src/command_services/trusted_lan_tests.rs \
  desktop/src-tauri/trusted-lan-sources.example.json
git commit -m "feat(command): persist model routing preference"
```

### Task 2: Strict LiteLLM SSE client

**Files:**
- Modify: `desktop/src-tauri/src/command_brief/cloud.rs`
- Create: `desktop/src-tauri/src/command_brief/cloud_tests.rs`
- Modify: `desktop/src-tauri/src/command_brief/mod.rs`

**Interfaces:**
- Produces: `parse_litellm_sse_body(body: &[u8]) -> Result<String, AdviserExecutionError>`
- Preserves: `CloudAdviserClient::run_specialist` and `run_chief_of_staff`

- [ ] **Step 1: Write failing SSE parser tests**

Cover a valid multi-event response, split content, `[DONE]`, malformed JSON,
missing `[DONE]`, empty content, non-content events, and oversized output.
Expected content is a hand-written literal such as:

```rust
assert_eq!(parse_litellm_sse_body(body)?, r#"{"status":"pong"}"#);
```

- [ ] **Step 2: Run the parser tests and verify RED**

Run:

```bash
. ./bin/activate-hermit
cargo test --manifest-path desktop/src-tauri/Cargo.toml command_brief::cloud_tests
```

Expected: compile failure because the parser does not exist.

- [ ] **Step 3: Implement bounded streaming reconstruction**

Set LiteLLM `"stream": true`, retain OpenAI’s existing payload, read response
chunks with cancellation and the existing 4 MiB bound, then parse only strict
SSE `data:` events and require `[DONE]`.

- [ ] **Step 4: Run parser and cloud-adviser tests**

Run the Task 2 command plus:

```bash
cargo test --manifest-path desktop/src-tauri/Cargo.toml command_brief::lmstudio_tests
```

- [ ] **Step 5: Commit**

```bash
git add desktop/src-tauri/src/command_brief/cloud.rs \
  desktop/src-tauri/src/command_brief/cloud_tests.rs \
  desktop/src-tauri/src/command_brief/mod.rs
git commit -m "feat(command): stream LiteLLM adviser responses"
```

### Task 3: Provider order and runtime capture

**Files:**
- Modify: `desktop/src-tauri/src/command_brief/orchestrator/providers.rs`
- Modify: `desktop/src-tauri/src/command_brief/orchestrator.rs`
- Modify: `desktop/src-tauri/src/command_brief/orchestrator_tests.rs`
- Modify: `desktop/src-tauri/src/startup.rs`
- Modify: `desktop/src-tauri/src/startup_tests.rs`

**Interfaces:**
- Produces: `provider_order(preference) -> [ProviderAttempt; 3]`
- Consumes: `TrustedLanConfig::routing_preference()`
- Consumes: `TrustedLanConfig::configuration_identity()`

- [ ] **Step 1: Write failing order and identity tests**

Assert exact literal sequences:

```rust
assert_eq!(
    provider_order(ModelRoutingPreference::CloudFirst),
    [LiteLlm, OpenAi, Local]
);
assert_eq!(
    provider_order(ModelRoutingPreference::LocalFirst),
    [Local, LiteLlm, OpenAi]
);
```

Add a startup test proving a preference-only configuration change produces a
different `RuntimeConfigIdentity`.

- [ ] **Step 2: Run focused tests and verify RED**

Run:

```bash
cargo test --manifest-path desktop/src-tauri/Cargo.toml \
  command_brief::orchestrator_tests
cargo test --manifest-path desktop/src-tauri/Cargo.toml startup_tests
```

- [ ] **Step 3: Implement ordered attempts and configuration capture**

Construct the fallback provider with its closed preference, use the same fixed
attempt order for specialists and Chief of Staff, and add the trusted-LAN
identity to runtime configuration hashing.

- [ ] **Step 4: Run focused tests and verify GREEN**

Repeat the Task 3 commands and require clean output.

- [ ] **Step 5: Commit**

```bash
git add desktop/src-tauri/src/command_brief/orchestrator/providers.rs \
  desktop/src-tauri/src/command_brief/orchestrator.rs \
  desktop/src-tauri/src/command_brief/orchestrator_tests.rs \
  desktop/src-tauri/src/startup.rs desktop/src-tauri/src/startup_tests.rs
git commit -m "feat(command): honor cloud or local routing order"
```

### Task 4: Native commands and Command Console toggle

**Files:**
- Modify: `desktop/src-tauri/src/commands/command_brief.rs`
- Modify: `desktop/src-tauri/src/lib.rs`
- Modify: `desktop/src/shared/api/tauriCommandBrief.ts`
- Modify: `desktop/src/shared/api/tauriCommandBrief.test.mjs`
- Create: `desktop/src/features/command-console/hooks/useCommandModelRouting.ts`
- Create: `desktop/src/features/command-console/hooks/useCommandModelRouting.hook.test.mjs`
- Create: `desktop/src/features/command-console/ui/CommandModelRouting.tsx`
- Create: `desktop/src/features/command-console/ui/CommandModelRouting.test.mjs`
- Modify: `desktop/src/features/command-console/ui/CommandConsoleScreen.tsx`
- Modify: `desktop/src/features/command-console/ui/CommandConsoleScreen.test.mjs`
- Modify: `desktop/src/testing/e2eBridge.ts`

**Interfaces:**
- Produces Tauri commands: `get_command_model_routing` and `set_command_model_routing`
- Produces TypeScript type: `CommandModelRoutingPreference = "cloudFirst" | "localFirst"`
- Produces hook: `useCommandModelRouting()`

- [ ] **Step 1: Write failing native/API/hook/UI tests**

Prove load/save parsing, invalid input rejection, optimistic save rollback,
disabled control while `busy`, and exact visible attempt-order copy.

- [ ] **Step 2: Run tests and verify RED**

Run:

```bash
cargo test --manifest-path desktop/src-tauri/Cargo.toml commands::command_brief
cd desktop
pnpm exec vitest run \
  src/shared/api/tauriCommandBrief.test.mjs \
  src/features/command-console/hooks/useCommandModelRouting.hook.test.mjs \
  src/features/command-console/ui/CommandModelRouting.test.mjs \
  src/features/command-console/ui/CommandConsoleScreen.test.mjs
```

- [ ] **Step 3: Implement the native commands and focused UI**

Use the protected config reader/writer, expose only the enum value, disable the
control during an active brief, and update the banner copy from the selected
preference.

- [ ] **Step 4: Run focused tests and verify GREEN**

Repeat the Task 4 commands and run:

```bash
pnpm lint
pnpm typecheck
```

- [ ] **Step 5: Commit**

```bash
git add desktop/src-tauri/src/commands/command_brief.rs \
  desktop/src-tauri/src/lib.rs desktop/src/shared/api/tauriCommandBrief.ts \
  desktop/src/shared/api/tauriCommandBrief.test.mjs \
  desktop/src/features/command-console desktop/src/testing/e2eBridge.ts
git commit -m "feat(command-console): add model routing toggle"
```

### Task 5: Commission credentials and run live acceptance

**Files:**
- Modify outside Git: `~/Library/Application Support/xyz.block.buzz.app/trusted-lan-sources.json`
- Modify outside Git: macOS Keychain service `buzz-desktop`, keys `command.cloud.litellm` and optionally `command.cloud.openai`

**Interfaces:**
- Consumes: protected web-01 `.env` master key
- Consumes: secure OpenAI Platform key flow

- [ ] **Step 1: Stop the signed app and back up the protected config**

Confirm no Buzz process can overwrite a warm Keychain cache, then copy the
existing config to a timestamped mode-`0600` backup.

- [ ] **Step 2: Transfer the LiteLLM key without plaintext output**

Read the remote key over the dedicated SSH identity directly into the existing
Keychain blob, preserve all existing entries, and verify only a boolean
presence result.

- [ ] **Step 3: Correct the installed route**

Write the verified web-01 endpoint, `chatgpt-5.4`, and `cloud_first` through the
protected atomic config writer.

- [ ] **Step 4: Install direct OpenAI fallback if the secure key flow succeeds**

Use the OpenAI Platform secure key flow and store the result only as
`command.cloud.openai`. If the user declines or the platform flow is
unavailable, leave direct OpenAI visibly unconfigured; LiteLLM and LM Studio
still form a working pair.

- [ ] **Step 5: Run live provider smokes**

Prove authenticated `/v1/models`, one streamed structured LiteLLM response, and
one app-level cloud specialist response without printing prompts, evidence, or
credentials.

### Task 6: Aggregate verification, signed bundle, and draft PR

**Files:**
- Modify: `docs/superpowers/specs/2026-07-27-command-model-routing-design.md`
- Modify: `docs/superpowers/plans/2026-07-27-command-model-routing.md`

- [ ] **Step 1: Run aggregate verification**

```bash
. ./bin/activate-hermit
just ci
```

Require zero failures. Diagnose any failure before changing code.

- [ ] **Step 2: Build and sign the release app**

Build the release Tauri bundle, restore only the already-verified companion
binaries if Tauri reproduces the known zero-byte sidecar defect, sign with the
installed Developer ID identity, and run:

```bash
codesign --verify --deep --strict /absolute/path/to/Buzz.app
```

- [ ] **Step 3: Run one cloud-first brief**

Require all five sources connected, provider preference `Cloud first`, a
signed/published terminal brief, all five specialist sections, citations, and
an audit-spool row.

- [ ] **Step 4: Switch to local-first and prove selection**

Change the toggle, restart the app, and verify the next adviser attempt reaches
LM Studio before either cloud route. A full second brief is unnecessary if the
route attempt is unambiguous and cancellation remains clean.

- [ ] **Step 5: Commit, push, and maintain the draft PR**

```bash
git add docs/superpowers/specs/2026-07-27-command-model-routing-design.md \
  docs/superpowers/plans/2026-07-27-command-model-routing.md
git commit -m "docs(command): close cloud routing phase"
git push -u origin codex/cloud-model-routing
```

Open or update the phase draft PR and include verification plus live acceptance
evidence without private source content.
