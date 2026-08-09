# Command Model Routing Design

## Goal

Add one persistent Command Console control that switches the whole Command
Adviser between cloud-first and local-first model routing without changing its
Apple, Memory, RAG, evidence, citation, or persistence behaviour.

## Scope

This is a bounded extension of the accepted macOS MVP:

- `Cloud first`: LiteLLM, then direct OpenAI, then LM Studio.
- `Local first`: LM Studio, then LiteLLM, then direct OpenAI.
- The selection applies to manual and scheduled Daily Command Briefs.
- The selection persists across app restarts.
- A run captures one selection when it starts; changing the selection affects
  only later runs.
- Existing cancellation, validation, evidence, citation, deterministic
  consolidation, signing, and publication behaviour remains unchanged.

This phase does not add per-adviser model selection, a general provider
management screen, billing controls, or new knowledge services.

## Verified Deployment Facts

- The live Caddy configuration is `/etc/caddy/Caddyfile` on `web-01`.
- `litellm.home.arpa` proxies to `localhost:4000`.
- The direct LAN endpoint is
  `http://192.168.1.26:4000/v1/chat/completions`.
- `192.168.1.31:1234` is the Mac Mini LM Studio backend, not LiteLLM.
- LiteLLM `1.82.3` is healthy and its database is connected.
- The existing `chatgpt-5.4` alias succeeds with streamed chat completions and
  produces raw JSON, while non-streaming chat completions return HTTP 500.

## Configuration and Credentials

`trusted-lan-sources.json` remains the protected, native-owned configuration
source. It gains:

```json
"model_routing_preference": "cloud_first"
```

The field accepts only `cloud_first` and `local_first`. Existing files without
the field load as `local_first`. Native writes use the existing restricted
atomic JSON writer and retain mode `0600`.

The installed configuration will use:

```json
"litellm": {
  "enabled": true,
  "endpoint": "http://192.168.1.26:4000/v1/chat/completions",
  "model": "chatgpt-5.4",
  "keychain_key": "command.cloud.litellm"
}
```

The LiteLLM master key is copied directly from the protected `web-01` runtime
into the existing `buzz-desktop` Keychain blob without printing or writing it
to a plaintext file. Direct OpenAI remains the second cloud route and uses
`command.cloud.openai` when a key is installed through the secure OpenAI
Platform flow.

## Native Routing

Introduce a closed `ModelRoutingPreference` enum in the trusted-LAN contract.
`FallbackAdviserProvider` receives that value when the production orchestrator
is constructed and uses one of two fixed attempt sequences:

```text
cloud_first: LiteLLM -> OpenAI -> Local
local_first: Local -> LiteLLM -> OpenAI
```

Unavailable routes are skipped. Only existing model-route-eligible execution
errors advance to the next provider. Cancellation, invalid request, evidence
rejection, and policy errors remain terminal.

The trusted-LAN configuration identity is included in
`RuntimeConfigIdentity`. A preference or endpoint change therefore installs a
new runtime for later runs while an active run finishes on its captured
runtime.

## LiteLLM Streaming

LiteLLM requests use `"stream": true`. The client reads the bounded response
body with cancellation checks, accepts only `data:` Server-Sent Events,
requires a terminal `[DONE]`, concatenates only
`choices[0].delta.content`, and rejects malformed, oversized, empty, or
unterminated responses. The reconstructed terminal string continues through
the existing strict adviser JSON validators.

Direct OpenAI retains its current Responses API request and response parser.

## User Interface

Add a compact `Model routing` card below System Status:

- Two radio-style options: `Cloud first` and `Local first`.
- Short live copy lists the exact attempt order.
- The control is disabled while a brief is active.
- Saving is immediate and persists natively.
- A failed save restores the prior value and shows a concise error.

The top Command Adviser banner reflects the selected order instead of claiming
that LM Studio is always preferred.

## Failure Behaviour

- If LiteLLM is unavailable in cloud-first mode, direct OpenAI is attempted,
  then LM Studio.
- If cloud credentials are absent, those routes are skipped.
- If all model routes fail, the existing partial-brief and deterministic
  consolidation behaviour remains authoritative.
- Switching routing never changes source admission or sends additional data;
  each attempted cloud provider receives only the same bounded evidence payload
  already produced by the source collector.

## Acceptance

- Rust tests prove both provider orders, terminal-error stopping, legacy
  defaulting, protected preference persistence, runtime identity changes, and
  strict LiteLLM SSE parsing.
- React/API tests prove load, save, rollback on failure, disabled-while-running
  behaviour, and visible route copy.
- A live LiteLLM specialist-format smoke succeeds using the Keychain credential.
- A cloud-first Daily Command Brief is signed, published, and persisted.
- Local-first mode is restored and proves LM Studio is attempted first.
- `just ci`, the signed bundle build, and strict code-sign verification pass.

