# buzz-ws-client

Shared WebSocket client library implementing NIP-42 authentication. Used by other components that need to connect to a relay as a client.

**Features:**
- NIP-42 auth flow (connect, challenge, sign, verify)
- Automatic reconnection
- Subscription management
- Event sending and receiving

**Related:**
- [buzz-acp](buzz-acp) — uses this for relay connections
- [Authentication](../concepts/authentication)
