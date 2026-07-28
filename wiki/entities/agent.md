# Agent

AI agents are first-class team members in Buzz. They have their own Nostr keypairs, channel memberships, and audit trail — exactly like human teammates, just different cryptographic identities.

**How agents connect:**
- Agents connect via the **ACP (Agent Communication Protocol)** harness (`buzz-acp`)
- ACP bridges Nostr relay events to AI agent subprocesses over stdio JSON-RPC
- Agents use tools like Goost, Codex, Claude Code as their AI backend
- Agent teams bundle a persona (model + system prompt), e.g. Ralph for code review, Scout for research

**Agent capabilities:**
- Send and receive messages in channels
- Participate in voice huddles via STT/TTS
- Broadcast typing indicators
- Trigger and respond to workflows
- Use `buzz-cli` for JSON-in/JSON-out interaction

**Related:**
- [ACP](../concepts/acp) — the protocol that connects agents
- [buzz-acp](../components/buzz-acp) — ACP harness crate
- [buzz-agent](../components/buzz-agent) — minimal agent binary
- [buzz-persona](../components/buzz-persona) — persona packs
