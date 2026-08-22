# Endpoint launch configs

Deployment-time mapping from manifest endpoint names to
`EndpointLaunchConfig` (`provider` / `api_key_env` / `env`), passed to
`harbor run` as `--agent-kwarg endpoint_config=<path>`. This is deployment
config, deliberately OUTSIDE the immutable condition manifest — the manifest
endpoint string remains the join key.

Every key in these files must be a manifest endpoint name; the loader treats
all entries as endpoint configs (no comment keys).

## m1-local.json

M1 wiring proof: both placeholder endpoints resolve to one local llama-server
(OpenAI-compatible, `http://127.0.0.1:8091/v1`, no cloud keys).

buzz-agent env contract (crates/buzz-agent/src/config.rs, pinned at the M1
binary SHA): `provider=openai` reads `OPENAI_COMPAT_API_KEY` +
`OPENAI_COMPAT_BASE_URL`; the runtime sets `BUZZ_AGENT_MODEL` from the
manifest endpoint name, which overrides `OPENAI_COMPAT_MODEL` — llama-server
ignores the model name, so the placeholder value is harmless there.
llama-server needs no real key; the provisioner's per-endpoint
`llm_api_keys` map supplies a dummy value.

The Databricks pilot config is the same file shape with real serving
endpoint hosts/keys.
