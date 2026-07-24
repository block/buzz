# Phase 2 Task 2 Implementer Report

## Outcome

Implemented the typed LM Studio-native runtime policy and transport boundary.

- `Classification` is exactly `PUBLIC | OFFICIAL`; omission defaults to
  `OFFICIAL`.
- Phase 2 native routing is local-only for both classifications and accepts
  only literal `http://127.0.0.1:<port>` or `http://[::1]:<port>` origins.
- Native MCP configuration accepts only explicit `ephemeral_mcp` records with
  literal-loopback URLs, unique bounded labels, and unique non-empty bounded
  tool allowlists. Unknown fields, duplicate JSON fields, plugin IDs, headers,
  fallbacks, hostile IP spellings, userinfo, queries, and fragments fail
  closed.
- Native requests are authorised immediately before send on the exact models,
  chat, continuation, and summary paths.
- The dedicated `reqwest` client disables proxies and redirects, bounds
  connect/read/total timeouts plus request/response/error bodies, and never
  retries native stateful/MCP requests.
- Returned tool evidence is accepted only from a configured ephemeral server
  and allowed tool. Plugin or unallowlisted evidence fails closed.
- Optional `LM_STUDIO_API_TOKEN` bearer authentication is bounded and redacted
  from debug and provider-error output. The native provider never constructs a
  cloud token source.
- Added `Provider::LmStudioNative` and a distinct
  `buzz-lmstudio-agent` entry point. It defaults to the native provider, refuses
  every other provider, and does not expose the generic `auth` subcommand.
- Existing Anthropic, OpenAI-compatible, and Databricks paths remain on their
  prior code paths.

## TDD evidence

Observed RED before implementation for:

- Missing classification, endpoint, integration, and request-authorisation
  types.
- Missing `LmStudioNative` provider and dedicated-provider resolver.
- Missing native transport, redirect/no-retry, denial-before-send, and evidence
  validation.
- Missing binary source/entry point.
- Native use of the cloud token-source path.
- Token redaction and oversized request denial.
- Native model identifier bounds.

Focused GREEN suites:

```text
cargo test -p buzz-agent egress::tests
9 passed

cargo test -p buzz-agent llm::tests::native_
7 passed (before the later oversized-request test was added)

cargo test -p buzz-agent --test lmstudio_entrypoint
2 passed
```

## Final verification

```text
cargo fmt --all -- --check
passed

cargo clippy -p buzz-agent --all-targets -- -D warnings
passed

cargo test -p buzz-agent -- --test-threads=1
297 unit + 75 integration tests passed; 0 failed
```

## Deliberate Task 3 boundary

The native client and entry point are available, but the existing ACP session
loop still rejects `Provider::LmStudioNative` through its legacy stateless
`Llm::complete`/`summarize` methods. Task 3 must bind ACP sessions to this
client, create exact requests from validated runtime integrations, persist
`response_id`, and map native ordered output into ACP updates.
