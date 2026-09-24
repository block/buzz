# Local task runner

`buzz-acp run` runs one prepared task in one fresh agent process and ACP session,
then exits. Scripts and automation can use it without starting a conversational
`buzz-acp` service or sending a chat message.

```sh
buzz-acp run --task ./task.json
cat task.json | buzz-acp run --task -
```

Put command options after `run`. Use the normal trusted launch settings for
identity, relay, adapter, model, effort, permissions, prompts, MCP tools, and
memory. `buzz-acp run --help` lists them. The working directory is the launch
working directory, not the task file's parent. The task cannot change it.

## Task document, version 1

Supply exactly one UTF-8 JSON object, at most 1 MiB including whitespace:

```json
{
  "version": 1,
  "taskId": "example-001",
  "agentPubkey": "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
  "prompt": "Inspect the current workspace. Report your findings using the task's specified destination.",
  "maxDurationMs": 120000
}
```

The example public key is illustrative; use the public key of the configured
agent. Never put a private key in a task document.

| Field | Contract |
| --- | --- |
| `version` | Integer `1`. Other versions fail before agent launch. |
| `taskId` | Nonblank string, 1–256 UTF-8 bytes, no control characters. Correlation only, not a claim or deduplication key. |
| `agentPubkey` | Exactly 64 lowercase hexadecimal characters. Must equal the configured agent's public key. |
| `prompt` | Nonblank prepared task string. Include relevant context, original evidence, and reporting instructions here. |
| `maxDurationMs` | Positive unsigned 64-bit integer. The shorter of this limit and the trusted `--max-turn-duration` cap wins. |

All fields are required. Unknown fields, duplicate fields, wrong types, trailing
JSON documents, and malformed JSON are rejected. File/stdin input has a ten-second
read limit. Close stdin after the document. The input limit is separate from the
execution deadline.

Launch configuration is the authority for credentials, executable, tools,
permissions, and relay. A document is **executable task input**, not a sandboxed
message: only feed tasks that are trusted to use that agent's capabilities. The
identity check prevents accidental execution by another agent; it does not
verify the document's author. File/stdin input does not need a signature.

## Session behavior

- Load normal base, persona, and team instructions. Add the task-session context
  instead of a conversational session description. Modern adapters receive
  standing instructions through session setup; legacy adapters receive them with
  the first prompt. `--no-base-prompt` and custom base files still work.
- Load enabled core memory for the resolved owner and identity before session
  creation. `--no-memory` skips this. No owner means no memory namespace. The
  normal three-second fetch bound applies. Confirmed absence supplies the normal
  onboarding hint; a failed read supplies no hint, not a false empty-memory claim.
- Apply model, supported effort and permission settings through the existing
  session setup. Unsupported settings retain the existing adapter fallback
  behavior. Pass configured identity and relay to the child and MCP tools; keep
  the inherited authorization and other launch environment. The shared runtime
  prepares Git identity, scoped credentials, and signing helpers for both task
  and conversational sessions. Temporary key material lives until adapter
  cleanup is complete.
- Submit one prepared task, allowing multiple model/tool exchanges. There is no
  initial-message turn, heartbeat turn, conversation history, automatic channel
  context, or second task. Context must be supplied or fetched with tools.
- Do not start ordinary relay subscriptions, a standing pool, presence, typing,
  a setup listener, observer publication, or an isolated-turn socket. Inherited
  service-only options cannot enable these features. Shared option parsing still
  validates launch settings.
- Do not discover, borrow, stop, or reconfigure an existing host. Memory, files,
  credentials, and external side effects may be shared with concurrent sessions.
  Session isolation is not a security sandbox or a concurrency lock.
- Do not automatically publish final text. Use task reporting instructions and
  tools. Do not retry the task after failure, timeout, or uncertain execution.

Launch configuration preparation (including prompt-file reads) has a separate
ten-second bound. A blocked configuration read reports `configuration_timeout`
with exit 2; SIGINT/SIGTERM during that read reports `cancelled` with exit
130/143 and a null `taskId`. No adapter is started during this phase.

The execution deadline covers adapter startup, memory loading, session setup,
and the task turn. SIGINT/SIGTERM cancel the invocation. If a prompt is active,
allow up to five seconds for cooperative cancellation, then kill and reap the
child, with a further five-second reap bound. Startup and setup cancellation do
not wait for a session to become ready. On Unix, cleanup kills the process group,
including ordinary descendants. Processes that deliberately escape that group
are outside this guarantee. On non-Unix platforms cleanup uses the existing
ACP direct-child cleanup; descendant containment is not promised.

## Terminal output

For each attempted invocation, stdout contains one JSON line (except help/version
output). Diagnostics and child stderr stay on stderr. No prompt, private key,
agent final text, or raw adapter error is included in the terminal record.

```json
{"version":1,"taskId":"example-001","status":"completed","stopReason":"end_turn"}
```

`taskId` is null until the document is validated. `status` is one of `completed`,
`failed`, `invalid`, `timed_out`, or `cancelled`. `stopReason` is present only for
normal completion: `end_turn`, `max_tokens`, `max_turn_requests`, or `refusal`.
Failures include a stable, sanitized `error` code when available, such as
`agent_identity_mismatch`, `invalid_task_json`, or `agent_spawn_failed`.

| Exit | Meaning |
| --- | --- |
| `0` | Turn returned normally. Inspect `stopReason`; this is not proof of task success. |
| `1` | Startup/execution failure, adapter cancellation, or terminal-output failure. |
| `2` | Invalid arguments, launch configuration, or task input/read. |
| `124` | Execution deadline exceeded. |
| `130` | SIGINT; bounded cleanup ran. |
| `143` | SIGTERM; bounded cleanup ran. |

A killed host or broken stdout can leave no terminal record. A record is not a
persistent result ledger. Launching the same task twice can execute it twice.

## Future work

URL input and task-server retrieval are **not implemented**. URL-shaped sources
are rejected, not fetched. A future design can GET a supplied URL and validate
the same task document, with explicit authentication, size/time bounds, and
redirect rules. This project adds no HTTP task endpoint or job queue.

## Shared runtime ownership

`runtime::AgentRuntime` prepares shared capabilities and owns their lifetime.
Both entry points obtain adapter configuration and prompt context from it;
add new shared capabilities there rather than in either entry point. Session
creation, tool configuration, model/effort/permissions, and legacy prompt
framing use the existing pool code. Mode-specific code owns task input and
terminal output or conversational intake and scheduling, not agent equipment.

## Build and verification

```sh
. ./bin/activate-hermit
cargo build --release -p buzz-acp
cargo test -p buzz-acp
```

`tests/run_task.rs` invokes the actual binary with a deterministic ACP peer. It
checks stdin/file parity, fresh process/session setup, legacy and modern standing
instructions, tools, identity, model/effort/permissions, memory reads and opt-out,
invalid input, clean JSON, deadlines, signals, and Unix descendant cleanup.
These checks do not require a model provider or production relay.
