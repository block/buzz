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

Exactly one orchestrator is required; **workers are optional**. A roster of one
orchestrator and nothing else is the single-agent baseline — see
`manifests/tb-solo-sonnet.yaml` and `personas/solo-tb.md`. The lone agent gets
byte-identical wiring to a worker (same binaries, same MCP toolset, same env),
because a handicapped baseline would flatter every multi-agent condition
measured against it.

`generation` has no `temperature`: `buzz-agent` exposes no temperature
environment variable, so the value could never reach the provider. It used to
be accepted, hashed into the condition identity, and silently discarded — with
a default of `0.0` while the measured provider default is `1.0`. It is now
rejected outright rather than quietly lying.

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

## Trial artifacts

Every trial writes a self-contained bundle to its `buzz/` artifact directory —
the container is gone once the trial ends, so this is what survives:

```
manifest.json                the frozen condition and its sha256
<agent-id>.system-prompt.md  the composed prompt as that agent saw it
<agent-id>.stdout/.stderr.log
receipts.jsonl               one priced usage row per agent
endpoints.redacted.json      provider and env-var *names* only, never values
summary.json                 the index to start from
```

The bundle exists so system prompts can be tuned after a sweep: the exact
prompt bytes sit next to what that agent cost. It is written even when the
trial fails or times out — a trial that burned tokens and then stalled still
cost money, and dropping it would bias cost figures toward successes.

Because every agent runs with a live provider token and a Nostr private key in
its environment, the bundle is scanned for credential-shaped strings and the
result is recorded in `summary.json` under `secret_scan`. Check it before
sharing a bundle.

### Container trust store

Many Terminal-Bench images ship no `ca-certificates` package, so
`/etc/ssl/certs/ca-certificates.crt` does not exist and **every https client in
the container fails** — apt, curl, git, pip. `buzz-agent` is the exception: it
gets `SSL_CERT_FILE=/opt/buzz/ca-certificates.crt`, which nothing else reads.

That asymmetry cost real scores. On the A1 sweep an agent hit a transient apt
error, rewrote `sources.list` from http to https to work around it, and left
the container in a state where the **verifier's** own `apt-get update` could
not validate a certificate. The verifier reported `E: Unable to locate package
curl`, never installed pytest, and the task was recorded as reward 0.0 —
identical in `result.json` to a model that got the answer wrong.

So `_install_stack` copies the same bundle to `/etc/ssl/certs/ca-certificates.crt`
before the agents launch. It is offline (the bundle is already uploaded), so it
adds no egress dependency, and it **only writes when that path is missing or
empty** — a task whose subject is certificate handling keeps whatever store its
image shipped.

The outcome is recorded per trial in `result.json` under
`agent_result.metadata.container_trust_store`:

| value | meaning |
|---|---|
| `present` | the image had its own store; we changed nothing |
| `seeded` | the image had none and we installed ours |
| `failed` | the image had none and we could not install ours |

`failed` is deliberately not fatal — the agent still reaches its provider
through `SSL_CERT_FILE`, and killing a runnable trial would trade a partial
handicap for a total one. But **read a 0.0 on a `failed` trial as suspect**,
not as a wrong answer.

### Verifier dependencies

81 of the 89 Terminal-Bench `test.sh` files begin with `apt-get install -y
curl`, to fetch the uv installer that installs pytest. Only 3 of 12 sampled
task images ship curl, so most trials do that install at scoring time — the one
moment when a transient apt failure is unrecoverable and lands as a 0.0.

So the runtime installs `VERIFIER_DEPS` itself, and the timing is the point:

- **After the agents are stopped.** Installing before would hand the agent a
  tool its task image chose not to ship, which changes the thing being
  measured. Doing it in teardown changes only what the verifier finds.
- **Before Harbor's verifier phase**, which is what needs it.

The win is not saving the verifier's apt call — it still makes one. It is that
with curl already in dpkg's status file, `apt-get install -y curl` resolves
from the installed version and succeeds *even when `apt-get update` left the
index empty*. That is exactly the failure that scored `compile-compcert` 0.0.

Recorded as `agent_result.metadata.container_verifier_deps`: `present` (image
shipped it) | `installed` | `unavailable`. Best-effort, like the trust store —
measured at ~7s per trial on the EC2 runner, which at `--n-concurrent 16`
is under a minute across a full 89-task sweep.

The package list is deliberately just `curl`. The rest of the task set's apt
requests are a long tail no blanket pre-install should chase: git 3, binutils
1, everything else once.

### Preflight

`benchmark.py` refuses to start a sweep that cannot be scored. Two checks run
before the stack, the binaries, and the money:

- `check_tls_not_intercepted()` — the **host's** path to PyPI is not being
  MITM'd. A Cloudflare WARP session once turned a live 89-task sweep into 0/89.
- `check_container_can_be_scored()` — a **container** can seed its trust store
  and install curl. The host reaching PyPI says nothing about this: the docker
  proxy, the image's trust store and apt are a separate path, and it is the one
  every verifier depends on.

The container check runs the trials' own two setup commands —
`seed_trust_store_command()` and `install_verifier_deps_command()`, imported
from `container_runtime`, not reimplemented — against `ubuntu:24.04`, the base
most task images derive from and which likewise ships neither `ca-certificates`
nor curl. A preflight that proved something subtly different from what the
trials do would be worse than none, because it would read as a clean bill of
health. It costs ~7s.

Both follow the same rule: a check that *cannot see* (no docker, no openssl, a
pull that timed out) prints `skipping check` and lets the run proceed. Only a
positive signal of breakage stops it.

### What none of this covers

An agent that switches apt to a mirror the network cannot reach at all.
`compile-compcert` failed that way — the agent moved apt to
`https://azure.archive.ubuntu.com`, whose Azure address is unreachable from the
Square egress path at :443 regardless of trust anchors. Restoring
`sources.list` before the verifier runs would close it; that is not implemented.

`verifier_health.py` carries `unable to locate package` as the backstop, so a
verifier that still cannot install its dependencies is reported as broken by
`summarize.py` rather than averaged in as a zero.

### Token and cost accounting

`buzz-agent` reports cumulative token counts per turn, `buzz-acp` logs them
under the `acp::usage` tracing target, and the runtime parses them out of the
downloaded logs to price each agent against the manifest's `prices` table. The
runtime sets `RUST_LOG` to enable that target; if an endpoint config sets its
own `RUST_LOG`, the directive is appended rather than replaced, because losing
it would present as a free trial rather than as an error.

**Cache reads are estimated, not measured.** `LlmResponse.input_tokens` is an
inclusive sum of plain, cache-read, and cache-write input and nothing
downstream carries the split. Each endpoint's `Price` therefore declares a
`cache_read_rate` (0.0–1.0); that fraction of input is billed at
`cached_input_per_million_usd` and the rest at the full rate. Every receipt
also carries `cost_usd_no_cache_discount`, so the assumption's leverage is
visible rather than baked into one number.

The rate lives in the manifest, not in code, so it is frozen into the condition
hash — two cost figures are only comparable under the same assumption.

Choosing a rate:

- **Anthropic-route endpoints: `0.0`, and that is exact, not conservative.**
  `buzz-agent` never sends `cache_control` and Anthropic prompt caching is
  opt-in, so no cache reads occur. (It also means Buzz is leaving a real
  discount on the table — worth fixing in the agent, not in the harness.)
- **OpenAI-route endpoints: non-zero.** OpenAI caches automatically above ~1k
  prompt tokens and reports `cached_tokens`, and an agent loop resends a
  growing prefix every turn, so the true rate is substantial. Calibrate it from
  the gateway's own `cached_tokens` on a real trial rather than guessing.

Leaving every rate at 0.0 gives the old upper-bound behaviour, but it is not
neutral across conditions: it over-charges exactly the endpoints that really do
cache, which biases OpenAI-route workers against Anthropic-route ones.

**Reasoning tokens are not separable, and this does not affect cost.** Providers
bill thinking at the output rate, so reasoning tokens inside `output_tokens` are
already priced correctly. Only the thinking-vs-answer split is unavailable —
an analysis detail, not a cost error.

**Auto-compaction is billed but unreported, so it is bounded instead.** When a
session crosses its compaction threshold, `buzz-agent` summarises the history
into a fresh context (a "handoff"). That summarisation is a real provider call,
but `Llm::summarize` returns a bare `String` and throws its usage away, so those
tokens never reach the counters above. On a long trial with the default
`max_handoffs` of 80 the omission can exceed everything else we model.

The runtime parses the handoff log lines — which carry the pre-handoff context
size — and publishes a worst case beside the metered figure: each handoff is
charged at most its pre-handoff context as input plus tokens of output, at
full rate. So `cost_usd` is the floor, `cost_usd_including_handoff_bound` is the
ceiling, and the truth is between them. Receipts keep `handoffs`,
`handoff_input_tokens_upper_bound`, and `handoff_cost_usd_upper_bound` separate
from the metered fields; nothing is silently merged. Getting the exact number
requires a `buzz-agent` change (return usage from `summarize` and fold it into
the turn counters).

Handoffs do **not** fail reconciliation — the metered numbers are still genuine —
but the reconciliation note says the total is a floor whenever any occurred.

A separate signal to watch: if the handoff cap is reached or summarisation
fails, the agent falls back to *truncating* its own history. The runtime counts
those as `handoff_truncations` and raises a warning, because dropped context is
a task-fidelity problem, not just a cost one.

A trial whose agents ran but reported no tokens is marked
`accounting_reconciled: false` with a reason. Such a trial is an
instrumentation failure, not a free trial, and must not be averaged into a cost
figure as though it were a real zero.

### Compaction threshold

`buzz-agent` fires a handoff at whichever comes first: a percentage of the
context window (`BUZZ_AGENT_HANDOFF_PERCENT`, default 90) or an absolute
ceiling (`BUZZ_AGENT_HANDOFF_AT_TOKENS`, default 272,000; `0` disables it). The
ceiling is inert at a 200k window, where 90% (180k) binds first, and takes over
on large ones — at 1M, 90% would mean summarising ~900k tokens of history in a
single call, and every request would sit above OpenAI's 272k long-context
pricing boundary.

Two manifest fields pass straight through, both optional:

```yaml
generation:
  max_output_tokens: 4096
  context_window_tokens: 1000000
  compact_at_percent: 30       # → BUZZ_AGENT_HANDOFF_PERCENT
  compact_at_tokens: 272000    # → BUZZ_AGENT_HANDOFF_AT_TOKENS
```

Leave them unset to inherit the agent's defaults; the runtime then omits the
variables entirely, so a variable's presence in a trial bundle means the
condition really did pin it. `context_window_tokens` is always reported to the
agent as the real window — the harness never bends it to move the trigger.

`compact_at_tokens` is validated against `context_window_tokens - max_output_tokens`,
since buzz-agent reserves room for the response and a target above that could
never fire. A ceiling merely larger than the percentage is fine — that is the
default at 200k. Because compaction cadence moves both cost and task
performance, both fields are part of the condition hash.

## Leaderboard runs

`just benchmark` is the one-command path: it stands up a dedicated Docker
stack (`buzz-benchmark` compose project — relay :3600, Postgres :5633, secrets
generated once into the gitignored `.benchmark/`), applies the benchmark
schema, and defaults to leaderboard-eligible settings
(`terminal-bench/terminal-bench-2-1`, 5 attempts per problem, the Sonnet+Haiku
team). All selectors pass through:

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

Harbor accepts two dataset forms, resolved from different places: `org/name[@ref]`
from the hub package registry, and `name[@version]` from `registry.json`.
Terminal-Bench 2.1 exists only as a package — `terminal-bench@2.0` in
`registry.json` is the older 2.0 cut — which is why the default is the
slash form. Whichever form is given is validated against the right source
before the stack comes up, so a typo or an upstream rename fails in a second
rather than after the first trial.

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
