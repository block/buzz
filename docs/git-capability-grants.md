# Explicit Git Capability Grants

`experimental`

Buzz normally derives Git push authority from the pusher's channel role. A
repository announcement can opt in to `explicit-grants-v1` when channel
membership must remain useful for communication without also authorizing every
member or bot to write the repository.

This mode is a narrow authorization layer in front of the existing branch
protections. It does not replace `no-force-push`, `no-delete`, or
`require-patch` rules.

## Tag grammar

The signed kind:30617 repository announcement selects the policy and binds a
Nostr actor to a repository ref pattern:

```text
["buzz-git-policy", "explicit-grants-v1"]
["buzz-git-grant", "<lowercase-pubkey-hex>", "<ref-pattern>", "push", "<expires-at-unix>"]
["buzz-protect", "refs/**", "push:owner"]
```

The `buzz-protect` baseline is mandatory. A relay that predates explicit grants
ignores the two new tag types but understands this owner-only protection, so an
ordinary channel member or bot fails closed instead of inheriting write access.

## Enforcement

- The NIP-98-authenticated pusher must still belong to the repository channel,
  except for the existing repository-owner and managed-agent-owner identity
  relationships.
- In explicit mode, channel membership and ownership are necessary identity
  bindings but are not write authorization. Every pusher, including a
  repository owner or managed-agent owner, needs a current grant.
- Every ref in one atomic push must match a grant for the authenticated actor.
  One uncovered ref rejects the entire push.
- A grant expires at the Unix timestamp in its tag. Expiry is exclusive.
- A repository announcement may contain at most 50 grants. Every grant must be
  valid for no more than 24 hours from the announcement's signed `created_at`
  timestamp. A future announcement timestamp or any invalid grant lifetime
  rejects the push.
- A grant satisfies only the role threshold for the matching refs. The relay
  still evaluates all non-role branch-protection rules.
- The `push` capability covers create, fast-forward, non-fast-forward, and
  delete updates inside the granted pattern. Operators that want append-only
  task branches must add `no-force-push` and `no-delete`; `require-patch`
  continues to block every direct push.
- Repositories without `buzz-git-policy` keep the legacy channel-role behavior.
  A grant tag without a policy tag, duplicate policy tags, or an unknown policy
  version fails closed.

## Compatibility boundary

The owner-only fallback protects ordinary members and bots on older relays. It
cannot stop an older relay from honoring its existing repository-owner or
managed-agent-owner bypass. Operators must therefore verify the relay version
before publishing a repository announcement that opts in to explicit grants.
Removing or replacing the kind:30617 announcement revokes the signed grants;
there is no separate unsigned grant store.

This compatibility boundary makes the policy safe to evaluate in a controlled
pilot, but it is not a signal to activate it in production without positive
and negative authorization tests against the exact deployed relay image.
