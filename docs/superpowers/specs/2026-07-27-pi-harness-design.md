# Pi (pi.dev) as a First-Class Buzz Agent Harness — Design

**Date:** 2026-07-27
**Status:** Approved design, pending implementation plan

## Goal

Make [pi](https://pi.dev) (Mario Zechner's minimal coding-agent harness,
`@earendil-works/pi-coding-agent`) a first-class agent runtime in Buzz, with
full parity across surfaces: desktop runtime catalog, discovery/install flow,
readiness, config panel, orphan cleanup, icons, and docs.

## Background

Buzz integrates harnesses via ACP (Agent Client Protocol): the desktop spawns
`buzz-acp`, which spawns an ACP-speaking agent subprocess over stdio. First-class
runtimes live in the static `KNOWN_ACP_RUNTIMES` registry
(`desktop/src-tauri/src/managed_agents/discovery.rs`) — currently Goose,
Claude Code (`claude-agent-acp`), Codex (`codex-acp`), and Buzz Agent.

Pi does not speak ACP natively (first-party support is under discussion
upstream, earendil-works/pi#4444). Two community adapters exist. This design
uses **`@victor-software-house/pi-acp`** — it embeds pi via the official SDK
and is the more ACP-compliant of the two (model/thinking `configOptions`,
session load/resume, terminal auth, FS/terminal delegation).

Pi has no built-in MCP support ("No MCP" is a stated philosophy); MCP arrives
via pi extension packages that read Claude-Code-style `mcp.json` files,
including project-level `.pi/mcp.json`.

## Decisions (settled during brainstorming)

| Question | Decision |
|---|---|
| Scope | Full first-class parity, single effort |
| ACP bridge | Community adapter `@victor-software-house/pi-acp`, version-pinned; swap when pi lands first-party ACP |
| Model/provider | Pi config owns it (Claude Code pattern) — Buzz does not inject provider/model |
| buzz-dev-mcp | Yes — wired via config bridge (per-agent `.pi/mcp.json` + pi MCP extension), plus `mcp_command` in `session/new` as future-proofing |
| Install flow | Guided install via catalog commands; nothing bundled with Buzz |

## Architecture

```
Buzz Desktop ──spawns──► buzz-acp ──ACP/stdio──► pi-acp
                                                   └─ embeds pi via @earendil-works/pi-coding-agent SDK
```

`buzz-acp` requires **zero changes**. It is harness-generic, and already
frames the system prompt into the first user message for harnesses without
native system-prompt support (pi-acp is one — same path Goose takes when its
system-prompt extension probe fails).

## Components

### 1. Runtime catalog entry

New `KnownAcpRuntime` in `discovery.rs` (`KNOWN_ACP_RUNTIMES`):

| Field | Value | Rationale |
|---|---|---|
| `id` | `"pi"` | |
| `label` | `"Pi"` | |
| `commands` | `&["pi-acp"]` | Global-install binary name |
| `aliases` | `&["pi.dev", "pi-dev"]` | |
| `underlying_cli` | `Some("pi")` | Detects "pi installed, adapter missing" |
| `cli_install_commands` | `npm install -g @earendil-works/pi-coding-agent` | pi.dev install script linked as docs fallback |
| `adapter_install_commands` | `npm install -g @victor-software-house/pi-acp@<version>`, then `pi install npm:pi-mcp-extension` | Adapter pinned to an exact version (chosen at implementation time = latest tested); MCP extension enables `.pi/mcp.json` loading |
| `cli_install_instructions_url` | `https://pi.dev` | |
| `adapter_install_instructions_url` | `https://github.com/victor-software-house/pi-acp` | |
| `mcp_command` | `Some("buzz-dev-mcp")` | Sent in `session/new`; no-op with today's adapter (known MUST-level gap upstream), harmless and future-proof. Working path is the config bridge. |
| `mcp_hooks` | `false` | |
| `model_env_var` / `provider_env_var` | `None` / `None` | Pi config owns model/provider |
| `provider_locked` | `false` | Pi is multi-provider |
| `required_normalized_fields` | `&[]` | Nothing required from Buzz's side |
| `config_file_path` | `Some("~/.pi/agent/settings.json")`, format `json` | Read-only surfacing in config panel |
| `skill_dir` | `Some(".pi/skills")` | Buzz symlinks the `buzz-cli` skill into the nest |
| `supports_acp_native_config` | `false` | |
| `supports_acp_model_switching` | `false` | Adapter exposes model via `configOptions`; scaffolding field, currently unused |
| `thinking_env_var` / `max_tokens_env_var` / `context_limit_env_var` | `None` | Config-file / configOptions only |
| `login_hint` | "Run `pi` and use /login, or set a provider API key (e.g. ANTHROPIC_API_KEY)." | |
| `auth_probe_args` | `None` | Pi has no `auth status` CLI subcommand; v1 ships without a probe (same as Goose). Auth failures surface as runtime errors in agent output. |

Model/provider selection still appears in the config panel post-spawn for
free: the adapter advertises ACP `configOptions` (categories `model`,
`thought_level`), which Buzz's existing tier-1b session cache surfaces.

### 2. Readiness policy

`readiness.rs::collect_missing_requirements`: add an explicit `"pi" => vec![]`
arm with a doc comment (deliberate no-requirements policy, not fallthrough).
Discovery-level readiness (binary missing → install offer) comes from the
catalog automatically.

**Node version:** pi and pi-acp hard-require Node 24+. During implementation,
check Buzz's managed-node version; if older, either run installs through a
suitable node or surface a human-readable requirement hint in the install
flow. This is an implementation-time checkpoint, not a design change.

### 3. MCP via config bridge (the one novel piece)

Neither community adapter forwards `session/new` `mcpServers` into pi today.
Bridge instead:

- New `config_bridge/pi.rs` writes `.pi/mcp.json` **into the agent's nest
  directory** (the per-agent cwd Buzz controls) at spawn time:

  ```json
  { "mcpServers": { "buzz": { "command": "buzz-dev-mcp" } } }
  ```

  - **Per-agent, not machine-global** — Buzz never edits `~/.pi/agent/*`.
  - **No secrets in the file** — `buzz-dev-mcp` inherits `BUZZ_RELAY_URL`,
    `BUZZ_PRIVATE_KEY`, `BUZZ_AUTH_TAG` from the process environment that
    `buzz-acp` already injects into agent subprocesses.
  - **Idempotent, merge-preserving** — if `.pi/mcp.json` exists, merge the
    `buzz` server key in without clobbering other entries.
  - `buzz-dev-mcp` is referenced by bare command name, resolved via the
    augmented child `PATH` the runtime already builds.

- `config_bridge/pi.rs` also implements the read side:
  `~/.pi/agent/settings.json` → `RuntimeFileConfig`, registered in
  `reader.rs` dispatch (`"pi" => ...`) and `mcp_config_file_path_for_runtime`
  (pointing at the nest `.pi/mcp.json`).

- Requires the `pi-mcp-extension` package in pi (installed as the second
  adapter install command). If absent, pi runs fine with its native
  read/write/edit/bash tools — graceful degradation, no hard failure.

### 4. Process cleanup

`runtime/process.rs`: add `pi-acp` and `pi` to the known binary names so the
orphan sweep and instance reaper recognize Node-hosted adapter process trees.

### 5. UI + assets

- Icon entries in `desktop/src/features/onboarding/ui/RuntimeIcon.tsx` and
  `desktop/src/features/settings/ui/DoctorSettingsPanel.tsx`; icon asset in
  `desktop/public/runtime-icons/`.
- Everything else (definition dialog runtime picker, onboarding list, install
  offers, Doctor) is data-driven from the Rust catalog via
  `AcpRuntimeCatalogEntry` — no parallel TS registry (per
  `desktop/src/features/agents/AGENTS.md`).

### 6. Docs

- Harness lists in README/CONTRIBUTING where runtimes are enumerated.
- Extend the `runtime_metadata.rs` vendor-metadata test to cover pi.

## Data flow (spawn)

1. User creates/edits an agent definition, selects runtime **Pi**.
2. Discovery resolves `pi-acp` on PATH (or offers guided install of pi CLI +
   adapter + MCP extension).
3. Readiness: no Buzz-side requirements → Ready.
4. Spawn: runtime writes/merges nest `.pi/mcp.json`, builds env
   (`BUZZ_ACP_AGENT_COMMAND=pi-acp`, no model/provider env), starts `buzz-acp`.
5. `buzz-acp` spawns `pi-acp`, initializes ACP, opens sessions with cwd =
   nest dir; pi's MCP extension loads `.pi/mcp.json` → buzz-dev-mcp tools.
6. System prompt: delivered via `buzz-acp`'s first-user-message framing.
7. Model: whatever pi's own config resolves; visible/adjustable post-spawn
   via ACP configOptions in the config panel.

## Error handling

| Failure | Surface |
|---|---|
| `pi-acp` not installed | Discovery "not installed" + guided install offer |
| `pi` CLI missing (adapter present) | `underlying_cli` partial-install detection |
| Pi unauthenticated / no API key | Runtime error in agent channel output; Doctor shows `login_hint` |
| MCP extension not installed in pi | Pi runs with native tools only; buzz-dev-mcp tools absent (degraded, not broken) |
| Node < 24 | Install-flow requirement hint (implementation-time checkpoint) |
| Adapter breaking change | Version pin in install command; bump deliberately |

## Testing

- **Unit (Rust):** catalog entry metadata test (mirroring
  `vendor_metadata_distinguishes_cli_and_adapter_guidance`); readiness arm
  test; `config_bridge/pi.rs` read-side parse tests and write-side
  round-trip + merge-preservation tests (existing `.pi/mcp.json` with foreign
  servers survives).
- **Manual E2E:** install pi + adapter + extension via the guided flow; create
  a pi definition; spawn; verify mention → response in a channel; verify
  buzz-dev-mcp tools callable from pi; verify orphan sweep kills the process
  tree on stop.

## Out of scope

- Bundling pi or the adapter as Tauri sidecars.
- Buzz-authored ACP adapter for pi.
- Upstream contributions (mcpServers wiring in pi-acp, first-party ACP in pi)
  — desirable follow-ups, not blockers.
- Provider/model injection from Buzz definitions into pi.
- `sprout-backend-blox` / remote-deploy provisioning of pi (local desktop
  launch only for v1; provider backends install their own toolchains).

## Known risks (accepted)

- Third-party adapter, self-described as subject to minor breaking changes →
  mitigated by version pinning.
- Node 24+ requirement stricter than other runtimes.
- MCP depends on a community pi extension package.
- Pi evolves fast; catalog metadata (paths, install commands) may need
  occasional refresh.
