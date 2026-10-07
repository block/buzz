# Federated Identity Adapter: Desktop implementation guide

Buzz Desktop learns that a relay requires enterprise identity from the relay's
NIP-11 `federated_identity` advertisement. Discovery deliberately contains no
issuer URL, tenant ID, audience, or vendor-specific login details; those stay in
operator-controlled Desktop build configuration.

When a trusted build connects to a trusted enterprise relay, Desktop speaks the
HTTP contract defined in [NIP-FA](nips/NIP-FA.md) (Federated Identity Adapter)
to the configured adapter. This guide covers only Desktop build configuration
and Desktop behavior; the wire contract lives in the NIP.

## Build configuration

Enterprise builds set:

- `BUZZ_BUILD_ENTERPRISE_AUTH_RELAYS`: comma-separated allowlist of relay URLs
  for which Desktop will honor NIP-FI enterprise login.
- `BUZZ_BUILD_ENTERPRISE_AUTH_ADAPTER_BASE_URL`: base URL of the adapter
  implementing NIP-FA. Remote adapter URLs must use HTTPS. Plaintext
  HTTP is accepted only for loopback development hosts (`localhost`,
  `127.0.0.0/8`, or `::1`). Required when the relay allowlist is set.
- `BUZZ_BUILD_ENTERPRISE_PROFILE_PROJECTION`: optional `1`/`true` opt-in. When
  unset, adapter identity fields are private login state only and are never
  projected into a public Nostr profile. This is the operator opt-in described
  in NIP-FA's Privacy section.

Hosted-community Builderlab configuration is separate:

- `BUZZ_BUILD_BUILDERLAB_API_BASE_URL` continues to configure only Builderlab
  hosted-community management. It is not used for enterprise login.

## Desktop behavior

- Browser login callback: Desktop always supplies
  `http://127.0.0.1:{ephemeral_port}/callback/{nonce}`.
- Adapter session: Desktop checks or reuses an in-memory enterprise adapter
  session.
- Browser login failures: Desktop surfaces the callback's `error_description`
  when present, otherwise `error`.
- Session check failure: Desktop clears only the enterprise adapter session.
  Builderlab hosted-community state is not affected.
- `profile_projection` is ignored unless
  `BUZZ_BUILD_ENTERPRISE_PROFILE_PROJECTION` opts into publishing those fields
  as the user's public Buzz profile.
