# Running ACP harnesses from WSL on Windows

Buzz Desktop discovers ACP harness commands (Hermes Agent's `hermes-acp`, Oh
My Pi's `omp`, custom harnesses, …) by resolving them on the host PATH. On
Windows, many of these CLIs are installed inside a **Windows Subsystem for
Linux (WSL)** distribution instead of on the Windows host — Hermes Agent, for
example, is a Linux-first install. Without help, Doctor shows such harnesses
as *Not installed* even though they work fine inside WSL.

Buzz bridges this automatically.

## How it works

1. **Discovery fallback.** When a preset or custom harness command is not
   found on the Windows PATH, Buzz probes the *default* WSL distribution with
   `wsl.exe -e sh -c …`, checking `~/.local/bin`, `~/bin`, `/usr/local/bin`,
   and finally the distro PATH (`command -v`). A hit marks the harness
   **Available** with a `wsl://…` binary path in Doctor.
2. **Spawn wrapping.** At agent start, Buzz Desktop passes the in-distro path
   to `buzz-acp` via `BUZZ_ACP_AGENT_WSL_PATH` (and optionally
   `BUZZ_ACP_AGENT_WSL_DISTRO`). `buzz-acp` launches the agent through
   `wsl.exe -e <path> <args>`; WSL interop bridges the stdio pipes, so the
   ACP/NDJSON channel is byte-transparent.
3. **Environment forwarding.** Windows processes do not automatically share
   environment variables with WSL. `buzz-acp` computes a `WSLENV` list of
   every variable it injects for the agent (per-runtime defaults such as
   `HERMES_ACP_SKIP_CONFIGURED_MCP`, persona env such as `GOOSE_PROVIDER`) so
   they arrive inside the Linux process. A pre-existing `WSLENV` (including
   flagged entries like `GOPATH/p`) is preserved and merged.
4. **Session cwd translation.** The desktop spawns `buzz-acp` from
   `~/.buzz`, so the `cwd` carried into `session/new` is a Windows drive path
   that does not exist inside the distro — agents use it as the root for
   edit-approval policies and workspace grounding. For WSL-targeted agents,
   `buzz-acp` translates drive-letter paths to their `/mnt/<drive>/…` form
   (`C:\Users\x\.buzz` → `/mnt/c/Users/x/.buzz`); UNC, relative, and already
   POSIX paths pass through untouched.

`BUZZ_ACP_AGENT_COMMAND` keeps the original bare command identity
(`hermes-acp`), so all per-runtime defaults keyed on the command name keep
working unchanged. Together these close the four WSL boundary breaks
catalogued in [block/buzz#3122](https://github.com/block/buzz/issues/3122)
(env, cwd, and — via discovery — the missing-harness path; skill placement
and teardown reaping remain follow-ups, see the issue's item list).

## Verifying

Doctor → harness catalog: a WSL-only harness shows *Available* with a binary
path like `wsl:///home/<you>/.local/bin/hermes-acp`. Starting an agent on it
logs `spawning agent through WSL` from `buzz-acp`.

## Limits

- Only the **default** WSL distribution is probed today. The spawn contract
  (`BUZZ_ACP_AGENT_WSL_DISTRO`) already supports naming a distro; multi-distro
  discovery is future work.
- Only bare command names are probed (never paths), and probing runs at most
  once per command per app launch (cached).
- Auth probes and install scripts for tier-1 built-in runtimes still assume a
  host-native install; the WSL fallback covers preset and custom harnesses.

## Manual alternative (no rebuild)

The same result is possible without this feature by registering a custom
harness (`<app-data>/custom_harnesses/hermes-wsl.json`) that runs
`C:\Windows\System32\wsl.exe` with args `["-e", "/home/<you>/.local/bin/hermes-acp"]`
and env `{"HERMES_ACP_SKIP_CONFIGURED_MCP": "1", "WSLENV": "HERMES_ACP_SKIP_CONFIGURED_MCP"}`.
The native fallback exists so users don't have to hand-author this.
