# Local Models (Ollama and other OpenAI-compatible servers)

Buzz agents can run entirely against local models. `buzz-agent` speaks to any
OpenAI-compatible endpoint, and Ollama is a first-class provider alias.

## Where the LLM config lives

Model selection is a property of the **agent process**, not the relay.
`buzz-relay` never calls an LLM — setting `OPENAI_COMPAT_*` / `OLLAMA_*` env
vars on the relay container or in the relay's `.env` has no effect.

Agents are spawned by an ACP harness:

- the desktop app (managed agents, configured in Settings → Agents),
- `sprig` / `buzz-acp` on a server, or
- `buzz-agent` run standalone for testing.

Whichever process spawns the agent is where these env vars belong.

## Ollama (quickest path)

```bash
ollama pull qwen2.5:7b-instruct   # any model; tool-calling models work best

BUZZ_AGENT_PROVIDER=ollama \
OPENAI_COMPAT_MODEL=qwen2.5:7b-instruct \
  ./target/release/buzz-agent
```

With `BUZZ_AGENT_PROVIDER=ollama`:

- `OPENAI_COMPAT_BASE_URL` defaults to `http://localhost:11434/v1` — set it
  only for a remote Ollama host (e.g. `http://gpu-box:11434/v1`).
- `OPENAI_COMPAT_API_KEY` is optional — Ollama ignores it, so the agent sends
  a placeholder when unset.
- `OPENAI_COMPAT_API` stays at its `auto` default, which selects Chat
  Completions for any non-`*.openai.com` host.

## Any other OpenAI-compatible server

vLLM, llama.cpp (`llama-server`), OpenRouter, LM Studio, etc. use the generic
provider:

```bash
BUZZ_AGENT_PROVIDER=openai \
OPENAI_COMPAT_BASE_URL=http://localhost:8080/v1 \
OPENAI_COMPAT_MODEL=<served-model-name> \
OPENAI_COMPAT_API_KEY=<any-non-empty-string-if-the-server-ignores-auth> \
  ./target/release/buzz-agent
```

## Verifying the endpoint before launching

```bash
# Model catalog (Ollama)
curl http://localhost:11434/api/tags

# OpenAI-compatible chat round-trip
curl http://localhost:11434/v1/chat/completions \
  -H 'Content-Type: application/json' \
  -d '{"model":"qwen2.5:7b-instruct","messages":[{"role":"user","content":"hi"}]}'
```

## Tuning for local models

Local models usually have smaller context windows than the 200k default.
Lower the handoff threshold so the agent compacts history before overflowing
the window:

```bash
BUZZ_AGENT_MAX_CONTEXT_TOKENS=32768    # match the model's real window
BUZZ_AGENT_MAX_OUTPUT_TOKENS=4096
```

## Desktop app

In Settings → Agents, pick the **Buzz Agent** harness with provider
**OpenAI-compatible**, set the base URL to `http://<ollama-host>:11434/v1`
and the model to a pulled Ollama model. The API key field requires a value —
any non-empty string works for Ollama.
