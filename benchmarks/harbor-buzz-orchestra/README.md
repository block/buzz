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

### Buzz-native tasks

The local [`benchmarks/buzz-dataset`](../buzz-dataset) suite — a sibling
directory of this harness, not a subdirectory of it — scores Buzz product
behavior alongside task correctness. It covers direct thread replies, callback
user mentions, targeted reads of named paths, exact channel membership,
multiline delivery, non-waking narrative names, batched reports, cross-thread
isolation, ambiguous identities, and explicit cold-memory retrieval. Run one
task with the production base prompt from the checked-out source build:

```bash
just benchmark \
  --path benchmarks/buzz-dataset/reply-to-thread \
  --manifest benchmarks/harbor-buzz-orchestra/manifests/buzz-native-solo-luna.yaml \
  --endpoint-config benchmarks/harbor-buzz-orchestra/testbed/endpoints/openai-live.json \
  --n-concurrent 1
```

Buzz-native tasks declare one of two evaluation layers in `task.toml`.
**Regression** tasks are deterministic product/prompt regression checks and
default to k=1. **Workflow** tasks exercise multi-step collaboration
capabilities and default to k=3 (not 5). Run a layer by metadata with
`--path benchmarks/buzz-dataset --layer regression` or `--layer workflow`;
task identities stay unchanged.

When the Buzz dataset root is passed without `--layer` or `--attempts`, the
wrapper starts two sequential Harbor jobs so each layer gets its own default.
A direct task path infers its layer's default. An explicit `--attempts`/`-k`
overrides the defaults and permits one mixed-layer job. Terminal-Bench and
other unrelated paths keep their existing k=5 default.

The default condition is `buzz-native-solo-luna.yaml` — one solo agent on
`gpt-5.6-luna` at `thinking_effort: medium`. What this suite scores comes from
the base prompt rather than from model strength, so the cheap model at a
middling effort is the right yardstick: a weak result here is a prompt finding,
not a model finding. It needs `OPENAI_COMPAT_API_KEY` and the explicit
`--endpoint-config` above, because `--endpoint-config` defaults to
`anthropic-live.json`. Swap in `buzz-native-solo-sonnet.yaml` (no
`--endpoint-config`, needs `ANTHROPIC_API_KEY`) to compare against Sonnet 4.6.

A roster entry that does not pin `generation.thinking_effort` runs at the
runtime default (`THINKING_EFFORT`, currently `medium`) rather than at whatever
the provider defaults to, so the level is always recorded. Leaving it unset
does not change a condition's hash — manifests written before the effort axis
existed keep their identity and stay comparable to their earlier receipts.

Replace the path with `benchmarks/buzz-dataset/create-channel-invite-users`
to run the channel task. Its provisioner seeds a stable directory of 50 users
and 10 bots, while the verifier checks the created channel's TTL and exact
membership through post-agent CLI evidence.

After the agent stops, the runtime snapshots public relay state (source
messages plus any task-declared channels and members) to
`/logs/artifacts/buzz-evidence.json`. The task verifier reads that post-agent
artifact; relay credentials and database access are never exposed to the model
or verifier. If the snapshot cannot be exported the trial **fails** rather than
scoring 0 — a harness fault and a model fault stay distinguishable — and the
cause is written to the trial's `buzz/buzz-evidence-error.txt`.

Some tasks declare additional signed relay events. The provisioner creates
their actors as normal channel identities and the runtime publishes the events
through the production CLI immediately after the task message. Evidence exports
only public actor metadata and event IDs; their signing credentials never enter
the task container or verifier artifact.

Each task ships its own `README.md` documenting its reward dimensions and, for
the tasks whose graded Buzz behavior is deliberately absent from
`instruction.md` (`reply-to-thread`, `user-mention`), why that omission is the
point. Read it before editing a task's instruction or verifier.

## Leaderboard runs

`just benchmark` is the one-command path: it stands up a dedicated Docker
stack (`buzz-benchmark` compose project — relay :3600, Postgres :5633, secrets
generated once into the gitignored `.benchmark/`), applies the benchmark
schema, and defaults to leaderboard-eligible settings (Terminal-Bench 2.1,
5 attempts per problem, the Sonnet+Haiku team). All selectors pass through:

```bash
just benchmark                                   # full TB 2.1, k=5
just benchmark --path <TASK_DIR> -k 1            # one local task, one attempt
just benchmark --path benchmarks/buzz-dataset --layer regression   # Buzz k=1
just benchmark --path benchmarks/buzz-dataset --layer workflow     # Buzz k=3
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
