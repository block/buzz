# Journal handoff v0.1

Status: implemented by `buzz-ctx`, `buzz-handoff-state`, `buzz-runner`, and
`buzz-steward` on the `feature/local-relay` integration branch. Unattended
execution remains disabled until the claim transition has relay-enforced
exclusivity. Cross-node result-artifact verification remains a closure gate.

## Purpose

A journal handoff is a durable, signed delegation lifecycle carried by
ordinary kind-1 Nostr events. Labels such as `agent`, `runner`, `machine`, and
`verifier` are presentation metadata. They confer no authority.

The lifecycle is:

```text
open -> claim -> return -> close
```

Every transition:

- is a valid signed kind-1 event;
- carries exactly one lifecycle `t` tag;
- carries the same `h` context as its open;
- is strictly newer than its causal predecessor;
- is reduced from the union of the sovereign and rendezvous views.

The effective owner of a lifecycle event is its author when it carries no
`auth` tag, or the owner recovered from exactly one cryptographically valid,
event-covering NIP-OA `auth` tag. Mutable JSON labels are never consulted for
authorization.

## Open

An open carries:

- `t=handoff:open`;
- one `h` context;
- one or more `p` tags naming the only keys allowed to claim;
- a canonical lowercase 40-character `base_commit`;
- scope, acceptance, and embodiment contracts;
- optional referenced artifact `x` tags.

The opener's effective owner is the only owner identity allowed to verify and
close the handoff. A direct root signer is its own owner; an agent signer must
carry its owner attestation.

## Claim

A claim:

- links the open with `["e", "<open>", "", "root"]`;
- is signed by a key named in an open `p` tag;
- posts after the open.

Duplicate claims from one signer reduce to its earliest `(created_at, id)`
claim. Claims from more than one authorized signer reduce deterministically to
`CONFLICT`; no return is accepted until the conflict is resolved. Unauthorized
claim signers and spoofed runner labels are ignored.

Query-then-post is not compare-and-set. The CLI evaluates both node views and
reports deterministic conflict, but unattended execution stays disabled until
the relay supplies an exclusive claim primitive.

## Return

A return:

- links the open with an `e/root` tag;
- links the accepted claim with an `e/claim` tag;
- restates `claim_id` in content;
- is signed by the exact cryptographic claimant;
- posts after the claim;
- carries status `done` or `failed`.

Before a return may advertise an `x` tag, `buzz-ctx` requires:

1. a manifest referencing the hash in the handoff's `h` context is present at
   the Cloudflare rendezvous;
2. the bytes are readable through the authenticated rendezvous artifact path;
3. the fetched bytes hash to the advertised SHA-256.

`buzz-ctx announce` uploads bytes to both the sovereign and rendezvous stores,
posts the manifest locally, synchronizes it, then performs the same
rendezvous-read and byte-hash verification.

An independent node verifies a returned result with:

```bash
buzz-ctx handoff verify-artifacts <return-event-id>
```

The command reads the exact return from the rendezvous, requires its content
artifact list to equal its `x` tags, fetches every blob through that node's
authenticated reader identity, and byte-compares every advertised SHA-256.

## Close

A close:

- links the open with an `e/root` tag;
- links the exact verified return with an `e/return` tag;
- restates `return_id` in content;
- posts after that return;
- is signed directly by the opener owner or by a signer carrying a valid
  attestation from that owner.

Only a close for the newest valid return produces `CLOSED`. A close for an
earlier return cannot suppress a later return. Legacy closes that identify
only the open are not causally valid and do not suppress steward findings.

## Invalid archival acknowledgment

An invalid open remains invalid: it cannot be repaired by relaxing current
validation or replaying its legacy transitions as if they met the hardened
contract. Its effective owner may instead publish one
`t=handoff:ack-invalid` archival acknowledgment that:

- carries the same `h` context as every referenced invalid open;
- identifies each exact open with an `e/invalid` tag;
- restates the same unique event IDs in `content.open_ids`;
- carries `status=acknowledged-invalid`;
- posts after every referenced open;
- is signed directly by each opener owner or by a signer with a valid
  attestation from that owner.

The reducer reports `ACKNOWLEDGED_INVALID`, preserving the original validation
reason and acknowledgment event ID. This state is neither `CLOSED` nor evidence
that the legacy lifecycle was valid. The steward omits recurring invalid
findings for acknowledged records while retaining their archival count.
Exact event IDs avoid insecure timestamp cutoffs and non-portable custody
sequence assumptions.

## Execution boundary

Host mode is manual and explicit. `buzz-runner` refuses unattended host
execution and installed copies require an explicit
`BUZZ_RUNNER_REPOSITORY`.

Runner base commits must:

- be canonical full object IDs;
- resolve without identity change to a commit;
- be ancestors of the configured `BUZZ_RUNNER_TRUSTED_REF`.

The Podman contract requires a digest-pinned image, read-only root filesystem,
all capabilities dropped, `no-new-privileges`, PID/CPU/memory/time limits,
ephemeral workspace, and an explicit no-network policy. The execution
environment is an allowlist; host `HOME`, SSH agent sockets, credential stores,
Cloudflare/GitHub tokens, and Buzz/Nostr private keys are not inherited.

Pre-claim failures remove registered execution worktrees. Post-claim evidence
is retained so an operator can diagnose and return an explicit failure.

## Closure evidence

Closing a hardening handoff additionally requires:

- focused authorization, causal-order, conflict, custody, Git, cleanup, and
  environment tests;
- a signed return naming exact commits and result artifacts;
- independent second-node fetch and SHA-256 byte verification for every
  result artifact.
