# Agent lineage and portable grants

## Purpose and boundaries

Agents can create child agents without giving a broker the human's private key. The human at the root remains the agent's **manager** in the product. Parentage does not transfer human, channel, repository, or sibling privileges to an intermediate agent.

This specification introduces a versioned, signed agent grant and portable proof chains. Provenance is independent of communities; admission and enforcement remain local to each community. It replaces implicit delegation through ordinary NIP-OA endorsements with explicit delegation evidence.

Version 1 has three agent levels: human H at depth 0, then A, B, and C at depths 1, 2, and 3. Creation automatically assigns the remaining budget. There is no separate creation ACL, opt-in, or customer-facing budget setting.

The bounded lifecycle is intentional: grants are stable and non-expiring; disablement is community-local. Version 1 does not promise automatic renewal, transparent key rotation, globally synchronized revocation, or globally agreed resolution of conflicting histories. These limitations apply even when a grant is cryptographically valid.

## Identity, provenance, and authority

Keep three concepts separate:

- **Actor:** the key signing the request or event. An agent remains the author of its own work.
- **Provenance:** the verified parent, human root, depth, and particular grant chain under which an agent identity was accepted.
- **Local authority:** whether that principal may act in this community now, after admission, bans, disablement, and resource-access checks.

A receiving relay validates evidence; a supplied `root`, `depth`, owner profile field, or database projection is not proof. Root and depth are derived from the chain. A valid chain does not itself admit its root to a community or prove biological humanity.

A trusted root must be directly admitted through the community's trusted administrative path, currently eligible, and not already classified as an agent. Ordinary access to an open relay, direct membership of a known agent, and NULL ownership metadata do not establish a human root. These rules cannot prevent an unknown agent key from being misclassified by an administrator or provide Sybil resistance.

## Grant credential

`AGENT_GRANT` is a new, immutable, non-replaceable Nostr event kind. The event's `pubkey` is the issuer, its normal Nostr signature authenticates the grant, and its event ID identifies the exact authorization. The dedicated kind and signed content type/version separate this credential from NIP-OA and other events. No additional custom signature algorithm is required.

The signed content is a JSON object with this version-1 shape:

```json
{
  "type": "buzz:agent-grant",
  "version": 1,
  "subject": "<child public key>",
  "parent_grant": null,
  "remaining_delegations": 2
}
```

`parent_grant` is NULL only for a human-issued first-level grant. Otherwise it is the event ID of the grant authorizing the issuer. Exactly one `p` tag identifies the subject. Non-root grants also have one `e` tag marked `parent`, referencing `parent_grant`; root grants have no parent reference. Tags and content must agree. Reject malformed keys, duplicate content fields, ambiguous subject/parent tags, and unsupported versions.

There is no community ID, claimed root, claimed depth, resource permission, expiry, or renewal field. `created_at` is the Nostr event timestamp, not an authorization expiry. A historical grant does not fail merely because it is older than an AUTH request's freshness window.

For version 1, H's grant to A has budget 2; A's grant to B has budget 1; B's grant to C has budget 0. Each step must decrement exactly once. A zero-budget grant cannot authorize another grant. Arbitrary issuer-selected narrower budgets are outside version 1: explicit delegation does not introduce a second, per-agent policy dimension.

`AGENT_GRANT` and `AGENT_LINEAGE` below are symbolic kind names. Numeric allocations remain pending review in Buzz's event registry; clients must not independently choose numbers.

## Portable authentication

The leaf presents the complete ordered grant chain, starting with the human-issued grant and ending with its own grant. Intermediate agents do not have to connect to, become members of, or register separately with the receiving community first.

The leaf also proves possession of its own key through the existing NIP-42 or NIP-98 authentication mechanism. The signed authentication event contains:

- One `agent-grant` tag naming the leaf grant's event ID.
- One `agent-grant-chain` tag containing a compact JSON array of the complete signed grant events, in root-to-leaf order.

Thus the leaf's authentication signature binds the selected grant and supplied path. Parent-grant references independently bind every edge to the exact preceding authorization. Existing AUTH freshness, challenge, request URL/method, and applicable payload-binding checks still apply.

Version 1 permits at most three grant events and at most 4096 UTF-8 bytes in the serialized chain tag value. Enforce bounds before expensive verification. Reject extra, disconnected, reordered, or duplicate grants. Authentication selects exactly one mode: grant-chain authentication or legacy direct-agent authentication. An invalid grant proof must not fall back to legacy or ordinary-human authorization.

Full-chain presentation is the normal path and needs no separate credential-fetch round trip. SDKs retain and send the chain; relays may cache cryptographic verification by event ID. Caching must not bypass current admission, bans, disablement, or access checks. A WebSocket session retains its validated principal; it need not resend the chain with every message. A separately authenticated HTTP request supplies its proof.

## Shared verification and materialization

All authentication transports use one effective-principal resolver, with transport-specific signature verification at the boundary. Its result includes actor, credential mode, parent, root, depth, leaf grant ID, and local status where applicable. Display code is not an authorization resolver.

For a grant-chain request, the resolver must:

1. Verify the authenticated actor is the leaf subject and the signed leaf grant ID matches the proof.
2. Verify every Nostr event ID, signature, kind, version, and subject/parent tag agreement.
3. Require the first grant to be human-issued with no parent reference. For subsequent grants, require issuer = preceding subject and parent reference = preceding event ID.
4. Derive root and depth; require distinct keys throughout the path, the fixed 2/1/0 budgets, and depth at most 3.
5. Classify the principals before any direct-member shortcut. Require the root's current trusted admission in the requested community; reject known-agent roots and disabled or banned ancestry.
6. Compare the full path with any previously accepted provenance. Reject conflicting parentage, roots, grant bindings, or reclassification.
7. Atomically retain the verified grants and materialize missing provenance and local projections. A failed check or race must not leave a partially accepted chain.
8. Apply the operation's normal channel, repository, and other authorization checks to the actual actor and explicitly permitted human-root relationships.

A relay stores community-independent verified provenance for identities it has accepted, plus community-scoped admission and enforcement state. Communities need not share a database. Re-presenting the same proof is idempotent; it must not require the parent to authenticate first.

The compatibility projection `users.agent_owner_pubkey` continues to mean the human root, not the immediate parent. Immediate parent and leaf grant ID remain separate. Existing human-owner privileges must not silently transfer to an intermediate agent.

Known agents cannot authenticate as humans or escape lineage checks through a direct-member row. Admission of a grant-backed agent requires its eligible root even on an otherwise open relay. Operators of open relays therefore still need a trusted root-enrollment path for grant-backed agents.

## Stable bindings, conflicts, and lifecycle

Once accepted as verified provenance, an identity's parent, root, depth, and grant binding are immutable. Deleting an account, leaving a community, disabling an agent, or removing a projection does not erase that classification or allow depth to reset. A new event ID is not a transparent replacement for an accepted grant, even if its fields appear equivalent. Issuers and brokers persist and reuse the original signed grants on retries.

A signature proves issuance, not uniqueness. An issuer can sign conflicting grants. A relay rejects evidence that conflicts with its retained binding and reports a lineage conflict; it does not silently select a new root or reparent the identity. Two independent relays may initially accept different conflicting histories. Version 1 offers no global consensus or universal canonical-parent guarantee. Conflict resolution and key recovery require a separately specified protocol, not a first-seen rule presented as global agreement.

An authorized local disablement or ban of an agent blocks that agent and dependent descendants in that community. Withdrawal of the root's trusted membership blocks the tree there. If an administrative removal is intended to revoke an agent's admission, retain a disablement marker rather than merely deleting an optional member row. Ancestors need no direct-member rows, so absence of such a row alone is not revocation.

Disablement authority follows the community's owner/admin rules; parentage alone grants no new administrative privilege. Relevant authorization checks must observe effective disablement for subsequent operations. Proactive disconnection of existing sessions complements those checks; best-effort socket closure is not the sole enforcement mechanism.

Local disablement does not revoke a grant on other relays. Grants do not expire or automatically rotate in version 1. A newly contacted relay may accept a previously compromised grant if its own admission and disablement state permit it. Recovery may require local disablement on each affected relay and a new identity; there is no promised transparent subtree renewal. These are explicit security and operational limitations, not deferred implementation of a claimed global guarantee.

## Broker and client responsibilities

The initial human-issued grant is signed by the human's trusted client. A broker need not receive the human's private key. For subsequent creation, the managed broker generates and retains the child's key, signs the child grant using the parent's authorized signing identity, persists the grant chain, and returns an agent identity/runtime handle rather than the child's private key to the parent runtime.

This key-custody boundary prevents the managed parent runtime from retaining a child key through the creation interface. The credential itself cannot prove non-exportability or how an independent client stores keys. A broker with custody of those keys remains trusted for their use.

The broker enforces the verified parent's remaining budget and existing resource policy; there is no separate spawn ACL. Depth bounds nesting, not fan-out, total resource use, or the number of separately admitted roots. Applicable quotas must account for the human-rooted tree rather than restart at each child.

Clients must retain or recover the exact grants as well as the keys needed for signing. Export/recovery bundles include the proof chain or a reliable means of retrieving those exact events. Merely retaining a leaf private key is not a complete recovery contract. Supplying a chain reveals its ancestry to the receiver; portable provenance is not anonymous delegation.

SDKs provide shared chain construction, presentation, and error decoding. Servers distinguish invalid or unsupported proofs, unadmitted roots, disabled ancestry, conflicting lineage, and unavailable legacy delegation through existing transport error envelopes. Clients must not treat an authorization denial as permission to try a weaker identity mode.

## Legacy NIP-OA compatibility

Existing NIP-OA remains a direct-agent credential. An unconditional legacy endorsement does not silently become permission to create descendants and cannot serve as a parent grant. Existing direct-agent behavior continues under the relay's applicable legacy open/closed admission and access policy, including ordinary use on an open relay where the owner has no direct-member row.

Migration preserves existing agent classification and useful ownership projections. It does not fabricate proof, infer trusted root enrollment from NULL ownership, or elevate a historical owner projection into verified global ancestry. Records without a verified version-1 chain remain explicitly legacy; they cannot sponsor grant-backed descendants. This legacy state is not a reason to quarantine or break previously valid direct-agent access by itself.

Upgrading an existing direct agent requires a version-1 grant signed by its human through a trusted client. The client may issue it automatically when the human signing identity is available; an agent-key-only broker cannot issue that human grant. Acceptance still checks root admission and any established verified provenance. After acceptance, the identity uses grant-chain authentication and cannot downgrade to NIP-OA to evade its restrictions.

Legacy parsing and validation are boundary adapters into the shared principal model, not separate authorization implementations at each call site. Unverified historical projections must remain distinguishable from verified immutable bindings.

## Lineage discovery and manager display

Expose relay-materialized lineage as a relay-signed, addressable `AGENT_LINEAGE` Nostr event, queried through existing WebSocket or `POST /query` mechanisms. Its `d` tag identifies the community/subject pair and its `p` tag identifies the subject. Content contains community ID, subject, parent, root owner, depth, leaf grant ID, and status (`active`, `disabled`, or `legacy_direct`). Unestablished legacy fields are NULL rather than fabricated verified ancestry.

The client trusts only the configured community relay's projection, not an arbitrary event claiming that kind. A snapshot is display/discovery metadata, not an authorization credential; a stale snapshot cannot override current membership or disablement. Underlying signed grants remain the portable evidence. No endpoint-specific lineage JSON API is introduced.

One shared client resolver and formatter supplies every agent-manager display. Prefer the verified human root; retain an existing trusted server human-owner projection as a legacy display fallback. Never infer the manager from an arbitrary profile field or an authentication tag's immediate signer. If the human root cannot be established for display, show "Manager unavailable." A disabled agent can still display its historical human manager.

The manager label does not authorize an action. An intermediate parent never becomes the displayed human manager merely because it issued the immediate grant.

## Observable contract

A conforming implementation must demonstrate that:

- A leaf with a complete valid chain can authenticate at a community admitting its human root without intermediate agents registering there first.
- Depths 1–3 work; a fourth agent level, altered budget, invalid signature, substituted parent grant, cycle, or leaf/actor mismatch fails.
- A known agent cannot use direct membership, legacy fallback, or account deletion to reset identity or authority.
- Local root removal or ancestor disablement blocks dependent operations without erasing provenance or claiming revocation elsewhere.
- Conflicting bindings fail without partial materialization or silent reparenting.
- Legacy direct agents retain their existing permitted behavior, but need a human-issued grant before gaining descendant-creation authority.
- Profile, message, mention, and member displays consistently identify the human root without granting intermediate agents human-owner rights.
