# Local user capabilities: consolidation boundary

This branch changes how the desktop obtains cryptographic capabilities, not who
owns the user's key. Production still uses the existing local identity and
keychain. It adds no remote service, login flow, feature gate, relay protocol,
key migration, or enterprise-session dependency.

## Review map

The first branch owns **caller factoring** and the local implementation. A second
branch can add remote custody behind these capabilities without creating remote
versions of messages, profiles, snapshots, or agent-management features.

| Boundary | Local implementation and callers | Second-branch responsibility |
| --- | --- | --- |
| Captured lifetime | `CapabilityLifetime`, optional on `ActiveUserSigner`; absent for local keys; shared signing/query/publication boundaries execute through `run` | Implement immutable validity/cancellation and same-key replacement generation; no credential lookup in shared callers |
| Public identity | `AppState::identity_public_key`, cached `ActiveUserSigner::public_key` | Obtain a stable public identity without a local secret |
| Event signing | `ActiveUserSigner` wraps `nostr::NostrSigner`; event builders stay unchanged | Implement structured-event signing and verify the returned artifact |
| NIP-04/NIP-44 | The existing `NostrSigner` crypto methods | Supply supported ciphers; report unsupported operations explicitly |
| Owner authorization | `ActiveUserSigner::authorize_agent`; create/import/repair use captured owner | Implement the exact NIP-OA authorization operation, not arbitrary digest signing |
| Agent memory | `ActiveUserSigner::read_agent_memory`; preserve event and keyed-address validation | Provide authenticated memory validation without exposing a conversation key or owner secret |
| Relay HTTP | `relay/submit.rs`, signer-aware query/get and NIP-98 helpers | Supply signer; keep destination and identity bound to the initiating operation |
| Relay WebSocket/audio | `buzz-ws-client` signer-aware authentication and desktop native/huddle adapters | Bind connection lifetime to remote authorization; no streaming proxy required |
| Signed local retention | Prepare/sign/revalidate/commit helpers | Supply async crypto; retain the existing persistence policy |

Event signing and publication are deliberately separate. A retained tombstone
needs a signed artifact before local persistence; silently publishing during
`sign_event` would break that contract. `relay/submit.rs` remains the convenience
boundary for ordinary sign-then-publish work. A future sign-and-submit service can
be introduced there separately, with a specified uncertain-outcome/retry contract.

## Local compatibility contract

- Preserve public IPC argument/result shapes, event kinds/tags/content/timestamps,
  encryption formats, retention schemas, and keychain storage. Schnorr signatures
  and encryption nonces are randomized; compatibility is semantic and
  cryptographic, not byte equality of independently generated signatures.
- `active_signer` preserves the existing lost/locked identity denial for owner
  signing. `legacy_local_signer` names existing recovery-mode exceptions for
  reads, binding, local archive and missing-tag repair. It is not a fallback for
  unavailable remote custody, nor permission to add new recovery exceptions.
- Capture identity and destination before suspension. Never re-read the active
  workspace after signing and send previously prepared content to another relay.
- Keep independent agent keys and already-running agents independent of the
  human's identity lifetime. Local agent keys are not leaks of the human key.
- An identical local owner/relay workspace reapply is not a new authentication
  session. Actual scope transitions, including away-and-back, invalidate pending
  scope-bound mutations. Remote reauthentication is a separate second-branch
  lifetime rule.
- Admission checks belong before effects. Once an agent has been durably created
  or stopped, preserve its result/recovery diagnostics rather than returning an
  undifferentiated retry error that hides committed work.

## Why some callers change more than a helper name

Local signing was sometimes performed under a SQLite transaction or agent-store
mutex. An async interface introduces suspension there. This branch's existing
prepare/sign/revalidate split avoids holding those locks during crypto and checks
relevant retained heads/cascade inputs before committing. It preserves the
existing best-effort deletion policy: a signature failure does not automatically
prohibit local deletion.

That split is not a universal architectural requirement. A bounded synchronous
wait on a suitable worker can be valid, but would need a total wait budget and a
lock audit. This branch does not introduce that alternative or a new outbox
protocol. SQLite rollback does not roll back filesystem, process, or relay effects.

Archive preparation distinguishes invalid local content from an unavailable
cryptographic operation. Existing invalid observer frames retain their raw-event /
NULL-index behavior; invalid metrics are dropped. Remote availability, bounded
retry scheduling, retained ciphertext and reconnect/expiry are second-branch
responsibilities. A generic async result boundary alone is not a recovery queue.

## Explicit local-secret exceptions

These are separate contracts, not ordinary crypto callers to route through a
`get_secret()` escape hatch:

- NIP-49 export/import/verification, identity recovery and storage migration.
- Device pairing: its current protocol intentionally transfers local identity.
- Git credential subprocesses (`commands/project_git_exec.rs` and the merge
  workflow): an external helper currently consumes an nsec. Remote support needs
  a process/credential adapter, not an in-process signer substitution.
- Independent agent runtime credentials and agent-key snapshot decryption.
- Legacy raw-key library helpers retained for those agent/local callers and tests.

Remote mode must explicitly implement or decline these capabilities. It must not
fall back to a generated or cached local human key. The raw-key inventory should
be reviewed by call path; a grep hit in an agent-key or test helper is not evidence
of an unconverted human capability.

## What must stay out of this branch

Kgoose HTTP transport/configuration, native enterprise credentials and expiry,
remote generation headers, SSO renderer/bootstrap, remote-only settings gates,
keyless onboarding, community enrollment, new cipher policy, revocation policy,
remote archive retry scheduling, and publication proxying. Those require their
own compatibility and live-workflow validation.

## Validation expectations

Run the full desktop Rust workspace and full `buzz-ws-client` package suites,
formatting and clippy. Tests should exercise production seams with delayed and
failing capabilities, not merely a mock's own methods. Compare local event
fields/verification, encryption interoperability, owner authorization, keyed
memory validation, recovery exceptions, retained timestamps, destructive conflict
checks and captured relay destinations. Preserve legacy `buzz-ws-client` key-based
entrypoints for external consumers.

Unit/loopback success is not a full desktop acceptance test. Before claiming
unchanged app behavior, exercise local login/recovery, community switching,
message read/send/reconnect, agent create/import/delete/defaults, memory and media
in an isolated app profile. This document is the review contract, not a claim
that all these checks have already run.
