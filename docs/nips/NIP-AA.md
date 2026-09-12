NIP-AA
======

Agent Authentication
--------------------

`draft` `optional` `relay`

**Depends on**: NIP-OA (Owner Attestation), NIP-43 (Relay Access Metadata and Requests), NIP-42 (Authentication of Clients to Relays)

## Abstract

This NIP defines how a relay that implements NIP-43 relay membership SHOULD handle connection requests from agent keys that carry NIP-OA credentials. An agent whose ownership chain reaches an active relay root member within a relay-configured depth limit MAY gain implicit relay access — without being explicitly enrolled in the member list — by presenting a NIP-OA `auth` tag during NIP-42 authentication.

## Motivation

NIP-43 defines relay membership metadata; relays that enforce membership restrict access to an explicit member list. NIP-OA establishes that an owner key has authorized a specific agent key to act on its behalf. These two NIPs are complementary but disconnected: an operator who adds a human member must also separately enroll every agent that human runs.

This creates friction and a synchronization hazard. When a human's membership is revoked, their agents remain enrolled until manually removed. When a human spawns a new agent, it cannot connect until the operator adds it.

NIP-AA closes this gap. An agent presents its NIP-OA credential during NIP-42 authentication. The relay verifies the credential and resolves the ownership chain to an active root member. If the chain satisfies relay policy, the agent connects. If the root's membership is later revoked, the agent's next connection attempt fails automatically — no separate cleanup required.

## Terminology

This document uses MUST, MUST NOT, SHOULD, SHOULD NOT, MAY, and RECOMMENDED as defined in RFC 2119.

- **owner key**: The Nostr keypair that issued the NIP-OA authorization. The owner may itself be an agent.
- **agent key**: An AI agent, bot, or automation process with its own Nostr keypair. The agent need not be a relay member.
- **`auth` tag**: The NIP-OA credential tag `["auth", "<owner-pubkey-hex>", "<conditions>", "<sig-hex>"]`.
- **NIP-42 AUTH event**: A `kind:22242` event sent by a client in response to a relay's `AUTH` challenge.
- **virtual membership**: Connection access derived from root membership, with no persistent membership record created for the agent.
- **active member**: A pubkey is an *active member* if the relay's authoritative access-control state lists it as an unrevoked, current member with an explicit membership record. Virtual members (agents granted access via NIP-AA) are not active members. NIP-43 `kind:13534` events MAY advertise or reflect this state but are not themselves the authoritative source.
- **root member**: An active member independently admitted and classified as a non-agent by the relay's authoritative access-control state. Missing ownership evidence alone MUST NOT establish root status.
- **ownership depth**: The number of owner-to-agent edges from the root to the authenticating agent. Root → agent is depth 1; root → agent → child is depth 2.

Relays MUST enforce a finite, operator-configurable non-negative integer maximum ownership depth `n`. The default MUST be `n = 1` (direct agents only); `n = 0` disables NIP-AA virtual admission. Setting `n > 1` enables agents to authorize descendant agents up to that depth. Root membership and agent classification are relay-local policy, not claims established by a profile's absence.

## Protocol Flow

```
Agent                                  Relay
  |                                      |
  |<-- ["AUTH", "<challenge-string>"] ---|  (NIP-42 step 1)
  |                                      |
  |  Build kind:22242 event:             |
  |    pubkey    = agent_pubkey          |
  |    tags      = [                     |
  |      ["relay",     "wss://..."],     |
  |      ["challenge", "<nonce>"],       |
  |      ["auth", <owner-pubkey-hex>,    |
  |               <conditions>,         |
  |               <sig-hex>]            |
  |    ]                                 |
  |  Sign with agent secret key         |
  |                                      |
  |---- ["AUTH", <kind:22242 event>] -->|  (NIP-42 step 2)
  |                                      |
  |                   Verify NIP-42     |
  |                   Check member list |
  |                   Verify auth tag   |
  |                   Resolve ownership |
  |                                      |
  |<-- ["OK", "<event-id>", true, ""] --|  (access granted)
  |                                      |
  |  Subsequent events MAY carry auth   |
  |  tag per NIP-OA for provenance.     |
  |  NIP-AA membership is established   |
  |  by the AUTH event; the auth tag    |
  |  on subsequent events is not        |
  |  required for relay access.         |
```

On failure the relay MUST respond per the error prefix rules in the verification algorithm below. If the AUTH payload is too malformed to yield a parseable event id, the relay MUST close the WebSocket connection (optionally preceded by a `NOTICE` message). This is an explicit exception to NIP-42's requirement that AUTH messages be answered with `OK` — that requirement is impossible to satisfy without a parseable event id to reference. The relay MAY close the WebSocket on any AUTH failure but is not required to; an independently failed AUTH attempt does not implicitly invalidate prior authenticated identities on the connection. This rule does not prevent a relay from deliberately revalidating or terminating sessions for other reasons (e.g., owner membership revocation).

## Relay Verification Algorithm

When a relay receives a NIP-42 AUTH event (`kind:22242`), it MUST execute the following steps in order. Any failure MUST result in a rejected AUTH attempt. For Step 1 failures (malformed event, invalid `id`/`sig`, wrong `relay` tag, stale `created_at`), the relay MUST respond with `["OK", "<event-id>", false, "invalid: <reason>"]`. For Steps 2–5 failures (local disablement, missing or invalid credential, or rejected ownership chain), the relay MUST respond with `["OK", "<event-id>", false, "restricted: <reason>"]`. A failed NIP-AA AUTH attempt does not necessarily invalidate other authenticated pubkeys on the same WebSocket connection.

**Step 1 — Standard NIP-42 verification**

Verify the AUTH event per NIP-42: `event.kind` is `22242`, the event `id` and `sig` are valid for `event.pubkey`, the `relay` tag matches this relay's URL, and the `challenge` tag matches the nonce issued to this connection.

For NIP-AA authentication, the AUTH event's `created_at` MUST be within a relay-defined freshness window. A ±120-second window is RECOMMENDED. AUTH events outside this window MUST be rejected.

If any check fails, reject.

**Step 2 — Direct membership check**

If `event.pubkey` is locally disabled or denied access, reject. Otherwise, if it is a root member, grant access per the normal NIP-43 flow. Known agents MUST continue through Steps 3–6 even if explicitly enrolled; direct membership MUST NOT bypass ancestry validation. Relays MUST retain known-agent classification independently of profile availability; omitting an `auth` tag or removing a profile MUST NOT clear it.

**Step 3 — NIP-OA credential extraction**

For pubkeys not admitted by Step 2, inspect the AUTH event's tags for an `auth` tag. If no `auth` tag is present, reject. If more than one `auth` tag is present, reject.

**Step 4 — NIP-OA credential verification**

Verify the `auth` tag using the following NIP-AA-specific procedure. This procedure reuses NIP-OA's cryptographic construction but is NOT equivalent to full NIP-OA verification — `kind=` clauses are not evaluated here (see §Kind Conditions).

1. The tag MUST have exactly four elements.
2. `<owner-pubkey-hex>` MUST be a valid 64-character lowercase hex BIP-340 public key.
3. `<sig-hex>` MUST be a valid 128-character lowercase hex string.
4. `<owner-pubkey-hex>` MUST NOT equal `event.pubkey` (no self-attestation).
5. `<conditions>` MUST be a syntactically valid NIP-OA conditions string (see NIP-OA §The Tag).
6. Reconstruct the preimage: `nostr:agent-auth:` || `event.pubkey` || `:` || `<conditions>`. The `<conditions>` string MUST be used verbatim from the `auth` tag — implementations MUST NOT reorder, deduplicate, normalize, or canonicalize the conditions before computing the preimage.
7. Compute `SHA256(preimage)`.
8. Verify `<sig-hex>` as a BIP-340 Schnorr signature over the SHA256 hash using `<owner-pubkey-hex>`.
9. Evaluate any `created_at<t` and `created_at>t` clauses against the AUTH event's `created_at` field. If the AUTH event does not satisfy a timestamp clause, reject.

If any check fails, reject.

**Step 5 — Ownership chain check**

Start with the verified `<owner-pubkey-hex>`, depth 1, and a visited set containing `event.pubkey`. Repeat:

1. If the depth exceeds `n`, the current owner is already visited, or the current owner is locally disabled or denied access, reject. Add the current owner to the visited set.
2. If the current owner is a root member, the chain is complete. A known agent MUST NOT terminate the chain, even if it has an explicit membership record.
3. Otherwise, resolve that owner's latest available `kind:0` metadata event using NIP-01 replaceable-event ordering over the relay's available evidence. Verify the event's `id`, `sig`, and `pubkey`, and its single `auth` tag under NIP-OA. Missing or invalid evidence MUST cause rejection, not root classification or fallback to an older profile.
4. This intermediate `auth` tag MUST have empty conditions. A conditional endorsement MUST NOT authorize descendant admission: this NIP does not define composition of ancestor kind or timestamp restrictions. The authenticating agent's own credential retains Step 4 and §Kind Conditions handling.
5. Follow the verified tag's owner pubkey and increment the depth.

Relays MUST bound total lookup and verification work as well as depth, and reject if that budget is exhausted. Profile discovery is implementation-defined; intermediates need not hold active sessions. Resolution uses available evidence, not a claim of globally latest profiles or immutable parentage.

**Step 6 — Grant virtual membership**

Grant the agent virtual membership for the pubkey in `event.pubkey` of the successful AUTH event. MUST NOT create a persistent membership record for the agent. Record the authenticating key and every non-root key in the verified path as known agents. The relay MUST retain the verified ownership path, including the immediate owner and root pubkeys, in the virtual session state for the duration of the connection, to support ancestor-scoped session enumeration, termination, and root-scoped quota aggregation. The agent's access is virtual, derived from the root's membership, and scoped to that specific pubkey — not to the WebSocket connection as a whole. If the connection has multiple authenticated pubkeys (per NIP-42), virtual membership applies only to the pubkey that completed NIP-AA authentication.

If the same agent pubkey completes NIP-AA authentication again on the same connection (e.g., with a different `auth` credential), the relay MUST replace the previously stored credential and ownership path with the newly verified ones. The relay MUST NOT combine credentials from multiple AUTH events for the same pubkey.

### Kind Conditions

`kind=` clauses in the NIP-OA credential are NOT evaluated at connection admission and do not affect whether the relay grants access. This applies to the authenticating agent's credential only; intermediate credentials MUST be unconditional (Step 5). They are a signal of the owner's intent — a declaration of which event kinds the owner intended to authorize — but the relay's enforcement is at the connection level.

**Credential scope warning**: An `auth` tag presented during NIP-42 authentication grants connection-level access regardless of any `kind=` clauses in the credential. Owners SHOULD be aware that issuing any valid `auth` tag — even one with narrow `kind=` conditions — grants the agent full relay-level read and write access unless the relay implements optional per-event enforcement.

Owners who intend to restrict agents to specific event kinds MUST ensure the relay enforces per-event `kind=` restrictions (see enforcement paragraph below), and SHOULD NOT rely on `kind=` clauses alone for access control. A credential issued for event-provenance purposes (e.g., `kind=1`) becomes a relay-login credential when used in NIP-AA; this semantic expansion is by design.

A relay that enforces `kind=` restrictions MUST retain the verified `auth` credential from the AUTH event for the duration of the connection and evaluate every `kind=` clause from that credential against each event where `event.pubkey` matches the virtual member's pubkey before accepting, storing, or forwarding it. This per-event enforcement applies only to `kind=` clauses. The `created_at<` and `created_at>` clauses are evaluated at connection admission (Step 4) and are not re-evaluated against subsequent events. When per-event enforcement rejects an `EVENT`, the relay MUST respond with `["OK", "<event-id>", false, "restricted: <reason>"]`.

Multiple `kind=` clauses in a single credential are conjunctive per NIP-OA: an event must satisfy every clause. A credential with conditions `kind=1&kind=7` authorizes no single event, since no event can have two different `kind` values simultaneously. Owners SHOULD use a single `kind=` clause per credential. Authorizing multiple event kinds requires either separate credentials on separate connections (since NIP-AA accepts exactly one `auth` tag per AUTH event) or an unconstrained credential with no `kind=` clause.

## Virtual Member Privileges

An agent granted virtual membership via NIP-AA MAY pass relay-level membership checks, including both read (subscriptions) and write (event publishing) access. Channel-level, group-level, quota, and role checks MUST continue to evaluate the agent's own pubkey (`event.pubkey`) unless another specification explicitly defines owner inheritance. NIP-AA does not grant the agent any ancestor's channel memberships, group roles, or administrative privileges.

For `EVENT` submissions, the relay MUST verify that `event.pubkey` is an authenticated pubkey on the connection that holds active or virtual membership; events from unauthenticated or non-member pubkeys MUST be rejected. For `REQ`, `COUNT`, and other non-`EVENT` operations, relay-level access MUST pass if at least one authenticated pubkey on the connection holds active or virtual membership. Channel-level, group-level, and resource-scoped access checks MUST evaluate the specific pubkey that holds virtual membership — not the owner's pubkey. When multiple pubkeys are authenticated on a single connection, the relay MUST NOT combine their privileges; each pubkey's access is evaluated independently. A resource-scoped operation passes only if at least one authenticated pubkey independently satisfies all required relay-level and resource-level checks for that operation.

Relays SHOULD aggregate rate limits and quotas by root pubkey across all virtual members derived from that root, in addition to per-agent-pubkey enforcement. Without root-scoped aggregation, a single member can mint many agent keys and multiply per-pubkey quotas.

Virtual members MUST NOT be granted relay administration privileges. The specific mechanism for restricting administrative access is implementation-defined. For example, an implementation might assign a restricted role that excludes admin operations, or it might check virtual membership status before processing admin commands.

Virtual members MUST NOT be permitted to modify relay membership (add or remove members). Issuing a child's `auth` tag within the configured depth limit grants only eligibility for virtual admission, not permission to enroll an explicit member.

Implementations SHOULD identify virtual members as such in relay audit logs and any membership introspection APIs.

## Revocation Semantics

Virtual membership, including the full ownership path and current relay policy, is checked on each authentication, not cached across reconnects. A later profile replacement affects the next authentication; it does not silently change an established session's retained path.

**Root removal or ancestor disablement**: When a root's membership is revoked or any ancestor is locally disabled, all agents whose resolved paths include that key will fail Step 5 on their next authentication attempt. Active sessions are not forcibly terminated; they continue until the underlying WebSocket connection closes. Operators who require immediate session termination MUST disconnect active WebSocket connections when revoking a root or disabling an ancestor. The relay SHOULD expose a mechanism to enumerate and terminate sessions by any ancestor pubkey.

**Auth tag expiry**: If the `auth` tag's conditions include a `created_at<t` clause, the relay evaluates that clause against the AUTH event's `created_at` field at connection time (step 4). This constrains the AUTH event's self-declared `created_at` field. It provides a bounded authorization window only when combined with relay-enforced AUTH event freshness (see Step 1). Auth-tag condition evaluation occurs only at connection admission (Step 4). The relay does not re-evaluate conditions during an active session unless it implements explicit session revalidation.

> **Note**: `created_at` is agent-controlled. A misbehaving agent can set `created_at` to any value. Operators who require hard wall-clock expiry MUST enforce it independently. Issuing `auth` tags with short `created_at<` windows and rotating them provides bounded authorization only because Step 1 requires the AUTH event's `created_at` to be within the relay's freshness window — preventing the agent from backdating past an expired condition.

**Agent key compromise**: An agent that possesses a valid `auth` tag can reconnect as long as its ownership chain remains admissible and any `created_at` conditions in its own credential are satisfied. Revocation requires one of: (a) removing the root from the relay's member list, (b) the `auth` tag's `created_at` conditions expiring, or (c) the relay applying an independent denylist to the agent or an ancestor. Removing an intermediate agent's explicit membership is not sufficient: virtual admission does not require that record. NIP-OA credentials are reusable capabilities — the owner cannot unilaterally revoke a previously issued `auth` tag without one of these mechanisms.

## Security Considerations

**Replay prevention**: The NIP-42 AUTH event is bound to a specific relay challenge nonce and cannot be replayed across sessions. The NIP-OA `auth` tag within it is a reusable credential — any holder of the agent's secret key can construct a new AUTH event carrying the same `auth` tag. This is by design: NIP-OA credentials are capabilities, not one-time tokens. Implementers MUST enforce NIP-42 challenge freshness. Because NIP-AA's replay prevention depends entirely on NIP-42 challenge quality, relays implementing NIP-AA SHOULD use cryptographically unpredictable, connection-unique challenge strings.

**Credential scope**: The `auth` tag is not bound to a specific relay or purpose. An agent that connects to multiple relays presents the same `auth` tag at each; a credential issued for event provenance is equally valid for NIP-AA relay admission. Operators SHOULD use `created_at<` conditions to limit the authorization window when appropriate.

**Chained authority**: Enabling `n > 1` allows existing unconditional endorsements to sponsor descendants without a new root signature. This is an intentional expansion of admission authority; there is no signed per-agent delegation budget or opt-out in this format. A compromised intermediate key can authorize descendants within the relay's cap. Empty intermediate conditions provide no expiry, so local disablement or root removal is needed to stop that authority. Profile replacement is not global credential revocation, and a child's credential does not bind its owner to a particular ancestor profile.

**Owner key exposure**: The owner pubkey is visible in the `auth` tag on the AUTH event. This links the owner and agent identities to any relay that processes the connection. See §Privacy Considerations.

**Self-attestation**: An `auth` tag where `<owner-pubkey-hex>` equals `event.pubkey` MUST be rejected (step 4). This prevents an agent from bootstrapping its own access by signing its own credential.

**Forged credentials**: The relay verifies the Schnorr signature in step 4. A forged `auth` tag (wrong signature) fails cryptographic verification. An `auth` tag whose ownership chain does not reach an active root within the configured limit fails Step 5. Neither attack grants access.

**Kind=overbroad**: Because `kind=` conditions are not enforced at the connection level, a credential issued with `kind=1` conditions grants the same connection-level access as an unconstrained credential. Operators who require kind-level restrictions MUST implement optional per-event enforcement (see §Kind Conditions).

## Privacy Considerations

Presenting an `auth` tag during NIP-42 authentication discloses the owner-agent relationship to the relay. The relay learns that `<owner-pubkey-hex>` authorized `event.pubkey` (the agent), and chained admission discloses the resolved ancestors. Publishing endorsements on `kind:0` profiles also makes those links discoverable to profile readers. This is an intentional disclosure — the relay needs this information to perform the membership check.

Relays SHOULD NOT expose the owner-agent relationship to other relay members beyond what is necessary for virtual member identification.

Known agents cannot avoid ancestry validation by omitting the `auth` tag and relying on explicit membership enrollment (Step 2).

## Verification Examples

The following examples use the NIP-OA test keys:

```text
owner_secret = 0000000000000000000000000000000000000000000000000000000000000001
owner_pubkey = 79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798

agent_secret = 0000000000000000000000000000000000000000000000000000000000000002
agent_pubkey = c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5
```

The NIP-OA `auth` tag (from NIP-OA test vectors, conditions `kind=1&created_at<1713957000`):

```text
["auth",
 "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
 "kind=1&created_at<1713957000",
 "8b7df2575caf0a108374f8471722b233c53f9ff827a8b0f91861966c3b9dd5cb2e189eae9f49d72187674c2f5bd244145e10ff86c9f257ffe65a1ee5f108b369"]
```

The cryptographic verification of this tag (preimage, SHA256, and signature) is covered by the NIP-OA test vectors. The examples below describe expected relay behavior for various scenarios; they are not independently verifiable without a complete NIP-42 event `id` and `sig`.

### Accept: agent connecting with valid NIP-OA credential

**Conditions**: `owner_pubkey` is a root member and `n >= 1`. AUTH event `created_at = 1713956400`. Relay wall-clock time is assumed to be near `1713956400` (within the ±120-second freshness window). The `created_at<1713957000` condition is satisfied.

- Step 1: NIP-42 verification passes; `created_at` within freshness window.
- Step 2: `agent_pubkey` is not in member store → continue.
- Step 3: Exactly one `auth` tag found → continue.
- Step 4: Tag has four elements; `owner_pubkey` is valid; `owner_pubkey` ≠ `agent_pubkey`; conditions string is syntactically valid; Schnorr signature verifies; `created_at<1713957000` is satisfied by `1713956400` → pass.
- Step 5: `owner_pubkey` is an active root at depth 1 → pass.
- Step 6: Agent pubkey granted virtual membership.

### Reject cases

Relays MUST reject each of the following:

| Scenario | Failing Step |
|----------|-------------|
| `auth` tag signature is invalid (wrong owner key) | Step 4 |
| `auth` tag `<owner-pubkey-hex>` equals `event.pubkey` | Step 4 |
| `auth` tag has fewer or more than four elements | Step 4 |
| `auth` tag `<conditions>` is malformed (e.g., `kind=01`) | Step 4 |
| AUTH event `created_at` is `1713957001` with conditions `created_at<1713957000` | Step 4 |
| AUTH event `created_at` is outside relay freshness window | Step 1 |
| The ownership chain does not reach an active root within `n` edges | Step 5 |
| A required intermediate profile is missing, invalid, or has zero or multiple `auth` tags | Step 5 |
| An intermediate `auth` tag has non-empty conditions | Step 5 |
| An ownership key repeats, an ancestor is disabled, or the work budget is exhausted | Step 5 |
| AUTH event has two `auth` tags | Step 3 |
| AUTH event has no `auth` tag and `agent_pubkey` is not a root member | Step 3 |
| Virtual member submits a relay membership admin command (e.g., add/remove member) | Virtual Member Privileges (post-admission) |

### Chained admission examples

Assume root `H` is independently admitted, agent `A` publishes a valid `kind:0` carrying an unconditional endorsement from `H`, and child `B` authenticates with a valid endorsement from `A`. All keys are distinct and not locally disabled.

| Scenario | Result |
|----------|--------|
| `B` authenticates with `n = 2` | Accepted: `H → A → B` has two edges |
| `B` authenticates with `n = 1`, even if `A` is explicitly enrolled | Rejected: an agent is not a root; depth exceeds `n` |
| `A`'s profile endorsement has `kind=0` or a timestamp condition | `B` rejected: intermediate endorsements must be unconditional |
| `H` is revoked or `A` is locally disabled | `B` rejected on its next authentication |
| `B` later authorizes `C`, with `n = 2` | `C` rejected: three edges exceed `n` |
| Any agent authenticates with `n = 0` | Virtual admission rejected |

These are normative behavior examples, not executable test vectors.

### Kind enforcement examples

The following examples illustrate optional per-event `kind=` enforcement behavior. The credential used has conditions `kind=1&created_at<1713957000`.

| Scenario | Enforcement enabled? | Result |
|----------|---------------------|--------|
| Virtual member publishes `kind:1` | No | Accepted |
| Virtual member publishes `kind:7` | No | Accepted (connection-level access only) |
| Virtual member publishes `kind:1` | Yes | Accepted (`kind=1` clause satisfied) |
| Virtual member publishes `kind:7` | Yes | Rejected (`kind=7` not in credential) |

## Relation to Other NIPs

**NIP-42**: NIP-AA extends the NIP-42 AUTH flow. The `kind:22242` event is the credential presentation vehicle. NIP-AA adds no new event kinds.

**NIP-OA**: NIP-AA consumes NIP-OA credentials at the relay connection layer. NIP-OA defines the `auth` tag format, signing preimage, and conditions grammar. NIP-AA defines what a relay does with that tag during NIP-42 authentication. NIP-AA's step 4 reuses NIP-OA's cryptographic construction but applies it selectively: `kind=` clauses are not evaluated at connection admission. This is a deliberate divergence from NIP-OA's "verifiers MUST evaluate every clause" rule, which applies to event-level verification, not connection admission.

**NIP-43**: NIP-AA is an extension to NIP-43 (Relay Access Metadata and Requests). Relays that do not implement NIP-43 have no membership concept and SHOULD NOT implement NIP-AA. Relays that implement NIP-43 MAY implement NIP-AA; it is not required.

**NIP-26**: NIP-OA reuses NIP-26's credential format but not its semantics. NIP-AA inherits this distinction. An `auth` tag MUST NOT be interpreted as NIP-26 delegation. The agent remains the sole author of its events.
