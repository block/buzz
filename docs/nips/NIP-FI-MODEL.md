NIP-FI-MODEL
============

Composed authorization model (non-normative)
--------------------------------------------

This companion is explanatory. It defines no requirement, invariant, wire
value, denial mapping, or conformance claim. Normative requirements live in
[NIP-FI](NIP-FI.md) and the claimed profile documents. In particular,
`FI-INV-01` through `FI-INV-16` are defined only by NIP-FI core.

## State sketch

One useful implementation model keeps these authoritative relations per domain:

```text
B_D : active identity-to-key relation
T_D : retired identity/key pairs
Y_D : revoked keys
H_D : immutable lifecycle history
V_D : binding and lifecycle versions
```

NIP-FI-LIFECYCLE adds disabled identities and pending replacement lineage.
NIP-FI-DELEG adds relationship state but no delegate binding. NIP-FI-EDGE adds
transport-provenance and replay witnesses. Implementations may use different
storage as long as their observable behavior satisfies the owning normative
documents.

## Composed direct decision

The core decision can be read as this equation:

```text
validated issuer-qualified identity
+ fresh request/connection-bound Nostr proof
+ current durable partial-bijection state
+ current local policy
+ atomic final admission
= authority for exactly the proven key and operation
```

Preparation gathers immutable evidence and snapshots every dependency without
mutation. Final admission compares exact context and stable contract identities,
checks all deadlines, revalidates changed snapshots, recomputes from current
binding and policy state, then commits replay claims, optional enrollment, and a
receipt atomically. The special concurrent-enrollment normalization is narrow:
an `enroll(i,k)` proposal may become the same eligible `existing(i,k)` result;
a different winner is not equivalent.

## Profile composition

Profiles contribute witnesses, never alternate final authority:

```text
core witnesses
∪ EDGE provenance/replay witnesses
∪ LIFECYCLE eligibility/lineage witnesses
∪ DELEG owner/relationship witnesses
```

The lease deadline is the minimum of every bound in the resulting set. A missing
or unreadable required witness denies. A profile cannot remove a core witness,
extend a core deadline, replace the proven actor, or create a second admission
lineage.

For direct authorization, the path dependency is the normalized assertion and
its current snapshot/status witnesses. For delegated authorization, it is the
exact eligible owner binding plus relationship evidence; direct assertion fields
are absent. Both paths share context resolution, Nostr-proof validation, local
policy, read-only preparation, and atomic final admission.

## Lifecycle intuition

Bindings are durable; leases are ephemeral. Retirement makes one exact pair
permanently ineligible for ordinary recreation. Revocation makes a key
ineligible throughout the domain. Rotation retires the old pair and creates a
new binding version but does not globally revoke the old key. Extended lifecycle
operations may add disabled identity and one-shot pending-lineage state, as
specified by NIP-FI-LIFECYCLE.

## Privacy intuition

Private reasons collapse to fixed public bytes. In particular, binding
conflicts, tombstones, lifecycle gates, key mismatch, enrollment requirements,
and local-policy decisions are indistinguishable. Operational diagnostics may
retain bounded private reason codes, but such records are not protocol objects
and never become authorization witnesses.

## Reading order

1. NIP-FI for core state, wire behavior, invariants, and direct admission.
2. NIP-FI-EDGE for a trusted-enterprise edge.
3. NIP-FI-LIFECYCLE for provisioning, disablement, and re-enablement.
4. NIP-FI-DELEG for delegated agents.
5. NIP-FI-CONF for claim and evidence rules.
