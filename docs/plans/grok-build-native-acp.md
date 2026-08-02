# Native Grok Build ACP Runtime

## Status
- Owner: Radu Lupu
- Last updated: 2026-08-02
- Current phase: Review and publish
- Decision log: Promote the existing Grok Build ACP preset into the Rust runtime catalog. Keep the invocation `grok agent --always-approve stdio` so managed Buzz agents can execute tools without an interactive approval prompt.

## Problem

Buzz already knows how to launch Grok Build through a tier-2 preset, but Grok is not a first-class runtime. That leaves it outside the native runtime metadata path used for discovery, install guidance, authentication guidance, and managed-agent configuration.

## Scenarios

1. A user with `grok` installed sees Grok Build in the native ACP runtime catalog as available.
2. A user without `grok` sees official installation guidance and can use Buzz's native CLI installation flow where supported.
3. Creating an agent with Grok Build launches `grok agent --always-approve stdio` and routes the ACP session through Buzz.
4. Grok authentication guidance points to the standard `grok login` flow.
5. Existing personas and agents using the current `grok` preset continue resolving to the same command and arguments.

## Scope

- Add Grok Build to `KNOWN_ACP_RUNTIMES`.
- Move its definition out of the preset catalog to avoid duplicate runtime IDs.
- Add platform installation commands, documentation, authentication guidance, and spawn defaults.
- Add focused Rust and mock-catalog coverage.
- Update the README runtime list.

## Non-Goals

- Implementing an ACP adapter, because Grok Build speaks ACP natively.
- Adding Grok model/provider fields to Buzz's generic configuration model.
- Changing Grok Build itself or its authentication flow.
- Changing the relay protocol or remote-agent architecture.

## Current Evidence

- The current Buzz fork has a `grok` preset invoking `grok agent --always-approve stdio`.
- The native runtime catalog currently contains Goose, Claude Code, Codex, and Buzz Agent.
- Grok's official documentation lists `grok agent stdio` as its ACP server mode and `grok login` as its login command.
- Grok's official installer is `curl -fsSL https://x.ai/cli/install.sh | bash`, with a PowerShell installer for Windows.
- Official references: https://docs.x.ai/build/cli/reference and https://github.com/xai-org/grok-build.

## Requirements

- Grok's catalog ID is `grok` and its display label is `Grok Build`.
- The primary command is `grok` with default args `agent --always-approve stdio`.
- Discovery classifies the runtime as unavailable when the CLI is absent and available when the CLI is present.
- The catalog provides official CLI installation instructions and native auto-install commands for macOS/Linux and Windows.
- The runtime has no adapter install step, no model/provider env injection, and no MCP sidecar.
- The runtime exposes `grok login` as authentication guidance. Grok's documented CLI does not provide a `login status` subcommand, so Buzz will not add a brittle probe based on an unrelated model-list command.
- The former preset entry is removed so discovery emits exactly one Grok entry with `source: builtin`.

## Architecture

`KnownAcpRuntime` remains the single source of truth:

`KNOWN_ACP_RUNTIMES[grok]`
`  -> discovery and availability`
`  -> AcpRuntimeCatalogEntry over IPC`
`  -> agent configuration and readiness UI`
`  -> managed spawn: grok agent --always-approve stdio`

The existing preset registry remains for other tier-2 runtimes. Grok is removed from it to prevent ID collisions and conflicting spawn metadata.

## Implementation Stages

1. Add native Grok metadata and default argument normalization in `discovery/grok.rs`; remove the duplicate preset.
2. Add focused discovery, metadata, and runtime tests, then align the e2e mock catalog.
3. Update the runtime documentation and inspect the complete diff.
4. Run the narrow Rust and desktop checks, then the repository CI gate if available.
5. Commit with DCO signoff, push `agent/native-grok-acp`, and open a draft PR from the fork to `block/buzz:main`.

Rollback: revert the native catalog change and restore the existing preset definition. No persisted schema or relay migration is involved.

## Testing Plan

- `cargo test --manifest-path desktop/src-tauri/Cargo.toml managed_agents::discovery`
- Targeted Rust tests for Grok metadata, args, catalog source, and auth probe configuration.
- Desktop TypeScript typecheck/build or the narrow agent test command available in `desktop/package.json`.
- `git diff --check` and review of the final diff.
- If the full gate is runnable, `just ci`.

Acceptance evidence: one native `grok` catalog entry, `default_args == ["agent", "--always-approve", "stdio"]`, correct install metadata, and no remaining Grok preset definition.

Validation completed: discovery tests 100 passed, custom-harness tests 42 passed, desktop TypeScript typecheck passed, Biome checks passed, the desktop unit suite reported 3,905 passed with 0 failures, and `just ci` passed through Rust, desktop, and web checks before stopping at the local mobile prerequisite because `dart` is not installed.

## Review Plan

- Wrong-problem review: confirm this promotes the existing ACP path rather than adding a redundant adapter.
- Regression review: verify existing preset IDs and persona resolution remain intact.
- Security review: inspect installer commands and ensure no new secrets are persisted or logged.
- Complexity review: keep Grok-specific behavior in runtime metadata and avoid frontend ID checks.
- Evidence review: require focused tests and a clean diff before opening the PR.

## Definition Of Done

- Grok Build appears as a native runtime with `source: builtin`.
- Installed Grok is discoverable and launches through ACP with always-approve enabled.
- Missing Grok receives official install guidance.
- Auth guidance points users to `grok login`; no unsupported auth probe is added.
- Tests pass for the changed discovery paths.
- The branch is pushed and a draft PR targets `block/buzz:main`.

## Change Log

- 2026-08-02: Initial plan. Promote the existing Grok Build preset to a native ACP runtime.
- 2026-08-02: Implementation complete and focused plus full desktop tests passed; ready for review.
- 2026-08-02: Moved Grok metadata into a dedicated discovery module to respect the repository's file-size ratchet; full CI is locally blocked only by the missing Dart toolchain.
