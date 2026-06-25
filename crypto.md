# Buzz Cryptography And Key Review

Reviewed against the current checkout on 2026-06-25.

## Executive Summary

Buzz uses familiar primitives: BIP-340 Schnorr signatures over secp256k1 for
Nostr events, SHA-256 for event IDs and payload hashes, HMAC-SHA256 and
HKDF-SHA256 for keyed derivations, NIP-44 v2 for encrypted Nostr payloads, and
random bearer tokens for a smaller set of HTTP and workflow flows. I did not
find evidence that the project chose a broken primitive or implemented an
obviously broken signature scheme.

The problems are mostly at the composition and lifecycle layer. The highest
risk issue is that the mobile NIP-AB pairing target decrypts, deserializes, and
retains payload plaintext before the target user confirms the SAS, even though
the protocol explicitly makes that confirmation a local confidentiality
boundary. The second high-risk issue is that relay admission verifies NIP-OA
delegation signatures but does not enforce signed time conditions, while the
desktop currently mints managed-agent credentials with no expiry at all. That
turns a delegation credential into a durable bearer capability once the key and
tag are copied.

The remaining issues are narrower but real: Git Smart HTTP intentionally turns
NIP-98 into a short-lived repo-scoped bearer credential; generic NIP-98 body
binding is optional even on JSON POST routes; the managed-agent key store does
not enforce owner-only file permissions the way the human desktop key store
does; mobile pairing cleanup leaves ephemeral secret material referenced after
completion; the audit documentation claims HMAC-backed tamper evidence while
the implementation is only an unkeyed hash chain; and the mobile hand-written
NIP-44 parser accepts non-canonical padding.

This report does not count intended local agent authority as a cryptographic
finding. If a user deliberately runs an agent with shell and file access on
their machine, that agent is inside that machine's trust boundary. The crypto
question is whether keys and credentials have the properties the product and
protocols claim once they cross a boundary.

## Severity Model

- Critical: direct key recovery, signature forgery, or broad impersonation
  across an expected trust boundary with little precondition.
- High: an explicit cryptographic protocol guarantee is broken, or a credential
  intended to be bounded becomes materially more durable or replayable.
- Medium: a real composition, replay, or key-custody weakness that lowers a
  claimed guarantee but needs a narrower precondition.
- Low: parser strictness, defense-in-depth, or test coverage gaps that are worth
  fixing before they become interoperability or maintenance problems.

No Critical findings were identified in the reviewed code.

## Findings

### High

#### C-01. Mobile pairing deserializes and retains payload plaintext before dual consent

**Severity:** High

NIP-AB makes the target user's SAS confirmation a local confidentiality
boundary. The protocol allows the target to decrypt early only as much as is
necessary to classify a message type. It explicitly says the target must not
deserialize, extract, log, persist, or act on the `payload` field before the
transcript is verified and the target user approves the SAS; the safest behavior
is to buffer raw ciphertext until then. See
`crates/buzz-core/src/pairing/NIP-AB.md:316-318` and
`crates/buzz-core/src/pairing/NIP-AB.md:645-649`.

The mobile implementation does more than that. It decrypts and `jsonDecode`s
incoming pairing messages immediately in
`mobile/lib/features/pairing/pairing_provider.dart:276-333`. When the message
type is `payload`, it stores the full plaintext `Map<String, dynamic>` in
`_pendingPayload` before the user confirms the SAS in
`mobile/lib/features/pairing/pairing_provider.dart:380-389`. The actual `nsec`
is extracted later in `mobile/lib/features/pairing/pairing_provider.dart:416-456`,
but the sensitive plaintext is already resident in application memory and
available to any bug, instrumentation, crash capture, or future code path that
touches `_pendingPayload`.

The Rust reference state machine preserves the intended property. It moves the
target into `AwaitingConfirmation`, requires `confirm_target_sas()` before
`handle_payload()`, returns the secret in a `Zeroizing<String>`, and zeroizes
session material on drop. See `crates/buzz-core/src/pairing/session.rs:329-415`
and `crates/buzz-core/src/pairing/session.rs:740-750`.

This is not a theoretical nicety. The protocol is designed so that a target
device can receive an encrypted payload before the human has accepted the
channel, while still withholding the secret from the local application until
that final approval. The mobile code removes that distinction.

Recommended change: keep the incoming payload event or ciphertext opaque until
`confirmSas()` succeeds, then decrypt and deserialize exactly once. If early
message classification is unavoidable, parse only enough to distinguish
`payload` from other message types and do not retain any decoded payload fields.
Use mutable byte buffers for secret plaintext where practical so cleanup can do
more than drop references.

#### C-02. Relay admission drops signed NIP-OA time bounds and managed agents mint non-expiring delegation credentials

**Severity:** High

NIP-OA auth tags are signed delegation credentials. The SDK verifies the tag
signature and returns the owner pubkey, but it only syntax-checks conditions; it
does not evaluate them. See `crates/buzz-sdk/src/nip_oa.rs:33-109` and
`crates/buzz-sdk/src/nip_oa.rs:139-239`. The NIP-OA documentation is clear that
empty conditions impose no constraints and that verifiers must evaluate every
condition. See `docs/nips/NIP-OA.md:13-17`, `docs/nips/NIP-OA.md:43-71`, and
`docs/nips/NIP-OA.md:93-103`.

The relay admission path treats a valid auth tag signature plus owner
membership as sufficient delegated membership. It does not evaluate
`created_at<` or `created_at>` conditions when resolving the effective member
for NIP-42 authentication. See `crates/buzz-relay/src/api/mod.rs:54-158` and
`crates/buzz-relay/src/handlers/auth.rs:68-223`. NIP-AA requires those time
conditions to be evaluated during AUTH admission, in addition to the normal
NIP-42 freshness check. See `docs/nips/NIP-AA.md:71-125` and
`docs/nips/NIP-AA.md:143-167`.

There is one path that does enforce time conditions: the NIP-IA request-borne
identity archive path in `crates/buzz-relay/src/handlers/identity_archive.rs:245-347`.
Git signing also evaluates NIP-OA conditions against its signed envelope
timestamp in `crates/git-sign-nostr/src/lib.rs:600-629`,
`crates/git-sign-nostr/src/lib.rs:1034-1068`, and
`crates/git-sign-nostr/src/lib.rs:1264-1356`. That makes the relay admission
gap look like an omitted semantic check, not a deliberate protocol-wide rule.

The current desktop issuance path makes the problem worse by generating managed
agent auth tags with an empty condition string in
`desktop/src-tauri/src/commands/agents.rs:407-420`. Deleting the local agent
record only stops the process and removes local state; it does not revoke a
credential that has already been copied elsewhere. See
`desktop/src-tauri/src/commands/agents.rs:1045-1097`.
`crates/buzz-db/src/archived_identities.rs:1-6` also states that archive state
is a relay-local UI visibility hint, not an authorization revocation mechanism.

The result is that a copied managed-agent private key plus auth tag remains
usable as long as the owner remains a relay member, even if a future caller
believes it issued a time-bounded tag. `kind=` conditions are intentionally not
admission restrictions under NIP-AA, so the missing property is specifically
time-bound admission and credential retirement, not generic event-kind
restriction.

Recommended change: add a single NIP-AA admission verifier that accepts the
AUTH event timestamp and evaluates all admission-relevant NIP-OA conditions
after NIP-42 freshness verification. Make every relay admission caller use it.
Issue managed-agent credentials with an explicit short lifetime or add an
independent relay-side revocation/rotation mechanism keyed by the delegated
agent identity. Deleting or retiring an agent should revoke future admission,
not just remove the desktop process record.

### Medium

#### C-03. Git Smart HTTP uses NIP-98 as a replayable repo-scoped bearer credential

**Severity:** Medium

The Git Smart HTTP path intentionally relaxes NIP-98 request binding so that one
credential works across the multiple HTTP requests Git emits for a repository.
The relay strips a request down to the repo root URL, does not bind the
credential to the actual HTTP method, passes `body=None`, and intentionally
does not deduplicate event IDs. See
`crates/buzz-relay/src/api/git/transport.rs:111-175`. The credential helper
matches that behavior by minting a repo-root auth event in
`crates/git-credential-nostr/src/lib.rs:182-231`.

This means a captured credential is a short-lived bearer token for the same
repository, not a proof for one exact request. Within the verifier freshness
window, the holder can replay it against clone, fetch, or push endpoints for
that repository, subject to the original account's repo authorization and the
pre-receive push policy. The code comments acknowledge the need for this
tradeoff, but the actual scope is broader than an operation-specific signature.

This is not a broken signature scheme. It is a deliberate reduction from
"signed request" to "signed short-lived repo session". That can be a reasonable
engineering choice, but it should be treated and documented as a bearer
credential because its compromise behavior is different.

Recommended change: either document the Git token as an explicit short-lived
repo-scoped bearer credential, or add a protocol layer that binds the
credential to a smaller session scope and operation set. If the current design
remains, avoid describing it as method or service bound when the verifier
actually accepts the repo-root token across multiple Git endpoints.

#### C-04. Bridge NIP-98 body integrity is optional on body-carrying JSON routes

**Severity:** Medium

The generic NIP-98 verifier checks a SHA-256 payload tag only when both a
`payload` tag and a request body are present. A body-bearing request with no
`payload` tag is accepted. See `crates/buzz-auth/src/nip98.rs:34-130`; the test
that codifies this behavior is `crates/buzz-auth/src/nip98.rs:267-273`.

The HTTP bridge routes for `/events`, `/query`, and `/count` all pass request
bodies through this verifier and use a replay cache. See
`crates/buzz-relay/src/api/bridge.rs:23-105`,
`crates/buzz-relay/src/api/bridge.rs:167-234`,
`crates/buzz-relay/src/api/bridge.rs:241-325`, and
`crates/buzz-relay/src/api/bridge.rs:497-615`. First-party desktop, ACP, and
CLI callers do the stronger thing and include both a payload hash and a nonce.
See `desktop/src-tauri/src/relay.rs:94-136`,
`crates/buzz-acp/src/relay.rs:202-248`,
`crates/buzz-acp/src/relay.rs:311-336`, and
`crates/buzz-cli/src/client.rs:85-117`.

The replay cache limits repeated use, but it cannot make the first accepted body
be the body the signer intended if the auth event itself does not commit to a
payload hash. A captured no-payload auth event can be paired with a different
body before the intended request wins the race. The Git path needs weaker body
binding for streaming protocol reasons; JSON bridge routes do not.

Recommended change: split NIP-98 verification into explicit modes. Require a
`payload` tag for non-streaming body-carrying routes such as `/events`,
`/query`, and `/count`, and keep the Git exception local to the Git transport
code rather than making optional body integrity the generic verifier default.

#### C-05. Managed-agent key store does not enforce owner-only file permissions

**Severity:** Medium

Managed agent records contain the raw agent `nsec` and auth tag in plaintext.
See `desktop/src-tauri/src/managed_agents/types.rs:85-99` and
`desktop/src-tauri/src/commands/agents.rs:377-385`,
`desktop/src-tauri/src/commands/agents.rs:516-522`. The store writes JSON with
`std::fs::write` followed by rename in
`desktop/src-tauri/src/managed_agents/storage.rs:11-70`; it does not explicitly
set or validate owner-only permissions.

The desktop human identity store is stricter. It uses an atomic write path and
sets Unix mode `0600` for the key file in
`desktop/src-tauri/src/app_state.rs:127-234`. The managed-agent store therefore
has a weaker custody guarantee for credentials that are still capable of relay
admission and event signing.

Actual exposure depends on the app data directory mode and the user's umask, so
this is not a claim that every install is immediately world-readable. The
problem is that the code does not enforce the property at the file that matters.

Recommended change: give the managed-agent store the same atomic owner-only
write and load-time permission validation used for the desktop human key. For
longer-lived deployments, consider moving both human and agent private keys
into an OS key store rather than relying only on plaintext files protected by
filesystem ACLs.

#### C-06. Mobile pairing cleanup leaves ephemeral secret material live after completion and abort

**Severity:** Medium

The NIP-AB specification requires ephemeral private keys, the session secret,
and decrypted plaintext to be zeroed after completion, abort, or timeout. See
`crates/buzz-core/src/pairing/NIP-AB.md:389` and
`crates/buzz-core/src/pairing/NIP-AB.md:555-562`. The Rust implementation does
that for its session and QR objects in `crates/buzz-core/src/pairing/session.rs:740-750`
and `crates/buzz-core/src/pairing/qr.rs:34-57`.

The mobile provider's `_cleanup()` only cancels timers and subscriptions,
clears status flags, and drops `_pendingPayload`. It leaves
`_ephemeralPrivkey`, `_ephemeralPubkey`, `_sessionSecret`, `_sourcePubkey`,
`_sessionId`, `_sasInput`, and `_conversationKey` referenced on the notifier.
See `mobile/lib/features/pairing/pairing_provider.dart:119-143`.

Dart strings and ordinary immutable objects are not ideal secret containers, so
true zeroization may require a different representation. Even then, the current
code can at least shorten lifetime by clearing references and avoiding plaintext
objects that survive past the session state machine.

Recommended change: clear every session secret field on all terminal paths and
move secret-bearing values toward mutable byte buffers or platform-backed
containers that can be overwritten before release. Pair this with tests that
assert terminal transitions drop all retained secret state.

#### C-07. Audit documentation claims HMAC-backed tamper evidence but implementation is an unkeyed hash chain

**Severity:** Medium

`SECURITY.md:67-72` describes the audit log as HMAC chained and suitable for
strong tamper evidence. The implementation computes a plain SHA-256 digest over
the entry fields plus the previous hash in `crates/buzz-audit/src/hash.rs:1-77`.
`crates/buzz-audit/src/service.rs:19-193` serializes writes and verifies the
stored chain, but there is no secret key and no external anchor.

An unkeyed hash chain detects accidental corruption and some incomplete edits.
It does not stop an attacker with write access to the audit table from
rewriting a suffix, recomputing every following hash, and leaving a
self-consistent chain. A keyed MAC would raise the bar for a database-only
attacker if the MAC key lived outside the database. An external signed anchor
or append-only transparency system would be needed for stronger guarantees
against a compromised relay host.

Recommended change: either narrow the documentation to the actual property, or
implement a keyed chain with the key stored outside Postgres and anchor chain
heads externally. The threat model should say whether the intended adversary is
accidental corruption, database-only compromise, or relay-host compromise.

### Low

#### C-08. Mobile NIP-44 accepts non-canonical padding and lacks direct vector coverage

**Severity:** Low

The mobile client carries a hand-written NIP-44 v2 implementation in
`mobile/lib/shared/crypto/nip44.dart:12-176`. Its `_unpad()` routine checks that
the declared length is non-zero and fits within the decrypted buffer, but it
does not verify that the total padded length matches the canonical NIP-44
padding calculation or that trailing padding bytes are zero. See
`mobile/lib/shared/crypto/nip44.dart:127-134`.

Because the ciphertext is MAC authenticated, this is not a practical plaintext
forgery or key-recovery issue by itself. It is a parser strictness divergence:
the mobile client can accept authenticated plaintext encodings that stricter
implementations would reject. That creates avoidable cross-language behavior
differences in the code that carries identity-transfer secrets.

The current mobile tests exercise pairing behavior, but I did not find direct
official NIP-44 cross-language vector coverage for this implementation. Pairing
tests live in `mobile/test/features/pairing/pairing_provider_test.dart`; the
only direct NIP-44 use found in tests was
`mobile/test/features/channels/agent_activity/observer_subscription_test.dart`.

Recommended change: enforce canonical padding in `_unpad()` and add official
NIP-44 v2 vectors plus Rust-to-Dart interoperability cases for encryption,
decryption, padding boundaries, and malformed authenticated payloads.

## Primitive Inventory

| Primitive | Where It Is Used | Assessment |
| --- | --- | --- |
| BIP-340 Schnorr over secp256k1 | Nostr event signatures, relay-signed events, Git NIP-GS envelopes, pairing ephemeral events | Appropriate primitive. The main risk is private-key custody and semantic verification around the signed object, not signature forgery. |
| SHA-256 | Nostr event IDs, NIP-98 payload hashes, media content hashes, approval/API token hashing, audit chain, Git object and envelope hashing | Appropriate hash. Its use in the audit chain is not a substitute for a keyed MAC or external anchor. |
| HMAC-SHA256 | NIP-44 authentication, NIP-AB derivations via HKDF, Git hook authentication, engram opaque tags, proxy shadow-key derivation | Appropriate when the key is well protected and domain separation is explicit. The Git hook and engram uses are among the better composed parts of the codebase. |
| HKDF-SHA256 | NIP-44 and NIP-AB key derivation | Appropriate. Pairing derivations are domain separated and reviewed below. |
| ChaCha20 in NIP-44 v2 | Encrypted Nostr application payloads, pairing, reminders, observers, read state, engrams | Appropriate as part of the NIP-44 v2 construction. The significant question is when plaintext is exposed and what metadata remains public. |
| UUID v4 bearer values | Workflow webhooks, workflow approvals, proxy invites, some API-facing tokens | Roughly 122 random bits after UUID version and variant bits, which is adequate against guessing. These values are bearer secrets and need short lifetime, hashing where possible, and careful transport. |
| TLS / WSS / HTTPS | Transport confidentiality for normal messages, audio, Git, API tokens, and webhook secrets | External trust boundary, not application-layer cryptography. Ordinary channel messages and huddle audio rely on relay and transport confidentiality unless separately wrapped in NIP-44. |

Encoding mechanisms such as bech32, hex, base64, and JSON canonicalization are
not cryptographic protections by themselves. Their security relevance here is
whether they preserve an unambiguous signed or MACed representation.

## Key And Secret Inventory

| Material | Generation And Storage | Used For | Review Notes |
| --- | --- | --- | --- |
| Desktop human Nostr private key | Created and stored by `desktop/src-tauri/src/app_state.rs:127-234` in an atomic `0600` file | User event signing, NIP-42, NIP-98, NIP-44, pairing source payload | Plaintext at rest behind OS file permissions. Better hardened than the managed-agent store. |
| Mobile human Nostr private key | Stored inside workspace JSON through `FlutterSecureStorage` in `mobile/lib/shared/workspace/workspace_storage.dart:1-95`, `mobile/lib/shared/workspace/workspace.dart:6-71`, and `mobile/lib/shared/auth/auth_provider.dart:16-134` | User event signing, NIP-42, self-encryption, imported pairing identity | Better at-rest posture than a plain file, but runtime code necessarily handles plaintext key material. |
| Managed-agent private key | Freshly generated in `desktop/src-tauri/src/commands/agents.rs:377-385`, stored in agent JSON record at `desktop/src-tauri/src/managed_agents/types.rs:85-99`, injected into child env by `desktop/src-tauri/src/managed_agents/runtime.rs:1549-1561` | Agent Nostr identity, relay auth, event signing | Durable local credential. Store permissions are not explicitly hardened; copied key plus auth tag remains powerful. |
| NIP-OA auth tag | Signed by owner in `desktop/src-tauri/src/commands/agents.rs:407-420`; passed to agents in `desktop/src-tauri/src/managed_agents/runtime.rs:1648-1656` | Delegated owner context for relay membership and agent attribution | Signature is verified, but relay admission does not enforce time conditions. Current desktop issuance uses empty conditions. |
| Relay Nostr private key | Loaded or derived during startup in `crates/buzz-relay/src/main.rs:83-105` and `crates/buzz-relay/src/main.rs:204-225` | Relay identity, NIP-11 identity, relay-signed state and lifecycle events | Production membership mode needs a stable key. Development fallback is deterministic scalar-1, which is suitable only for local development. |
| NIP-44 conversation keys | Derived per sender/recipient pair by Rust `nostr` code and by mobile `mobile/lib/shared/crypto/nip44.dart:12-176` | Engrams, observer frames, reminders, read state, pairing payloads | Content confidentiality only. Tags, timing, sender/recipient relationships, and other public metadata remain visible. |
| NIP-AB ephemeral private keys and session secret | Generated for each pairing session in `crates/buzz-core/src/pairing/session.rs:84-109` and QR handling in `crates/buzz-core/src/pairing/qr.rs:34-57`; mobile keeps equivalents in `mobile/lib/features/pairing/pairing_provider.dart:132-143` | ECDH, SAS derivation, one-time identity transfer | Rust zeroizes on drop. Mobile currently retains references after cleanup and exposes payload plaintext too early. |
| Git hook HMAC secret | Loaded or generated by relay config in `crates/buzz-relay/src/config.rs:320-350` | Authenticate relay-to-hook push policy context | Canonical length-prefixed HMAC construction is sound. Configured minimum is 32 hex characters, which is 16 bytes and still adequate, though wording should be precise. |
| Workflow webhook secret | UUID v4 bearer generated in `crates/buzz-relay/src/webhook_secret.rs:22-90` | Invoke workflow webhook endpoint | Stored as a bearer secret in workflow state, stripped from responses, constant-time compared. Query-string transport in `crates/buzz-relay/src/api/bridge.rs:831-973` can leak through logs and referrers more easily than header transport. |
| Workflow approval token | UUID v4 raw token generated in `crates/buzz-workflow/src/executor.rs:714-721`, SHA-256 hash stored via `crates/buzz-db/src/workflow.rs:27-33` and `crates/buzz-db/src/workflow.rs:605-755` | Approve one pending workflow step | Good pattern: raw value is not stored and DB update is written to avoid approval races. |
| API token | Hashed and expiry/revocation tracked in `crates/buzz-db/src/api_token.rs:1-290` | Media/API bearer authentication | Better than storing raw bearer values. Actual security depends on issuance, transport, and revocation behavior at callers. |
| Proxy server salt and derived shadow keys | Server salt feeds `HMAC-SHA256(salt, external_pubkey)` in `crates/buzz-proxy/src/shadow_keys.rs:1-117` | Deterministic local shadow identities for NIP-28 compatibility | Salt compromise lets an attacker derive every shadow key for known external pubkeys. Salt rotation changes every shadow identity. |
| Infrastructure secrets | Read from relay config for Postgres, Redis, S3, Typesense, and similar services in `crates/buzz-relay/src/config.rs` | Service authentication | These are ordinary service credentials rather than Nostr keys, but relay compromise exposes them and therefore bypasses many higher-level crypto guarantees. |

## Flow Review

### Nostr Event Signatures And Relay Identity

The event signing core is conventional and easy to reason about. Nostr event IDs
are recomputed and signatures are verified in
`crates/buzz-core/src/verification.rs:8-31`. The relay ingest path rejects
events that do not verify, and ordinary WebSocket event submission also checks
that an event's `pubkey` matches the authenticated connection except for
explicit protocol cases such as gift wraps and proxy behavior. This is the
right base property: if an event is accepted as authored by a Nostr pubkey, the
relay has verified a Schnorr signature for that pubkey.

The relay key itself is more than an implementation detail. It signs
relay-authored state and lifecycle events and becomes the identity clients see
through NIP-11. Membership mode requires a stable relay key in
`crates/buzz-relay/src/main.rs:83-105`. Production token mode also refuses to
start without a configured key, while development mode falls back to a
deterministic scalar-1 key in `crates/buzz-relay/src/main.rs:204-225`.
`crates/buzz-relay/src/nip11.rs:106-171` only advertises a stable relay identity
and NIP-43 capability when the configuration supports it.

The configuration comment in `crates/buzz-relay/src/config.rs:49-51` says an
absent relay key generates a fresh key, but the runtime behavior is now
different: development gets a deterministic key and production panics. That is
not a crypto vulnerability, but stale comments around identity material are a
good way to create operational mistakes.

### NIP-42 WebSocket Authentication

The main relay NIP-42 implementation is one of the stronger auth surfaces. It
generates 32 random challenge bytes, requires an exact challenge match, binds
the AUTH event to the relay URL, verifies signature and kind, and enforces a
freshness window of about 60 seconds. See `crates/buzz-auth/src/nip42.rs:35-85`.
That gives a client proof of key possession and prevents a captured AUTH event
from remaining useful for long.

The proxy NIP-42 implementation is looser. It uses a UUID v4 challenge, accepts
a 10-minute freshness window, and treats relay-tag mismatch as nonfatal in
`crates/buzz-proxy/src/server.rs:173-313`. A UUID v4 challenge still has enough
entropy for guessing resistance, but the proxy has a weaker domain-binding and
replay posture than the primary relay. That may be acceptable for its
compatibility role, but it should be an explicit choice rather than an
accidental divergence.

### NIP-98 HTTP Authentication

The first-party desktop, ACP, and CLI clients generally create well-bound NIP-98
requests: they sign URL, method, a payload hash, and a nonce. See
`desktop/src-tauri/src/relay.rs:94-136`,
`crates/buzz-acp/src/relay.rs:202-248`,
`crates/buzz-acp/src/relay.rs:311-336`, and
`crates/buzz-cli/src/client.rs:85-117`. That is the right shape for JSON API
requests.

The generic verifier is intentionally more permissive because Git Smart HTTP
cannot conveniently sign each streamed body. That exception has leaked into
the default verifier contract: payload binding is optional even when a caller
does have a body. C-04 is the resulting problem. The web helper in
`web/src/shared/lib/nip98.ts:1-33` also omits a payload hash, but the reviewed
use is the Git client in `web/src/features/repos/git-client.ts`, where it fits
the weaker Git path rather than the JSON bridge.

### NIP-OA Delegation And Agent Identity

NIP-OA is a signed capability system layered on top of Nostr identity. The
owner signs an auth tag naming an agent pubkey and conditions. The agent then
proves possession of its own key while presenting the owner's delegation. The
cryptographic binding itself is straightforward: without the owner's
signature, an arbitrary agent cannot claim owner context.

The difficult part is semantic verification. `verify_auth_tag()` in the SDK
answers "did the owner sign this tag for this agent?" It does not answer "is
this credential still valid for this admission at this time?" Several call
sites use it only as an owner-binding primitive, including ACP identity
resolution in `crates/buzz-acp/src/lib.rs:86-112`,
`crates/buzz-acp/src/lib.rs:159-285`, and
`crates/buzz-acp/src/lib.rs:1102-1143`, plus desktop conversion and relay helper
logic in `desktop/src-tauri/src/nostr_convert.rs:60-83` and
`desktop/src-tauri/src/relay.rs:317-380`.

That is fine when the caller only needs attribution. It is not fine for relay
admission, where NIP-AA gives time conditions authorization meaning. The lack
of one centralized semantic verifier is what allowed C-02 to exist. The design
would be easier to audit if the code had separate names for "verify delegation
signature" and "authorize delegated action under conditions".

### NIP-44 Application Encryption

Buzz uses NIP-44 in several places, and the Rust paths are generally careful.
Engrams derive an opaque `d` tag using HMAC with domain separation, encrypt
content with a NIP-44 conversation key, enforce a strict envelope, and rederive
the expected tag after decryption. See `crates/buzz-core/src/engram.rs:1-154`
and `crates/buzz-core/src/engram.rs:436-599`. The relay validates engram
envelopes before allowing replaceable heads to be overwritten in
`crates/buzz-relay/src/handlers/ingest.rs:822-957` and
`crates/buzz-relay/src/handlers/ingest.rs:1474-1477`.

Observer frames use NIP-44 v2 with size bounds and plaintext zeroization in
`crates/buzz-core/src/observer.rs:1-108`. The relay routes only the public
envelope and excludes sensitive kinds from search in
`crates/buzz-relay/src/handlers/event.rs:188-200`,
`crates/buzz-relay/src/handlers/event.rs:314-329`, and
`crates/buzz-relay/src/handlers/event.rs:826-885`.

NIP-ER reminders and NIP-RS read state are intentionally self-encrypted. The
reminder protocol keeps due-time metadata public so the relay can schedule
delivery, while hiding the reminder body. See `docs/nips/NIP-ER.md:9-13`,
`docs/nips/NIP-ER.md:38-40`, `docs/nips/NIP-ER.md:46-77`,
`docs/nips/NIP-ER.md:104-105`, and `docs/nips/NIP-ER.md:135-201`.
Read state uses self-encryption and random slot IDs as described in
`docs/nips/NIP-RS.md:9-17` and `docs/nips/NIP-RS.md:39-109`; mobile applies
that pattern in `mobile/lib/features/channels/read_state/read_state_manager.dart`.

Gift wraps are opaque NIP-17 events whose wrapper author is ephemeral. The relay
therefore cannot require wrapper pubkey to match the authenticated connection in
the normal way. It still p-gates them and excludes them from search. See
`crates/buzz-core/src/kind.rs:59-60`,
`crates/buzz-relay/src/handlers/ingest.rs:1146-1219`,
`crates/buzz-relay/src/handlers/event.rs:188-200`, and
`crates/buzz-relay/src/handlers/event.rs:287-301`.

The common caveat across all NIP-44 use is that content encryption is not
metadata encryption. The relay still sees event kind, timing, size, tags needed
for routing, and often the relationship between sender and recipient. That is
normal for Nostr-style encrypted events, but it matters when evaluating privacy
claims.

### NIP-AB Pairing

The Rust NIP-AB implementation is the strongest cryptographic subsystem in the
repository. It uses fresh ephemeral secp256k1 keypairs, a 32-byte session
secret, HKDF-SHA256 with explicit domain separation, constant-time comparison,
NIP-44 v2 payload encryption, strict state transitions, timeout handling,
recipient `p`-tag validation, duplicate suppression, and zeroization. See
`crates/buzz-core/src/pairing/crypto.rs:1-135`,
`crates/buzz-core/src/pairing/session.rs:84-109`,
`crates/buzz-core/src/pairing/session.rs:113-283`,
`crates/buzz-core/src/pairing/session.rs:288-428`, and
`crates/buzz-core/src/pairing/session.rs:432-750`.

The QR parser validates key and secret lengths, rejects all-zero secret
material, validates relay URLs and versioning, and zeroizes sensitive fields on
drop. See `crates/buzz-core/src/pairing/qr.rs:34-57`,
`crates/buzz-core/src/pairing/qr.rs:83-96`, and
`crates/buzz-core/src/pairing/qr.rs:110-225`. The sidecar pairing relay is
deliberately unauthenticated and history-free, but it validates event
signatures, freshness, deduplication, and bounds while only routing opaque
NIP-44 ciphertext. See `crates/buzz-pair-relay/src/lib.rs:1-27`,
`crates/buzz-pair-relay/src/lib.rs:60-91`, and
`crates/buzz-pair-relay/src/lib.rs:323-591`.

The protocol's hard limit is human SAS comparison. A QR observer can race the
real target, but the user should detect that because the SAS values differ.
That assumption and the 120-second QR exposure window are documented in
`crates/buzz-core/src/pairing/NIP-AB.md:538-605`. The mobile implementation
weakens the local half of that design through C-01 and C-06.

Desktop pairing sends a raw `nsec` inside the encrypted payload in
`desktop/src-tauri/src/commands/pairing.rs:35-60` and
`desktop/src-tauri/src/commands/pairing.rs:79-182`. The NIP permits that, though
it recommends NIP-49 `ncryptsec` for defense in depth in
`crates/buzz-core/src/pairing/NIP-AB.md:338-369`. This is not a finding because
the transport channel is intended to protect the payload, but encrypting the
transferred key at rest before it leaves the source would reduce the impact of
a later target-storage mistake.

### Git Commit Signing And Git Transport

`git-sign-nostr` is unusually careful code. It separates its trust limits in
`crates/git-sign-nostr/src/lib.rs:1-72`, uses `Zeroizing` for key material,
removes key environment variables after loading in
`crates/git-sign-nostr/src/lib.rs:404-464`, and opens key files with
`O_NOFOLLOW`, `O_NONBLOCK`, owner checks, and mode checks in
`crates/git-sign-nostr/src/lib.rs:783-898`. Its NIP-GS hash includes a domain
separator, timestamp, auth-tag binding, and payload in
`crates/git-sign-nostr/src/lib.rs:900-932`; its JSON envelope parser is strict
and canonical in `crates/git-sign-nostr/src/lib.rs:936-958` and
`crates/git-sign-nostr/src/lib.rs:1410-1519`.

The Git signer also separates cryptographic verification from advisory trust
decisions and actually evaluates NIP-OA conditions. See
`crates/git-sign-nostr/src/lib.rs:600-629`,
`crates/git-sign-nostr/src/lib.rs:1034-1080`, and
`crates/git-sign-nostr/src/lib.rs:1264-1356`. That is the right pattern for
delegated signatures.

The credential helper is less hardened around key-file loading. It checks file
metadata and mode but does not use the same anti-symlink and owner-race
protections as the signer. See `crates/git-credential-nostr/src/lib.rs:30-74`.
That is a key hygiene inconsistency, though the more important transport issue
is C-03.

The relay-to-hook push policy authentication is well composed. It uses a
canonical length-prefixed payload, HMAC-SHA256, constant-time verification, and
a 30-second TTL in `crates/buzz-relay/src/api/git/policy.rs:1-235` and
`crates/buzz-relay/src/api/git/policy.rs:386-409`. The shell hook mirrors that
format and fails closed in `crates/buzz-relay/src/api/git/hook.rs:1-176`. The
relay also signs ref-state events in
`crates/buzz-relay/src/api/git/manifest_event.rs:59-117`.

### Media Upload Authentication

The Blossom-style media path is stronger than generic NIP-98 for body integrity.
`crates/buzz-media/src/auth.rs:1-135` verifies signature, kind, verb, expiry,
freshness, server tags, and a mandatory `x` content hash. The upload handlers
compute SHA-256 and verify it after buffering or streaming in
`crates/buzz-media/src/upload.rs:25-115` and
`crates/buzz-media/src/upload.rs:211-404`. The relay API requires
`X-SHA-256`, matches it against the signed `x` tag, enforces token signer-owner
binding, and checks membership in `crates/buzz-relay/src/api/media.rs:27-122`
and `crates/buzz-relay/src/api/media.rs:567-665`.

That is the right model for large uploads: the signed authorization commits to
the content hash, and the receiver recomputes the hash over the bytes actually
stored.

### Bearer Tokens, Webhooks, And Approval Secrets

Not every secret in Buzz is a signing key. Workflow webhook secrets, approval
tokens, API tokens, and proxy invite tokens are bearer credentials. Their
security depends on entropy, transport, storage, and lifetime rather than
signature verification.

Webhook secrets are UUID v4 values, are stripped from normal workflow
responses, and are compared in constant time in
`crates/buzz-relay/src/webhook_secret.rs:22-90`. The endpoint accepts them
either through `X-Webhook-Secret` or a `?secret=` query parameter in
`crates/buzz-relay/src/api/bridge.rs:831-973`. Header transport is materially
better because query strings are more likely to be copied into access logs,
browser history, monitoring traces, and referrer-bearing URLs.

Approval tokens follow a better storage pattern: the executor creates a random
raw value, while the database stores only a SHA-256 hash and consumes it with a
race-safe update. See `crates/buzz-workflow/src/executor.rs:714-721`,
`crates/buzz-db/src/workflow.rs:27-33`, and
`crates/buzz-db/src/workflow.rs:605-755`. API tokens also have hashed storage,
expiry, and revocation fields in `crates/buzz-db/src/api_token.rs:1-290`.

Proxy invite tokens are UUID v4 bearer values stored raw in memory and consumed
by lookup in `crates/buzz-proxy/src/invite.rs:8-60`,
`crates/buzz-proxy/src/invite_store.rs:10-70`, and
`crates/buzz-proxy/src/server.rs:807-910`. That is adequate for short-lived
in-memory invites, but they should be treated as secrets in logs and metrics.

### Proxy Shadow Keys

The proxy deterministically derives a local shadow identity from an external
pubkey using `HMAC-SHA256(server_salt, external_pubkey_bytes)` in
`crates/buzz-proxy/src/shadow_keys.rs:1-117`. This is a reasonable deterministic
derivation if the salt is high entropy and remains secret. It also creates a
single high-value secret: anyone who gets the salt can derive every shadow
private key for every known external pubkey. Salt rotation changes every
derived identity, so rotation is operationally expensive.

That construction is not inherently wrong, but the salt should be handled like
a master key, not like a casual configuration value.

### Audit Chain

The audit chain is useful for detecting accidental breaks and some partial
tampering, but it is not cryptographically equivalent to a keyed append-only
log. C-07 covers the mismatch between documentation and implementation. The
important design question is who the audit system is intended to detect:
application bugs, database-only attackers, or a compromised relay operator.
Those are different threat models and require different cryptographic anchors.

### Huddle Audio

Huddle audio is not application-layer encrypted. The relay receives and forwards
audio frames whose body is treated as opaque Opus data in
`crates/buzz-relay/src/audio/mod.rs:1-10` and
`crates/buzz-relay/src/audio/wire.rs:1-19`,
`crates/buzz-relay/src/audio/wire.rs:56-85`. Authentication uses NIP-42, and
room lifecycle events are relay-signed in
`crates/buzz-relay/src/audio/handler.rs:1-12`,
`crates/buzz-relay/src/audio/handler.rs:93-145`, and
`crates/buzz-relay/src/audio/handler.rs:719-785`.

"Opaque to the relay code" is not the same as "encrypted from the relay". Audio
confidentiality relies on the relay host and transport security unless another
layer is added above the current wire format.

## Trust Boundaries And Non-Findings

The desktop renderer is inside the human account-key trust boundary. Tauri
commands expose event signing, `get_nsec`, import, and self-NIP-44 operations in
`desktop/src-tauri/src/commands/identity.rs:67-90`,
`desktop/src-tauri/src/commands/identity.rs:151-197`, and
`desktop/src-tauri/src/commands/identity.rs:199-244`. A renderer compromise is
therefore a key compromise. That may be an acceptable product boundary, but it
should not be confused with a hardware-backed or isolated signer design.

Managed agents are also intentionally inside the local machine trust boundary.
The runtime injects `BUZZ_PRIVATE_KEY`, `BUZZ_AUTH_TAG`, and
`NOSTR_PRIVATE_KEY` into child processes in
`desktop/src-tauri/src/managed_agents/runtime.rs:1549-1561`,
`desktop/src-tauri/src/managed_agents/runtime.rs:1648-1656`, and
`desktop/src-tauri/src/managed_agents/runtime.rs:1682-1687`. The environment
filtering in `desktop/src-tauri/src/managed_agents/env_vars.rs:58-145` is useful
defense in depth, but an agent that is deliberately allowed to run local tools
is not a secret-isolated principal.

Ordinary messages are signed, not end-to-end encrypted. The relay, its storage,
and its operators can see plaintext unless a specific feature wraps content in
NIP-44. Huddle audio has the same property. TLS protects transport hops; it
does not remove the relay from the confidentiality boundary.

Encrypted Nostr events still expose routing metadata. NIP-44 hides content, but
the relay can often observe timing, size, event kind, and tags. Any privacy
claim should be phrased as content confidentiality, not traffic-analysis
resistance.

Some relay-signed state is trusted because it came from the configured relay,
not because every client re-verifies every relay signature. The NIP-DV docs
state this explicitly for desktop query behavior in `docs/nips/NIP-DV.md:118-128`.
That is a reasonable trust model for a single configured workspace relay, but
it is different from an offline-verifiable transparency model.

## Recommended Fix Order

1. Fix mobile NIP-AB payload handling so target consent actually gates payload
   plaintext exposure, then clear all pairing secret state on every terminal
   path.
2. Centralize NIP-OA/NIP-AA semantic authorization and enforce signed time
   conditions during relay admission; add real managed-agent credential
   retirement.
3. Make NIP-98 payload binding mandatory for JSON bridge routes and keep the Git
   weakening explicit and local.
4. Harden managed-agent key storage to the same permission standard as the
   desktop human identity key.
5. Decide what audit tamper-evidence threat model is intended and align either
   the implementation or the documentation.
6. Add strict mobile NIP-44 vector tests and canonical padding checks.
