# Phase 2 Task 1 Implementer Report

## Scope

Implemented the native LM Studio `/api/v1/chat` wire contract and bounded
response parser only. No provider configuration, HTTP transport, egress policy,
ACP session integration, or desktop code changed.

## Changed files

- `crates/buzz-agent/src/lmstudio.rs`
  - exact non-streaming native request serialization
  - stateful `previous_response_id` validation
  - exact ephemeral MCP integration serialization
  - native reasoning values and context/output token fields
  - ordered output parsing for message, reasoning, and executed tool calls
  - fail-closed invalid-call, terminal-message, shape, duplicate-field,
    response-ID, unknown-item, and size validation
  - deterministic synthetic ACP call IDs from request identity plus output index
- `crates/buzz-agent/src/types.rs`
  - shared executed-tool evidence and provider types
- `crates/buzz-agent/src/lib.rs`
  - exports the native LM Studio module

## TDD evidence

### RED 1 — request wire contract

Command:

```text
cargo test -p buzz-agent lmstudio::tests::new_chat_request_uses_only_native_fields --lib
```

Expected failure: unresolved native request, integration, reasoning, and limit
types. This established that the request behavior did not exist.

### GREEN 1

Command:

```text
cargo test -p buzz-agent lmstudio::tests:: --lib
```

Result after the minimal request implementation: 2 passed.

### RED 2 — bounded ordered parser

Command:

```text
cargo test -p buzz-agent lmstudio::tests::parser_preserves_provider_order_and_structured_tool_evidence --lib
```

Expected failure: unresolved parser, ordered output, response, executed-provider,
and response-size types. This established that provider output was not yet
parsed.

### GREEN 2

Command:

```text
cargo test -p buzz-agent lmstudio::tests:: --lib
```

Result: 9 passed. Coverage includes tool-looking message/reasoning text staying
inert, exact tool-call evidence, stable request-scoped call IDs, token stats,
state IDs, malformed shapes, duplicate fields, unknown item types, invalid
tool calls, missing terminal messages, bounded diagnostics, and oversized
bodies.

## Verification

```text
cargo fmt --all -- --check
cargo clippy -p buzz-agent --all-targets -- -D warnings
cargo test -p buzz-agent -- --test-threads=1
```

All passed. The package suite passed 348 tests across unit and integration
targets. An earlier default-parallel package run hit two timing-sensitive
pre-existing `fake_llm` failures; each passed on immediate isolated rerun and
the complete sequential package run passed.

## Remaining risks and follow-on boundaries

- Task 1 does not create an HTTP client or enforce endpoint/egress policy; that
  is Task 2.
- Plugin-shaped provider evidence is parsed because it is part of LM Studio's
  documented native response contract. Task 2 must forbid plugin integrations
  for `OFFICIAL`, and Task 3 must bind returned evidence to the exact requested
  ephemeral server/tool allowlist.
- The whole body is capped at 16 MiB and output count at 1,024. Task 2 must
  enforce the same body cap while reading HTTP so an oversized response is not
  fully buffered before this parser sees it.

## Review correction

Task 1 review found that the exported output/context maxima were serialized but
not enforced by the public constructor. A separate correction commit made
`LmStudioChatRequest::new` fallible and added a request-boundary invariant:

```text
1 <= max_output_tokens <= MAX_OUTPUT_TOKENS
max_output_tokens < context_length <= MAX_CONTEXT_TOKENS
```

The comparison widens `u32` output tokens with `u64::from` before comparing it
to the context length. The RED boundary test failed because the constructor
returned `Self`; after validation was added, all ten focused LM Studio tests
passed. Per-variant public documentation was also added for every native
reasoning value.

Model, input, and system-prompt size policy remains deliberately deferred. Task
1 has no HTTP transport or ACP-session request assembly: model configuration is
owned by the validated runtime work in Task 2, while prompt/system request
assembly and its transport-size budget are owned by Task 3. The Task 1
constructor documents this boundary and does not claim those strings are
independently bounded.
