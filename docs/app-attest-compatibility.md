# App Attest compatibility boundary

The push gateway verifies enrollment and assertions through a private
[Buzz-owned module](../crates/buzz-push-gateway/src/app_attest.rs). Extended authenticators must
remain compatible without weakening the checks that bind a request to an app,
key, challenge, and increasing counter.

## Receiver policy

Enrollment requires the fixed authenticator prefix, AAGUID, credential ID, and a
complete COSE P-256 key. The COSE key must match the leaf certificate key. Any
remaining bytes form one complete extension map. Assertions require the fixed
37-byte prefix and, when present, one complete extension map. Neither operation
uses a total length or flags allowlist to recognize an Apple SDK version.

Credential IDs use the fixed 32-byte SHA-256 size, preserving appattest 0.1.1's
workaround for reported iOS 26.x nonstandard credential-length fields. The two
length bytes remain authenticated; the credential ID must match the supplied
key ID and the certificate key hash, and the COSE key at offset 87 must match
the certificate. Signed synthetic tests cover nonstandard lengths, actual
missing/extra credential bytes, key mismatch, and unsigned length mutations.
No enrollment captured on iOS 26.x has yet validated this reported OS variant.
SDK 26.5 tests on iOS 27.0.1 do not substitute for that evidence.

Every original authenticator byte participates in the certificate nonce or
assertion signature. Verification does not truncate or re-encode that byte
string. Assertion signatures and enrollment certificate nonces are checked
before decoding the extension payload. Unknown extension identifiers and opaque
optional property values do not authorize or reject a request by themselves.
Malformed structure, ambiguous duplicate keys, and trailing data are rejected.

CBOR accepts well-formed definite/indefinite containers and strings, nonpreferred
integer widths, and opaque tagged/simple values under bounded resource limits.
Map-key equality follows the generic CBOR model, including compound keys;
integers and floating-point values remain distinct. Schema-bearing maps require
their specified text or COSE labels.

The gateway retains its configured Apple root and exact server-selected app and
attestation environment. Certificate validation uses webpki; field extraction
uses x509-parser. The nonce extension must have its complete canonical structure.
The leaf key, credential ID, supplied key ID, zero enrollment counter, and app
hash must agree. Assertions retain signature, app hash, challenge, and increasing
counter checks. Existing atomic database updates remain the replay boundary.

The verifier consumes EKU OIDs without applying TLS purposes to App Attest and
honors supplied digital-signature/key-cert-sign usage restrictions. It does not
perform revocation checking or Apple's separate receipt risk assessment.

Version text and raw four-byte category values are bounded diagnostic
observations only. Unsupported optional representations remain opaque. No
version or category restriction is introduced.

## Capacity and tests

Parser limits are service policy: 64 KiB encoded input, 4096 decoded nodes,
depth 32, and 64 KiB copied strings/bytes. Declared lengths do not drive
preallocation. Gateway transport limits remain 16 KiB for enrollment and 1 KiB
for assertions. Tests cover boundaries, oversized declared lengths, truncations,
duplicate equivalence, and structurally invalid signed payloads.

Signed synthetic enrollment tests cover COSE parity/reordering, extension
variation, certificate constraints, nonce integrity, wrong identity/key binding,
and incomplete credentials. Gateway tests verify legacy and extended assertions,
tamper rejection, wrong transcript/key/app, and replay rejection. The signed-certificate suite runs with the gateway unit tests. Fixture helpers
are compiled only for tests, including in personal-development builds.

Physical-device validation used one probe identity on iOS 27.0.1:

| SDK | Enrollment authenticator | Assertion authenticator | Key relationship |
| --- | ---: | ---: | --- |
| 26.5 | 164 bytes | 37 bytes | Original probe key |
| 27.1 | Previously enrolled | 102 bytes | Same original key |
| 27.1 | 229 bytes | 102 bytes | Additional fresh probe key |

All three assertions and both enrollments pass the gateway verifier. Same-key
counters increase; repeated counters and altered transcripts fail. Development
enrollments fail under the production environment. These are local verifier
results, not evidence of a deployed gateway or APNs delivery. Raw device captures
stay outside the repository; optional ignored tests consume protected local
inputs for repeatable validation.

## Source provenance

Adapted portions originate in [appattest 0.1.1](https://crates.io/crates/appattest/0.1.1),
[upstream commit d1ffda7](https://github.com/PrismaPhonic/appattest/commit/d1ffda733f6e7f267aab79ae42b8ec7f60ceada1).
The package SHA-256 is `5d0a66767aaf7d483c556386fb68ca2fba9347684d8bb17a4bd8b755851870f7`.
Both upstream MIT notices are retained in the module's
[LICENSE](../crates/buzz-push-gateway/src/app_attest/LICENSE).
The gateway owns its verification policy and tracks Apple protocol and underlying
cryptographic-library updates directly. There is no remaining appattest dependency.

## Resource characterization

A manual debug-build run on arm64 macOS (1,000 parses per case) measured about
1.61 ms for a 49,131-byte map near the 4,096-node limit, 247 microseconds for a
4,097-byte array at that node limit, 0.93 microseconds for a 65,536-byte byte
string, and 3.8 microseconds for depth 32. The test process peaked at 10.4 MB
resident memory; that includes the test harness and is not a parser-allocation
measurement. These are local observations, not latency guarantees or production
benchmarks. The manual `characterize_capacity_work` test reproduces the cases
without imposing a timing-sensitive CI assertion.

Node and depth limits bound traversal and key-comparison work; byte limits bound
copied payloads. Container capacity grows only after successfully decoding each
item, never from an untrusted declared length. The gateway's smaller transport
caps apply before these parser limits. Future valid objects can exceed service
capacity and receive the distinct internal capacity diagnostic; raising limits
requires review and repeating this characterization.

Generated tests cover 256 nested optional-value/signature-mutation cases and
512 arbitrary byte inputs per run. They complement the explicit boundary and
encoding-equivalence corpus. Apple's public historical attestation fixture also
exercises a complete credential key and extension tail with ED clear; its
certificate is not treated as a currently valid enrollment.
