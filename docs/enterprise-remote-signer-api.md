# Enterprise remote signer API (draft)

This public contract intentionally omits Block-internal service names and storage details.

Enterprise Buzz builds MAY opt into a corporate-authoritative signer by setting `VITE_ENTERPRISE_SIGNER_BASE_URL` (web) or the platform-equivalent managed configuration. OSS/self-custody builds leave it unset and continue to use local NIP-07/local keys.

All requests use HTTPS, include the corporate Auth0 access/session credential accepted by the enterprise backend, and fail closed on missing, expired, or disabled corporate access. The server chooses the signer key from the authenticated stable corporate account (`issuer` + `subject`, or the existing documented stable internal account id derived from it). Clients MUST NOT send a pubkey/nsec/key selector.

## POST /v1/buzz/enterprise-signer/session

Returns signer configuration for the authenticated account and provisions/repairs relay community membership durably.

Request: `{}`

Response:

```json
{
  "pubkeyHex": "32-byte lowercase hex",
  "relayWsUrl": "wss://community.example",
  "relayHttpUrl": "https://community.example",
  "communityId": "optional durable community id",
  "membershipState": "active|pending",
  "retryAfterMs": 1000
}
```

`pending` means the server has accepted the login and queued durable provisioning, but the client should not assume relay admission yet.

## POST /v1/buzz/enterprise-signer/events/sign

Signs one Nostr event template as the authenticated corporate account. Used for read path NIP-42 challenges and direct client publishes.

Request:

```json
{
  "event": { "kind": 22242, "created_at": 0, "tags": [], "content": "" },
  "purpose": "nip42-auth|publish|http-auth|media-upload"
}
```

Response:

```json
{ "event": { "id": "...", "pubkey": "server account pubkey", "sig": "...", "kind": 22242, "created_at": 0, "tags": [], "content": "" } }
```

Server requirements:
- derive `pubkey` server-side; reject any supplied `pubkey`, `id`, or `sig`
- validate `purpose`, kind, timestamps, tag cardinality, relay URL/challenge binding for NIP-42, media hash/host binding for uploads, and request size/time bounds
- do not log tokens, private keys, signatures containing bearer material, or event content beyond explicitly safe metadata

## POST /v1/buzz/enterprise-signer/events/publish

Durably signs and publishes a template, or replays the identical signed event/ack for the same idempotency key. This endpoint is preferred where the client cannot safely preserve retry identity.

Request:

```json
{
  "idempotencyKey": "client-stable retry key",
  "event": { "kind": 1, "created_at": 0, "tags": [], "content": "..." }
}
```

Response:

```json
{
  "event": { "id": "...", "pubkey": "server account pubkey", "sig": "..." },
  "relayAck": { "accepted": true, "message": "" },
  "state": "published|pending|indeterminate"
}
```

## POST /v1/buzz/enterprise-signer/media/read-credential

Returns short-lived, host-scoped read credentials for Buzz media. Clients cache only until `expiresAt` and only for the returned host.

Request: `{ "host": "media/community host" }`

Response: `{ "authorization": "Bearer ...", "expiresAt": "2026-09-11T00:00:00Z" }`

## Offboarding

Disabling the corporate account must disable signer access and remove/disable community membership. Active WebSocket disconnect is intentionally outside this PR and depends on the separate active-connection revocation branch. Token revocation is bounded by the backend's Auth0/session validation and any already-issued short-lived media credential expiry.

## Known unsupported operations in this proof

NIP-46 is optional and not required. End-to-end encrypted DM/NIP-44 operations that require client-side private-key access remain local/self-custody only until the product defines a server-side encryption authority model; enterprise clients must not silently export nsecs or claim encrypted-DM support through this signer.
