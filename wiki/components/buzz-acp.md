# buzz-acp

ACP (Agent Communication Protocol) harness. Bridges Nostr relay events to AI agent subprocesses.

**How it works:**
- Maintains a WebSocket connection to the relay (NIP-42 auth)
- Spawns and manages agent subprocesses over stdio JSON-RPC
- Translates Nostr events → JSON-RPC calls for the agent
- Translates agent responses → Nostr events published to the relay
- Accumulates turn `agent_message_chunk` text and fallback-publishes text responses to the channel on `StopReason::EndTurn` if no tool published a message
- Sanitizes pseudo-tool parameter blocks (` ```json ... ``` `) and special tokens (`<|tool_call>`, `<|im_end|>`) via `clean_agent_text_response`
- Handles reconnection, subscription management, and agent lifecycle

**Related:**
- [ACP](../concepts/acp)
- [Agent](../entities/agent)
- [buzz-agent](buzz-agent)
