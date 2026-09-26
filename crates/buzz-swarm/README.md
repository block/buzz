# buzz-swarm

Run Buzz agents from a laptop, Kubernetes pod, EC2 instance, or another
Linux/macOS host. Swarm resolves each agent's key, signs its NIP-OA owner
attestation using the existing SDK, then starts one `buzz-acp` process per
agent/relay pair. Every connection for an agent uses **the same key and auth
tag**. There is no fixed relay-count limit.

The existing harness handles relay connections, conversations, agent sessions,
and tools. Swarm adds only configuration, identity setup, and process
supervision. It runs where you launch it; your deployment platform provisions
the host, logs, and persistent storage.

## Start

Install `buzz-swarm` and the existing agent runtime:

```sh
cargo install --path crates/buzz-swarm --locked
cargo install --path crates/sprig --locked
# Create Sprig's standard personality links in the directory on PATH:
cd "${CARGO_HOME:-$HOME/.cargo}/bin"
ln -s sprig buzz-acp
ln -s sprig buzz-agent
ln -s sprig buzz-dev-mcp
```

Separately installed `buzz-acp`, `buzz-agent`, and `buzz-dev-mcp` binaries also
work. Create `buzz-swarm.yaml`:

```yaml
owner:
  nsec: { env: BUZZ_OWNER_NSEC }
defaults:
  relays: [wss://team.example.com, wss://project.example.com]
agents:
  - name: helper
  - name: reviewer
    nsec: { file: keys/reviewer.nsec } # optional existing identity
```

Supply the owner's private key through the referenced environment variable or
file. The runtime also needs its usual model/provider configuration and
credentials, inherited from the environment or provided under `env` in YAML.

```sh
buzz-swarm --config /etc/buzz/swarm.yaml validate
buzz-swarm --config /etc/buzz/swarm.yaml
buzz-swarm --config /etc/buzz/swarm.yaml --only helper
```

Missing agent keys are generated once and saved as `keys/<name>.nsec` beside
the config, with mode `0600`. Subsequent runs reuse them. Mount that directory
on persistent storage in ephemeral containers, or supply keys from your secret
manager. `validate` checks swarm wiring, identities, and the harness executable;
it writes no keys or directories and starts nothing. Generated preview
identities are temporary.
The runtime validates its own model and ACP settings when started.

The owner must have access to every target relay, and each relay must support
NIP-OA. Profiles, channel membership, conversation history, and memory remain
local to each community. Set a profile and invite agents to the relevant
channels on each relay. The harness defaults to responding only to its owner.

## Configuration

See [buzz-swarm.example.yaml](buzz-swarm.example.yaml). Agent fields replace
`defaults`; `env` merges by variable name. Unknown fields are rejected.

| Fields | Purpose |
|---|---|
| `owner.nsec`, `owner.conditions` | Owner key and optional NIP-OA conditions |
| `name`, `nsec`, `auth_tag`, `enabled` | Per-agent identity and selection |
| `relays` | Non-empty relay list, shared or per agent |
| `workdir` | Existing working directory; default `workspaces/<name>` beside the config, created `0700` on start |
| `harness` | Executable path or name on PATH, default `buzz-acp` |
| `restart`, `max_restarts` | `on-failure` (default), `always`, or `never`; default budget 10 |
| `env` | Existing ACP/agent options and credentials |

Keys and environment values accept literals, `{env: VARIABLE}`, or `{file:
PATH}`. Prefer references for secrets; file contents are trimmed. Swarm file
paths resolve relative to its directory, with `~` expansion. Paths embedded
inside runtime environment values retain the runtime's own semantics, usually
relative to `workdir`.

Configure the runtime directly using its existing environment contract:

```yaml
defaults:
  relays: [wss://team.example.com]
  env:
    BUZZ_AGENT_PROVIDER: anthropic
    BUZZ_AGENT_MODEL: your-model-id
    ANTHROPIC_API_KEY: {env: ANTHROPIC_API_KEY}
    BUZZ_ACP_SYSTEM_PROMPT: You are a helpful engineering teammate.
    BUZZ_ACP_RESPOND_TO: owner-only
```

Swarm defaults `BUZZ_ACP_AGENT_COMMAND` to `buzz-agent` and
`BUZZ_ACP_MCP_COMMAND` to `buzz-dev-mcp`. Override these under `env` for another
ACP runtime; an empty MCP command disables it. See `buzz-acp --help` and
[buzz-agent](../buzz-agent/README.md) for runtime options. Ambient `BUZZ_ACP_*`
settings are cleared; use YAML `env` to configure them explicitly. Model/provider
environment variables are inherited unless the file names them as a source.
Swarm owns relay and identity variables; YAML `env` cannot override them. Every
`{env: VARIABLE}` source in the file, including in `defaults` and disabled
agents, is removed from the inherited environment; only agents that declare it
receive its value, under the declared name. Processes sharing an OS user still
share that user's filesystem access.

An existing agent key and pre-signed `auth_tag` can be supplied without `owner`,
so a remote execution host need not hold the owner's key. Swarm verifies each
tag against its agent key, owner, and time conditions. All agents in a swarm
must have the same owner. Duplicate identity/relay pairs are rejected. Separate
deployments must also avoid running the same pair twice.

## Lifecycle and verification

Swarm stays in the foreground. Harness stdout/stderr go directly to the caller;
use the runtime or deployment platform for logging. Failures retry with 1–60
second backoff; the restart budget resets after 60 seconds of uptime. Clean
exits stay ended under the default policy. Final failures produce exit code 1.

Ctrl-C or SIGTERM stops harness process groups with 15 seconds of grace
(`start --grace SECONDS`). A second signal forces shutdown. Remaining members of
the harness's own group are killed before its leader is reaped, including on
natural exit. Swarm does not track processes a harness moves into other groups.
The default `buzz-acp` does this for each agent and stops those groups itself on
SIGTERM; after a forced shutdown or a harness crash they can outlive Swarm. Run
Swarm under container or service containment when that matters. Workdirs are
ordinary directories, not sandboxes.

```sh
cargo test -p buzz-swarm
cargo clippy -p buzz-swarm --all-targets -- -D warnings
```

Tests launch real fixture processes to check identity/tag reuse, key persistence,
config-relative invocation, failure before launch, and process cleanup. They do
not contact hosted relays or model providers.
