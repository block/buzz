# Persona capability delivery

Managed personas can carry native skill references, stdio MCP connections, a
working directory and Buzz lifecycle scripts. These are owner-controlled local
execution settings. They assign capabilities; they do not isolate an agent from
other readable files, native defaults, credentials or tools.

## Configuration and persistence

`PersonaCapabilities` in `buzz-persona` is embedded in the existing persona
definition, unified agent store, global defaults and portable agent/team
snapshots. There is no separate registry. Older records default to no assignments.
The persona editor saves the same value used by effective configuration and the
restart-required comparison. Linked agents use the current persona definition;
unlinked instances retain their stored assignments.

Example value inside the existing `capabilities` field:

```json
{
  "skills": ["skills/priorities"],
  "mcp_servers": [{
    "name": "context",
    "command": "node",
    "args": ["/workspace/tools/context.mjs", "--fixture", "operations"],
    "env": {"ROLE": "operator", "TOKEN": "${SERVICE_TOKEN}"}
  }],
  "working_directory": "/workspace/operations",
  "hooks": {
    "on_start": "hooks/start.sh",
    "on_message": "hooks/observe.sh",
    "on_stop": "hooks/stop.sh"
  }
}
```

Skills merge as a stable union of global and persona references. A persona MCP
entry replaces a global entry with the same name. Individual hook events and the
working directory inherit unless explicitly provided. Empty lists inherit; this
version has no per-persona suppression of a global assignment. Global settings
use the existing configuration API; the persona editor exposes persona overrides.

The complete encoded assignment value is limited to 64 KiB, with at most 64 skill
references and 32 MCP connections. A working directory must be absolute and
exist at launch. Relative skill and hook paths resolve there. Missing references,
duplicate skill directory names and unsupported harness delivery fail visibly.

Agent/team JSON and PNG snapshot files saved locally, and cards locked to the
owner/agent pair, preserve the references and configuration, not the referenced
files. Snapshots sent through Buzz and unlocked cards are readable by others, so
they omit these assignments, as they already omit environment variables. Import
previews show executable settings, without a collapsed summary, before the
existing confirmation. Literal MCP environment values are kept in saved files:
use `${VARIABLE}` references for credentials. Only a whole-value reference expands,
from the harness's existing environment; missing variables fail preparation.
This does not read a secret store or provide a credential isolation boundary.

Public persona/catalog events do not publish these local executable settings.
Inbound edits preserve existing local assignments. Cross-device sharing through
the public catalog therefore does not transfer these assignments. Legacy
`.persona.md`/pack imports remain rejected by the pinned Desktop importer; the
supported portable format is the existing `.agent.json`/`.agent.png` snapshot.

## Harness delivery

Desktop passes the effective value through reserved
`BUZZ_ACP_PERSONA_CAPABILITIES`, sets the working directory and `AGENT_CWD`, and
captures the configuration for restart detection. `buzz-acp` creates a temporary
Claude plugin with links to the assigned skill directories and passes it through
`session/new` at `_meta.claudeCode.options.plugins`. Native `SKILL.md` content,
frontmatter, CRLF, Unicode and supporting files are untouched. The plugin exists
for the harness lifetime and is removed on normal teardown.

Codex receives a temporary `.agents/skills` root through ACP
`additionalDirectories`; the supported Codex adapter translates it into native
`skills/extraRoots/set`. OpenCode receives a temporary skill root through its
native `OPENCODE_CONFIG_CONTENT` skills configuration. Existing array and legacy
`skills.paths` forms are preserved and extended, as are unrelated settings.
These roots last for the harness lifetime. Codex additional directories may also
be writable native workspace roots; this is capability delivery, not isolation.
Other harnesses reject nonempty skill assignments rather than silently ignore them. Structured stdio MCP entries
reach the existing ACP `mcpServers` field, preserving arguments and environment.
They are additive to the Buzz baseline MCP server; a name collision is rejected.
Changing a supported harness's working directory also delivers the existing Buzz CLI skill
from the nest. It does not recreate the rest of the nest or guarantee every
native memory workflow from an arbitrary working directory.

Model selection continues through Buzz's existing ACP model configuration path.
MCP discovery is not proof of a successful tool call. Native `allowed-tools` and
hook metadata retain their native meaning; Buzz does not interpret them as an OS
access-control policy. MCP child processes can inherit harness environment; the
explicit MCP assignment payload is not a confidentiality boundary.

Assignment Debug/restart diagnostics mask contents. Outgoing `session/new`
diagnostic and observer copies redact the MCP configuration; the adapter receives
the original value. Hook output is discarded rather than logged or injected.

## Buzz lifecycle scripts

Scripts run with `/bin/sh`, the resolved working directory, `AGENT_CWD` and
`BUZZ_HOOK_EVENT`. Each invocation has a five-second limit, a 1 MiB input limit
and bounded output capture. Ordinary child processes in the hook's process group
are terminated and the parent reaped. Owner-trusted scripts must not daemonize
or detach into a new process session; this mechanism is not a sandbox for hostile
scripts. POSIX shell lifecycle execution is the implemented platform path.

- `on_start` runs once during harness preparation, before adapter startup. Failure
  aborts launch. Once startup begins, teardown attempts `on_stop`, including after
  a failed start script.
- `on_message` receives the exact text blocks joined by two newlines, immediately
  before a prompt or native steering request is written. Failure or timeout drops
  that direct dispatch without retrying it and posts a failure notice. Rejected
  messages remain in Buzz history and may still reach the model through later
  history retrieval or context. Rejecting a native steering update does not
  cancel the agent's existing task or trigger cancel-and-merge fallback.
  It observes text; its output never replaces
  or augments a prompt and is not a memory-injection mechanism. It is not an OS
  read-only filesystem restriction.
- `on_stop` runs once during normal harness teardown; failure is logged. Abrupt
  process termination cannot guarantee a cleanup callback.

The prompt/steering rejection notice is:

> The persona’s before-message script failed or timed out, so this message was not forwarded as a prompt or steering update. It remains in Buzz history. Review the script before sending again.

These hooks are distinct from `buzz-agent` advisory MCP hooks and native
Claude/Codex/OpenCode hooks. Native settings remain in effect in ordinary use.
Tests use a separate project configuration to prevent stale user hooks from
contaminating results, without editing global settings. Native Buzz memory is
not replaced; Hindsight and additional memory capture are outside this change.

## Verification and compatibility

Existing records without assignments retain their default launch folder, including
its original spelling. If no nest or home directory is available, the parent
process's working directory remains the fallback. Existing global model, provider,
runtime and environment settings do not become capability assignments.

Remote deployment rejects nonempty effective assignments, including inherited
global assignments. This rejects a newly requested configuration; legacy remote
configuration without assignments continues through the existing deploy path.
There is no per-agent opt-out from a global assignment, so keep assignments on
local personas when also using remote agents. A newly assigned working folder on
an unsupported harness can fail because Desktop must deliver the baseline Buzz
CLI skill into that folder. It does not add skills to an unassigned harness's
unchanged default folder. Reply routing remains unchanged.

The skill-link and lifecycle implementation targets POSIX hosts. Live evidence is
from macOS ARM64 only; Linux live behavior and Windows execution have not been
verified. Windows rejects nonempty native skill delivery. Running local `just ci`
checks the host platform; it does not execute the Windows CI lane.

Focused tests cover real serialization, storage through a fresh app handle,
JSON/PNG round trips, global merging, restart differences, native plugin files,
hook lifecycle/termination, prompt and steering rejection, and diagnostic
redaction. These tests do not establish live model use or Desktop community
operation. Live checks must additionally demonstrate an assigned skill and MCP
tool during useful work, using the owner-authorised relay and development identities. Native runtime and
adapter versions must be recorded separately: installed Desktop adapters may
differ from a staged direct-test adapter. Model selection must be confirmed by
the runtime rather than inferred from the saved persona field.

The contribution was reconciled with upstream `bf8b356f3bf5` on 17 October 2026;
the original audit branch was preserved. The overlapping discovery-test script
writer fix and upstream dependency pins are retained. Related proposals remain
unmerged: native skill validation
in [#3532](https://github.com/block/buzz/pull/3532), rsaulo's explicit `AGENT_CWD`
proposal in [#4558](https://github.com/block/buzz/pull/4558), and BradGroux's
structured MCP proposal in [#6651](https://github.com/block/buzz/pull/6651).
The cwd behavior implements the same intent as #4558. No patch was adopted
wholesale. Native skill metadata is not forced through #3532's proposed strict
schema; HTTP MCP support from #6651 is not implemented here. The older
[persona-pack spawn proposal #7359](https://github.com/block/buzz/pull/7359)
was closed without merging. This change uses Desktop's existing persona/store
path and native harness configuration; it does not adopt that proposal's pack
CLI or HTTP MCP support.
