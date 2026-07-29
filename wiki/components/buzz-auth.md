# buzz-auth

Authentication, authorization, rate limiting, and scope enforcement.

**Key responsibilities:**
- NIP-42 WebSocket challenge/response auth
- NIP-98 HTTP request auth
- Rate limiting per connection and per pubkey
- SSRF protection (`is_private_ip()` checking IPv4/IPv6 private/loopback/link-local ranges)
- Scope enforcement for admin operations

**Related:**
- [Authentication](../concepts/authentication)
- [buzz-relay](buzz-relay) — integrates auth into connection lifecycle
- [ChannelMembership](../concepts/channel-membership)
