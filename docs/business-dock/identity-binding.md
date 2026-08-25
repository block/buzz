# Buzz identity and device binding

An authenticated Enterprise user proves possession of the current Buzz private
key without uploading it. One user may own several devices; an active Buzz
pubkey may belong to only one Enterprise user.

```mermaid
sequenceDiagram
  participant D as Desktop
  participant G as Gateway
  participant P as PostgreSQL
  D->>G: challenge(pubkey, device metadata) + Bearer
  G->>P: store hash, user, pubkey, device, audience, expiry
  G-->>D: canonical LF-delimited payload
  D->>D: sign existing kind 24243 with current Buzz key
  D->>G: verify(challengeId, signed event)
  G->>P: SELECT FOR UPDATE; verify; consume; create binding
  G-->>D: active binding
```

The signed protocol uses fixed version, field order and LF separators: challenge
ID, nonce, audience, issuer, subject, pubkey, device ID, issue time, and expiry.
Raw JSON is not signed. Challenges live 60–120 seconds and are single-use. The
server verifies Nostr event ID and Schnorr signature using the existing library.

Revocation atomically revokes the binding and its active Embed and Business
sessions. Workbench reports `device_revoked`; it must not silently open Business
Dock. Buzz private keys never leave Tauri.
