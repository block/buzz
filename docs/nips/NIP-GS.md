NIP-GS
======

Git Object Signing with Nostr Keys
-----------------------------------

`draft` `optional`

## Abstract

This NIP defines a signature format and verification protocol for signing git
commits and tags with Nostr secp256k1 keys, using git's pluggable signing
program interface (`gpg.x509.program`).

## Motivation

Git supports cryptographic commit and tag signing via GPG, SSH, or x509
programs. Nostr users already have secp256k1 keypairs (BIP-340 Schnorr). This
NIP allows those same keys to sign git objects — establishing a cryptographic
link between a Nostr identity and a git history without requiring GPG keys, SSH
keys, or certificate authorities.

The primary use case is autonomous agents that commit code on behalf of their
owners. The agent's Nostr keypair — already used for relay authentication
(NIP-98), channel membership, and owner attestation (NIP-OA) — now also signs
its commits. One identity, one key, across all surfaces.

Human developers with Nostr identities benefit equally: their commits carry the
same cryptographic identity as their relay messages, reviews, and approvals.

## Non-Goals

This NIP does not define a trust model beyond signature verification.
Web-of-trust, allowed-signer lists, and relay-side commit verification are out
of scope.

This NIP does not define a new git transport or hosting protocol. It operates
entirely within git's existing signing program interface.

This NIP does not require relay changes. Relays are uninvolved — signing and
verification happen locally between git and the signing program.

This NIP does not define key management, rotation, or revocation. Consequences
of key compromise are discussed in Security Considerations.

## How Git Invokes Signing Programs

Git's signing interface is the same for `openpgp` and `x509` formats
(both use `sign_buffer_gpg` and `verify_gpg_signed_buffer` in
`gpg-interface.c`). The configured program is invoked as:

**Signing:** `<program> --status-fd=<N> -bsau <signing-key>`

- Payload bytes are piped to stdin.
- The program writes the detached signature to stdout.
- The program writes `[GNUPG:]` status lines to file descriptor N.
- Git checks that `[GNUPG:] SIG_CREATED` appears in the status output.

**Verification:** `<program> --status-fd=<N> --verify <signature-file> -`

- Payload bytes are piped to stdin.
- The signature file path is passed as an argument.
- The program writes `[GNUPG:]` status lines to file descriptor N.
- Git parses `GOODSIG`, `BADSIG`, `VALIDSIG`, `ERRSIG`, and `TRUST_*` from
  the status output.

This NIP uses `gpg.format=x509` because:

1. The x509 format uses `-----BEGIN SIGNED MESSAGE-----` markers, which do not
   collide with PGP (`-----BEGIN PGP SIGNATURE-----`) or SSH
   (`-----BEGIN SSH SIGNATURE-----`) markers. Platforms that attempt to verify
   PGP signatures (e.g., GitHub) will not misparse them.
2. The x509 verify path passes no extra arguments (`x509_verify_args` is empty
   in git's source), while openpgp adds `--keyid-format=long`.
3. sigstore/gitsign (1,000+ stars) established this pattern successfully.

## Specification

### Signature Format

The signing program MUST produce a detached signature wrapped in armor:

```
-----BEGIN SIGNED MESSAGE-----
<base64>
-----END SIGNED MESSAGE-----
```

The armor MUST consist of exactly three lines separated by `\n` (LF, 0x0A),
followed by a final `\n`. That is, the output is exactly:

```
-----BEGIN SIGNED MESSAGE-----\n<base64>\n-----END SIGNED MESSAGE-----\n
```

The first line MUST be exactly `-----BEGIN SIGNED MESSAGE-----`.
The last line MUST be exactly `-----END SIGNED MESSAGE-----`.
The middle line MUST be a single base64-encoded string using the standard
alphabet (RFC 4648 §4) with `=` padding. Line wrapping MUST NOT be used.
The encoded base64 line MUST NOT exceed 4096 bytes (sufficient for the 2048-byte
decoded JSON limit with base64 overhead).
Trailing whitespace on any line MUST NOT be present.
CRLF line endings MUST NOT be used.

Verifiers MUST accept a trailing `\n` after the end marker (git may append one).
Verifiers MUST reject signatures with:
- Missing or malformed armor headers.
- Multiple armor blocks.
- Line-wrapped base64.
- Base64 line exceeding 4096 bytes.
- Any bytes after the end marker other than a single `\n`.

The base64 content decodes to a JSON object:

```json
{
  "v": 1,
  "pk": "<pubkey-hex>",
  "sig": "<signature-hex>",
  "t": <created-at>,
  "oa": ["<owner-pubkey-hex>", "<conditions>", "<sig-hex>"]
}
```

| Field | Type    | Required | Constraints | Description |
|-------|---------|----------|-------------|-------------|
| `v`   | integer | MUST     | MUST be `1` | Schema version. |
| `pk`  | string  | MUST     | Exactly 64 lowercase hex characters. MUST be a valid BIP-340 x-only public key (i.e., the x-coordinate of a point on the secp256k1 curve). | Signer's public key. |
| `sig` | string  | MUST     | Exactly 128 lowercase hex characters. | BIP-340 Schnorr signature over the git object. |
| `t`   | integer | MUST     | MUST be in the range 0 to 4294967295. MUST NOT be negative, a float, or a string. | Claimed unix timestamp (seconds) of the signing event. See Security Considerations for implications of signer-controlled timestamps. |
| `oa`  | array   | OPTIONAL | If present, MUST be a JSON array of exactly 3 strings. See Owner Attestation. | NIP-OA owner attestation proving the signer was authorized by an owner key. |

JSON parsing rules:
- The base64-decoded bytes MUST be valid UTF-8. Verifiers MUST reject if the
  decoded bytes contain invalid UTF-8 sequences.
- The content MUST be a single JSON object (not an array, string, or primitive).
- The JSON MUST use compact serialization: no whitespace outside of string
  values. Verifiers MUST reject JSON containing spaces, tabs, or newlines
  outside of string values. This prevents envelope malleability — since the
  signature envelope is embedded in the git commit object, any byte change
  alters the commit hash.
- Duplicate keys: verifiers MUST reject the signature. Implementations SHOULD
  use a JSON parser configured to fault on duplicate keys, or verify key
  uniqueness before parsing.
- For `v=1`, the only permitted keys are `v`, `pk`, `sig`, `t`, and `oa`.
  Any other key MUST cause rejection. Future versions (`v=2`, etc.) define
  their own field sets. This prevents unsigned extension fields from being
  injected into the envelope.
- The total decoded JSON MUST NOT exceed 2048 bytes (the `oa` field adds
  ~200 bytes to the base ~250-byte envelope).
- Implementations MUST reject non-canonical hex (uppercase, odd-length, whitespace).

### Signing Hash

All envelope metadata (`t`, `oa`) is included in the hash preimage so that it
is cryptographically bound to the signature. Tampering with any field
invalidates the signature.

Given a git object payload (the bytes git pipes to stdin), a signing timestamp
`t`, and an optional owner attestation:

```
hash = SHA-256( "nostr:git:v1:" || decimal(t) || ":" || oa_binding || payload_bytes )
```

Where:
- `"nostr:git:v1:"` is the domain separator: exactly 13 bytes of UTF-8
  (`6e6f7374723a6769743a76313a`).
- `decimal(t)` is the ASCII decimal encoding of `t` with no leading zeroes
  (except `0` itself). Example: `1700000000`.
- `":"` is a single colon byte (`3a`), separating the timestamp from the next
  field.
- `oa_binding` is:
  - If `oa` is present: `oa[0] || ":" || oa[1] || ":" || oa[2] || ":"` (the
    three `oa` array elements concatenated with colon separators, followed by a
    trailing colon). All elements are their exact string values (hex pubkey,
    conditions string which may be empty, hex signature).
  - If `oa` is absent: empty (zero bytes). The colon after `decimal(t)` is
    immediately followed by `payload_bytes`.
- `payload_bytes` is the raw bytes git pipes to stdin.

**Important:** Because the `oa` data is included in the signing hash, stripping
or modifying the `oa` field invalidates the NIP-GS `sig`. This is intentional —
the signature envelope is immutable once signed.

The domain separator prevents cross-protocol signature reuse:
- NIP-01 event signatures sign `SHA-256(serialized_event)` — different preimage.
- NIP-98 HTTP auth signatures sign a kind:27235 event — different preimage.
- NIP-OA attestations sign `SHA-256("nostr:agent-auth:" || ...)` — different
  domain separator.

### Signing Procedure

1. Record the current unix timestamp as `t`.
2. Read the git object payload from stdin. If the payload exceeds 100 MB,
   exit with code 1 and a diagnostic on stderr. MUST NOT write to stdout.
3. Compute the signing hash per the Signing Hash section:
   `hash = SHA-256("nostr:git:v1:" || decimal(t) || ":" || oa_binding || payload)`.
   If including `oa`, the `oa_binding` is `oa[0] || ":" || oa[1] || ":" || oa[2] || ":"`.
   If not including `oa`, the `oa_binding` is empty (zero bytes).
4. Produce a BIP-340 Schnorr signature over `hash` using the signer's secret
   key. Implementations MUST use a cryptographically secure nonce per BIP-340
   §4. Implementations SHOULD use auxiliary randomness (BIP-340 §4 default
   nonce generation) to mitigate side-channel attacks. Implementations MAY use
   deterministic nonce generation (RFC 6979 adapted to BIP-340) for
   reproducible test vectors.
5. Construct the JSON object with compact serialization (no whitespace).
   Field order MUST be `v`, `pk`, `sig`, `t`, then `oa` if present.
   Example without `oa`: `{"v":1,"pk":"<hex>","sig":"<hex>","t":<integer>}`
   Example with `oa`: `{"v":1,"pk":"<hex>","sig":"<hex>","t":<integer>,"oa":["<owner>","","<sig>"]}`
6. Base64-encode the JSON bytes (standard alphabet, with padding).
7. Write to stdout:
   ```
   -----BEGIN SIGNED MESSAGE-----\n<base64>\n-----END SIGNED MESSAGE-----\n
   ```
8. Write to the status file descriptor:
   ```
   [GNUPG:] BEGIN_SIGNING\n[GNUPG:] SIG_CREATED D 8 1 00 <t> <pk>\n
   ```
   Where `<t>` is the decimal timestamp and `<pk>` is the 64-character hex
   public key. The tokens `D 8 1 00` are fixed compatibility placeholders
   that satisfy git's `SIG_CREATED` parser. They do not carry semantic
   meaning for nostr signatures. (`D` = detached, `8` = SHA-256 in GPG's
   algorithm numbering, `1` and `00` are reserved fields git does not
   interpret.)

### Verification Procedure

1. Read the signature file. Validate armor format per the rules above.
2. Base64-decode the middle line. Verify the decoded bytes are valid UTF-8.
   Parse as JSON. **To prevent envelope malleability**, after parsing and
   validating all fields, the verifier MUST reconstruct the canonical JSON
   string using the exact parsed values in the required field order
   (`v`, `pk`, `sig`, `t`, then `oa` if present) with compact serialization
   (no whitespace). The reconstructed string MUST exactly match the
   base64-decoded string byte-for-byte. Any deviation in field order, number
   formatting (e.g., `1.7e9` instead of `1700000000`), or unexpected
   whitespace MUST result in `ERRSIG`, exit 1. This ensures the signature
   envelope is non-malleable — there is exactly one valid byte sequence for
   any given set of field values.
3. Validate all fields per the constraints table. If any field is invalid or
   missing, write `ERRSIG` (see below) and exit with code 1.
4. If `v` is not `1`, write `ERRSIG` and exit with code 1.
5. Validate that `pk` is a valid BIP-340 x-only public key (not just hex — the
   value must be the x-coordinate of a point on secp256k1, i.e., `lift_x(pk)`
   must succeed per BIP-340 §5.3.2).
6. Read the git object payload from stdin. If the payload exceeds 100 MB,
   write `ERRSIG` to the status fd and exit with code 1.
7. Compute the signing hash per the Signing Hash section. If the `oa` field is
   present and structurally valid (array of 3 strings), include the oa_binding:
   `hash = SHA-256("nostr:git:v1:" || decimal(t) || ":" || oa[0] || ":" || oa[1] || ":" || oa[2] || ":" || payload)`.
   If `oa` is absent:
   `hash = SHA-256("nostr:git:v1:" || decimal(t) || ":" || payload)`.
8. Verify the BIP-340 Schnorr signature `sig` over `hash` against public key
   `pk`.
9. If verification fails, write to the status fd:
   ```
   [GNUPG:] NEWSIG\n[GNUPG:] BADSIG <pk> <pk>\n
   ```
   Exit with code 1.
10. If verification succeeds, determine trust level:
    - Read `user.signingkey` from git config
      (`git config --get user.signingkey`).
    - If the value is an `npub1...` string, decode it to 64-character hex
      per NIP-19 before comparison.
    - If `user.signingkey` is set AND `pk` equals the normalized hex value
      (case-insensitive comparison) → trust level is `FULLY`.
    - Otherwise → trust level is `UNDEFINED`.

    Note: `TRUST_FULLY` means only "this is the locally configured signing
    key" — it is NOT a global trust assertion. For verifying other people's
    commits, applications SHOULD implement an allowed-signers mechanism
    (outside the scope of this NIP) rather than relying on `TRUST_FULLY`.
11. Write to the status fd:
    ```
    [GNUPG:] NEWSIG
    [GNUPG:] GOODSIG <pk> <pk>
    [GNUPG:] VALIDSIG <pk> <date> <t> 0 - - - - - <pk>
    [GNUPG:] TRUST_FULLY 0 shell
    ```
    Or `TRUST_UNDEFINED` instead of `TRUST_FULLY` if the key is unknown.
    Exit with code 0.

#### Status Line Formats

Each status line is `[GNUPG:] ` (9 bytes, including trailing space) followed by
the status keyword and space-separated fields, terminated by `\n`.

**SIG_CREATED** (signing success):
```
[GNUPG:] SIG_CREATED D 8 1 00 <t_decimal> <pk_hex_64>
```

**GOODSIG** (verification success):
```
[GNUPG:] GOODSIG <pk_hex_64> <pk_hex_64>
```
First field: key ID. Second field: user ID. Both are the hex pubkey.

**BADSIG** (signature cryptographically invalid):
```
[GNUPG:] BADSIG <pk_hex_64> <pk_hex_64>
```

**ERRSIG** (signature could not be processed — malformed, unknown version, etc.):
```
[GNUPG:] ERRSIG <key_id> 0 0 00 0 9
```
Where `<key_id>` is the `pk` field if parseable, or 16 zero bytes
(`0000000000000000`) if `pk` could not be extracted. The trailing fields are
fixed placeholders (algo, hash algo, class, timestamp, rc=9 meaning "no public
key" / general error).

**VALIDSIG** (fingerprint and timestamp — emitted after GOODSIG):
```
[GNUPG:] VALIDSIG <fpr> <date> <t_decimal> 0 - - - - - <primary_fpr>
```
Where:
- `<fpr>` is the 64-character hex pubkey (fingerprint).
- `<date>` is the signing date in `YYYY-MM-DD` format, derived from `t`
  interpreted as UTC. Implementations MUST use UTC for this conversion.
- `<t_decimal>` is the decimal unix timestamp from the signature.
- `0` is the expiration timestamp (no expiration).
- The five `-` tokens are reserved fields. Git's parser skips 9 space-separated
  tokens after the fingerprint to find the primary key fingerprint.
- `<primary_fpr>` is the primary key fingerprint (same as `<fpr>` — Nostr keys
  have no subkey hierarchy).

**TRUST_*** (trust level — emitted after VALIDSIG):
```
[GNUPG:] TRUST_FULLY 0 shell
[GNUPG:] TRUST_UNDEFINED 0 shell
```

### Key Loading

The signing program MUST load the signer's secret key from one of the following
sources, checked in order:

1. `NOSTR_PRIVATE_KEY` environment variable.
2. `BUZZ_PRIVATE_KEY` environment variable.
3. A keyfile at the path specified by `nostr.keyfile` git config key.

Each source accepts the key in either `nsec1...` (NIP-19 bech32) or 64-character
hex format. Leading and trailing whitespace MUST be trimmed before parsing.
Any other format MUST be rejected.

The program MUST zeroize secret key material from memory after use.

On Unix systems, if loading from a keyfile, the program MUST verify file
permissions are no broader than `0600` (owner read/write only). If permissions
are broader, the program MUST exit with an error and a diagnostic on stderr.

For verification, no secret key is needed — only the public key embedded in the
signature.

#### Signing Key Argument

Git passes the `user.signingkey` value as the `-u <key>` argument. The signing
program SHOULD verify that the loaded private key corresponds to the public key
specified in `<key>` (if `<key>` is a hex pubkey or npub). If they do not match,
the program MUST exit with an error. This prevents accidentally signing with the
wrong key.

If `<key>` is empty or not a recognizable key format, the program MAY ignore it
and sign with whatever key is loaded.

### Owner Attestation (Optional)

Agent processes — AI agents, CI bots, automation — often act on behalf of a
human owner. The optional `oa` field embeds a [NIP-OA](NIP-OA.md) owner
attestation directly in the signature envelope, allowing anyone to verify
offline that the signing key was authorized by a specific owner key.

Signing programs that have access to a NIP-OA auth tag SHOULD include it.
Human signers who are their own authority SHOULD omit it.

#### Format

The `oa` field, when present, MUST be a JSON array of exactly 3 strings:

```json
"oa": ["<owner-pubkey-hex>", "<conditions>", "<owner-sig-hex>"]
```

| Index | Type   | Constraints | Description |
|-------|--------|-------------|-------------|
| 0     | string | 64 lowercase hex characters. MUST be a valid BIP-340 x-only public key. MUST NOT equal `pk`. | Owner's public key. |
| 1     | string | UTF-8 string. MAY be empty. If non-empty, clauses separated by `&` per NIP-OA. | NIP-OA conditions string. |
| 2     | string | 128 lowercase hex characters. | BIP-340 Schnorr signature by the owner key. |

This mirrors the NIP-OA `auth` tag format (elements 1–3; the `"auth"` label is
omitted since the field name `oa` already identifies it).

#### Verification

To verify the owner attestation:

1. Confirm `oa` is an array of exactly 3 strings. If not, reject the entire
   signature (`ERRSIG`). Structurally malformed envelopes are always rejected
   to prevent malleability — the `oa` field is part of the signed hash
   preimage, so its structure must be valid.

2. Validate the owner pubkey (index 0): 64 lowercase hex, valid BIP-340 key,
   not equal to `pk`. If invalid, reject (`ERRSIG`).

3. Compute the NIP-OA signing preimage:
   ```
   preimage = "nostr:agent-auth:" || pk || ":" || conditions
   ```
   Where `pk` is the signer's pubkey from the `pk` field (the agent), and
   `conditions` is the string at index 1 (may be empty).

4. Compute `hash = SHA-256(preimage)`.

5. Verify the BIP-340 Schnorr signature at index 2 over `hash` against the
   owner pubkey at index 0.

6. If verification succeeds, the attestation is valid: the owner key authorized
   the agent key. If conditions are non-empty, evaluate them per NIP-OA rules
   (noting that `kind=` and `created_at` conditions reference Nostr event
   fields and are not meaningful for git commits — see below).

7. If verification fails, the NIP-GS commit signature (`sig`) may still be
   valid, but the owner attestation is invalid. Verifiers SHOULD report the
   commit as signed but the owner authorization as failed/unverified.

#### Conditions in Git Context

NIP-OA conditions (`kind=<n>`, `created_at<t>`, `created_at>t`) reference Nostr
event fields that do not exist in git commits. For git commit signing:

- An **empty conditions string** (unconditional authorization) is RECOMMENDED.
  It means "this owner authorized this agent key" with no constraints.

- If conditions are present, verifiers SHOULD evaluate only the conditions they
  can meaningfully check. `created_at<t` and `created_at>t` MAY be evaluated
  against the signature timestamp `t` from the NIP-GS envelope as a reasonable
  approximation, but this is not required.

- `kind=<n>` conditions have no git equivalent and SHOULD be ignored by git
  signature verifiers.

Signing programs SHOULD use auth tags with empty conditions for git signing.

#### Trust Display

When displaying verification results for a commit with a valid `oa` field:

- The commit is "signed by `<pk>`" (the agent).
- The commit is "authorized by `<owner-pubkey>`" (the owner).
- These MUST be displayed as distinct facts. The owner did not sign the commit;
  the owner authorized the key that signed the commit.

#### Immutability

The `oa` field is included in the NIP-GS signing hash (see Signing Hash). If
an attacker strips or modifies the `oa` field, the NIP-GS `sig` becomes
invalid. This means the signature envelope is immutable once signed — you
cannot downgrade an owner-authorized commit to an agent-only commit without
invalidating the entire signature.

This also means the signing program must know at signing time whether to
include `oa`. The `oa` field cannot be added after the fact.

#### Loading the Auth Tag

The signing program SHOULD load the NIP-OA auth tag from one of the following
sources, checked in order:

1. `BUZZ_AUTH_TAG` environment variable — a JSON-encoded array of 4 strings
   (`["auth", "<owner>", "<conditions>", "<sig>"]`). The program extracts
   elements 1–3 for the `oa` field.
2. `nostr.authtag` git config key — same JSON format.

If no auth tag is available, the `oa` field is omitted. This is not an error.

### Git Configuration

To enable nostr signing for commits and tags:

```ini
[gpg]
    format = x509
    x509.program = /path/to/git-sign-nostr
[commit]
    gpgsign = true
[tag]
    gpgsign = true
[user]
    signingkey = <hex-pubkey>
```

These MAY be set via `GIT_CONFIG_COUNT` / `GIT_CONFIG_KEY_*` /
`GIT_CONFIG_VALUE_*` environment variables for ephemeral, per-process
configuration (e.g., when spawning agent processes).

Implementations SHOULD use process-scoped configuration to avoid interfering
with the user's existing GPG or SSH signing setup.

**Warning:** `GIT_CONFIG_COUNT` replaces any parent-process `GIT_CONFIG_*`
variables entirely. Spawning processes that set `GIT_CONFIG_COUNT` MUST account
for any existing `GIT_CONFIG_*` variables they need to preserve.

### CLI Interface

Implementations MUST accept the following argument patterns:

| Pattern | Mode |
|---------|------|
| `--status-fd=<N>` or `--status-fd <N>` | File descriptor N for `[GNUPG:]` status output |
| `-bsau <key>` | Signing mode. `<key>` is the signing key identifier from `user.signingkey`. |
| `--verify <file> -` | Verification mode. `<file>` is the path to the detached signature file. |

Implementations SHOULD silently ignore unrecognized arguments for forward
compatibility with future git versions (e.g., `--keyid-format=long` from the
openpgp path, though x509 does not currently pass it).

If `--status-fd` is not provided, the program SHOULD write status lines to
stderr (fd 2) as a fallback. If the status fd is not writable, the program
SHOULD continue signing/verifying but skip status output. Git will treat the
absence of `SIG_CREATED` as a signing failure.

## Test Vectors

### Test Key

```
Secret key (hex): 0000000000000000000000000000000000000000000000000000000000000003
Public key (hex): f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9
```

### Test Payload

A minimal git commit object (170 bytes, no trailing LF):
```
tree 4b825dc642cb6eb9a060e54bf899d69f7cb46101
author Test User <test@example.com> 1700000000 +0000
committer Test User <test@example.com> 1700000000 +0000

Initial commit
```

Payload hex (170 bytes):
```
7472656520346238323564633634326362366562396130363065353462663839
39643639663763623436313031 0a 617574686f7220546573742055736572
203c74657374406578616d706c652e636f6d3e2031373030303030303030202b
30303030 0a 636f6d6d6974746572205465737420557365722 03c7465737440
6578616d706c652e636f6d3e2031373030303030303030202b30303030 0a0a
496e697469616c20636f6d6d6974
```

Note: the `0a` bytes are LF line endings within the commit object. There is no
trailing `0a` after `Initial commit`. Git pipes exactly these bytes to the
signing program's stdin.

### Signing Hash

```
Domain separator: "nostr:git:v1:" (13 bytes, hex: 6e6f7374723a6769743a76313a)
Timestamp:        1700000000
Timestamp ASCII:  "1700000000" (10 bytes)
Separator colon:  ":" (1 byte, hex: 3a)

Preimage: "nostr:git:v1:" || "1700000000" || ":" || <payload_bytes>
Preimage length: 13 + 10 + 1 + 170 = 194 bytes

SHA-256(preimage): a11a32173aa35125aaefaad8854f2eda5a144268a4a355905c841f79ff44aa18
```

Verification: any conforming implementation MUST produce this hash for the given
payload and timestamp.

### Deterministic Signature Vector

The following signature was produced using `sign_schnorr_no_aux_rand` (BIP-340
signing with auxiliary randomness set to 32 zero bytes). This is deterministic:
any implementation using the same nonce derivation MUST produce this exact
signature.

```
Signature (hex, 128 chars):
c35062148d95b820068c18ab9cf69a8dd2322c606890366d084df7617570b96b
7a1aca0a8fcabb2eb4032ebbdf5b43e6bf8633e0d85bcecce28a9e08705b875f
```

JSON (compact, no whitespace):
```
{"v":1,"pk":"f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9","sig":"c35062148d95b820068c18ab9cf69a8dd2322c606890366d084df7617570b96b7a1aca0a8fcabb2eb4032ebbdf5b43e6bf8633e0d85bcecce28a9e08705b875f","t":1700000000}
```

Base64:
```
eyJ2IjoxLCJwayI6ImY5MzA4YTAxOTI1OGMzMTA0OTM0NGY4NWY4OWQ1MjI5YjUzMWM4NDU4MzZmOTliMDg2MDFmMTEzYmNlMDM2ZjkiLCJzaWciOiJjMzUwNjIxNDhkOTViODIwMDY4YzE4YWI5Y2Y2OWE4ZGQyMzIyYzYwNjg5MDM2NmQwODRkZjc2MTc1NzBiOTZiN2ExYWNhMGE4ZmNhYmIyZWI0MDMyZWJiZGY1YjQzZTZiZjg2MzNlMGQ4NWJjZWNjZTI4YTllMDg3MDViODc1ZiIsInQiOjE3MDAwMDAwMDB9
```

Full armored output:
```
-----BEGIN SIGNED MESSAGE-----
eyJ2IjoxLCJwayI6ImY5MzA4YTAxOTI1OGMzMTA0OTM0NGY4NWY4OWQ1MjI5YjUzMWM4NDU4MzZmOTliMDg2MDFmMTEzYmNlMDM2ZjkiLCJzaWciOiJjMzUwNjIxNDhkOTViODIwMDY4YzE4YWI5Y2Y2OWE4ZGQyMzIyYzYwNjg5MDM2NmQwODRkZjc2MTc1NzBiOTZiN2ExYWNhMGE4ZmNhYmIyZWI0MDMyZWJiZGY1YjQzZTZiZjg2MzNlMGQ4NWJjZWNjZTI4YTllMDg3MDViODc1ZiIsInQiOjE3MDAwMDAwMDB9
-----END SIGNED MESSAGE-----
```

Verification: any conforming implementation MUST accept this signature for the
test key and payload above. Implementations using random auxiliary randomness
will produce different (but equally valid) signatures.

Expected signing status output:
```
[GNUPG:] BEGIN_SIGNING
[GNUPG:] SIG_CREATED D 8 1 00 1700000000 f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9
```

### Verification Status Output

For a valid signature where `pk` matches `user.signingkey`:
```
[GNUPG:] NEWSIG
[GNUPG:] GOODSIG f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9 f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9
[GNUPG:] VALIDSIG f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9 2023-11-14 1700000000 0 - - - - - f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9
[GNUPG:] TRUST_FULLY 0 shell
```

For a valid signature where `pk` does NOT match `user.signingkey`:
```
[GNUPG:] NEWSIG
[GNUPG:] GOODSIG f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9 f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9
[GNUPG:] VALIDSIG f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9 2023-11-14 1700000000 0 - - - - - f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9
[GNUPG:] TRUST_UNDEFINED 0 shell
```

For an invalid signature:
```
[GNUPG:] NEWSIG
[GNUPG:] BADSIG f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9 f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9
```

### Owner Attestation Test Vector

Using the same agent key (secret=`0x03`) and an owner key (secret=`0x01`):

```
Owner secret:  0000000000000000000000000000000000000000000000000000000000000001
Owner pubkey:  79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798
Agent pubkey:  f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9
Conditions:    "" (empty — unconditional authorization)
```

NIP-OA preimage:
```
"nostr:agent-auth:f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9:"
```

SHA-256 of preimage:
```
05113b24677b87bedf6498a3addad720003e6af36820e859a26814f149f5a837
```

Owner signature (deterministic, no aux randomness):
```
54b97dfd2b7d61c1bc1b5facab9d12a991fe0ac3dcb9044b3176f63bebb6f673
40eb0ad866f2d5568b78b58ba234ee9f490f8c41e64a949c200315801520ed25
```

NIP-GS signing hash (with `oa` binding):
```
Preimage: "nostr:git:v1:" || "1700000000" || ":" || oa[0] || ":" || oa[1] || ":" || oa[2] || ":" || payload
SHA-256:  b61f1658836a4f63a2d2f5d621014a064435dde0765dd9c1dc79c9530fe879f0
```

NIP-GS signature (deterministic, no aux randomness):
```
15592857980b8656ff50303d86acaffcbda397b9c0bb40aebd2fb87a723e466f
db1a74404d39f9eb7ac220b4f2e061f27523f1af24cbdf991cf42ff9b47034c0
```

Full JSON with `oa` (compact):
```json
{"v":1,"pk":"f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9","sig":"15592857980b8656ff50303d86acaffcbda397b9c0bb40aebd2fb87a723e466fdb1a74404d39f9eb7ac220b4f2e061f27523f1af24cbdf991cf42ff9b47034c0","t":1700000000,"oa":["79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798","","54b97dfd2b7d61c1bc1b5facab9d12a991fe0ac3dcb9044b3176f63bebb6f67340eb0ad866f2d5568b78b58ba234ee9f490f8c41e64a949c200315801520ed25"]}
```

Verification chain:
1. Verify `sig` over `SHA-256("nostr:git:v1:1700000000:" || oa_binding || payload)` against `pk` → commit signed by agent, `oa` is bound ✅
2. Verify `oa[2]` over `SHA-256("nostr:agent-auth:" || pk || ":" || oa[1])` against `oa[0]` → agent authorized by owner ✅

For a malformed signature (unparseable JSON, unknown version, etc.):
```
[GNUPG:] ERRSIG 0000000000000000 0 0 00 0 9
```

## Invalid Cases

Implementations MUST handle the following:

- Signature file with missing or malformed armor headers — `ERRSIG`, exit 1.
- Base64 content that does not decode — `ERRSIG`, exit 1.
- Decoded content that is not a JSON object — `ERRSIG`, exit 1.
- JSON with duplicate keys — `ERRSIG`, exit 1.
- JSON with `v` absent or not equal to integer `1` — `ERRSIG`, exit 1.
- JSON with unknown keys (for `v=1`, only `v`, `pk`, `sig`, `t`, `oa` are
  permitted) — `ERRSIG`, exit 1.
- `oa` present but not an array of exactly 3 strings — `ERRSIG`, exit 1.
  (Structurally malformed envelopes are always rejected to prevent
  malleability.)
- `oa[0]` (owner pubkey) equals `pk` (self-attestation) — `ERRSIG`, exit 1.
- `oa[2]` (owner signature) fails NIP-OA BIP-340 verification — the NIP-GS
  commit signature (`sig`) is still valid, but the owner attestation is
  invalid. Verifiers SHOULD report `GOODSIG` (the commit is signed) but
  display the owner authorization as failed/unverified.
- `pk` not exactly 64 lowercase hex characters — `ERRSIG`, exit 1.
- `pk` that is not a valid BIP-340 x-only public key (`lift_x` fails) — `ERRSIG`, exit 1.
- `sig` not exactly 128 lowercase hex characters — `ERRSIG`, exit 1.
- `t` not an integer, or outside range 0–4294967295 — `ERRSIG`, exit 1.
- Decoded JSON exceeding 2048 bytes — `ERRSIG`, exit 1.
- Payload exceeding 100 MB — the program MUST reject with an error on stderr.
- Secret key not available during signing — exit 1 with a diagnostic on stderr.
  MUST NOT write to stdout (git interprets any stdout as signature data).
- Signing key argument (`-u <key>`) does not match loaded key — exit 1 with
  diagnostic on stderr.

## Security Considerations

### Domain Separation

The `nostr:git:v1:` prefix in the hash preimage ensures that a signature over a
git object cannot be replayed in another context. The timestamp is included in
the preimage so that `t` is cryptographically bound — tampering with `t`
invalidates the signature.

### Signer-Controlled Timestamp

The timestamp `t` is set by the signer and included in the signed hash, so it
cannot be altered by a third party. However, the signer can choose any value —
including past or future timestamps. The `t` field represents a *claimed*
signing time, not a verified one. Applications that require trusted timestamps
SHOULD cross-reference `t` with the commit's `author` and `committer`
timestamps, or with external timestamping services.

### Replay Across Repositories

A signed git object (commit or tag) is valid wherever it appears. If the same
commit object is cherry-picked or grafted into another repository, the signature
remains valid. This is intentional and consistent with how GPG-signed commits
behave in git. The signature attests "this key signed this content at this time"
— not "this content belongs in this repository."

### Key Compromise

This NIP provides no built-in key revocation or expiration mechanism. If a
signer's secret key is compromised:

- All past signatures remain valid. There is no way to retroactively invalidate
  them within this protocol.
- The attacker can sign arbitrary commits as the compromised identity.
- Applications SHOULD implement out-of-band revocation (e.g., publishing a
  revocation event on Nostr relays, updating allowed-signer lists) and SHOULD
  NOT rely solely on commit signatures for authorization decisions.

### Key Exposure via Environment Variables

The signing program reads secret keys from environment variables
(`NOSTR_PRIVATE_KEY`, `BUZZ_PRIVATE_KEY`). Environment variables are visible
to:

- The process itself and its children.
- On Linux, any process that can read `/proc/<pid>/environ` (same UID or root).
- Shell history if the variable was set inline (e.g., `NOSTR_PRIVATE_KEY=... git commit`).
- CI/CD logs if the variable is echoed or logged.

This exposure model is acceptable for agent processes running in a controlled
environment (e.g., spawned by a desktop app with process-scoped env vars). For
human users with higher security requirements, implementations MAY support
NIP-46 (Nostr Remote Signing) in a future version.

Implementations MUST NOT log, print, or include the secret key in error messages.

### Signing Program Trust

The signing program path is configured via `gpg.x509.program`. A malicious
program at that path could steal the secret key or produce fraudulent signatures.
Implementations that inject this configuration (e.g., desktop apps spawning
agents) SHOULD use absolute paths resolved from trusted locations (e.g., Tauri
sidecar binaries) and SHOULD NOT rely on `PATH` resolution in untrusted
environments.

Repository-local `.gitconfig` can override `gpg.x509.program`. Users SHOULD be
aware that cloning an untrusted repository could redirect signing to a malicious
program if `include.path` or `safe.directory` settings allow it.

### Nonce Generation

BIP-340 §4 specifies nonce generation using auxiliary randomness. Poor nonce
generation (e.g., reusing a nonce across two different messages) can expose the
private key. Implementations MUST use a cryptographically secure random number
generator for auxiliary randomness, or use a deterministic nonce derivation
scheme that is provably secure (e.g., RFC 6979 adapted to BIP-340).

### Envelope Immutability

The `oa` field and all other envelope metadata are included in the NIP-GS
signing hash. Any modification to the JSON envelope — adding, removing, or
changing fields — invalidates the `sig`. This prevents:

- **Stripping attacks**: removing `oa` to downgrade from "owner-authorized" to
  "agent-only."
- **Injection attacks**: adding a fraudulent `oa` to claim false authorization.
- **Whitespace attacks**: adding spaces or newlines to the JSON to create a
  different git commit hash while keeping the signature valid.

Because the signature envelope is embedded in the git commit object (in the
`gpgsig` header), and the git commit hash covers the entire object, any change
to the envelope also changes the commit hash. By binding the envelope contents
to the NIP-GS signature, we ensure that a valid signature corresponds to exactly
one commit hash.

### Identity Binding

A verified nostr commit signature proves "this secp256k1 key signed this git
object." It does NOT prove:

- The signer is a specific person (that requires out-of-band identity
  verification, e.g., NIP-05).
- The signer is authorized to commit to this repository (that requires
  application-level access control).
- The commit content is trustworthy (that requires code review).

Applications that display signature status SHOULD make these distinctions clear
to users.

### Denial of Service

The 2048-byte JSON limit, 4096-byte base64 limit, and 100 MB payload limit
bound resource consumption during verification. The base64 decoding step is bounded by the JSON limit.
Implementations SHOULD also bound the time spent on BIP-340 verification (a
single verification is fast, but a malicious actor could craft many signed
objects).

## Relationship to Other NIPs

| NIP | Relationship |
|-----|-------------|
| NIP-01 | Nostr event signing uses the same secp256k1 keys but different hash preimages (domain separation). |
| NIP-34 | Git repository metadata and patches. This NIP adds commit-level signatures to NIP-34 workflows. |
| NIP-98 | HTTP auth for git transport. NIP-98 authenticates the pusher; this NIP authenticates the committer. They are complementary. |
| NIP-OA | Owner attestation. The optional `oa` field embeds a NIP-OA credential in the signature envelope, proving the agent was authorized by an owner. With empty conditions, this is pure key-to-key identity binding. |
| NIP-46 | Remote signing. Future implementations MAY delegate signing to a NIP-46 bunker, keeping the secret key on a separate device. |

## Kind Usage

This NIP does not define any Nostr event kinds. Signatures are embedded in git
objects, not published to relays.

## Backwards Compatibility

This NIP introduces no changes to existing Nostr event kinds, relay behavior, or
git protocols. It uses only git's standard pluggable signing program interface.
Repositories signed with this NIP are readable by any git client — unsigned
clients simply see unverified signatures. The `-----BEGIN SIGNED MESSAGE-----`
armor markers are recognized by git's x509 signature detection and will not
collide with PGP or SSH signatures.

## References

- [BIP-340](https://github.com/bitcoin/bips/blob/master/bip-0340.mediawiki) — Schnorr Signatures for secp256k1
- [Git `gpg-interface.c`](https://github.com/git/git/blob/master/gpg-interface.c) — Git's signing program interface
- [sigstore/gitsign](https://github.com/sigstore/gitsign) — Prior art for `gpg.format=x509` custom signing programs
- [NIP-01](https://github.com/nostr-protocol/nips/blob/master/01.md) — Basic protocol
- [NIP-34](https://github.com/nostr-protocol/nips/blob/master/34.md) — Git stuff
- [NIP-98](https://github.com/nostr-protocol/nips/blob/master/98.md) — HTTP Auth
- [RFC 4648](https://datatracker.ietf.org/doc/html/rfc4648) — Base Encodings
