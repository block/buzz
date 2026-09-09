# FounderPortal × Buzz V1 launch seam

Buzz Desktop already performs NIP-42 signing with its native Rust-held identity.
The private key is persisted in the operating-system keyring, with the existing
permission-restricted `identity.key` fallback, and is deliberately stripped
from workspace `localStorage` records. FounderPortal must not provision or
transport a private key.

The V1 human launch contract is therefore:

1. FounderPortal authenticates the human and resolves tenant membership on its
   server.
2. FounderPortal submits only `(tenant_id, actor_type=human, fp_user_id,
   buzz_pubkey, desired_version)` to the trusted bridge service. The public key
   comes from an explicit user-approved Buzz identity binding.
3. The bridge converges `ensureCommunity` and `ensureMember` before launch.
4. FounderPortal launches the installed Buzz Desktop application to the mapped
   community URL. The URL contains no pubkey, private key, token, role,
   permission, employee/entity id, or tenant authority.
5. Buzz Desktop uses its existing keyring-backed identity to answer the relay's
   NIP-42 challenge. Relay admission is enforced by the durable `relay_members`
   row.

A plain browser cannot safely complete this flow because Buzz's secure signer
is native Desktop. V1 must show an “Open Buzz Desktop” handoff and may show an
installation prompt when the protocol handler is unavailable. It must not add
NIP-FI, inject an nsec into browser configuration, or fall back to
`localStorage`.

Key rotation/rebinding requires a new explicit human consent and a higher
desired version. A channel membership is never evidence of FounderPortal
business authority.
