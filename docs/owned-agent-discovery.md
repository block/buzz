# Authenticated owned-agent discovery

An owner-authored kind 30177 coordinate seeds discovery independently of local
runtime inventory and shared-channel membership. It is not ownership proof. The
latest agent kind 0 profile must have a valid envelope and exactly one valid
NIP-OA auth tag. Every signed condition is evaluated against event time, never
wall-clock time. The owner remains provenance, not the agent's author identity.

Owner policy coordinates must match that verified owner and have valid signed
envelopes. Invalid latest policy reserves the coordinate and fails closed: it
cannot revive a legacy permission. Missing policy retains existing OSS legacy
compatibility; marked builds require verified owner policy. This change does not
invent a missing-policy default or broaden policy audiences.

Membership comes from the latest relay-signed kind 39002 snapshot for each
channel. Known owned agents need not carry the cosmetic bot role. Ownership does
not fabricate membership: a nonmember may be discovered with empty channel_ids.
Selection queries constrain exact requested keys and destination. Relay authority
is obtained through the existing community-bound relay admission boundary.

This native change expands discovery only. Preparing an invitation and freshly
authorizing publication to its exact destination belong to the subsequent
mention-routing change; discovery alone does not authorize a message.

## Regression gates

- `commands/agent_discovery/relay_directory/owned_tests.rs`: signed loopback
  discovery with no local record, forged ownership exclusion, ordinary-role
  membership, wrong destination, revocation and unsupported latest policy.
- `nostr_convert/oa_profile_tests.rs`: signed envelope, duplicate/malformed auth,
  canonical conditions and signed-event time, wrong-owner policy, invalid latest
  policy, forged and stale membership.
- `nostr_convert/tests.rs`: signed conversion fixtures and OSS compatibility.
- `nostr_convert/runtime_policy_tests.rs`: known signed runtime status survives
  policy overlay; absent/unrecognized status, policy-only discovery, and an
  invalid or status-less latest runtime cannot fabricate or revive availability.

NIP-OA time conditions are not wall-clock expiry (see `docs/nips/NIP-OA.md`). Relay
membership and policy reads remain separate snapshots, not an atomic publication
transaction. Independent security review is required before landing.

## Presentation preserves evidence

Policy-only agents carry `status: "unknown"` through IPC. When the same agent has
an independently verified latest kind 10100 runtime record, its explicit
`online`, `away`, or `offline` status survives the owner-policy overlay. Missing
or unrecognized runtime status remains unknown, not the generic converter's
legacy offline default. Policy still controls ownership and permissions; claimed
runtime membership is not restored. An invalid latest policy still excludes the
agent rather than falling back to runtime permissions.

Discovery alone supplies no liveness evidence: Pulse and Projects omit the status
dot rather than inventing offline or active presence, and active-agent lookups
require positive evidence.
Channel and profile activity projections preserve unknown rather than claiming a
deployed observer; local managed runtime status still takes precedence.
Authenticated `ownerPubkey` is retained by New Message, member-add, and search
candidates even when no user-search profile duplicates the relay result.
