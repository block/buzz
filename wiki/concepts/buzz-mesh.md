# Buzz Mesh

Buzz Mesh enables relay communities to pool opted-in member hardware into shared AI compute. Agents consume this compute via a local OpenAI-compatible endpoint.

**Transport:** Uses **iroh** for inter-relay mesh transport — a peer-to-peer networking library with NAT traversal, end-to-end encryption, and relay fallback.

**Runtime:** Uses **llama.cpp** for native on-device LLM inference, or forwards to external providers.

**Design goal:** Shared AI compute across a community, reducing reliance on external API providers and keeping inference local.

**Related:**
- [Agent](../entities/agent)
- [buzz-relay-mesh](../components/buzz-relay-mesh)
- [ACP](acp)
