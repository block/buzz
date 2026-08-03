# Buzz execution node

`buzz-node` is a standalone, runtime-neutral relay client for paired execution
nodes. It can run under any process supervisor; the supervisor only needs to
provide environment variables and a writable persistent data directory.

Run it with:

```bash
BUZZ_RELAY_URL=ws://localhost:3000 cargo run -p buzz-node -- run
```

Configuration:

- `BUZZ_RELAY_URL` — required relay WebSocket URL.
- `BUZZ_AUTH_TAG` — optional JSON-encoded NIP-OA authentication tag.
- `BUZZ_NODE_DATA_DIR` — optional durable data directory; defaults to
  `.buzz-node`. It must survive restarts and stores the node identity, paired
  owners, workload/credential state, and command idempotency journal. Identity
  and state files are created with owner-only permissions on Unix.
- `BUZZ_NODE_NAME` — optional display name used in node announcements.
- `BUZZ_NODE_HEALTH_ADDR` — optional local HTTP bind address for probes;
  defaults to `127.0.0.1:8081`.
- `BUZZ_NODE_MAX_CONCURRENT_COMMANDS` — optional positive limit for concurrent
  encrypted command processing; defaults to `8`.
- `BUZZ_NODE_SUBSTRATE` (or `--substrate`) — workload substrate for `run`;
  defaults to `process`, which launches each deployed agent as a supervised
  `buzz-acp` harness child process. `docker` runs each agent as a container of
  the agent body image. `inert` accepts commands without launching anything.
- `BUZZ_NODE_HARNESS_PATH` (or `--harness-path`) — optional explicit path to
  the `buzz-acp` harness binary used by the process substrate; defaults to a
  `buzz-acp` sibling of the node executable, then `PATH` lookup.
- `BUZZ_NODE_AGENT_IMAGE` (or `--image`) — base agent body image run by the
  docker substrate for the bundled `buzz-agent` runtime and unknown runtimes.
  Defaults to the published, digest-pinned buzz-sprig image
  (`ghcr.io/block/buzz-sprig:sha-…@sha256:…` — pinned by digest because the
  body holds an nsec; kept in sync with the Kubernetes backend's default).
  Fully overridable with any tag, digest, or custom registry reference — the
  operator's trust decision; `--image buzz-agent:local` remains the local-dev
  path (build it with `just agent-image` from `Dockerfile.agent`). Catalog
  runtimes with their own image variant resolve from
  `BUZZ_NODE_VARIANT_IMAGE_REPO` instead, never from this image. A deploy
  whose base image is missing fails closed with the exact
  `docker pull <reference>` command — the node never pulls or builds images.
- `BUZZ_NODE_VARIANT_IMAGE_REPO` (or `--variant-image-repo`) — local
  repository the docker substrate resolves per-runtime image variants from:
  catalog runtimes with their own variant (goose/claude/codex) run
  `<repo>:<runtime>`. Defaults to `buzz-agent`, so the goose runtime runs
  `buzz-agent:goose` etc., built on the node with
  `just agent-image goose|claude|codex` (`Dockerfile.agent`) — these are the
  "buzz-sprig plus your tools" override images. A deploy for a runtime whose
  variant image is missing fails closed with the exact
  `just agent-image <runtime>` build command.
- `BUZZ_NODE_DOCKER_PATH` (or `--docker-path`) — docker CLI used by the docker
  substrate; defaults to `docker` on `PATH`.
- `BUZZ_NODE_CONTAINER_RELAY_URL` (or `--container-relay-url`) — relay URL as
  reachable from inside agent containers. When absent, loopback relay hosts
  (`localhost`, `127.0.0.1`, …) are rewritten to `host.docker.internal`.
- `BUZZ_NODE_INACTIVITY_SECONDS` (or `--inactivity-seconds`) — seconds of
  inactivity (no dispatched events and no turn in flight) after which a
  workload body exits on its own, fed to both launching substrates' bodies as
  `BUZZ_ACP_EXIT_AFTER_INACTIVITY` (docs/remote-agents.md §Auto-Stop).
  Defaults to `7200`, mirroring the Kubernetes backend. `0` is legal and
  means no inactivity bound (the variable is omitted; the harness default is
  disabled). A body that reaps itself is recorded as stopped and never
  resurrected.

Both launching substrates hand each agent body its identity and relay
settings via environment variables (the same contract the Desktop launcher
uses) and forward a documented allowlist of the node's own environment — LLM
provider credentials such as `ANTHROPIC_API_KEY`/`OPENAI_API_KEY`, or
`CLAUDE_CODE_OAUTH_TOKEN` from `claude setup-token` for headless Claude Code
subscription auth, are node operator environment, never workload
configuration. Bodies that exit on their
own are recorded (clean exit → stopped, failure → failed) and never restarted
automatically.

An agent's private key is a one-time launch handoff. For the process
substrate it lives only in memory and in the child process environment, so
after a node restart a `start` fails closed until the owner redeploys from
Desktop. For the docker substrate the container is the key store: the key is
injected once via a short-lived owner-only env-file (never on a command line)
and lives on in the container's environment, so `start`/`restart` survive
node restarts and fail closed only once the container is gone. The node
refuses to start a docker-substrate `run` when the Docker daemon is
unreachable.

Both substrates resolve agent runtimes against the same catalog the Desktop
launcher uses: `goose`, `claude` (the `claude-agent-acp` npm adapter fronting
the Claude Code CLI via `CLAUDE_CODE_EXECUTABLE`), `codex` (the `codex-acp`
npm adapter plus `buzz-dev-mcp`), and the bundled `buzz-agent` (plus
`buzz-dev-mcp`). The process substrate resolves these to executables on the
node host; the docker substrate runs each catalog runtime in its own image
variant (`Dockerfile.agent`, `RUNTIME` build arg — build with
`just agent-image goose|claude|codex`), which bakes that runtime onto the
container `PATH` using the same official installer the Desktop auto-install
runs, with `CLAUDE_CODE_EXECUTABLE` pointing at the in-image Claude CLI in
the `claude` variant. The base image (the digest-pinned buzz-sprig default,
or the slim local `just agent-image` build) carries only the sprig
personalities and runs `buzz-agent`; every locally built image records its
runtime in the `buzz.runtime` OCI label and the `BUZZ_IMAGE_RUNTIME` env
var. Unknown runtime identifiers are attempted verbatim as a command name on
either substrate (in the configured base image on docker), so custom harness
setups and custom images keep working.

The data directory contains `identity.nsec` (the node identity), `owners.json`
(paired owner public keys), and `execution-state.json` (the durable workload
ledger — admitted workloads and removal tombstones — plus encrypted credential
state, receipt sequences, and the command idempotency journal). The process
substrate also keeps per-workload scratch directories under `workloads/` and
harness logs under `logs/`; the docker substrate keeps transient env-files
under `env/` only while a `docker run` is in flight. Back up and restore
the directory as one unit; restoring only part of it can detach workloads from
their identity or replay protection. Keep backups access-controlled because
the identity file can authenticate the node.

Operational endpoints:

- `GET /health` (or `/healthz`) returns `200` while the process is alive.
- `GET /ready` (or `/readiness`) returns `200` only after relay
  authentication, announcement, and command subscription succeed; it returns
  `503` while connecting or reconnecting.

The service reconnects with exponential backoff up to 30 seconds, drains
in-flight command work before a connection attempt is retried, logs lifecycle
and relay failures through `tracing`, and exits cleanly on Ctrl-C or SIGTERM.

While connected, the node publishes an ephemeral kind:20001 presence
heartbeat every 60 seconds over its WebSocket connection — the same presence
mechanism members and managed agents use. The relay keeps that presence in
Redis with a 180-second TTL and clears it on clean disconnect, so Desktop
derives node availability from live presence rather than announcement
freshness: a node with a Ready announcement and live presence shows as
connected, a Ready node without presence shows as unavailable within roughly
three minutes of an unclean drop, and a graceful shutdown (which publishes an
`offline` presence before disconnecting) flips immediately. The kind:30630
announcement itself stays slow-moving capability and pairing state; it is
only republished when owners or workload status change.

With a local relay already running, verify the standalone contract with:

```bash
./scripts/smoke-buzz-node.sh ws://localhost:3000
```

The relay URL must use the same host as the Desktop community it should join;
relay communities are scoped by host.

Pair an owner with `buzz-node pair --qr <desktop-qr-uri>`. Pairing runs as a
separate process and persists the owner attestation to `owners.json` in the
shared data directory; a node started afterwards announces it immediately. An
already-running `buzz-node run` process re-reads `owners.json` every few
seconds and, when the paired owners change, republishes its replaceable
announcement with the updated attestations and starts authorizing commands
from the new owner — no restart required. Command payloads contain only safe
workload data and credential references; credential material remains
node-local.
