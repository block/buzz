NIP-OA
======

Owner Attestation
-----------------

`draft` `optional`

This NIP defines an optional `auth` tag by which an owner key authorizes an agent key to publish events under the agent's own authorship.

## Motivation

NIP-26 defines a sound Schnorr-signature mechanism for proving that one key authorized another key subject to explicit conditions.
NIP-26 assigns the event to the delegator semantically, and that semantic MUST NOT be reused for agent provenance.
This NIP reuses NIP-26 as prior art for the credential format and signing flow and defines the credential as authorization evidence only.
A valid `auth` tag is a reusable capability: the same tag MAY appear on multiple events by the same agent key provided each event satisfies the conditions.
An event that includes a valid `auth` tag remains authored by `event.pubkey`.

## Non-Goals

This NIP does not define impersonation.
This NIP does not define key derivation.
This NIP does not define relay-side author rewriting.

## The Tag

Events MAY include zero or one `auth` tag.
If an event contains more than one `auth` tag, verifiers and clients MUST treat the event as having no valid `auth` tag.
Agents MAY publish events without an `auth` tag.
Agents that require provenance to be respected by verifiers SHOULD include a valid `auth` tag.
The `auth` tag MUST contain exactly four elements:

```json
["auth", "<owner-pubkey-hex>", "<conditions>", "<sig-hex>"]
```

- `<owner-pubkey-hex>`: 64-character lowercase hex encoding of the owner's 32-byte x-only public key as defined in BIP-340.
- `<conditions>`: UTF-8 string containing zero or more clauses separated by `&`.
- `<sig-hex>`: 128-character lowercase hex encoding of the 64-byte Schnorr signature.

An `auth` tag with fewer or more than four elements is malformed and MUST be rejected.

The signing preimage is the UTF-8 byte sequence of `nostr:agent-auth:` || `event.pubkey` || `:` || `<conditions>`.
The domain separator string is exactly `nostr:agent-auth:`.
The signed message is `SHA256(preimage)`.
The owner MUST produce `<sig-hex>` as a BIP-340 Schnorr signature over the signed message with the owner's secret key.

Each clause in `<conditions>` MUST be one of:

- `kind=<decimal>`
- `created_at<unix-timestamp>`
- `created_at>unix-timestamp`

The `<conditions>` string MUST be either the empty string or a non-empty ASCII string of the form `clause` or `clause&clause&...`.
Whitespace is not permitted anywhere in `<conditions>`.
Empty clauses are invalid.
Clause names and operators are case-sensitive and MUST appear exactly as specified above.
A trailing `&`, a leading `&`, or `&&` is malformed and MUST be rejected.

The decimal encoding in a clause MUST be canonical base-10 with no leading zeroes except `0`.
Values in `kind=` clauses MUST be in the range `0` to `65535`.
Values in `created_at<` and `created_at>` clauses MUST be in the range `0` to `4294967295`.
An empty `<conditions>` string imposes no additional event constraints.
Verifiers MUST evaluate every clause.
Verifiers MUST reject an `auth` tag that contains an unsupported clause, malformed decimal encoding, invalid public key, or invalid signature.
If `<owner-pubkey-hex>` equals `event.pubkey`, the `auth` tag is invalid and MUST be rejected.
An event satisfies `kind=<n>` if and only if `event.kind = n`.
An event satisfies `created_at<t>` if and only if `event.created_at < t`.
An event satisfies `created_at>t` if and only if `event.created_at > t`.
Clause order is part of the signed preimage and verifiers MUST use the exact `<conditions>` string from the tag when verifying the signature.
Implementers MUST NOT reorder, deduplicate, normalize, or canonicalize the `<conditions>` string before computing the preimage.
Verifiers MUST NOT reinterpret a valid `auth` tag as an identity override.

## Relay Behavior

Relays require no changes to support this NIP.
Relays MAY store, index, and forward the `auth` tag as any other event tag.
Relays MUST NOT rewrite event authorship on the basis of an `auth` tag.
Relays MUST NOT be required to verify an `auth` tag.

## Client Behavior

Clients MUST validate the event according to the core Nostr event rules, including that `id` and `sig` are valid for `event.pubkey`, before treating an `auth` tag as verified provenance.
A valid `auth` tag on an otherwise invalid event does not establish provenance.
Clients that process an `auth` tag SHOULD verify the owner signature and the conditions against the event.
Clients MUST treat the agent key in `event.pubkey` as the only author key for the event.
Clients MUST NOT display the owner key as the author of the event solely because of a valid `auth` tag.
Clients MUST NOT merge the event into owner-authored timelines, author indexes, or pubkey-filtered results for the owner solely because of a valid `auth` tag.
Clients SHOULD display provenance only when the `auth` tag verifies successfully, and any such display MUST be clearly distinguished from authorship (for example, "authorized by \<owner\>").
Clients SHOULD ignore an invalid `auth` tag for protocol purposes.
Clients MUST NOT display owner provenance when the `auth` tag is invalid.

## Security Properties

The owner key and the agent key are independent keys.
Compromise of the agent secret key MUST NOT imply compromise of the owner secret key.
Compromise of the agent secret key permits only signatures by the compromised agent key.
Owners SHOULD bound authorization lifetime with a `created_at<...` clause when revocation latency matters.
Owners MAY revoke future authorization by refusing to issue new `auth` tags.
A `created_at<...` or `created_at>...` clause constrains the event's self-declared `created_at` field, which the agent controls.
These clauses do not enforce wall-clock expiry; a misbehaving agent can backdate `event.created_at` to satisfy an expired window.
Relays or clients that require wall-clock freshness MUST enforce it independently of this NIP.
Verification MUST NOT depend on the verifier's local clock, receipt time, or relay storage time.

## Privacy Considerations

Including an `auth` tag intentionally links the owner key and the agent key.
Verifiers MAY correlate all events that reuse the same owner key and agent key pair.
Agents that omit the `auth` tag avoid this disclosure but also omit the provenance claim defined by this NIP.

## Test Vectors

The following vector uses `owner_secret = 0000000000000000000000000000000000000000000000000000000000000001`.
The corresponding `owner_pubkey` is `79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798`.
The following vector uses `agent_secret = 0000000000000000000000000000000000000000000000000000000000000002`.
The corresponding `agent_pubkey` is `c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5`.

```text
conditions=kind=1&created_at<1713957000
preimage=nostr:agent-auth:c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5:kind=1&created_at<1713957000
sha256(preimage)=08cdecd55af4c28d3801fd69615dcf5cc04fab3bc134b38a840bf157197069a6
auth_sig=8b7df2575caf0a108374f8471722b233c53f9ff827a8b0f91861966c3b9dd5cb2e189eae9f49d72187674c2f5bd244145e10ff86c9f257ffe65a1ee5f108b369
tag=["auth","79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798","kind=1&created_at<1713957000","8b7df2575caf0a108374f8471722b233c53f9ff827a8b0f91861966c3b9dd5cb2e189eae9f49d72187674c2f5bd244145e10ff86c9f257ffe65a1ee5f108b369"]
tag-bytes-hex=5b2261757468222c2237396265363637656639646362626163353561303632393563653837306230373032396266636462326463653238643935396632383135623136663831373938222c226b696e643d3126637265617465645f61743c31373133393537303030222c223862376466323537356361663061313038333734663834373137323262323333633533663966663832376138623066393138363139363663336239646435636232653138396561653966343964373231383736373463326635626432343431343565313066663836633966323537666665363561316565356631303862333639225d
```

## Signed Event Example

```json
{
  "id": "d892a65e7677e0554ebb70ee16deeb6a0727dba46450fb4bc001291d7bff971b",
  "pubkey": "c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5",
  "created_at": 1713956400,
  "kind": 1,
  "tags": [["auth", "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798", "kind=1&created_at<1713957000", "8b7df2575caf0a108374f8471722b233c53f9ff827a8b0f91861966c3b9dd5cb2e189eae9f49d72187674c2f5bd244145e10ff86c9f257ffe65a1ee5f108b369"]],
  "content": "owner-attested agent event",
  "sig": "7fd38992b70b5e9e113644e51b4c8ee2227f3bdd402b1855f8786c0600394ab3ec2621742a7bad0b0000b93d4d1ae6e39525f286a3c1029f43f46c3359a6c76f"
}
```

## Invalid Test Vectors

Verifiers MUST reject each of the following:

- An event containing two `auth` tags.
- An `auth` tag with fewer or more than four elements.
- An `auth` tag whose `<conditions>` string is `kind=1&` (trailing delimiter).
- An `auth` tag whose `<conditions>` string is `kind=01` (leading zero).
- An `auth` tag whose `<owner-pubkey-hex>` equals `event.pubkey` (self-attestation).
- An otherwise well-formed `auth` tag attached to an event whose Nostr `id` or `sig` is invalid.
