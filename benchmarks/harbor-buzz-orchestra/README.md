# Harbor Buzz Orchestra

A stock-Harbor custom agent that runs a manifest-defined team through the real
Buzz stack. Harbor sees one `BuzzOrchestraAgent`; behind that adapter, one
orchestrator and N workers coordinate over the production relay/Postgres.
Each agent runs *inside* the Harbor task container as the same
`buzz-acp` → `buzz-agent` → `buzz-dev-mcp` process tree the desktop app
launches: the production MCP toolset (shell, file tools, todo) with the
`buzz` CLI on the shell's PATH. No Harbor fork or patch is required.

## Define the team

The manifest is the benchmark condition. Each roster entry selects an agent
class's count, model endpoint, byte-pinned system prompt, generation settings,
and budget:

```yaml
condition: my-team
roster:
  - id: orch
    kind: orchestrator
    role: lead
    count: 1
    endpoint: databricks/frontier
    prompt: {path: personas/orchestrator.md, sha256: <sha256>}
    generation: {max_output_tokens: 4096, context_window_tokens: 128000}
  - id: worker
    kind: worker
    role: implementer
    count: 4
    endpoint: databricks/fast-worker
    prompt: {path: personas/worker.md, sha256: <sha256>}
    generation: {max_output_tokens: 4096, context_window_tokens: 128000}
```

`endpoint_config` maps those endpoint names to providers, URLs, and API-key
environment variables. The adapter contains no fixed roster or model.

## Run

With the production compose stack and model endpoints already running, execute
one task (`-p`), a directory of tasks, or replace `-p` with Harbor's dataset and
task selectors:

```bash
uv run --project benchmarks/harbor-buzz-orchestra/testbed harbor run --yes -p <TASK_OR_DIRECTORY> --agent harbor_buzz_orchestra:BuzzOrchestraAgent --agent-kwarg manifest=<CONDITION.yaml> --agent-kwarg provisioner_factory=harbor_buzz_testbed:provisioner_from_dict --agent-kwarg provisioner_config=<PROVISIONER.json> --agent-kwarg endpoint_config=<ENDPOINTS.json> --agent-kwarg artifact_root=benchmarks/harbor-buzz-orchestra --agent-kwarg buzz_acp_binary=<LINUX_BIN>/buzz-acp --agent-kwarg buzz_agent_binary=<LINUX_BIN>/buzz-agent --agent-kwarg buzz_dev_mcp_binary=<LINUX_BIN>/buzz-dev-mcp --agent-kwarg buzz_cli_binary=target/debug/buzz --agent-kwarg run_id="bench-$(date -u +%Y%m%dT%H%M%SZ)" --agent-timeout-multiplier 15 --n-concurrent 1
```

`buzz_acp_binary`/`buzz_agent_binary`/`buzz_dev_mcp_binary` must be **Linux**
builds matching the task image architecture — they are uploaded into each task
container (`just benchmark` cross-builds them automatically; musl-static, so
any Linux base image works). `buzz_cli_binary` is the **host** CLI the harness
uses to act as the trial user.

`--n-concurrent 1` is the safe laptop setting for a serialized local model; it
is not an orchestration requirement. Some TB graders install dependencies from
public package registries at verification time — run benchmarks off networks
that block those installs (e.g. corporate VPNs).

Each trial gets fresh keys and a private Buzz channel. The provisioner archives
rather than deletes that channel, leaving the relay/Postgres event timeline
and the per-agent acp/agent logs (downloaded into the trial's `buzz/`
artifacts) available for analysis.

## Leaderboard runs

`just benchmark` is the one-command path: it stands up a dedicated Docker
stack (`buzz-benchmark` compose project — relay :3600, Postgres :5633, secrets
generated once into the gitignored `.benchmark/`), applies the benchmark
schema, and defaults to leaderboard-eligible settings (Terminal-Bench 2.1,
5 attempts per problem, the Sonnet+Haiku team). All selectors pass through:

```bash
just benchmark                                   # full TB 2.1, k=5
just benchmark --path <TASK_DIR> -k 1            # one local task, one attempt
just benchmark -i "cobol*" --attempts 3          # dataset subset
just benchmark --gui                             # watch the run live
```

One pinned user identity fronts the whole benchmark environment: it owns
every trial channel (named after the task) and posts every task prompt, and
trial channels are kept rather than archived. `--gui` adds that user to the
relay membership list and opens the Buzz desktop app logged in as them, so
channels fill the sidebar as the run progresses — watch, don't type; a human
message mid-trial would taint the run. `just benchmark-down` stops the stack.

Networking: the relay is host-header tenant-bound, so agents must dial its
canonical address (`ws://localhost:3600`) even from inside a task container.
`just benchmark` uploads a tiny std-only loopback forwarder
([`forwarder/relay_forwarder.rs`](forwarder/relay_forwarder.rs)) with the
agent stack; it listens on the container's loopback and bridges the byte
stream to the Docker host gateway (`host.docker.internal`, overridable via
`BUZZ_BENCHMARK_DOCKER_HOST`).

`scripts/run_leaderboard.py` is the layer underneath, for running against an
already-provisioned stack. It wraps the invocation above with only
leaderboard-legal settings — it does not accept or forward timeout or resource
overrides, so the job directory it produces passes Harbor's static validation
as-is. Give it a problem set, attempts per problem, and a team manifest:

```bash
uv run --project benchmarks/harbor-buzz-orchestra/testbed \
    benchmarks/harbor-buzz-orchestra/scripts/run_leaderboard.py \
    --dataset terminal-bench/terminal-bench-2-1 \
    --attempts 5 \
    --manifest benchmarks/harbor-buzz-orchestra/manifests/<TEAM>.yaml \
    --endpoint-config benchmarks/harbor-buzz-orchestra/testbed/endpoints/<ENDPOINTS>.json \
    --provisioner-config <PROVISIONER.json>
```

`--path` replaces `--dataset` for local task directories; `--include-task` /
`--exclude-task` filter by glob; `--dry-run` prints the underlying `harbor run`
command. After the job finishes the script derives a `metadata.yaml` from the
manifest roster (validated schema; review the display names before submitting)
and prints the `harbor upload` / `harbor leaderboard submit` commands.

## Validate

```bash
cd benchmarks/harbor-buzz-orchestra
uv run --extra dev pytest -q
uv run --extra dev ruff check .
cd testbed
uv run --extra dev pytest -q
uv run --extra dev ruff check .
```

Live provisioner tests require the benchmark compose stack and opt-in
environment described in `testbed/tests/test_provisioner_live.py`.
