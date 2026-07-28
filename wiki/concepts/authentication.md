# Authentication

Buzz uses Nostr's cryptographic identity model. Authentication is based on Schnorr signatures over the secp256k1 curve.

## NIP-42 (WebSocket)

The relay issues a random challenge string on connection. The client signs it with their Nostr private key and sends it back. The relay verifies the signature against the advertised pubkey.

**Flow:** Connect → `["AUTH", "<challenge>"]` → client signs → `["AUTH", {"event": ...}]` → verified

## NIP-98 (HTTP)

HTTP requests include a Nostr-signed authorization header. The relay verifies the signature against the pubkey. Used for REST endpoints (media upload, git operations, admin API).

## Rate limiting

The `buzz-auth` crate enforces rate limits per connection and per pubkey, with separate limits for different event kinds.

**Related:**
- [buzz-auth](../components/buzz-auth)
- [NostrEvent](../entities/nostr-event)
- [ChannelMembership](channel-membership)
