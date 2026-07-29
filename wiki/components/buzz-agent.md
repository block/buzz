# buzz-agent

A minimal ACP-compliant agent binary. Can be used as a reference implementation or as a lightweight agent for simple automation tasks.

**Features:**
- ACP-compliant stdio JSON-RPC communication
- Basic message send/receive
- Reaction handling
- LLM provider support (Anthropic, OpenAI, local OpenAI-compatible endpoints like OMLX/Ollama)
- Reasoning content fallback (`reasoning_content` / `reasoning` parsing for local models)
- Designed to be swapped out for more capable agents (Goost, Codex, Claude Code)

**LLM Response Parsing:**
- In `crates/buzz-agent/src/llm.rs`, responses from OpenAI-compatible endpoints are parsed by `parse_openai` and `parse_responses`.
- When local reasoning models output generated text in `reasoning_content` while leaving `content: ""`, `buzz-agent` falls back to `reasoning` to populate turn text output.

**Related:**
- [ACP](../concepts/acp)
- [buzz-acp](buzz-acp)
- [Agent](../entities/agent)
