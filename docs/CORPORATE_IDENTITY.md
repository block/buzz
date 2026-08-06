# Corporate identity

Corporate identity is an optional relay policy enabled with
`BUZZ_REQUIRE_CORPORATE_IDENTITY=true`. The relay verifies an asymmetric JWT
after the request proves control of a Nostr key, then admits the request only
when the existing community policy also succeeds.

## Required JWT policy

- `BUZZ_CORPORATE_IDENTITY_JWKS_URI` must be HTTPS and contain no credentials.
- JWTs must have a supported asymmetric algorithm, a `kid`, and valid `exp`,
  `iss`, and `aud` claims. A present `nbf` claim is enforced.
- `BUZZ_CORPORATE_IDENTITY_NPUB_CLAIM`, when configured, is mandatory and must
  equal the authenticated Nostr key. Leaving it unset enables first-use
  uid-to-key enrollment in the private binding table.
- JWKS requests have connect and total timeouts, reject redirects, cap the
  response at 1 MiB, cache keys for five minutes, and coalesce refreshes.

`BUZZ_REQUIRE_CORPORATE_IDENTITY` and
`BUZZ_ALLOW_CORPORATE_IDENTITY_DELEGATION` are strict booleans. Misspellings and
non-UTF-8 values stop configuration loading instead of silently disabling a
gate.

## Binding and revocation lifecycle

JWT validation is read-only. The relay creates or refreshes a binding only
after admission, allowlist, role, and community membership checks succeed.
Invite claims commit the binding, membership, policy evidence, and invite use
in one PostgreSQL transaction.

Revocation has three explicit meanings:

- `principal` disables every key for an issuer-qualified uid. Normal
  authentication cannot re-enroll the principal with another key.
- `key` revokes one key but does not silently authorize a replacement.
- `rotation` is the audit state written by an explicit atomic old-key to
  new-key rotation.

WebSocket and audio sessions revalidate the authoritative binding at least
every 30 seconds. Direct sessions also close at JWT expiry. Delegated sessions
check the owner's binding, so disabling an owner evicts the owner's agents as
well as the direct owner session.

Corporate NIP-OA delegation is transport-wide and therefore accepts only an
empty conditions string. Conditional tags must be evaluated for a specific
operation and are not treated as blanket corporate identity authority.

## Privacy and public assertions

`BUZZ_CORPORATE_IDENTITY_DISPLAY_CLAIM` is private. Its default (`email`) is
stored only in the community-scoped binding table and audit data; it is not
published to Nostr.

Public projection is separately opt-in with
`BUZZ_CORPORATE_IDENTITY_PUBLIC_DISPLAY_CLAIM`. When set, that claim is
published as a relay-signed NIP-85 label. Assertions carry both `active=true`
and an `expiration` no later than one hour or the JWT's expiry, whichever comes
first. Clients require the relay signature, active status, and a future
expiration. Removing the opt-in publishes an inactive replacement only when a
prior public assertion exists.

NIP-85 events are replaceable relay events and may also remain in downstream
caches or archives after replacement. Operators must choose a non-sensitive,
user-approved public label and account for that retention when configuring the
public claim.

## Route policy

Corporate identity applies to authenticated WebSocket and audio connections,
the NIP-98 event/query/count bridge, moderation reads, invite mint and claim,
Git smart HTTP, media uploads, and protected media reads.

Intentional exemptions are public media reads when media GET authentication is
disabled, health/readiness/metrics endpoints, NIP-11 and NIP-05 discovery,
operator and admin control planes with their own authentication, secret-backed
workflow hooks, public join-policy documents, invite policy-acceptance
callbacks, and static local web callbacks. These exemptions must remain in the
central route-policy test matrix when routes change.
