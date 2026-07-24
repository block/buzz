# Command Console Phase 2 local agent runtime

Phase 2 adds `buzz-lmstudio-agent`, a separately packaged ACP runtime for
LM Studio's native REST API. It keeps `OFFICIAL` model and MCP HTTP egress on
literal loopback, preserves native response state per ACP session, and treats
only native `tool_call` output items as evidence of a tool that LM Studio
already executed.

This phase does not add the six-adviser orchestration, local RAG mirror, Memory
replication, Apple inputs, scheduled brief, cloud routing, or workspace
actions. It remains advisory and is not an accredited operational system.

The wire implementation follows LM Studio's
[REST API overview](https://lmstudio.ai/docs/developer/rest),
[native chat endpoint](https://lmstudio.ai/docs/developer/rest/chat), and
[MCP integration model](https://lmstudio.ai/docs/developer/core/mcp).

## Runtime configuration

Choose **Buzz LM Studio Agent** in the existing agent runtime configuration and
select a currently loaded model from native model discovery. The runtime
catalog locks the provider to `lmstudio-native`; the desktop does not infer LM
Studio readiness from mesh-compute status.

The trusted runtime projection owns these values:

| Variable | Phase 2 rule |
| --- | --- |
| `BUZZ_AGENT_CLASSIFICATION` | Forced to `OFFICIAL` when the desktop launches this runtime. Omission in the standalone binary also defaults to `OFFICIAL`. |
| `BUZZ_AGENT_PROVIDER` | Forced to `lmstudio-native`; no fallback provider is accepted. |
| `LM_STUDIO_MODEL` | Comes from the structured agent, live persona, or global model selection. The runtime is not ready without a selection. |
| `LM_STUDIO_BASE_URL` | Defaults to `http://127.0.0.1:1234`; only canonical literal `127.0.0.1` or `[::1]` HTTP URLs with a non-default explicit valid port are accepted. |
| `LM_STUDIO_MCP_INTEGRATIONS` | Defaults to `[]`; only validated loopback `ephemeral_mcp` entries are accepted. |
| `LM_STUDIO_REASONING` | Native vocabulary only: `off`, `low`, `medium`, `high`, or `on`. Use only values the selected model advertises. |
| `LM_STUDIO_API_TOKEN` | Removed from inherited and user environment layers; the desktop loads the optional value from Keychain only. |

Classification, provider, base URL, integration policy, fallback, token, and
model keys are reserved from agent/persona/global free-form environment
configuration. The desktop removes inherited copies before applying the
trusted values after every user layer. The Rust runtime revalidates the route
before every request.

The base URL and MCP policy may be supplied to the trusted desktop process for
development or managed deployment. They are not ordinary persona variables.
Phase 2 intentionally provides no UI or shell command that rewrites LM
Studio's global configuration.

### Keychain token

The optional token key is `lm-studio-api-token` inside Buzz's shared
`SecretStore` JSON blob. The production Keychain service is `buzz-desktop`;
development instances use their instance-specific
`buzz-desktop-dev.<instance>` service. The blob's Keychain account is
`secrets`.

Do not use `security add-generic-password` to replace this entry: it is a
shared JSON map containing other Buzz secrets. Provision the token through a
trusted Buzz secret-store path that preserves the rest of the map. The
non-mutating checker can read an already-provisioned entry without printing it:

```bash
./scripts/check-lmstudio-native.sh \
  --keychain-service buzz-desktop \
  --model qwen/qwen3.6-27b
```

For a one-off standalone check, use a silent shell read and the checker's
standard-input path. The token is not placed in process arguments:

```bash
read -rs LM_STUDIO_CHECK_TOKEN
printf '%s\n' "${LM_STUDIO_CHECK_TOKEN}" |
  ./scripts/check-lmstudio-native.sh \
    --token-stdin \
    --model qwen/qwen3.6-27b
unset LM_STUDIO_CHECK_TOKEN
```

The checker and desktop first make a tokenless catalog request. A token is sent
only after the service proves enforcement with `401` or `403`. A stored token
is never treated as evidence that authentication is enabled.

## Native MCP allowlists

Phase 2 accepts only explicit `ephemeral_mcp` integrations. Every server needs
a unique bounded label, a canonical literal-loopback HTTP URL, and a non-empty
exact tool allowlist:

```json
[
  {
    "type": "ephemeral_mcp",
    "server_label": "memory",
    "server_url": "http://127.0.0.1:8006/mcp",
    "allowed_tools": [
      "recall_for_entity",
      "search_events"
    ]
  }
]
```

The example is configuration shape, not a claim that port 8006 is currently
running locally. DNS names, `localhost`, LAN/private/public addresses, plugin
IDs, caller-supplied headers, duplicate labels/tools, empty allowlists, and
legacy ACP stdio MCP servers are rejected. Returned native tool evidence must
match the configured server label and tool name; otherwise the whole response
fails closed.

Retrieved or generated text cannot create a tool execution. Message and
reasoning strings that resemble tool markup remain inert. A native
`tool_call` is emitted to ACP as adjacent `pending` then `completed` observer
evidence with `executedByProvider: true`; Buzz never executes it a second time.

No usable pre-existing read-only Memory or RAG HTTP MCP was found on a
literal-loopback listener during the 24 July 2026 verification. The configured
Memory service remains a home-LAN service and is therefore ineligible for
`OFFICIAL` egress. A real structured Memory/RAG call is blocked until Phase 3
provides the MacBook-local services. Deterministic loopback tests prove the
native evidence path without editing LM Studio's `mcp.json`.

## Stateful sessions and failures

The native chat API owns conversation state. Each ACP session keeps its own
private `response_id`; a second prompt sends that value as
`previous_response_id`. Session state is cleared when the process/session ends.
It is not written to desktop configuration.

After an LM Studio restart, model reload, expired response, or lost ACP
process, continuation fails explicitly. The runtime does not reconstruct an
incomplete history, retry on a new branch, cross-use another session's ID, or
fall back to OpenAI-compatible/cloud transport. Change of model during an
active native session is rejected; start a new session instead.

Expected diagnostics are fixed and bounded:

- **unreachable** — no bounded response from the literal-loopback API;
- **authentication required** — the API returned `401/403` and no accepted
  Keychain/token value was available;
- **no loaded LLM model** — the catalog is healthy but has no loaded
  `type == "llm"` entry;
- **configured model unavailable** — the selected model is not loaded;
- **malformed/oversized response** — native schema or body limits failed;
- **expired/restarted native state** — continuation failed and requires a new
  ACP session.

Provider error bodies, prompt text, response text, tool arguments, tool output,
and bearer values are excluded from checker diagnostics.

## Non-mutating runtime check

The default recipe only reads `/api/v1/models`:

```bash
LM_STUDIO_MODEL=qwen/qwen3.6-27b just check-lmstudio-native
```

Opt in to one bounded `/api/v1/chat` request with the model's exact native
reasoning vocabulary:

```bash
LM_STUDIO_MODEL=qwen/qwen3.6-27b \
BUZZ_LMSTUDIO_SMOKE=1 \
BUZZ_LMSTUDIO_REASONING=off \
just check-lmstudio-native
```

The checker never loads, unloads, downloads, or changes a model/server setting.
It validates the endpoint before invoking `curl`, disables environment
proxies, does not follow redirects, bounds connect/total time and body sizes,
and prints only validated model identity, output item types, token statistics,
tool-call count, and response-ID validity. It never prints prompt or response
content.

Its deterministic contract suite is:

```bash
./scripts/tests/check-lmstudio-native-test.sh
```

The suite covers zero-request route denials, the explicit HTTP port 80
normalisation edge, byte-bounded identifier and header-injection denials,
ambient proxy bypass, unreachable/auth/no-load/mismatch states, malformed and
oversized catalogs, bearer/body redaction, exact reasoning `off|on`, native
request shape, pseudo-tool text, structured tool items, and the Just
entrypoint.

## Offline and security boundary

The Rust native client permits only the configured literal-loopback LM Studio
origin and literal-loopback MCP origins. It has no cloud fallback, disables
environment proxies, refuses redirects, sends no OpenAI-compatible `tools` or
`tool_choice`, and does not attach `buzz-dev-mcp`.

That is an application-side model/MCP egress boundary, not whole-host network
containment:

- the separately running LM Studio process's telemetry, update traffic, and
  its own internal MCP redirect behavior have not been contained or proven;
- successful loopback access does not prove LM Studio is bound only to
  loopback;
- the Buzz ACP harness still exchanges signed prompts/results through its
  configured relay.

Offline `OFFICIAL` operation therefore also requires the MacBook-local Buzz
relay and the Phase 3 local Memory/RAG services. Phase 2 does not prevent an
operator from configuring a remote relay, and this verification did not prove
the current relay configuration is local. Do not introduce real `OFFICIAL`
material until the relay, host egress, backups, and Defence information
handling have passed the later assurance gate.

## Verified evidence on 24 July 2026

The non-mutating live check observed:

- LM Studio application version `0.4.13+1`;
- native API reachable at `http://127.0.0.1:1234`;
- one loaded LLM, `qwen/qwen3.6-27b`;
- the API accepted a tokenless request, so authentication was not enforced;
- host inspection showed LM Studio listening on `*:1234`, while the API itself
  has no trustworthy bind-exposure field;
- reasoning capabilities advertised exactly `off` and `on`;
- `off` returned model instance `qwen/qwen3.6-27b:2`, item type `message`,
  statistics `input=30, output=2, reasoning=0`, and a valid response ID;
- `on` returned the same model instance, ordered item types
  `reasoning,message`, statistics `input=28, output=150, reasoning=146`, and a
  valid response ID; and
- neither live run returned a native tool call.

Only those shape and statistics facts were retained. Prompt and response
content were not included in logs or this document.

Deterministic ACP integration tests separately proved:

- a second prompt uses the first response's private `response_id`;
- message/reasoning pseudo-tool markup contributes zero structured calls;
- one allowlisted native `tool_call` produces exactly one ordered
  `pending`/`completed` observer pair without re-execution;
- cancellation, authentication, timeout, expired state, malformed responses,
  model mismatch, and session isolation fail explicitly; and
- proxy, redirect, fallback, LAN/public endpoint, plugin, and stdio-MCP routes
  remain denied.

The unsigned Tauri build produced:

```text
desktop/src-tauri/target/aarch64-apple-darwin/release/bundle/macos/Buzz.app
```

Its executable
`Contents/MacOS/buzz-lmstudio-agent` was present and mode `0755`. Full Xcode is
installed, but the current global developer directory is
`/Library/Developer/CommandLineTools`; Task 4 selected full Xcode per command
to produce the `.app`. Tauri's later DMG AppleScript automation did not
complete in that environment, so no DMG success is claimed. Signed and
notarised release packaging remains governed by
[RELEASING.md](../../RELEASING.md).

## Phase 3 dependencies

Phase 3 must supply and verify:

- a read-only local Memory/RAG MCP service on canonical literal loopback;
- the encrypted signed RAG snapshot, encoders, sparse search, reranker, golden
  queries, staging import, atomic switch, and rollback;
- the writable local Memory node and authenticated replication to the home
  authority;
- host-level LM Studio network/update/telemetry containment evidence;
- offline restart exercises with internet and home LAN disabled; and
- proof that the configured Buzz relay is the MacBook-local authority.
