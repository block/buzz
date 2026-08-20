# NIP-FI-EDGE: Trusted Edge Profile

`draft` `optional`

## Scope

This profile lets a trusted enterprise edge deliver federated assertion evidence to
a NIP-FI verifier. It defines two constructions:

- `trusted-proxy-hmac-v2`, a portable request-bound HMAC envelope; and
- a private authenticated-edge assertion adapter, for platforms that provide an
  equivalent closed trust boundary without the stock envelope.

NIP-FI-EDGE is optional. A deployment can implement NIP-FI core using only
`client-attached`. Claiming this profile does not weaken core assertion validation,
independent Nostr proof, binding, lifecycle, policy, final-admission, or lease rules.
The key words **MUST**, **MUST NOT**, **REQUIRED**, **SHALL**, **SHALL NOT**,
**SHOULD**, **SHOULD NOT**, **RECOMMENDED**, **NOT RECOMMENDED**, **MAY**, and
**OPTIONAL** are to be interpreted as described in BCP 14 when, and only when,
they appear in all capitals.

Every identifier this document serializes on the wire — header names, the
profile identifier, provenance envelope fields, and proof transport codes — is
interoperability-critical. `transport_contract_id` remains deployment-local as
core classifies it: its value is opaque outside a deployment, while the
canonical contract semantics this profile contributes to it are normative and
fixed here. Local adapter revision identifiers and key identifiers are
deployment-local and MUST NOT appear in public discovery.

## Common trusted-edge requirements

Server-owned listener, route, and authorization-domain configuration selects exactly
one edge profile before protected traffic is accepted. Request evidence cannot
select, negotiate, or downgrade that profile. Missing, repeated, comma-combined,
malformed, oversized, mixed-profile, or profile-inconsistent evidence denies without
fallback to `client-attached` or another edge profile.

Every trusted edge MUST:

1. strip every inbound copy of each `Nostr-Federated-Identity`,
   `Nostr-Federated-Identity-Provenance`, and `Nostr-Federated-Identity-Client-Peer`
   field, and of every other edge profile's assertion, identity, capability,
   provenance, and client-peer field, before inserting its own fields. A trusted
   edge MUST NOT remove or modify the `Authorization` field, which remains reserved
   for the independent NIP-98 proof and MUST reach final admission unmodified;
2. cryptographically authenticate the immediate edge to the accepting origin and
   isolate the origin from direct or alternate ingress;
3. integrity-protect every request component used by authorization other than
   an independent Nostr proof, which is protected by its own signature;
4. apply a positive finite provenance deadline that is included in final admission
   and every resulting lease;
5. validate a closed upstream identity and authorization claim set and produce the
   same normalized assertion result required by core;
6. preserve the server-resolved domain, operation, resource, method, authority,
   path/query, body semantics, proof transport, and Nostr actor key through final
   admission; and
7. keep assertions, credentials, signatures, MACs, raw client addresses, and private
   claims out of URLs, public protocol output, logs, metrics, and traces.

Header presence, source address, private-network location, hostname, or reachability
alone is not provenance. Accepting unsigned identity or capability headers, or
accepting signed headers without authenticating and isolating the immediate caller,
is nonconformant. A trusted edge that strips, rewrites, or reorders the
`Authorization` field, or that admits a proof-transport-`0x02` request whose
`Authorization` field did not arrive at the verifier byte-identical to the
client-sent value, is nonconformant.

An adapter's reviewed contract MUST identify its accepting origins, direct-origin
controls, field-stripping point, immediate-caller authentication, protected request
components, upstream assertion and policy validation, freshness bounds, independent
Nostr-proof path, compromise impact, and conformance evidence. It MUST deny when any
part of this boundary is absent or unreadable.

### Authenticated-edge assertion adapters

A deployment MAY install a private authenticated-edge adapter instead of HMAC-v2.
The adapter MUST satisfy all common requirements and demonstrate together:
origin isolation, cryptographically authenticated immediate caller, inbound-field
stripping, integrity of the complete authorization-relevant request, bounded
assertion and policy freshness, no direct-origin fallback, and the core final-
admission path with independent Nostr proof.

The adapter maps only its closed, validated claim set into the normalized result.
An opaque edge token is acceptable only inside this complete contract; opacity does
not make an unchecked header authoritative. Vendor names, issuer details, caller
identities, private field names, capability semantics, and adapter identifiers MUST
NOT appear in NIP-11 or portable examples.

## `trusted-proxy-hmac-v2`

The stock profile identifier is `trusted-proxy-hmac-v2`. Core computes the
`transport_contract_id` from a canonical contract that includes this profile's exact
wire format, protected components, replay rules, deadline rules, configured code
meanings, and adapter semantics. Changing any of those inputs produces a different
contract identity; the profile identifier itself remains stable. The proxy strips
all inbound assertion, provenance, and client-peer fields and inserts exactly one of
each:

```text
Nostr-Federated-Identity: Bearer <compact-JWS>
Nostr-Federated-Identity-Provenance: v2.<timestamp>.<nonce>.<mac>
Nostr-Federated-Identity-Client-Peer: <client-peer>
```

The assertion field follows core's compact-JWS and size rules. `timestamp` is
canonical unsigned decimal without leading zeroes, except zero is `0`. `nonce` and
`mac` are canonical unpadded base64url. Padding, the standard base64 alphabet,
ignored whitespace, or another encoding denies. The proxy generates a fresh nonce
containing at least 128 bits from a cryptographically secure random source. The
decoded MAC is exactly 32 octets. Finite field and decoded-nonce maxima are applied
before decoding, replay lookup, hashing, or allocation.

`client-peer` is at most 64 ASCII octets. IPv4 uses dotted decimal with no leading
zeroes. IPv6 uses lowercase RFC 5952 text. The edge converts an observed IPv4-mapped
IPv6 address to canonical IPv4 before constructing the field; a textual mapped IPv6
field is noncanonical. Empty, repeated, comma-combined, whitespace-padded, non-IP,
or noncanonical values deny. After verification, the verifier MAY retain only a
domain-separated keyed digest of this value in bounded private state.

The profile uses HMAC-SHA-256 with a deployment secret containing at least 256 bits.
Let `LP(x) = uint64be(len(x)) || x`, where length is in octets. The literal prefix is
14 ASCII octets and is not length-prefixed. The pre-MAC input is exactly:

```text
"NIP-FI-PROXY-2" ||
LP(timestamp_u64be) || LP(nonce_bytes) || LP(SHA256(jwt_ascii)) ||
LP(authorization_domain_id) ||
LP(method_ascii) || LP(authority_ascii) || LP(path_and_query_ascii) ||
LP(SHA256(payload_octets)) || LP(proof_transport_octet) || LP(client_peer_ascii)
```

`mac = HMAC-SHA-256(secret, pre_mac_input)`. The transmitted `mac` is canonical
unpadded base64url of the raw 32-octet result. The verifier compares it in constant
time.

### Canonical components

- **Timestamp:** Parse canonical decimal into an unsigned 64-bit integer, rejecting
  overflow, then serialize it as exactly eight-byte big-endian. Freshness checks are
  separate from serialization.
- **Nonce:** Decode the exact canonical base64url field before serialization.
- **Assertion:** Hash the exact ASCII compact-JWS octets after the one space in
  `Bearer `. No whitespace, Unicode, JSON, or base64 normalization is allowed.
- **Authorization domain:** Configuration contains a canonical lowercase,
  hyphenated RFC 9562 UUID named `authorization_domain_uuid`. Parse its 32 displayed
  hexadecimal digits into the exact 16 UUID octets in display/network order. For
  example, `00112233-4455-6677-8899-aabbccddeeff` becomes
  `00112233445566778899aabbccddeeff`. UTF-8 UUID text, hashing, truncation,
  namespace derivation, mixed-endian GUID encoding, uppercase, and unhyphenated
  configuration are forbidden. The UUID is generated once, is immutable for the
  domain's lifetime, and is shared through authenticated proxy/verifier
  configuration. Duplicate UUIDs among active domains MUST fail startup.
- **Method:** Use the exact uppercase ASCII method token after trusted route
  resolution. Lowercase or noncanonical input denies; the verifier does not repair it.
- **Authority:** Use server-configured lowercase ASCII host plus explicit effective
  decimal port. IPv6 uses brackets and RFC 5952. Userinfo, a trailing dot, an omitted
  port, percent encoding, or an authority derived solely from `Host`, `Forwarded`, or
  `X-Forwarded-Host` denies.
- **Path and query:** Use the exact post-rewrite ASCII origin-form. Empty path becomes
  `/`; a present query includes `?`. Percent octets and hex case, an empty query,
  repeated names, and parameter order are preserved. No decoding, sorting,
  dot-segment removal, or re-encoding may occur after the edge snapshot. An
  unaccounted rewrite denies.
- **Payload:** Hash the complete HTTP payload octets after transfer-coding removal and
  before content-coding decompression. These are exactly the octets forwarded by the
  edge and exposed to verification. HTTP framing, chunk delimiters, and trailers are
  excluded; `Content-Encoding` is not decoded. A WebSocket upgrade uses the empty
  payload. Substitution of the protected octets after the snapshot denies.
- **Proof transport:** Serialize exactly one assigned octet from the registry below.
- **Client peer:** Serialize the exact canonical ASCII field value.

No authorization decision, target, resource, capability, or effect selector
derives from any request or connection component outside the protected pre-MAC
components, except an independent Nostr proof validated on its own signature,
such as the NIP-98 event in `Authorization` or the NIP-42 event after connect,
which the MAC does not protect; body interpretation follows the server-resolved
body semantics, never unprotected transport metadata such as `Content-Type` or
`Content-Encoding`.

### Freshness, replay, and key rotation

The deployment configures a positive finite `maximum_provenance_age` and a
non-negative finite `future_skew`. Evidence is live exactly when, using overflow-safe
comparisons:

```text
timestamp <= now + future_skew
now < timestamp + maximum_provenance_age
```

Equality at the age bound is expired. A direct lease deadline is no later than
`timestamp + maximum_provenance_age` and every core assertion, proof, policy, and
state deadline.

Absent, malformed, stale, future-dated, wrong-key, or mismatched provenance denies.
On a route that requires edge provenance, absent or incomplete provenance —
including provenance that omits the proxy-authenticated end-client peer — maps
to the `missing_evidence` public class, regardless of whether an assertion is
present. Provenance that is present and complete but fails verification maps to
`evidence_rejected`.
A v1 envelope denies. A verifier MAY try only a configured finite set of active
secrets. Rotation does not change nonce identity: replay uniqueness is scoped to
`(authorization_domain_id, trusted-proxy-hmac-v2, nonce)` and is independent of the
secret that verifies the MAC. A committed nonce is retained through at least
`timestamp + maximum_provenance_age`.

Preparation consumes neither nonce nor Nostr-proof replay identity. Final admission
atomically consumes both with any enrollment, receipt, and authorization decision.
A failed or rolled-back admission consumes neither. Two concurrent admissions with
the same nonce commit at most one authorization. The proxy-to-verifier hop still
requires confidentiality and integrity.

## Proof-transport code registry

| Code | Meaning and allocation policy |
|---|---|
| `0x00` | Invalid; MUST deny. |
| `0x01` | NIP-42 connection proof. |
| `0x02` | NIP-98 HTTP proof. |
| `0x03` | Git smart-HTTP session proof: the proxy verifies a session-scoped Nostr authorization for a Git smart-HTTP request before forwarding. Reserved; allocation completes on publication of its transport contract (see below). |
| `0x04` | Blossom media proof: the proxy verifies a Blossom media-HTTP authorization event for the request before forwarding. Reserved; allocation completes on publication of its transport contract (see below). |
| `0x05`–`0x7f` | Unassigned; allocation requires a published stable specification. |
| `0x80`–`0xfe` | Private use under an explicit shared proxy/verifier contract only. |
| `0xff` | Reserved for a future extended encoding; invalid in HMAC-v2. |

An allocation MUST define exact proof validation, request binding, freshness, replay
identity and window, and conformance vectors. Assigned semantics never change; an
incompatible meaning receives a new code. Unknown, unconfigured, or private-use
codes without the same configured contract at proxy and verifier deny. Private-use
codes MUST NOT be advertised as portable NIP-FI-EDGE interoperability.

Codes `0x03` and `0x04` are reserved to fix their meanings and prevent
reassignment; their transport contracts are not yet published, so their
allocations are not complete. Until the contract for such a code is published,
the code is valid only under an explicit shared proxy/verifier contract,
exactly as for private use, and MUST NOT be presented as portable NIP-FI-EDGE
interoperability.

## Bounded payload acquisition

Every protected `(authorization_domain_id, route, proof_transport_code)` tuple MUST
configure a finite `maximum_payload_octets` and finite per-request
`maximum_spool_octets >= maximum_payload_octets`. Zero is allowed only for a route
that requires an empty payload. Proxy and verifier configuration MUST agree and is
part of the transport contract.

If trusted `Content-Length` exceeds the route limit, the edge denies before reading,
hashing, JWT verification, replay lookup, or authoritative mutation. For absent,
unknown, or streamed length, acquisition uses a bounded counter and spool and stops
on octet `limit + 1`. Incremental SHA-256 is allowed, but no digest or prefix can
authorize until EOF proves completeness.

Spooling uses memory or access-controlled temporary storage with finite per-request
and aggregate quotas, cleanup on every outcome, no public or log output, and no reuse
across requests. Quota exhaustion fails closed and creates no nonce claim, proof
claim, receipt, lease, or application mutation. At or below the limit, the exact
captured payload is replayed unchanged. HMAC verification and core final admission
complete before application effects. Forwarding to a rollback-safe private spool is
not an application effect; forwarding to a parser, decoder, handler, or origin that
can act is.

A content decoder, multipart parser, Git/Blossom handler, framework, or intermediary
that cannot expose and replay the exact stage defined above before effects cannot
claim HMAC-v2 for that route. It MUST use core `client-attached` or another specified
edge profile, never a partial-body MAC.

## Normative HMAC-v2 vectors

All vector integers and lengths are big-endian. Common values are:

```text
secret_hex = 000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f
nonce_hex = 000102030405060708090a0b0c0d0e0f
nonce_base64url = AAECAwQFBgcICQoLDA0ODw
authorization_domain_uuid = 00112233-4455-6677-8899-aabbccddeeff
authorization_domain_id_hex = 00112233445566778899aabbccddeeff
jwt_ascii = eyJhbGciOiJFUzI1NiIsInR5cCI6Im5pcC1maStqd3QifQ.eyJpc3MiOiJodHRwczovL2lkLmV4YW1wbGUiLCJzdWIiOiIxMjMifQ.c2ln
assertion_digest_hex = 6103b52a52730bc065d65673247603a63c9810488c90d0ada3d8d227eee5285f
```

The fixture JWT represents a separately minted `nip-fi+jwt` assertion and is opaque
test input; its deliberately synthetic signature is not an assertion-validation
vector. Implementations MUST reproduce each field, complete pre-MAC input,
diagnostic input digest, raw MAC, and wire MAC exactly
(`FI-TRACE-EDGE-VECTORS`).

### Vector 1: HTTP / NIP-98 / non-empty payload

```text
timestamp_decimal = 1700000000
timestamp_u64be_hex = 000000006553f100
method_ascii = POST
authority_ascii = api.example:443
path_and_query_ascii = /upload?part=1&part=2&x=%2F
payload_hex = 68656c6c6f0a
body_digest_hex = 5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03
proof_transport_hex = 02
client_peer_ascii = 203.0.113.9
pre_mac_input_hex = 4e49502d46492d50524f58592d320000000000000008000000006553f1000000000000000010000102030405060708090a0b0c0d0e0f00000000000000206103b52a52730bc065d65673247603a63c9810488c90d0ada3d8d227eee5285f000000000000001000112233445566778899aabbccddeeff0000000000000004504f5354000000000000000f6170692e6578616d706c653a343433000000000000001b2f75706c6f61643f706172743d3126706172743d3226783d25324600000000000000205891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03000000000000000102000000000000000b3230332e302e3131332e39
pre_mac_input_sha256 = df2870230d2170595dccd17d9e61a82282d8cd8b978ac18bff07419ed59091d5
mac_hex = 761d3ecbf609f0f558b4a02a1a18a25070f3dbe89fce9cac59a80bce4436ade5
mac_base64url = dh0-y_YJ8PVYtKAqGhiiUHDz2-ifzpysWagLzkQ2reU
provenance = v2.1700000000.AAECAwQFBgcICQoLDA0ODw.dh0-y_YJ8PVYtKAqGhiiUHDz2-ifzpysWagLzkQ2reU
```

### Vector 2: WebSocket / NIP-42 / empty payload / mapped peer

The edge observed `::ffff:192.0.2.128` and emitted canonical `192.0.2.128`.

```text
timestamp_decimal = 1
timestamp_u64be_hex = 0000000000000001
method_ascii = GET
authority_ascii = relay.example:443
path_and_query_ascii = /
payload_hex =
body_digest_hex = e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
proof_transport_hex = 01
client_peer_ascii = 192.0.2.128
pre_mac_input_hex = 4e49502d46492d50524f58592d32000000000000000800000000000000010000000000000010000102030405060708090a0b0c0d0e0f00000000000000206103b52a52730bc065d65673247603a63c9810488c90d0ada3d8d227eee5285f000000000000001000112233445566778899aabbccddeeff0000000000000003474554000000000000001172656c61792e6578616d706c653a34343300000000000000012f0000000000000020e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855000000000000000101000000000000000b3139322e302e322e313238
pre_mac_input_sha256 = 67564d241499491b3ea53b31d6111fbc9efac37a294f6ce591519e4bf21b53e9
mac_hex = f71a179a018637a0582cf3de39ccb7b976216c18ada312127d4c983c14af4b20
mac_base64url = 9xoXmgGGN6BYLPPeOcy3uXYhbBitoxISfUyYPBSvSyA
```

### Vector 3: IPv6 authority and path/query byte preservation

```text
timestamp_decimal = 1700000000
timestamp_u64be_hex = 000000006553f100
method_ascii = GET
authority_ascii = [2001:db8::1]:443
path_and_query_ascii = /a%2Fb?b=2&a=1&a=0
payload_hex =
body_digest_hex = e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
proof_transport_hex = 02
client_peer_ascii = 2001:db8::2
pre_mac_input_hex = 4e49502d46492d50524f58592d320000000000000008000000006553f1000000000000000010000102030405060708090a0b0c0d0e0f00000000000000206103b52a52730bc065d65673247603a63c9810488c90d0ada3d8d227eee5285f000000000000001000112233445566778899aabbccddeeff000000000000000347455400000000000000115b323030313a6462383a3a315d3a34343300000000000000122f61253246623f623d3226613d3126613d300000000000000020e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855000000000000000102000000000000000b323030313a6462383a3a32
pre_mac_input_sha256 = 8a93a29c4ac30b0f2551d346d0636040b639bb1f109d287e93ce44ddaed73e33
mac_hex = df2936f81d752f3d6bac2a36d3381c38db2c9abc3570236cb121274ad34a6161
mac_base64url = 3yk2-B11Lz1rrCo20zgcONssmrw1cCNssSEnStNKYWE
```

### Serialization and negative matrix

The following timestamp values MUST serialize as shown before freshness evaluation:

| Decimal | `uint64be` hex |
|---:|---|
| `0` | `0000000000000000` |
| `1` | `0000000000000001` |
| `255` | `00000000000000ff` |
| `256` | `0000000000000100` |
| `18446744073709551615` | `ffffffffffffffff` |

`00`, `01`, `+1`, surrounding whitespace, negative values, and
`18446744073709551616` deny before MAC comparison. The maximum value above is an
encoding vector; ordinary freshness policy will reject it.

Every implementation MUST run these normative negative cases:

| Class | Required cases and result |
|---|---|
| Envelope | Absent/repeated/comma-combined fields, `v1`, missing/extra component, padding, alternate alphabet, nonce below 16 octets or above configured max, and MAC lengths 31 or 33 all deny. |
| Domain | Uppercase/nonhyphenated UUID config fails configuration; mixed-endian UUID bytes or any one-bit domain transplant fails the baseline MAC; duplicate active UUID fails startup. |
| Request | Mutating assertion, method, authority, path/query, body, proof code, or peer while retaining Vector 1's MAC denies. |
| Metadata | Mutating `Content-Type` or `Content-Encoding` in flight changes no authorization decision, target, capability, or effect selector; a request whose server-resolved body semantics no longer hold denies. |
| Path | `%2F`→`%2f`, decoding to `/`, reordering repeated query values, or adding/removing an empty `?` fails the baseline MAC. |
| Authority | Unbracketed or non-RFC-5952 IPv6, uppercase host, trailing dot, or missing port denies before MAC comparison. |
| Peer | Textual `::ffff:192.0.2.128`, padded IPv4, uppercase/noncanonical IPv6, or whitespace denies before MAC comparison. |
| Proof | `0x00`, `0xff`, unknown stock code, or private code without a shared configured contract denies. |
| Body | Known and unknown lengths `0`, `limit-1`, and `limit` may proceed only after EOF; `limit+1`, disconnect before EOF, aggregate-quota exhaustion, or any post-snapshot substitution of the protected octets denies with no replay or authoritative mutation. |
| Replay | Concurrent final admissions of one valid envelope commit at most one; preparation and failed final admission consume none; secret rotation does not create a new nonce namespace. |
| Fallback | Direct ingress, mixed evidence, and failed HMAC never retry as `client-attached` or another adapter. |

## Discovery and conformance

A relay that completely implements the stock profile MAY add exactly
`"edge_transports": ["trusted-proxy-hmac-v2"]` inside the top-level NIP-11
`federated_identity` object. `edge_transports` is an array of unique ASCII string
profile identifiers in ascending bytewise order; this document assigns only the
single value shown. A relay that does not completely implement the stock profile
MUST omit the member. It MUST NOT advertise private adapters, keys, domains, field
names, or code contracts. No request may select behavior from this discovery
member; server-owned configuration selects the edge profile. Claiming FI-EDGE
requires every configured edge profile to pass the applicable core conformance suite
and these profile traces:

| Trace | Required oracle |
|---|---|
| `FI-TRACE-EDGE-VECTORS` | Reproduce all three normative vectors field-for-field, including each complete pre-MAC input, diagnostic input digest, raw MAC, and wire MAC; reproduce all five timestamp serialization rows; every listed serialization and negative-matrix case produces its required denial or configuration failure. |
| `FI-TRACE-PROXY-SPOOF` | Direct ingress, unsigned/header-only identity, unauthenticated caller, or invalid provenance denies without fallback. |
| `FI-TRACE-PROXY-REPLAY` | Two HMAC-v2 final admissions using one nonce commit at most one; preparation consumes neither. A private adapter proves its declared replay semantics. |
| `FI-TRACE-PROXY-CROSS-REQUEST` | Each protected component mutation denies. HMAC-v2 covers assertion, domain, method, authority, path/query, complete body, proof transport, and peer. On a `0x02` route the `Authorization` bytes at final admission equal the client-sent bytes, witnessed at both points; an edge that substitutes a valid proof from the same actor fails the witness. |
| `FI-TRACE-EDGE-BODY-BOUNDS` | Known and streamed boundary cases prove bounded work/storage, EOF completeness, cleanup, and no pre-authorization effect. |
| `FI-TRACE-EDGE-KEY-ROTATION` | A finite active-key set accepts an intended overlap without allowing nonce reuse or an unknown key. |

The conformance record binds the exact implementation, adapter, deployment,
assertion policy, transport contract, configured code meanings, and vector revision.
Two HMAC-v2 implementations interoperate only when they reproduce all valid vector
bytes exactly, reject every negative, agree on UUID and code configuration, and
preserve atomic replay and bounded complete-body behavior.

## Security considerations

HMAC-v2 limits header spoofing, replay, and cross-request transplantation only when
its secret remains confidential, the edge snapshots the final routed request, the
origin authenticates that edge, and final admission atomically consumes replay state.
It does not replace TLS or independent Nostr proof. A compromised edge or shared
secret can forge federated evidence within its configured domains; use distinct
secrets and UUIDs to limit blast radius.

Authenticated-edge adapters intentionally shift more proof to deployment controls.
A hostname, private network, or opaque token is not an equivalent construction unless
the complete boundary obligations above are demonstrated. Body buffering and replay
state are attacker-controlled resource surfaces, so all field, payload, spool,
aggregate, key-set, and retention bounds fail closed.
