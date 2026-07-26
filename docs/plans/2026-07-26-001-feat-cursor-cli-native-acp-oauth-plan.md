---
type: feat
title: "Add Cursor CLI native ACP OAuth and model selection"
date: 2026-07-26
origin: null
product_contract_source: ce-plan-bootstrap
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
execution: code
---

# Add Cursor CLI native ACP OAuth and model selection

## Goal Capsule

### Objective

Add Cursor CLI as a first-class Buzz ACP runtime. Buzz should discover an installed Cursor CLI, guide the user through vendor-owned authentication, launch Cursor's native ACP harness, and expose the models advertised by Cursor for launch-time selection.

### Authority and settled decisions

The repository's existing discovery, readiness, ACP, and agent-model paths are authoritative for implementation shape. The following session decisions are settled and must be carried forward:

- Cursor authentication remains vendor-owned: Buzz stores no Cursor OAuth tokens and delegates to Cursor's generic ACP `authMethods` when advertised, with a visible `agent login` / `cursor-agent login` fallback. (session-settled: user-directed — chosen over Buzz-managed OAuth storage: preserves Cursor's own account/session handling and avoids a new credential boundary)
- Model selection is launch-time only. Buzz must not promise switching the active Cursor model inside an already-running session. (session-settled: user-directed — chosen over live ACP switching: Cursor runtime switching is unreliable)
- Models come from Buzz's existing ACP discovery path, not hardcoded IDs or parsing human-readable Cursor output. (session-settled: user-directed — chosen over CLI-output parsing and hardcoded catalogs: keeps the model list vendor-authoritative)
- Detection resolves the command first and validates real ACP/model/auth operations when used; no hardcoded minimum Cursor version and no ACP handshake on every catalog refresh. (session-settled: user-directed — chosen over version gates and eager handshakes: tolerates Cursor CLI churn while keeping discovery responsive)
- Support is native macOS/Linux and Windows only through WSL when Cursor is available there. Do not promise a native Windows installer or native Windows Cursor runtime. (session-settled: user-directed — chosen over native Windows support: matches Cursor's documented availability boundary)

### Stop conditions

Stop when Cursor appears in the shared runtime catalog, can be installed or guided into readiness, can complete vendor authentication through the supported path, can launch `agent acp`/`cursor-agent acp`, and can launch with a selected ACP-discovered model. Do not expand into mid-session model switching, Buzz token storage, Cursor API-key management, mobile/web surfaces, or a separate non-ACP Cursor adapter.

### Execution profile

Deep implementation plan. The work crosses Tauri discovery/readiness/auth, the Rust ACP helper, process argument construction, model discovery, frontend capability projection, platform handling, and regression coverage.

### Tail ownership

The implementer owns focused tests, `just ci`, cleanup of abandoned experiments, and the final contribution handoff. The plan file is not an execution-status ledger.

## Product Contract

### Summary

Buzz already supports vendor CLIs such as Claude Code and Codex through its shared ACP runtime catalog. Cursor CLI now exposes a native ACP entrypoint and vendor-owned login flow. Adding it through the same catalog and onboarding surfaces lets a Buzz user select Cursor as the harness, authenticate through Cursor, choose an available Cursor model before launch, and run the Cursor harness without giving Buzz custody of Cursor credentials.

### Problem frame

Cursor CLI is installed and authenticated independently from Buzz, but Buzz currently has no runtime metadata or launch strategy for it. Treating Cursor as an npm adapter would bypass the vendor's native harness and would make authentication/model behavior brittle. The integration must therefore preserve the existing generic ACP contract while allowing a native CLI subcommand and a startup-only model argument.

### Actors

- Buzz desktop user configuring or starting a managed agent.
- Buzz desktop discovery/readiness/auth/model IPC.
- `buzz-acp`, which owns ACP subprocess startup and model catalog extraction.
- Cursor CLI (`agent`, with `cursor-agent` compatibility), which owns OAuth/session state and native ACP.
- Windows WSL environment, when the desktop host is Windows and Cursor is installed only inside WSL.

### Requirements

#### R1. Cursor runtime discovery

Add a stable runtime ID for Cursor and expose it through the existing `AcpRuntimeCatalogEntry` path. Resolve the modern `agent` command first and support `cursor-agent` as a compatibility entrypoint. Discovery must distinguish Cursor's native ACP command from an external adapter and must not require an npm package.

#### R2. Install and platform guidance

Show the official Cursor CLI installation guidance and installation action where the existing runtime installer can safely provide it. On macOS/Linux use Cursor's official installer path. On Windows guide the user to install/use Cursor CLI in WSL; do not present a native Windows Cursor installer as supported. Preserve the existing command-resolution/PATH behavior used by Claude and Codex.

#### R3. Readiness and authentication

When Cursor is detected but not authenticated, show a readiness requirement with a copyable/runnable vendor command. Prefer the generic ACP `authMethods` flow when Cursor advertises one; otherwise launch a visible terminal for `agent login` or `cursor-agent login`, selected from the resolved entrypoint. Re-running discovery/readiness after login must clear the requirement. Buzz must never persist Cursor OAuth tokens or API keys.

#### R4. Native ACP lifecycle

Every Cursor ACP helper and managed-agent launch must invoke the native ACP subcommand (`agent acp` or `cursor-agent acp`) with the correct working directory, environment, stdio, and shutdown behavior. `buzz-acp models`, `auth-methods`, `authenticate`, and normal relay startup must all use the same normalized launch specification.

#### R5. ACP model catalog

Use the existing `buzz-acp models --json` → ACP `initialize`/`session/new` → catalog normalization path. Surface stable `configOptions` model values and unstable model state when Cursor advertises them, deduplicate them through the shared normalizer, and preserve empty/failure semantics. Do not parse `agent models` output and do not hardcode Cursor model IDs.

#### R6. Launch-time model selection

Persist the selected model in the existing managed-agent model field and translate it into Cursor's startup `--model <model-id>` argv when the managed agent is spawned. Discovery must enumerate the full catalog without pinning to the persisted model. If no model is selected or the catalog is empty, omit `--model` and let Cursor choose its default. Avoid duplicate/conflicting startup model flags and pass model IDs as argv values, never shell-interpolated command text.

#### R7. Shared UI and onboarding behavior

Cursor must flow through the existing runtime catalog, setup/readiness, authentication, and model controls without frontend hardcoded harness checks. The shared capability projection must mark Cursor as ACP-native with model discovery available and live model switching unavailable. Empty model discovery may hide the optional selector using the same existing Claude/Codex rule, while discovery failures remain visible/actionable.

#### R8. Compatibility and failure behavior

Command resolution is the detection signal. Real ACP/model/auth operations are the compatibility validation. Missing CLI, missing/unsupported WSL, unauthenticated CLI, failed ACP handshake, and empty model catalogs must produce actionable existing-state errors rather than crashes, panics, or silent fallback to a different harness.

### Primary flow

1. Buzz discovers the Cursor entrypoint and records availability/auth status in the shared runtime catalog.
2. The user selects Cursor; if needed, Buzz shows official install or login guidance.
3. Buzz runs the generic ACP auth method when advertised, or opens the vendor login command in a visible terminal.
4. Buzz runs ACP model discovery and populates the existing model picker from Cursor's response.
5. The user selects a model and saves the managed-agent configuration.
6. On launch, Buzz starts `buzz-acp`; `buzz-acp` starts Cursor native ACP with `--model <id>` when selected.

### Acceptance examples

- **AE1 — macOS/Linux happy path:** With `agent` on PATH and Cursor logged in, runtime discovery reports Cursor available and authenticated; model discovery returns Cursor-advertised models; selecting one causes the next launch to include exactly one `--model <id>` before `acp` reaches Cursor.
- **AE2 — legacy command:** With only `cursor-agent` available, Buzz discovers it, uses `cursor-agent acp`, probes `cursor-agent status`, and uses `cursor-agent login` if auth is needed.
- **AE3 — generic ACP auth:** If Cursor's `initialize` response advertises `authMethods`, Buzz presents those methods and invokes the selected method through the generic ACP helper without writing a token to Buzz storage.
- **AE4 — terminal fallback:** If no usable ACP auth method is advertised, Buzz opens a visible terminal with the resolved Cursor CLI login argv. Completing login and refreshing readiness makes the runtime ready.
- **AE5 — empty/failed model discovery:** An empty catalog omits the optional model selector and preserves Cursor's default. A timeout or malformed ACP response leaves an actionable discovery error and does not claim model selection is available.
- **AE6 — launch-time boundary:** Changing the saved model applies on the next process launch. An already-running Cursor process is not reconfigured and the UI does not claim that it was switched live.
- **AE7 — Windows boundary:** A Windows build never offers native Cursor installation as supported. When WSL and Cursor are available, Buzz can use the WSL launch specification; otherwise it explains that Cursor CLI must be installed in WSL and leaves the runtime unavailable.

### Success criteria

- Cursor is selectable from the same managed-agent runtime catalog used by Claude Code and Codex.
- Authentication remains in Cursor's own CLI/ACP flow; no Cursor credential material is added to Buzz persistence or logs.
- A selected model is observable in the child argv/launch contract and is applied only at process startup.
- Model options are sourced exclusively from the ACP response and remain resilient to Cursor model additions/removals.
- Existing Claude, Codex, Goose, Buzz Agent, custom runtime, and non-Cursor model/readiness behavior remains unchanged.
- Focused Rust/frontend tests cover direct launch, legacy alias, auth paths, WSL boundary, model selection, empty discovery, and failure behavior.

### Scope boundaries

In scope: desktop runtime catalog metadata, command resolution, install/readiness guidance, generic ACP auth integration, native ACP launch, ACP model discovery, launch-time model argv, shared capability projection, Windows WSL boundary, docs/tests.

Out of scope: Buzz-managed Cursor OAuth/token storage, direct Cursor API-key entry or secret migration, mid-session model switching, parsing `agent models` text, hardcoded model catalogs, native Windows Cursor support, mobile/web runtime surfaces, Cursor-specific non-ACP adapter packages, automatic Cursor account logout.

### Dependencies

- Cursor CLI must expose the native ACP entrypoint and the advertised ACP protocol/auth behavior used at implementation time.
- Cursor's official CLI installation/authentication commands may evolve; keep URLs and command behavior centralized in runtime metadata and test the actual operation rather than a version gate.
- WSL must be installed and contain a usable Cursor CLI for the Windows path; Buzz must not silently execute a Windows alias that is actually a WSL launcher.

### Sources

- Cursor CLI authentication: https://docs.cursor.com/en/cli/reference/authentication
- Cursor CLI parameters and `--model`: https://docs.cursor.com/en/cli/reference/parameters
- Cursor CLI overview: https://docs.cursor.com/en/cli/overview
- Cursor CLI installation/platform guidance: https://docs.cursor.com/en/cli/installation
- Cursor CLI changelog describing `agent` and `cursor-agent`: https://cursor.com/changelog/cli-jan-08-2026
- Cursor ACP/model-selection behavior observed in the vendor forum: https://forum.cursor.com/t/acp-model-selection-api-removed/160063

## Planning Contract

### Key technical decisions

- **KTD1 — Treat Cursor as a native ACP runtime.** Extend the runtime catalog and ACP helper to carry an executable-plus-argv launch specification. Cursor's executable is `agent` or `cursor-agent`; its ACP mode is the `acp` subcommand. Do not install or invoke an npm adapter. (session-settled: user-directed — chosen over adapter wrapping: preserves Cursor's own harness and OAuth/session behavior)
- **KTD2 — Keep auth vendor-owned and generic first.** Reuse `authMethods`/`authenticate` when advertised. Add a metadata-driven terminal fallback using the resolved Cursor entrypoint and `login`; do not add a Cursor token store or provider secret field. (session-settled: user-directed — chosen over Buzz-owned OAuth: avoids duplicating vendor credential lifecycle)
- **KTD3 — Use one launch-spec normalizer.** The desktop normalizer, `buzz-acp models`, auth helpers, and normal relay spawn must consume the same native-runtime command/argv rules. This prevents the current desktop/helper split from turning `agent` into an accidental zero-arg launch.
- **KTD4 — Apply model at startup through argv.** Keep `BUZZ_ACP_MODEL` as the existing logical selection transport, then convert it in the Cursor launch-spec builder to a safe `--model`, `<id>` pair only for normal Cursor process startup. The model-discovery helper intentionally omits the persisted selection so it can enumerate all advertised models. (session-settled: user-directed — chosen over ACP live switching: Cursor runtime switching is unreliable)
- **KTD5 — Make readiness probe aliases dynamic.** Cursor auth status must be probed as `<resolved-entrypoint> status`, and fallback login as `<resolved-entrypoint> login`, so the same metadata supports both `agent` and `cursor-agent`. Existing Claude/Codex probe behavior remains compatible.
- **KTD6 — Represent WSL explicitly.** Do not classify a WindowsApps `bash.exe`/WSL alias as a native Cursor executable. Add a WSL launch/discovery branch that invokes the real `wsl.exe` command with the Cursor ACP argv, uses the default WSL distribution and an explicit working-directory conversion, and transports only the required Buzz environment through the existing WSL environment mechanism rather than putting secrets in argv. Report WSL-specific install guidance and use the same model/auth contract. Native Windows discovery remains unavailable by design.
- **KTD7 — Preserve generic capability projection.** Add Cursor facts to `KnownAcpRuntime` and `AcpRuntimeCatalogEntry` only where required; let `agentConfigCore` derive model-control behavior from runtime metadata. No Cursor ID checks in React render paths.

### High-level technical design

```text
KnownAcpRuntime (Cursor metadata)
        |
        v
command/WSL launch-spec resolver ----> availability + auth status
        |                                      |
        |                                      +--> generic authMethods/authenticate
        |                                      +--> vendor login terminal fallback
        v
desktop IPC catalog --> shared agentConfigCore --> model picker
        |
        +--> buzz-acp models --json --> Cursor `... acp` --> ACP session/new catalog
        |
        +--> managed spawn --> BUZZ_ACP_MODEL --> native launch-spec builder
                                           --> Cursor `... --model <id> acp`
```

The launch-spec builder should be a pure, testable function with separate modes for model discovery, auth helper, and normal runtime startup. It must preserve explicit user args, add the native `acp` subcommand exactly once, and add/replace the selected model only in normal startup mode. WSL resolution should produce an explicit `wsl.exe` executable and argument vector rather than relying on shell aliases.

### Implementation constraints

- Follow root `AGENTS.md` and `desktop/src/features/agents/AGENTS.md`; use Hermit before Git/hooks and do not add production `unwrap`/`expect`.
- Keep all command arguments structured; never construct a shell command by interpolating a model ID or auth input.
- Keep token/API-key values out of logs, errors, persisted records, and plan/test fixtures.
- Preserve `KnownAcpRuntime` as the authority for harness facts and keep shared onboarding/model acceptance tests green.
- Do not add a version gate for Cursor. Handshake/model/auth failures are runtime compatibility signals.
- Update runtime comments that currently enumerate only Claude/Codex CLI-login runtimes.

### Sequencing

Implement in dependency order: U1 launch/discovery primitives → U2 readiness/auth → U3 ACP model discovery and startup model application → U4 shared capability/UI regression coverage → U5 documentation and full verification. U2 and U3 may share the launch-spec helper but must not duplicate its normalization logic.

### System-wide impact

This changes the external-process trust boundary and the persisted meaning of a selected model. Cursor owns OAuth/session state; Buzz owns only runtime metadata and the user's selected model ID. The model ID crosses desktop IPC, environment transport, `buzz-acp`, and child argv, so each boundary needs structured validation and redaction-aware errors. Discovery remains cheap; ACP handshakes run only for explicit auth/model operations or actual launch.

### Risks and mitigations

- **Cursor CLI churn:** Centralize command/install/auth metadata, support both entrypoints, and validate the actual ACP operation.
- **ACP auth shape differs by build:** Parse generic methods defensively and retain terminal fallback based on explicit vendor command metadata.
- **Model startup flag conflicts with user args:** Build a canonical argv and test empty, explicit, duplicate, and selected-model cases.
- **Windows/WSL path mismatch:** Keep WSL as an explicit launch mode, reject WindowsApps aliases, and make unsupported native Windows state actionable.
- **Credential leakage:** Never serialize tokens; redact subprocess stderr using the existing environment-redaction path; use visible terminal only for vendor-owned login.
- **Regression in existing runtimes:** Preserve current zero-arg/default-arg tests and run focused Claude/Codex auth/model/readiness suites before the full gate.

## Implementation Units

### U1. Add Cursor runtime metadata and native/WSL launch-spec resolution

**Goal:** Make Cursor a first-class discovered runtime with direct `agent`/`cursor-agent` support and an explicit Windows WSL boundary.

**Requirements:** R1, R2, R4, R8.

**Files:**

- `desktop/src-tauri/src/managed_agents/discovery.rs`
- `desktop/src-tauri/src/managed_agents/discovery/runtime_metadata.rs`
- `desktop/src-tauri/src/managed_agents/discovery/tests.rs`
- `desktop/src-tauri/src/managed_agents/types.rs`
- `desktop/src-tauri/src/managed_agents/runtime.rs`
- `desktop/src-tauri/src/commands/agent_model_process.rs`
- `crates/buzz-acp/src/config.rs`
- `crates/buzz-acp/src/lib.rs`
- `crates/buzz-acp/src/acp.rs` (only if the launch-spec integration requires a small ACP spawn seam)

**Approach:**

- Add Cursor metadata with a stable ID, label/avatar, `agent` primary command, `cursor-agent` fallback, no adapter install step, official install URL/commands, and native ACP default args.
- Introduce a shared structured launch-spec/argv normalization path that can distinguish direct Unix/macOS/Linux launch from Windows WSL launch. Keep discovery's `command`/`binary_path` catalog fields compatible with existing UI consumers.
- Ensure all helper paths pass `... acp` exactly once and resolve the executable through the existing augmented PATH. WSL must use `wsl.exe` explicitly and must not mistake a WindowsApps alias for a native binary.
- Keep model discovery on the base ACP argv; reserve selected-model insertion for U3's normal launch mode.

**Test scenarios:**

- `agent` resolves before `cursor-agent`; both normalize to `acp`.
- Explicit Cursor args remain intact, while a legacy/default `acp` value is normalized without duplication.
- Missing direct command is `NotInstalled`/actionable rather than `AdapterMissing`.
- WindowsApps WSL alias is rejected as a native executable; a real WSL launch spec is recognized only when WSL and Cursor are callable.
- Existing Goose/Claude/Codex argument normalization is unchanged.

**Verification:** Run the focused discovery and `buzz-acp` config tests; inspect the generated catalog entry and launch argv in unit fixtures.

### U2. Add Cursor readiness and vendor-owned authentication

**Goal:** Make Cursor setup and login behave like the existing Claude/Codex CLI flows while preferring generic ACP auth.

**Requirements:** R2, R3, R8.

**Files:**

- `desktop/src-tauri/src/managed_agents/readiness.rs`
- `desktop/src-tauri/src/managed_agents/readiness/cli_login.rs`
- `desktop/src-tauri/src/managed_agents/discovery.rs`
- `desktop/src-tauri/src/commands/agent_auth.rs`
- `desktop/src-tauri/src/managed_agents/discovery/tests.rs`
- `desktop/src-tauri/src/commands/agent_auth.rs` (unit tests)
- `desktop/src-tauri/src/managed_agents/readiness.rs` (unit tests)

**Approach:**

- Generalize the CLI-login readiness path so Cursor's status probe is derived from the resolved command (`agent status` or `cursor-agent status`) and its setup copy/login hint is derived from the same entrypoint.
- Feed Cursor's native ACP process into `buzz-acp auth-methods --json`. If an advertised method is usable, invoke `buzz-acp authenticate --method-id` through the existing generic IPC flow. If ACP initialization fails with an auth-required/unauthenticated result before `authMethods` can be read, classify that as the same fallback condition rather than surfacing a dead-end error.
- If no usable auth method is advertised, or auth-method discovery cannot initialize because Cursor is logged out, use explicit terminal auth metadata/fallback argv for `<entrypoint> login`; preserve visible-terminal behavior and do not add token persistence.
- Make missing CLI, missing WSL, logged-out, and invalid-config outcomes use existing `Requirement`/`AuthStatus` states and actionable hints.

**Test scenarios:**

- Logged-in `agent status` yields no login requirement; logged-out status yields `Requirement::CliLogin` with a safe setup command.
- Legacy `cursor-agent status`/`login` uses the legacy entrypoint.
- Generic ACP auth method discovery and authenticate route to Cursor native ACP.
- Empty auth methods invoke the visible terminal fallback; malformed auth metadata returns a clear error without shell execution.
- No token or secret is present in the catalog, requirement, or surfaced error.

**Verification:** Run readiness, auth command, and discovery tests; manually inspect a sanitized auth-method fixture and verify no credential-bearing output is persisted.

### U3. Discover Cursor models and apply the selected model at process startup

**Goal:** Populate the shared model picker from Cursor ACP and launch the selected model through Cursor's native startup flag.

**Requirements:** R5, R6, R8.

**Files:**

- `desktop/src-tauri/src/commands/agent_model_process.rs`
- `desktop/src-tauri/src/commands/agent_models.rs`
- `desktop/src-tauri/src/commands/agent_models_tests.rs`
- `desktop/src-tauri/src/managed_agents/runtime.rs`
- `desktop/src-tauri/src/managed_agents/runtime/tests.rs`
- `crates/buzz-acp/src/config.rs`
- `crates/buzz-acp/src/lib.rs`
- `crates/buzz-acp/src/acp.rs`
- `crates/buzz-acp/src/config.rs` (unit tests)
- `crates/buzz-acp/src/lib.rs` (unit tests)

**Approach:**

- Keep `buzz-acp models --json` as the only Cursor catalog source and run it with the base native ACP argv, without the persisted model selection.
- Extend the normalized launch configuration so `BUZZ_ACP_MODEL` becomes a structured Cursor startup `--model`, `<id>` pair only for normal managed-agent startup. Validate the selected value against the fresh ACP catalog when the catalog is available; if validation cannot run, preserve existing fallback/error semantics rather than inventing a model.
- Make duplicate `--model` handling deterministic: the Buzz-selected model is represented once, user-supplied conflicting startup flags are not duplicated, and no model ID is interpolated into a shell string.
- Keep existing ACP session model-switch code intact for runtimes that support it, but mark Cursor as not live-switchable in capability metadata so the UI only promises next-launch behavior.

**Test scenarios:**

- Cursor ACP `configOptions` and unstable model state normalize to a deduplicated model list.
- Empty catalog returns no models and leaves startup on Cursor default.
- Malformed/timeout discovery produces a failure state without claiming model control.
- Selected `gpt-5`-style value yields `--model`, `gpt-5`, `acp` exactly once in normal startup; no selection yields no `--model`.
- Discovery argv never contains the persisted model; Claude/Codex model behavior remains unchanged.
- Model IDs containing spaces, quotes, or shell metacharacters remain single argv values and are redacted safely in errors.

**Verification:** Run `agent_models_tests`, managed-agent runtime tests, `buzz-acp` config/lib tests, and a fake ACP subprocess fixture that records argv and emits stable/unstable model responses.

### U4. Project Cursor capabilities through shared agent configuration and regression surfaces

**Goal:** Make Cursor model/auth/readiness behavior appear through existing onboarding and agent configuration contracts without frontend-specific runtime branches.

**Requirements:** R3, R5, R6, R7, R8.

**Files:**

- `desktop/src/features/agents/lib/agentConfigCore.ts`
- `desktop/src/features/agents/lib/agentConfigCore.test.mjs`
- `desktop/src/features/agents/ui/agentConfigFieldsContract.test.mjs`
- `desktop/src/features/agents/ui/usePersonaModelDiscovery.test.mjs`
- `desktop/src/features/agents/ui/personaModelDiscoveryStatus.test.mjs`
- `desktop/src/features/agents/ui/editAgentProviderDiscovery.test.mjs`
- `desktop/tests/e2e/onboarding-agent-defaults.spec.ts`
- `desktop/tests/e2e/doctor-states.spec.ts`
- `desktop/tests/e2e/edit-agent.spec.ts`
- `desktop/tests/e2e/agents.spec.ts`
- `desktop/src/shared/api/tauri.ts` (only if the catalog IPC contract gains a field)

**Approach:**

- Project the Rust catalog's Cursor-native capability facts through the existing raw catalog and `agentConfigCore` descriptors. Do not add Cursor ID conditionals to render components.
- Verify the model control is optional/hidden after successful empty discovery, stays actionable on failure, and does not advertise live switching.
- Add onboarding/readiness/auth regression coverage using the existing shared fixtures or fake ACP agent rather than real Cursor credentials.
- If no IPC shape changes are required, leave `tauri.ts` untouched and record that the existing generic fields were sufficient.

**Test scenarios:**

- Cursor appears in the runtime catalog and shared renderer with the correct native ACP/model metadata.
- Auth-required Cursor shows the same setup/connect state as other CLI-login runtimes.
- Empty/failure model discovery produces the expected control/status contract.
- Selecting a Cursor model persists the managed-agent model field and does not offer live switching.
- Existing Claude/Codex onboarding acceptance tests remain green.

**Verification:** Run the focused desktop unit tests, then the affected Playwright E2E specs with the repository's supported desktop test setup.

### U5. Document the supported Cursor path and complete contribution verification

**Goal:** Make the new runtime discoverable to maintainers/users and leave a clean, reviewable contribution.

**Requirements:** R1, R2, R3, R6, R8.

**Files:**

- `crates/buzz-acp/README.md`
- `desktop/src/features/agents/AGENTS.md` (only if the implementation changes a durable agent-surface rule; otherwise add an explicit “no rules changed” note in the PR description, not this file)
- Any repository documentation page that currently lists supported ACP runtimes, located by `rg` during implementation.

**Approach:**

- Add Cursor to the supported ACP runtime documentation with the native `agent acp`/`cursor-agent acp` distinction, vendor login ownership, launch-time model selection, and Windows WSL boundary.
- Keep docs free of real account names, tokens, or machine-specific paths.
- Run the full repository gate and review the final diff for abandoned adapter experiments, duplicated model-switch code, credential leakage, and unintended changes to other runtimes.

**Test scenarios:**

- README examples use placeholders and match the implemented environment/argv contract.
- Documentation does not claim native Windows support or live Cursor model switching.
- `AGENTS.md` is changed only if a durable rule genuinely changed.

**Verification:** Run `just ci`; if environment/toolchain constraints prevent it, run the documented focused gates and report the exact skipped gate and reason. Confirm `git status` contains only intended implementation, test, and documentation changes.

## Verification Contract

### Focused gates

- `just test-unit`
- `cargo test -p buzz-acp`
- The focused Tauri Rust test targets covering discovery, readiness, auth, model process/normalization, and managed runtime tests.
- The focused desktop JavaScript tests listed in U4 through the repository's existing test runner.
- A fake ACP integration fixture that records the child argv and simulates Cursor initialize/session-new/auth responses, with no real credentials.

### Full gate

- Activate the repository's Hermit environment before running hooks/tooling as required by `AGENTS.md`.
- Run `just ci` before presenting the contribution as ready. This includes formatting, linting, desktop checks, unit tests, and builds according to the current repository task definitions.
- If desktop E2E requires a separately configured environment, run the affected specs and record any environment-only limitation separately from code failures.

### Behavioral evidence required

- Direct `agent` and legacy `cursor-agent` detection/launch evidence.
- macOS/Linux auth and model-discovery fixture evidence.
- Windows WSL supported/unsupported boundary evidence.
- Proof that no selected model is sent during catalog enumeration and that normal startup includes the selected model once.
- Proof that no Cursor credential material enters Buzz persistence, logs, test fixtures, or error messages.

## Definition of Done

### Global

- [ ] R1–R8 and AE1–AE7 are implemented or explicitly demonstrated by tests.
- [ ] Cursor uses native ACP; no separate Cursor adapter package or hardcoded model list was added.
- [ ] Auth remains vendor-owned and no Cursor token/API key is stored by Buzz.
- [ ] Launch-time model selection works, while live switching is not advertised or attempted for Cursor.
- [ ] Direct macOS/Linux and WSL-bounded Windows behavior is explicit and actionable.
- [ ] Existing Claude, Codex, Goose, Buzz Agent, and custom-runtime behavior remains green.
- [ ] Focused tests and `just ci` pass, or exact environment blockers are reported.
- [ ] All abandoned experiments, temporary fixtures, debug logging, and dead code are removed.

### Per-unit completion

- [ ] U1 has pure launch-spec/argv tests for direct, legacy alias, and WSL paths.
- [ ] U2 has readiness, generic ACP auth, terminal fallback, and redaction tests.
- [ ] U3 has model normalization, empty/failure, startup `--model`, and no-duplicate tests.
- [ ] U4 has shared capability, onboarding, auth/readiness, and model-control regression coverage.
- [ ] U5 has accurate docs and a clean contribution diff.

## Appendix

### Repository anchors

- `desktop/src-tauri/src/managed_agents/discovery.rs` is the current `KNOWN_ACP_RUNTIMES` catalog and command/argument normalization source.
- `desktop/src-tauri/src/managed_agents/readiness.rs` routes known CLI-login runtimes into `readiness/cli_login.rs`.
- `desktop/src-tauri/src/commands/agent_auth.rs` already provides generic ACP auth-method discovery plus visible terminal fallbacks.
- `desktop/src-tauri/src/commands/agent_model_process.rs` is the existing `buzz-acp models --json` subprocess boundary.
- `crates/buzz-acp/src/lib.rs` owns `models`, `auth-methods`, `authenticate`, and normal pool startup.
- `desktop/src/features/agents/lib/agentConfigCore.ts` projects Rust catalog facts into shared model/auth/config controls.

### Contribution notes

The current repository is clean on `main` before this plan artifact. The prior session stopped after cloning `block/buzz` into `Buzz-Opensource`; no product changes from that session were carried into this work.
