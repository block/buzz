# Local credential provider

`draft`

This document defines how a local process, such as the `buzz` CLI, asks a running
Buzz Desktop to act as one Nostr identity: to give its public key, sign events,
encrypt and decrypt with NIP-44, return NIP-FI evidence for a relay, and return
its NIP-OA `auth` tag. The private key and the sign-in session stay in the
desktop.

The CLI is the reference client and lives in this repository
(`crates/buzz-cli`). Buzz Desktop ([block/buzz-app](https://github.com/block/buzz-app))
is the reference provider. Any client or provider that follows this document
interoperates with both.

## Goals

- **One identity per caller.** A caller holds a token for exactly one identity.
  The desktop never decides on the caller's behalf which identity to use.
- **An agent never acts as its owner.** A process the desktop launches for an
  agent can act only as that agent.
- **The desktop sees what it signs.** The caller sends the complete unsigned
  event; the desktop classifies it into an operation and applies a policy before
  signing. There is no blind signing of opaque bytes.
- **Old and new versions interoperate.** The bundled CLI follows the desktop's
  pinned revision of this repository, but a user can also run a different CLI.
  An older client must work against a newer desktop, and the reverse.

## Non-goals

- Remote signing. The transport is loopback only. [NIP-46](https://github.com/nostr-protocol/nips/blob/master/46.md)
  covers remote signers.
- Windows. Version 1 has no Windows transport. See [Platforms](#platforms).
- Deciding which approval UI the desktop shows. This document fixes what needs
  approval, not how it is presented.

## Terms

- **Provider**: the desktop process that holds the key and answers requests.
- **Caller**: the local process that sends requests.
- **Token**: an opaque bearer secret that names one identity and the relays it
  may act on. Tokens carry at least 128 bits of entropy.
- **Instance**: one installed desktop build, named by a stable identifier such
  as its bundle identifier. Production and staging builds are separate
  instances and issue separate tokens.
- **Bound relay**: a community relay URL a token may act on. Relay URLs are
  compared whole after URL normalization: `http`/`https` read as `ws`/`wss`,
  lowercase host, no default port, `/` for an empty path. The path and query
  count, because one host can serve several communities. A caller sends the
  bound URL itself in `pair` and `federated_identity`; NIP-FA audience
  canonicalization is the provider's job.

## Discovery

A caller looks for a provider in this order. The first source that yields a
token and a reachable endpoint is used.

An inherited token decides the identity. A caller that has one MUST use only
it: when it is revoked, does not apply to the relay or fails, the caller fails.
It MUST NOT go on to a paired token, which acts as the human, so an agent never
acts as its owner.

### 1. Inherited token

When the desktop launches a process for an agent, through any agent harness, it
sets:

```text
BUZZ_CREDENTIAL_PROVIDER={"url":"http://127.0.0.1:<port>/<path>","token":"<token>"}
```

- The token's identity is that agent. It is never the owner.
- The token MUST reach only processes that act for that agent. A harness that
  hosts several agents in one process MUST give each agent's tool processes
  their own agent's token, and MUST NOT expose one agent's token to another
  agent's tools.
- The desktop SHOULD revoke the token when the process it launched exits, and
  MUST revoke it on the triggers in [Revocation](#revocation).
- A caller MUST reject a `url` whose scheme is not `http`, whose host is not
  exactly `127.0.0.1`, or that carries user information.
- Because the desktop handed the endpoint to the process directly, the caller
  does not need the same-user check below for an inherited token.

The desktop MUST set this variable only for processes it launches for an agent,
and only in their own environment. It MUST NOT write the token to disk.

### 2. Paired token

A human's own terminal, or a service the human runs such as slack2buzz, gets a
token by [pairing](#pairing). The caller stores the token in the OS keychain
(Keychain on macOS, the Secret Service on Linux), never in a file:

- The CLI keeps one keychain entry, service `buzz-cli` and account
  `desktop-pairings`, that maps each instance to its token, identity and bound
  relays. Each change reloads the entry under a short cross-process lock and
  touches only its own instance; the lock is never held across an approval
  wait. An entry that cannot be read is refused, never overwritten.
- A paired caller only tries a paired token when no inherited token is set.
  A desktop that answers `denied` or `unavailable` fails the command; only an
  unreachable or unconfirmed advertisement is skipped.
- When no keychain is reachable, pairing MUST fail with an error that says so.
  A caller MUST NOT fall back to writing the token to disk.
- A paired token survives a desktop restart. The desktop keeps it until it is
  revoked or expires (see [Revocation](#revocation)), so a caller pairs once.

The token does not say where the desktop is listening, because the port changes
on every launch. The desktop advertises its current endpoint, which holds no
secret, in:

```text
~/.buzz-foundation/credential-providers/<instance>.json
{"instance":"<instance>","url":"http://127.0.0.1:<port>/<path>"}
```

- The desktop writes the file whole under a dot name and renames it into place,
  and removes it when it quits.
- The directory MUST be mode `0700` and owned by the user, and each file MUST be
  owned by the user with no group or other permission bits. A caller MUST
  ignore any file that fails these checks, so another user cannot plant one.
- A caller uses the advertisement whose `instance` matches its stored token.
- **Same-user check.** An advertised port may now belong to another user's
  process, for example after the desktop crashed. Before it sends a token to an
  advertised endpoint, a caller MUST connect and confirm, by numeric user ID,
  that a process of the same user holds the server side of that connection
  (`lsof` on macOS, `/proc/net/tcp` on Linux). If it cannot confirm this within
  about one second, it MUST send nothing on that connection. This is the check
  `buzz-agent-command` performs for advertised agent sessions
  (block/buzz-app `crates/agent-command`).
- A refused connection means the desktop is not running. The caller MAY delete
  that advertisement.

### 3. No provider

Without an inherited token, if no paired token yields a provider, the desktop
link in the caller's credential chain does not apply, and the caller moves on
to its next source. A caller with an inherited token never falls through to a
paired token: when the inherited token cannot serve, the caller fails.

## Transport

- HTTP/1.1 `POST` to the discovered `url`, on `127.0.0.1` only. The provider
  MUST bind only to `127.0.0.1`.
- `Content-Type: application/json`.
- `Authorization: Bearer <token>` on every request except `pair`.
- `Buzz-Credential-Provider: <version>` on every request. See
  [Versions](#versions).
- One request per connection is sufficient. Callers MUST NOT rely on keep-alive.

### Envelope

Requests and responses use the NIP-46 message shape, unencrypted because they
never leave the machine. That lets both sides reuse existing NIP-46 types.

```json
{"id":"<random string>","method":"<method>","params":["<string>", "..."]}
```

```json
{"id":"<same id>","result":"<string>"}
{"id":"<same id>","result":"","error":"<code>","message":"<human text>","retry_after":30}
```

- `params` and `result` are strings, as in NIP-46. Structured values are
  JSON-encoded into the string.
- On failure, `error` is a machine code from [Errors](#errors). `message` is
  optional text for a person. `retry_after` is present only for `rate_limited`.
- Both sides MUST ignore fields they do not recognize.

## Methods

### NIP-46 methods

These have the same parameters and results as in NIP-46.

| Method | Params | Result |
|---|---|---|
| `get_public_key` | `[]` | Hex public key of the token's identity |
| `sign_event` | `[<unsigned event JSON>]` | Signed event JSON |
| `nip44_encrypt` | `[<peer pubkey hex>, <plaintext>]` | NIP-44 payload |
| `nip44_decrypt` | `[<peer pubkey hex>, <payload>]` | Plaintext |

For `sign_event`, the unsigned event carries `kind`, `tags`, `content` and
`created_at`. If it carries `pubkey`, the value MUST equal the token's identity,
or the provider MUST answer `denied`. The provider sets `pubkey`, computes `id`
and signs. `sign_event` is subject to the [operation policy](#operation-policy).

### Buzz methods

| Method | Params | Result |
|---|---|---|
| `describe` | `[]` | JSON: identity, role, bound relays and methods |
| `federated_identity` | `[<relay URL>]` | NIP-FI assertion, or empty string |
| `auth_tag` | `[]` | JSON-encoded NIP-OA `auth` tag, or empty string |
| `engram_d_tag` | `[<peer pubkey hex>, <slug>]` | NIP-AE `d` tag for that slug |
| `pair` | `[<client name>, <relay URL>]` | JSON: token and identity |

`describe` returns:

```json
{
  "instance": "<instance>",
  "pubkey": "<hex>",
  "role": "agent",
  "owner_pubkey": "<hex>",
  "relays": ["wss://community.example"],
  "methods": ["get_public_key", "sign_event", "..."]
}
```

- `role` is `agent` or `owner`. `owner_pubkey` is present only for `agent`.
- A caller MUST NOT use the provider for a relay that is not in `relays`. It
  treats the provider as not applicable to that relay: with a paired token it
  moves on to its next source; with an inherited token it fails (see
  [Discovery](#discovery)).

`federated_identity` returns a current [NIP-FI](nips/NIP-FI.md) assertion
whose `nostr_pubkey` is the token's identity and whose audience is the given
relay. It returns an empty string when the relay does not advertise NIP-FI or
the desktop has no enterprise login for it, so a caller sends no evidence on
such relays. It answers `denied` for a relay that is not bound to the token,
and `unavailable` when the relay requires NIP-FI but the desktop's session for
it has lapsed. A caller SHOULD cache the assertion until shortly before its
`exp` and MUST NOT write it to disk.

`auth_tag` returns the agent's NIP-OA `auth` tag as a JSON array, for example
`["auth","<owner>","<conditions>","<sig>"]`, or an empty string for an `owner`
token.

`engram_d_tag` returns the 64-hex `d` tag that [NIP-AE](nips/NIP-AE.md#addressing)
derives from the identity's NIP-44 conversation key with `peer` and the slug.
The provider MUST answer `invalid_request` for a slug outside NIP-AE's grammar,
so the method is not a general keyed-hash oracle. Engram content uses
`nip44_encrypt` and `nip44_decrypt` with the same peer, so no method exposes the
conversation key itself.

The provider never returns the private key or a NIP-44 conversation key. A
caller feature that needs either one, rather than an operation built on it,
needs an explicit key.

`pair` is described under [Pairing](#pairing).

## Pairing

`pair` is the only method accepted without a token. The provider MUST:

1. Show the human an approval that names the client and the relay, and the
   calling process when the provider can identify it.
2. Hold the request open until the human decides, for at most 120 seconds.
3. On approval, answer with
   `{"token":"<token>","instance":"<instance>","pubkey":"<hex>","relays":["<relay>"]}`.
   The identity is the human's own key for that relay.
4. On refusal, answer `denied`. On timeout, answer `approval_required`.

The provider MUST rate-limit `pair` so that a local process cannot flood the
human with approvals. A caller MUST treat a new pairing for an instance as
replacing its old token.

## Operation policy

The provider classifies each `sign_event` request before signing. Callers MUST
NOT rely on the policy for safety; it exists so that a token holder, such as a
prompt-injected agent, cannot do more than the identity's ordinary work without
a person noticing.

### Auth proofs

These kinds are accepted only when bound to one of the token's relays, and only
when fresh: `created_at` within 60 seconds of the provider's clock.

| Kind | Requirement |
|---|---|
| `22242` NIP-42 AUTH | Exactly one `relay` tag, equal to a bound relay after URL normalization, and exactly one `challenge` tag |
| `27235` NIP-98 HTTP auth | Exactly one `u` tag whose origin is a bound relay's origin (`wss`→`https`, `ws`→`http`), and exactly one `method` tag |
| `24242` Blossom auth | Exactly one `server` tag equal to a bound relay's host, a `t` tag of `upload` or `get`, and an `expiration` no later than `created_at + 60` |

A NIP-98 event for any other origin is denied. This rules out using the desktop
to sign in to other services, including a NIP-FA adapter: a caller using the
provider gets NIP-FI evidence from `federated_identity` and never mints its own.

### Ordinary content

Kinds an identity uses for everyday work are signed without a prompt: messages
and their edits, reactions, threads and forum posts, typing and presence, read
state and user status, the identity's own lists (mute, pin, bookmark, emoji,
follow sets), canvases, artifacts, git patches, issues and status events, DM
open and hide, archive requests, workflow triggers, jobs, agent memory
(`30174`) and observer frames. The provider keeps the exact table, keyed by the
constants in `crates/buzz-core/src/kind.rs`.

### Sensitive operations

These kinds change who the identity is, what it controls or what others see
about it. The provider MUST get a person's approval for each request, holding
the request open for at most 120 seconds. Without a decision it answers
`approval_required`; on refusal, `denied`.

| Category | Kinds |
|---|---|
| Profile and relays | `0` profile, `10002` relay list, `10100` agent profile |
| Social graph | `3` contact list |
| Deletion | `5` deletion, `9005` group delete event, `45011` artifact removal |
| Membership and admin | `9000`–`9009` group admin, `9021`/`9022` join and leave, `28936` NIP-43 leave, `41011` DM add member |
| Moderation | `9040`–`9044` |
| Agent and team management | `30175` persona, `30176` team, `30177` managed agent, `30179` private managed agent |
| Workflow authority | `30620` workflow definition, `46030`/`46031` approval grant and deny |
| Repositories and projects | `30617` repository announcement, `30618` repository state, `30621` project |

The provider MAY remember an approval for a short, visible period, such as
repeated deletions in one session, provided the human can see and revoke it.

### Unknown kinds

Any kind not in the provider's table is denied, with a `message` naming the
kind. A caller that needs it can use an explicit key instead. Adding a kind to
the table is an additive provider change and does not change the version.

### Encryption

`nip44_encrypt` and `nip44_decrypt` act only as the token's identity, so an
agent token can read only what was encrypted to the agent. They are subject to
the token's rate limits. The provider MAY require approval for decryption with a
peer other than the identity itself on an `owner` token.

## Limits

- The provider SHOULD rate-limit each token per method and answer
  `rate_limited` with `retry_after` in seconds.
- Tokens SHOULD expire. An inherited token lives no longer than the process it
  was issued to. A paired token SHOULD expire after a period of disuse that the
  human can see.

## Revocation

The provider MUST revoke tokens when:

- the human revokes the caller in the desktop,
- the owner signs out or the desktop's account changes, which revokes every
  token,
- an agent is deleted, which revokes that agent's tokens, or
- a token expires.

A revoked, expired or unknown token gets `revoked`. A caller holding a revoked
paired token SHOULD delete it and tell the human to pair again.

## Errors

| Code | HTTP | Meaning | Caller behavior |
|---|---|---|---|
| `approval_required` | 403 | A person must approve and did not decide in time | Tell the human to approve in Buzz, then retry |
| `denied` | 403 | Policy or the human refused | Fail; do not retry |
| `rate_limited` | 429 | Too many requests | Wait `retry_after` seconds |
| `revoked` | 401 | Token revoked, expired or unknown | Drop a paired token and try the next paired one; with an inherited token, fail (see [Discovery](#discovery)) |
| `unavailable` | 503 | The desktop cannot act now, for example signed out or its sign-in session lapsed | Fail with the message |
| `unsupported_version` | 400 | See [Versions](#versions) | Retry with a common version |
| `unknown_method` | 400 | The provider does not implement the method | Degrade (see below) |
| `invalid_request` | 400 | Malformed envelope or params. For `nip44_decrypt`, also a payload that is not valid ciphertext for this identity | Fail; for `nip44_decrypt`, treat the payload as invalid (NIP-AE filters it) |

Callers MUST decide on `error`, not on the HTTP status, and MUST treat an
unrecognized code as `denied`.

`sign_event`, `nip44_decrypt` and `pair` can be held for up to 120 seconds, so
callers MUST allow at least 150 seconds for them. Other methods SHOULD answer
within a few seconds.

## Versions

The protocol version is a positive integer. This document defines version `1`.

- A caller sends the highest version it supports in `Buzz-Credential-Provider`.
- A provider that does not support that version answers `unsupported_version`
  with a `versions` field listing the versions it supports, for example
  `"versions":[1,2]`. The caller retries with the highest version both support,
  or treats the provider as not applicable if there is none.
- A provider MUST keep supporting every version that a CLI it bundles, or a CLI
  released in the previous six months, speaks.

Additive changes do not change the version: new methods, new optional fields,
new error codes (treated as `denied` by older callers), and new kinds in the
policy table. Removing or changing the meaning of a method, field or code does.

When a provider answers `unknown_method`, a caller degrades rather than fails
where it can: no `auth_tag` means the caller sends no `auth` tag, and no
`federated_identity` means it sends no NIP-FI evidence.

## Platforms

Version 1 supports macOS and Linux. On Windows the desktop MUST NOT set
`BUZZ_CREDENTIAL_PROVIDER` or write advertisements, and callers MUST skip the
desktop link, because loopback has no same-user check there. A named pipe with
an access control list is the planned Windows transport.

## Security considerations

- **Loopback cannot tell processes apart.** Tokens, not the transport,
  identify callers. The same-user check stops a token reaching another user's
  process; it does not stop another process of the same user, which already has
  the user's files.
- **Evidence leaves the desktop.** The caller talks to the relay directly, so
  `federated_identity` hands it the assertion. NIP-FI's
  [assertion–key binding](nips/NIP-FI.md#assertionkey-binding) means a leaked
  assertion is useless without the identity's key.
- **Tokens in the environment.** Child processes of an agent inherit
  `BUZZ_CREDENTIAL_PROVIDER` and act as that agent. That matches the agent's
  existing authority; it never grants the owner's.
- **Approval fatigue.** The provider prompts only for sensitive operations,
  never for every signature, so prompts stay meaningful.
