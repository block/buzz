# Endpoint launch configs

Deployment-time mapping from manifest endpoint names to
`EndpointLaunchConfig` (`provider` / `api_key_env` / `env`), passed to
`harbor run` as `--agent-kwarg endpoint_config=<path>`. This is deployment
config, deliberately OUTSIDE the immutable condition manifest — the manifest
endpoint string remains the join key.

Every key in these files must be a manifest endpoint name; the loader treats
all entries as endpoint configs (no comment keys).

**These are examples.** A real run supplies its own file with
`--endpoint-config <path>`, because the workspace host, the credential
plumbing and the model slate are all operator-specific. Nothing here should
name a private host, an internal address, or a secret store entry.

## m1-local.json

Wiring proof: both placeholder endpoints resolve to one local llama-server
(OpenAI-compatible, `http://127.0.0.1:8091/v1`, no cloud keys). This is the
config to use when you want to exercise the harness without a provider.

buzz-agent env contract (`crates/buzz-agent/src/config.rs`):
`provider=openai` reads `OPENAI_COMPAT_API_KEY` + `OPENAI_COMPAT_BASE_URL`;
the runtime sets `BUZZ_AGENT_MODEL` from the manifest endpoint name, which
overrides `OPENAI_COMPAT_MODEL` — llama-server ignores the model name, so the
placeholder value is harmless there. llama-server needs no real key; the
provisioner's per-endpoint `llm_api_keys` map supplies a dummy value.

## openai-live.json

Models served by the OpenAI API directly. Endpoint names here are literal
OpenAI model ids, because the runtime sets `BUZZ_AGENT_MODEL` from the
manifest endpoint name.

No `OPENAI_COMPAT_BASE_URL` override: the default `https://api.openai.com/v1`
is correct, and leaving it unset also lets `OPENAI_COMPAT_API=auto` select the
Responses API, which is the route these reasoning models need.

`scripts/benchmark.py` populates `OPENAI_COMPAT_API_KEY` from `OPENAI_API_KEY`
when only the latter is exported, and resolves provider credentials only for
the endpoints a given config actually names — so an OpenAI run does not require
a working Databricks token.

## anthropic-live.json

Anthropic's API directly (`provider=anthropic`, `ANTHROPIC_API_KEY`). Same
shape; endpoint names are literal Anthropic model ids.

## openrouter-live.json

OpenRouter (`provider=openrouter`, `OPENROUTER_API_KEY`).

**Pin the upstream.** One OpenRouter model id is served by several providers at
different prices, quantisations and cache behaviours, so an unpinned run is not
a reproducible condition — it silently mixes them. `OPENROUTER_PROVIDER_ORDER`
names the upstream (`moonshotai/mxfp4`, `gmicloud/fp8`, …), and ordering alone
is not a pin unless fallbacks are disabled.

## databricks-example.json

The shape for a Databricks AI Gateway workspace (`provider=databricks_v2`,
`api_key_env=DATABRICKS_TOKEN`). Replace `DATABRICKS_HOST` with your own
workspace; it rides in each entry's `env` block and Harbor injects it into the
agent container, so it needs no host-side export.

Endpoint names must be the workspace's **serving endpoint names**, which on a
Databricks gateway carry a `databricks-` prefix — the console's display label
and the model id it echoes back both fail to resolve.

**Two gateway routes, selected per model by the provider, not by config.**
`databricks_v2_route_for_model` in buzz-agent sends `gpt-5`-named models to
`/ai-gateway/openai/v1/responses` and `claude`-named models to
`/ai-gateway/anthropic/v1/messages` (Opus 5 does not speak `/responses`).
Anything else goes to the shared MLflow chat-completions route. So the endpoint
name alone picks the route, with no extra config.

Unlike a workspace OAuth bearer, which expires roughly hourly and makes an
unattended sweep impossible, a personal access token in `DATABRICKS_TOKEN` is
long-lived. Fetch it from wherever your deployment keeps secrets; do not commit
one here.
