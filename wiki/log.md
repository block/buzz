# Log

Chronological record of wiki changes. Each entry uses the format:
`## [YYYY-MM-DD] type | Title`

- **ingest** — a new source was read and integrated
- **query** — a question was answered and the result was filed
- **lint** — a health check was performed
- **update** — pages were revised or created outside of ingest/query

---

## [2026-07-27] ingest | Initial Wiki Creation

Created the initial wiki for the Buzz project based on project source code and documentation.

**Pages created:**
- `CLAUDE.md` — schema, conventions, and workflows
- `index.md` — catalog of all pages
- `log.md` — this file
- `entities/relay.md`, `entities/community.md`, `entities/agent.md`, `entities/channel.md`, `entities/nostr-event.md`
- `concepts/architecture.md`, `concepts/event-pipeline.md`, `concepts/authentication.md`, `concepts/channel-membership.md`, `concepts/workflow-engine.md`, `concepts/audit-log.md`, `concepts/buzz-mesh.md`, `concepts/acp.md`, `concepts/git-integration.md`, `concepts/media-storage.md`, `concepts/search.md`, `concepts/multi-tenancy.md`, `concepts/nostr-protocol.md`
- `components/buzz-relay.md`, `components/buzz-core.md`, `components/buzz-db.md`, `components/buzz-auth.md`, `components/buzz-pubsub.md`, `components/buzz-search.md`, `components/buzz-audit.md`, `components/buzz-media.md`, `components/buzz-workflow.md`, `components/buzz-acp.md`, `components/buzz-agent.md`, `components/buzz-dev-mcp.md`, `components/buzz-persona.md`, `components/buzz-cli.md`, `components/buzz-sdk.md`, `components/buzz-admin.md`, `components/buzz-ws-client.md`, `components/buzz-test-client.md`, `components/buzz-conformance.md`, `components/buzz-push-gateway.md`, `components/buzz-relay-mesh.md`, `components/git-sign-nostr.md`, `components/git-credential-nostr.md`, `components/sprig.md`, `components/desktop-client.md`, `components/mobile-client.md`, `components/web-client.md`, `components/admin-web.md`
- `operations/development-setup.md`, `operations/deployment.md`, `operations/configuration.md`, `operations/cli-reference.md`

## [2026-07-27] update | Added Troubleshooting Page

Created `operations/troubleshooting.md` covering common issues: 521 errors when connecting to cloud relays, port conflicts, rate limiting, Docker health, and event rejection. Updated `index.md` to link the new page.

## [2026-07-27] query | Agent Shows Response Bubble But Doesn't Publish

Investigated agent response not publishing. Two root causes found:
1. Agent was connected to `wss://milimo.communities.buzz.xyz` instead of `ws://localhost:3000` — hardcoded in `managed-agents.json`. Responses went to the wrong relay.
2. Even on the correct relay, the LLM (Qwen3.6-35B-A3B-bf16 via OMLX) returns text via ACP `agent_message_chunk` but doesn't call the `shell` tool to run `buzz messages send`. The harness does not auto-publish agent output.

Updated `operations/troubleshooting.md` with both causes and fixes.

## [2026-07-28] update | Root Causes Resolved — Wiki Updated

Applied fixes from investigation:
- **Local LLM reasoning content**: `buzz-agent` now handles `reasoning_content` field from OpenAI-compatible endpoints (local models), falling back to `content` when `reasoning_content` is absent.
- **Fallback turn text publishing**: `buzz-acp` now accumulates `agent_message_chunk` text during a turn and publishes a `kind: 9` channel message on `StopReason::EndTurn` if no `send_message` tool call was made. Enables non-tool-calling models to participate.
- **Response sanitization**: `clean_agent_text_response` strips pseudo-tool JSON blocks (`` ```json ... ``` ``) and model tokens (`<|tool_call|>`, `<|im_end|>`) from generated text.
- **Owner pubkey seeding**: `agent_owner_pubkey` seeded in Postgres `users` table across all local dev community hosts (`localhost:3000`, `127.0.0.1:3000`, `localhost`, `127.0.0.1`).
- **Agent config schema**: Fixed `managed-agents.json` parsing for required field `acp_command`; cleaned up `.invalid` backup files.

Updated `operations/troubleshooting.md`, `components/buzz-agent.md`, `components/buzz-acp.md`.

## [2026-07-28] update | Response Cleaning & Duplicate Agent Cards

- Documented `clean_agent_text_response` stripping Python pseudo-code (`def reply_to_mention(...)`), CLI stdout objects, step-execution noise, and model tokens.
- Added "Duplicate Agent Cards" troubleshooting entry: unlinked managed agents (`"persona_id": null`) render separate cards alongside builtin personas; fix by mapping `"persona_id": "builtin:<name>"`.
- Updated `buzz-acp.md` component docs. Pushed to `mainza-ai/buzz:main`.

## [2026-07-28] update | Multi-tenant Loopback Host Normalization & NIP-29 Channel Discovery

- **Loopback Host Normalization (`buzz-core` & `buzz-auth`)**: `normalize_host` now folds loopback host variants (`localhost`, `127.0.0.1`, `[::1]`) to `127.0.0.1`, ensuring HTTP/WebSocket requests and NIP-98 auth signers resolve to the exact same community space. Resolves NIP-98 `401 Unauthorized` URL mismatch errors between `http://localhost:3000` and `http://127.0.0.1:3000`.
- **NIP-29 Channel Discovery (`buzz-acp`)**: `extract_channel_uuid_from_event` now inspects both `d` tags (NIP-29 group members `kind: 39002`) and `h` tags. Enables agents to dynamically subscribe to channels in real time as soon as they are added.
- Updated `operations/troubleshooting.md`.

