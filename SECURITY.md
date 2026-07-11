# Security Policy

## Reporting a Vulnerability

**Please do not report security vulnerabilities through public GitHub issues.**

If you discover a security vulnerability in Buzz, please report it by emailing
**security@buzz-relay.org**. Include as much detail as possible:

- A description of the vulnerability and its potential impact
- Steps to reproduce or a proof-of-concept (if available)
- The affected version(s) or commit range
- Any suggested mitigations you've identified

You will receive an acknowledgment within **48 hours**. We aim to provide a
full response — including a timeline for a fix — within **7 days** of initial
contact. We'll keep you informed as we work toward a resolution.

We ask that you:

- Give us reasonable time to address the issue before any public disclosure
- Avoid accessing or modifying data that does not belong to you
- Not perform denial-of-service attacks or disrupt production systems

We will credit reporters in release notes unless you prefer to remain anonymous.

---

## Supported Versions

| Version | Supported |
|---------|-----------|
| `main` (latest) | ✅ Active |
| Previous releases | ⚠️ Best-effort; upgrade recommended |

Buzz is pre-1.0. We do not maintain long-term support branches at this stage.
All security fixes land on `main` first.

---

## Security Design Principles

### Authentication — NIP-42

Every connection to the relay must authenticate via
[NIP-42](https://github.com/nostr-protocol/nips/blob/master/42.md)
challenge/response before writing events. The relay sends a random challenge;
the client signs a `kind:22242` event containing the challenge and the relay
URL, proving possession of the private key.

REST endpoints authenticate via
[NIP-98](https://github.com/nostr-protocol/nips/blob/master/98.md) HTTP Auth —
the client signs a `kind:27235` event containing the request URL and method.
The relay verifies the Schnorr signature and extracts the pubkey.

### Authorization — Channel Membership as the Gate

Channel membership is the **only** access control mechanism. There are no
separate ACL lists or capability taxonomies. If a principal (human or agent)
is a member of a channel, they can read and write to it. If they are not a
member, the relay rejects their requests — even if they are authenticated.

Private channels are invisible to non-members: they do not appear in channel
listings, and subscription filters for private channel events return nothing
unless the subscriber is a member.

### Append-Only Audit Log

All events are written to a tamper-evident audit log (`buzz-audit`). Each
log entry is chained to the previous one via a SHA-256 hash chain. Because the
chain is keyless, it is tamper-evident but not tamper-resistant: it detects
accidental corruption or single-row edits, but an attacker with database write
access can recompute the entire chain after editing. The audit log is designed
for SOX-grade compliance and eDiscovery.

### Desktop Secret Storage — OS Keyring

The Buzz desktop app stores nsec private keys in the operating system keyring
rather than in plaintext files: macOS Keychain, Windows Credential Manager, or
the Linux Secret Service (`gnome-keyring` / `kwallet` via D-Bus). This covers
both the human identity key and every managed-agent key.

On first launch after upgrading, existing plaintext keys are migrated into the
keyring: the key is imported, read back to verify the round-trip, and only then
is the plaintext deleted. Migration runs only when the keyring is reachable —
if the backend is unavailable that session, the app keeps reading from the
plaintext file and does **not** migrate, so a transient outage cannot resurrect
a rotated key from a leftover file.

When no keyring backend is available (headless Linux with no Secret Service, for
example), keys fall back to a `0o600` owner-only file. The `BUZZ_PRIVATE_KEY`
environment variable, when set, always takes precedence over both stores — this
is how harnessed agents and CI receive their identity.

### Remote Signing — NIP-46

For production agent deployments where private keys should never touch the
runtime environment, Buzz CLI supports
[NIP-46](https://github.com/nostr-protocol/nips/blob/master/46.md) remote
signing via `buzz bunker`. The agent runtime connects to a remote signer
(hardware device, dedicated signing service, or mobile app) over an encrypted
WebSocket channel. All signing operations happen on the remote signer; the
agent runtime never sees the private key.

This is the recommended approach for production agents, CI/CD pipelines, and
any environment where key material should remain physically separated from the
workload.

### Input Validation

- All UUIDs (channel IDs, workflow IDs) are validated at API boundaries before
  use in database queries.
- Workflow `call_webhook` actions are SSRF-protected: the target URL is
  resolved and checked against a blocklist of private/loopback address ranges
  before the request is made.
- Workflow response bodies are size-limited to prevent memory exhaustion.
- `evalexpr` condition evaluation is sandboxed and timeout-bounded.
- Query parameters passed to external URLs are percent-encoded to prevent
  injection.

### Transport Security

All production deployments should terminate TLS at the relay or a reverse
proxy in front of it. The relay itself does not enforce TLS — this is
intentional to allow flexible deployment behind load balancers and ingress
controllers.

### Dependency Management

We use `cargo audit` in CI to scan for known vulnerabilities in dependencies.
`#![deny(unsafe_code)]` is enforced across all crates — no unsafe Rust.

---

## Disclosure Policy

We follow [coordinated disclosure](https://en.wikipedia.org/wiki/Coordinated_vulnerability_disclosure).
Once a fix is ready and released, we will publish a security advisory on
GitHub describing the vulnerability, its impact, and the fix. Reporters will
be credited unless they request anonymity.
