# Core Buzz Local Pilot Implementation Plan

## Goal

Deliver a locally runnable, public/synthetic-data-only Buzz pilot for Core with
one banker, one observed channel, and one ambient research-and-drafting agent.
The language model may produce text but must never choose the destination,
author identity, tags, or delivery semantics of a Buzz event.

Azure deployment and live client data are explicitly gated on the frozen banker
evaluation. They are not part of this implementation branch.

## Global Constraints

- Follow the repository `AGENTS.md`, including Hermit activation, no new
  production `unwrap()`/`expect()`, no `unsafe`, public API docs, `just ci`, and
  signed-off commits.
- Use test-driven development: every behavioral production change begins with a
  test that is observed failing for the missing behavior.
- Defaults remain compatible with upstream Buzz. Every Core behavior is opt-in.
- Pilot model settings are exact: provider `openai`, model `gpt-5.6-terra`,
  Responses API, reasoning effort `medium`, base URL
  `https://api.openai.com/v1`, and no fallback.
- The agent has no MCP command, shell, filesystem tools, email sending, or
  external writes other than one trusted Buzz reply produced by `buzz-acp`.
- Pilot input is public or synthetic only. Secrets stay outside Git.
- One owner identity and exactly one UUID channel may trigger publishing.
- A model never supplies channel IDs, reply IDs, event tags, author keys, or
  event kinds.
- Search citations come only from OpenAI Responses API metadata. A searched
  terminal response without safe, valid citation/source metadata fails closed.
- Failures never publish partial model output.
- Do not implement Azure or enable real attachments on this branch.

### Task 1: Add fail-closed OpenAI hosted web search and citations

Implement the opt-in web-search surface in `buzz-agent`.

1. Add `Config.web_search: bool`, parsed from `BUZZ_AGENT_WEB_SEARCH` with
   numeric default `0`.
2. When enabled, startup validation must require provider `openai`, Responses
   API, and the canonical HTTPS API origin `https://api.openai.com/v1` (allow a
   trailing slash after normalization). Reject compatible third-party endpoints
   in this mode.
3. Extend `responses_body()` with hosted tool
   `{ "type": "web_search", "external_web_access": true,
   "search_context_size": "medium" }`, keep existing function tools, retain
   `tool_choice: "auto"`, and request
   `include: ["web_search_call.action.sources"]`.
4. Parse `web_search_call.action.sources` in provider order, exact-URL dedupe,
   allowing only `http` and `https`. Parse `output_text` `url_citation`
   annotations and convert character indices safely to Rust byte offsets.
5. Render visible clickable Markdown citation markers `[[n]](<url>)` and append
   `### Sources` containing every consulted source, including consulted but
   inline-uncited sources. Escape Markdown titles safely.
6. If a response used web search and the terminal result has missing sources,
   missing citations, malformed ranges, unsafe URLs, or citations not present in
   the complete source list, return an LLM error before any ACP message chunk is
   emitted. Ordinary non-search output and function calls remain unchanged.
7. Cover config validation, request shape, Unicode indices, ordering, dedupe,
   escaping, consulted-only sources, malformed/missing/unsafe cases, and normal
   Responses/function-call regressions.

### Task 2: Add trusted single-channel ACP output publishing

Implement an opt-in `buzz-acp` publishing mode.

1. Add public enum `PublishAgentOutput::{Off, TriggerReply}` and configuration
   flag/env `--publish-agent-output` / `BUZZ_ACP_PUBLISH_AGENT_OUTPUT` with
   default `off` and opt-in value `trigger-reply`.
2. In `trigger-reply` mode, fail startup unless the normalized agent command is
   `buzz-agent`, agents mode is enabled, exactly one valid channel UUID exists,
   subscribe mode is `all`, kinds are exactly `9`, response policy is
   `owner-only` with a configured owner, MCP command is empty, ignore-self is
   true, heartbeat is zero, dedup is `queue`, and multiple-event handling is
   `queue`.
3. Add a bounded per-prompt message accumulator to `AcpClient`. Clear it at each
   new prompt/session, append only `agent_message_chunk`, expose a result-taking
   method, cap at 65,536 UTF-8 bytes, and fail closed on overflow. Any ACP
   `tool_call` invalidates the prompt output. Discard initialization/heartbeat
   output.
4. Only a real channel `FlushBatch` can publish. Select `batch.events.last()` as
   the trusted trigger and require its sole `h` tag to equal the batch channel.
   For a top-level message use trigger ID as root and parent. For a threaded
   trigger keep the existing root as both root and parent so agent replies remain
   flat under the human root.
5. After terminal `EndTurn` or `Refusal`, publish non-empty accumulated text once
   as a signed kind-9 event using `buzz_sdk::build_message`. Do not add `p`,
   `broadcast`, media, or caller-supplied tags. Empty output is silence.
6. Build/sign once. Retry the identical event ID. Treat `accepted: true` as
   success; after an ambiguous result query the exact ID before retrying. Never
   rerun the LLM or re-sign because of ambiguity. Maintain one bounded in-memory
   pending event for the local pilot.
7. Cancellation, timeout, max-token, agent error, oversized output, ACP tool
   call, malformed thread tags, or channel mismatch produces operator-visible
   diagnostics and no channel message. Ignore-self prevents feedback loops.
8. Cover invariant acceptance/rejection, accumulation/reset/thought/tool/
   overflow, top-level/nested/batch-last/malformed/mismatched targeting, signed
   kind/tags/author, identical-ID retry/ambiguous confirmation, silence, errors,
   no cross-channel publish, literal command-shaped output, and self-loop
   prevention.

### Task 3: Add an opt-in non-coding relay capability

1. Add `BUZZ_GIT_ENABLED`, default `true` for upstream compatibility.
2. When false, do not construct the Git store and do not mount Git smart-HTTP or
   Git policy routes. Other relay APIs and health endpoints remain unchanged.
3. Surface disabled Git capability consistently in relay metadata if an existing
   capability mechanism exists; do not invent an unrelated HTTP endpoint.
4. Add tests proving Git routes are absent when disabled and unchanged by
   default.

### Task 4: Add reproducible Core pilot assets

Add reviewed, non-secret assets for launching and evaluating the pilot.

1. Add `config/core-pilot/core-research-partner.md` with these policies:
   selective ambient response; silence for thanks/chatter/duplicates; public or
   synthetic data only; refuse and request sanitization for client identifiers,
   live deals, MNPI, or PII; prefer SEC/regulator/issuer IR sources; separate
   facts/inference/assumptions/draft language; mark emails `DRAFT — NOT SENT`;
   at most one response per banker message; no progress acknowledgements; no
   tools, internal systems, email sending, or external side effects.
2. Add a checked-in environment template with exact safe Core settings but no
   secrets. It must include `BUZZ_ACP_NO_BASE_PROMPT=1`, the system-prompt file,
   `BUZZ_ACP_NO_MEMORY=1`, `BUZZ_AGENT_NO_HINTS=1`,
   `BUZZ_AGENT_REQUIRE_REPLY=0`, one channel, owner-only response, queue modes,
   safe publish mode, exact OpenAI settings, web search, and Git disabled.
3. Add idempotent launch/preflight scripts that read secrets from a user-owned
   file outside Git, reject placeholder secrets or unsafe configuration, start
   only the local relay/ACP/agent stack, and print readiness without printing
   secret values. Scripts must not reset Docker volumes.
4. Add a concise operator runbook for Windows Desktop + WSL + Docker Desktop,
   including stop/restart behavior and explicit warnings against `down -v`,
   `just reset`, or real client data.
5. Add the frozen ten-task banker scorecard and hard-fail criteria from the plan:
   overall 82; research 80; deliverables 82; ambient 85; no task below 70;
   citation coverage 95%; numerical accuracy 98%; at least 8/9 usable with light
   edit; at least 4/5 useful interventions; zero responses to 7 silence controls;
   and automatic failure for fabricated citations, material financial error,
   private data, external write/send, cross-channel leak, non-owner response, or
   response to an explicit silence control.
6. Test launch/preflight behavior through observable exit codes/output and
   controlled temporary inputs. Do not test prose by grepping exact text.

### Task 5: Whole-branch integration and local launch

1. Run focused tests after each task, then repository formatting and the full
   relevant unit/CI gates under Hermit.
2. Build release binaries for `buzz-relay`, `buzz-admin`, `buzz-cli`, `buzz-acp`,
   and `buzz-agent` in WSL.
3. Download the current official Buzz Windows installer from the repository's
   GitHub release, verify it against release metadata, and scan it using the
   available Windows malware scanner without changing security settings.
4. Install/launch Buzz Desktop, start the local infrastructure, create `Core Lab`
   and `core-research`, register stable banker and agent identities, and launch
   the Core Research Partner when credentials are available locally.
5. Execute a synthetic smoke test proving: banker message enters the configured
   channel; the agent either stays silent or produces one signed cited reply;
   no second channel or non-owner can trigger it; restart preserves channel
   history; and no external send/tool path exists.
6. If no OpenAI credential is available locally, complete every deterministic
   step and leave the stack stopped at a clearly reported credential gate. Never
   request that a secret be pasted into chat.

## Completion Evidence

- Per-task red/green test evidence and task review.
- Whole-branch review with no unaddressed critical or important findings.
- Fresh formatting, lint, unit-test, and release-build outputs.
- Local readiness and synthetic smoke-test evidence, or an explicit credential
  gate after all deterministic setup is complete.
