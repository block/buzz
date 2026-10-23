# Plan: run desktop-managed agents inside a nono sandbox

Status: implemented (steps 0–4). Target: nono 0.79+, desktop managed agents
(local backend). User guide: [agent-sandbox.md](agent-sandbox.md).

## Goal

Every `buzz-acp` process the desktop app starts — and therefore every agent
CLI, MCP server, and shell command beneath it — runs inside an OS-level
[nono](https://nono.sh/docs/core/overview) sandbox. The sandbox policy is a nono
profile that Buzz ships and keeps current, plus a user-owned layer that each
Buzz user can edit locally without Buzz overwriting it.

## Product contract

`VISION_AGENT.md` says "the shell runs at the operator's trust level, like bash
itself." This plan deliberately narrows that: an agent can still do anything
the operator could do *inside its granted scope*, but not read `~/.ssh`,
browser data, shell history, or keychains unless the operator grants it. This
is an intentional tension; the user-editable profile and the explicit opt-out
(below) keep the operator in control, which is the part of the vision that
matters.

## Current state (verified)

- **One spawn site.** Every local agent starts in
  `spawn_agent_child` (`desktop/src-tauri/src/managed_agents/runtime.rs:602`),
  which builds a `std::process::Command` for `record.acp_command`
  (default `buzz-acp`, `types.rs:816`; custom ACP commands go through the same
  path). All callers (interactive start, restore, `start_managed_agent_process`)
  funnel through it.
- **Workdir** is `~/.buzz` (the nest), falling back to `$HOME`
  (`managed_agents/mod.rs:134`). `~/.buzz/REPOS` may be a symlink to a
  user-chosen repos directory (`repos.rs`).
- **Process ownership** is checked by binary name plus the
  `BUZZ_MANAGED_AGENT` env marker (`runtime/process.rs:8`
  `KNOWN_AGENT_BINARIES`, `process_belongs_to_us`, orphan sweep, instance
  reaper). Termination signals the process group (`process.rs:287`,
  `kill(-pid, …)`); the child is spawned with `process_group(0)`.
- **Codex nested Seatbelt.** `buzz-acp` forces Codex into
  `sandbox_workspace_write` (`crates/buzz-acp/src/acp.rs:270`,
  `config.rs:824`). On macOS, Codex applies its own Seatbelt profile, and macOS
  refuses to apply a second Seatbelt profile inside one
  (`sandbox_apply: Operation not permitted`, reproduced in this session).
- **Relay WebSocket client has no proxy support.** No `HTTPS_PROXY` handling
  in `crates/buzz-ws-client` or `crates/buzz-acp`.
- The desktop app is not App-Sandboxed (`Entitlements.plist`), so it can apply
  Seatbelt to its children.

## nono facts this plan depends on (verified against nono 0.79.0)

- `nono wrap` applies the sandbox and `exec`s the target: **nono disappears,
  PID, process group, stdio, and environment are unchanged.** `nono run`
  instead keeps an unsandboxed supervisor parent and is required for
  proxy-based domain filtering, credential injection, rollback, and cgroup
  limits.
- `--profile` accepts a name or a file path, but `extends` accepts **only a
  profile name** (a path fails with `invalid base profile name`). Bases must
  therefore be installed in `~/.config/nono/profiles/` (respects
  `$XDG_CONFIG_HOME`).
- Filesystem, groups, commands and diagnostics are additive across `extends`;
  CLI flags win over profiles.
- When domain filtering is active, direct outbound TCP is blocked and traffic
  must go through nono's local HTTP(S) proxy.
- The built-in `default` profile denies credentials, keychains, browser data,
  shell history and shell configs. The registry `claude-code` profile shows the
  extra paths Claude Code needs (`~/.claude`, `~/.claude.json`, keychain via
  `bypass_protection`).

## Design

### 1. Two-layer profile

| File | Owner | Written |
|------|-------|---------|
| `~/.config/nono/profiles/<nest>-agent.json` | Buzz | On every launch, if its content differs from the embedded copy |
| `~/.config/nono/profiles/<nest>-agent-local.json` | User | Once, if missing; never overwritten |

`<nest>` is the nest directory name without its dot (`buzz`, `buzz-dev`,
`buzz-demo-<slug>`), so prod, dev, and demo builds never rewrite each other's
base profile. The examples below use the prod names. Implemented in
`managed_agents/sandbox_profile.rs`, with the template at
`managed_agents/sandbox_profile.json`. It is installed at boot right after
`ensure_nest()`.

`buzz-agent.json` is embedded in the binary with `"extends": "default"`. It
grants:

- `workdir.access: readwrite` (the nest).
- Read: the app bundle's sidecar directory (`buzz-acp`, `buzz`, `buzz-dev-mcp`,
  `git-credential-nostr`), `~/.local/bin`, plus nono's `user_tools`,
  `homebrew_*`, `node_runtime`, `python_runtime`, `rust_runtime`, `git_config`
  groups.
- Read-write: the agent-runtime config homes the known runtimes need
  (`~/.claude`, `~/.claude.json*`, `~/.codex`, `~/.config/goose`,
  `~/.cache`), and temp directories.
- Keychain (macOS): granted in the seeded **local** profile, not the base
  (`platform_overrides.macos`: allow plus `bypass_protection` on
  `~/Library/Keychains`, matching nono's registry `claude-code` profile).
  nono merges `extends` and `platform_overrides` additively ("can only add or
  tighten"), so a local profile can never remove a base grant; keeping the
  grant in the user's file lets them delete it. Neither
  `buzz-acp` nor Desktop reads the keychain; Claude Code reads its own
  subscription OAuth token from the login keychain. Desktop's readiness probe
  (`claude auth status`, `readiness.rs:452`) runs unsandboxed, so without this
  grant Desktop would report a subscription agent as ready while the
  sandboxed agent fails to authenticate. The cost is that every agent can
  reach the keychain database files; individual items stay behind macOS
  keychain access control. A user on API-key auth can delete the block from
  the local profile.
- Not granted: the desktop app-data directory. `buzz channel-templates`
  reads `<app data>/templates/channel-templates.json`, but the same directory
  can hold `managed-agents.json` with plaintext agent keys (when no keyring is
  available). Step 2 grants read access to the `templates` subdirectory per
  spawn.
- Network: unrestricted (see §4).

There is one shared base profile for all runtimes in v1.

`buzz-agent-local.json` is seeded as:

```json
{
  "meta": { "name": "buzz-agent-local", "version": "1", "description": "..." },
  "extends": "buzz-agent",
  "platform_overrides": {
    "macos": {
      "filesystem": {
        "allow": ["$HOME/Library/Keychains"],
        "bypass_protection": ["$HOME/Library/Keychains"]
      }
    }
  }
}
```

Spawns always use `--profile buzz-agent-local`, so Buzz upgrades its base
without clobbering user edits. Before every spawn, run
`nono profile validate buzz-agent-local`. A user profile that fails
validation blocks the spawn with the validator output in the agent error
(rule 1: no silent fallback to unsandboxed).

### 2. Wrap at the single spawn site

In `spawn_agent_child`, replace
`Command::new(&resolved_acp_command)` with a small helper
(`managed_agents/sandbox.rs`) that returns either the unwrapped command or:

```
nono wrap --silent --allow-cwd --profile buzz-agent-local \
  --workdir <nest> \
  --allow <resolved REPOS target, if it is a symlink outside the nest> \
  -- <resolved_acp_command>
```

The agent log is inherited as stdout and stderr, so writes need no grant. It is
still granted read-only (`--read-file`): Node `fstat`s its stdio at startup,
Seatbelt checks that against the file's path, and Node aborts without a message
when it is denied.

Why `wrap`, not `run`:

- PID, process group, and process name stay `buzz-acp`, so receipts,
  `process_belongs_to_us`, the orphan sweep, the instance reaper, and the
  `kill(-pgid)` stop path keep working unchanged.
- The full env Buzz builds (`BUZZ_PRIVATE_KEY`, model, gate vars) passes
  through untouched.
- No supervisor process is left to orphan.

Implemented in `managed_agents/sandbox.rs`. Static grants live in the
profile. Per-spawn grants go on the command line (`spawn_grants`):

- Read: the directories of the resolved `buzz-acp`, agent CLI, and MCP
  binaries (dev builds run them from the repo's `target/`), user `PATH`
  entries under `$HOME`, and the CLI's channel-templates directory.
- Read-write: a `REPOS` symlink target outside the nest, and the demo config
  root.
- Never `$HOME` or an ancestor of it; only existing directories.

`NONO_*` env vars are stripped from the command (inherited ones included) and
reserved in user env (`reserved_env_keys.rs`), because nono reads them as
flags. A sandboxed spawn also refuses when the workdir is not the nest:
`default_agent_workdir` falls back to `$HOME`, and the profile's workdir grant
would then share all of it.

`agent_command` and `finish_sandboxed_command` build the spawn `Command`; the
tests in `sandbox/tests.rs` exercise those production functions. A sandboxed
command that skipped `finish_sandboxed_command` runs bare `nono`, which exits
with usage, so the mistake fails closed rather than running unsandboxed.

### 3. Nested sandboxes inside the agent

- **Codex (macOS):** when the harness is under nono, Codex must not apply its
  own Seatbelt. `codex-acp` (1.10 through 2.2, verified in the published
  bundles) sends a `sandboxPolicy` with every turn, taken from the session's
  agent mode, so a `CODEX_CONFIG` `sandbox_mode` would be overridden. The
  initial mode comes from `INITIAL_AGENT_MODE` (default `agent`, which uses
  `workspaceWrite`). Desktop sets `BUZZ_ACP_OUTER_SANDBOX=nono` (reserved in
  user env) on sandboxed launches. On macOS, `buzz-acp` then forces
  `INITIAL_AGENT_MODE=agent-full-access` on the Codex adapter
  (`config::outer_sandbox_agent_env`), overriding inherited or persona values.
  That mode also sets Codex's approval policy to `never`; nono is the boundary.
  On Linux, Landlock stacks, so the Codex sandbox stays on.
  - Known limitation: `codex-acp`'s agent-file change report runs one
    internal turn with a hard-coded `readOnly` sandbox, which fails under nono
    on macOS. That affects only that report, not agent turns.
- **Claude Code:** its optional Bash sandbox has the same macOS nesting failure.
  Document it in the profile-error hint; do not change user Claude settings.

### 4. Network

Out of scope. v1 limits filesystem access only and leaves the network open
(`network.block: false`, no `allow_domain`). For the record, domain filtering
later would need proxy support in `buzz-ws-client` (direct connections are
blocked under filtering) and `nono run` (supervisor), which changes the PID and
process-group assumptions in §2.

### 5. Settings and recovery

Settings → Agents → **Agent sandbox** (`AgentSandboxSettingsCard`, hidden on
Windows):

- **Run agents in a sandbox** toggle, on by default on macOS and Linux.
  Turning it off asks for confirmation; turning it on does not. It is written
  only by `set_agent_sandbox_enabled`, which restarts running local agents.
  `set_global_agent_config` keeps the on-disk value, so a defaults-editor
  save carrying a stale copy cannot flip it, and both commands do their
  read-modify-write under one lock. The spawn snapshot records the setting too.
- **Problem banner:** `get_agent_sandbox_status` runs the same checks a
  sandboxed spawn runs (sidecar present, nest workdir, profiles installed,
  local profile valid) and the card shows the reason sandboxed agents cannot
  start. The toggle stays available, so the user always has a way out
  (rule 6). Spawns never fall back to unsandboxed silently (rule 1).
- **Sandbox profile:** shows the local profile path; **Edit** opens it in the
  default app, **Reset** (confirmed) replaces it with the default seed.
  Changes apply when agents restart.
- **Denials:** not surfaced yet. With `nono wrap`, nono is gone after `exec`,
  so it cannot log runtime denials; agents see ordinary permission errors in
  their own output and the agent log.

### 6. Distribution and platforms

- macOS and Linux: bundle a pinned nono as a Tauri sidecar so sandboxing works
  out of the box.
  - Add `binaries/nono` to `externalBin` in `tauri.conf.json`, but not in
    `tauri.windows.conf.json`.
  - Add `scripts/fetch-nono.sh <target>`. It downloads
    `nono-v<VERSION>-<target>.tar.gz` from the GitHub release (assets exist
    for `aarch64`/`x86_64-apple-darwin` and `aarch64`/`x86_64-unknown-linux-gnu`),
    checks it against SHA-256 hashes pinned in the repo, and installs it as
    `desktop/src-tauri/binaries/nono-<target>`. Pin the version and hashes in
    one file (`desktop/src-tauri/nono.lock`). Upstream publishes
    `SHA256SUMS.txt` but no signatures, so the repo pin is the trust anchor.
  - Wire the script into `just setup`, the sidecar stub step (`justfile:167`),
    and the release pipeline. `squareup/buzz-releases` must sign the extra
    binary in `Contents/MacOS` along with the existing sidecars.
  - At spawn time, resolve nono only from the app's sidecar directory (the
    exe's parent, as for `buzz-acp`), never from `PATH`, so the pinned version
    is what runs. Bumping nono is a reviewed change to `nono.lock`.
- Windows: nono supports only WSL2. The sandbox is unavailable and the setting
  is hidden; agents run as today.
- Remote backends (`backend.rs`, Kubernetes): out of scope; they do not run
  `buzz-acp` on this machine.

## Implementation steps

0. Bundle the pinned nono: `nono.lock`, `scripts/fetch-nono.sh`,
   `externalBin`, justfile wiring, and a release-signing note for
   `buzz-releases`.
1. Add `resources/nono/buzz-agent.json` and an installer in the nest
   bootstrap (`ensure_nest`) that writes the base and seeds the local stub
   (atomic temp-file + rename; one write per file).
2. Add `managed_agents/sandbox.rs`: nono discovery, validation, and command
   wrapping. Wire it into `spawn_agent_child`. Include the sandbox mode in
   `SpawnConfigSnapshot`.
3. `buzz-acp`: honor `BUZZ_ACP_OUTER_SANDBOX=nono` for the Codex overlay
   (macOS only), with tests in `acp/launch/tests.rs`.
4. Desktop settings UI: toggle, status, edit and reset actions, and the
   missing-nono error. Add accessibility labels per rule 7.
5. Tests (below), then docs: a short section in `desktop/README.md` and this
   file updated with final decisions.

## Testing

- Unit (Tauri): the spawn-site `Command` shape for on, off, missing nono, an
  invalid profile, and a symlinked REPOS; the profile installer is idempotent
  and never overwrites the local file.
- Unit (buzz-acp): the Codex overlay switches only under the outer-sandbox
  env, only on macOS.
- Manual on macOS and Linux against `just relay`:
  - Start a Claude, a Codex, and a Goose agent; mention each; confirm a reply.
  - Ask an agent to `cat ~/.ssh/id_ed25519` (denied) and to write a file in
    `~/.buzz/REPOS/<repo>` (allowed).
  - Add `"read": ["~/Documents/notes"]` to the local profile, restart the
    agent, and confirm the read now succeeds.
  - Stop the agent and quit the app: no `buzz-acp` survives (`pgrep -fl buzz-acp`).
  - Uninstall or rename nono: the spawn is refused with the install hint.

## Decisions

- nono is bundled with the app as a pinned, hash-verified sidecar (§6).
- One shared base profile, `buzz-agent`, for all runtimes.
- Network filtering is out of scope; v1 limits filesystem access only.
- The keychain is granted in the base profile so Claude subscription login keeps
  working (§1).

## Open questions

None at the moment. (Resolved: a local profile cannot remove an inherited
grant, which is why the keychain grant lives in the local profile.)
