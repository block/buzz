NIP-AB
======

Device Pairing
--------------

`draft` `optional`

## Versions

This NIP is versioned to allow future algorithm upgrades without breaking existing implementations.

Currently defined versions:

| Version | Status | Description |
|---------|--------|-------------|
| `1` | Active | secp256k1 ECDH, HKDF-SHA256, SAS-6digit, NIP-44 v2 encryption |

The version is communicated in two places:

1. **QR URI**: `nostrpair://<pubkey>?secret=<hex>&relay=<url>&v=1`
   - The `v` parameter defaults to `1` if absent (backward compatibility).
   - _target_ MUST reject URIs with an unrecognized `v` value and display a human-readable error: "This QR code requires a newer version of [App]. Please update."

2. **Offer message**: the `offer` JSON MUST include a `version` field:
   ```jsonc
   {
     "type": "offer",
     "version": 1,
     "session_id": "<hex, 32 bytes>"
   }
   ```
   _source_ MUST reject offers with a `version` it does not support.

Implementations MUST NOT silently ignore an unrecognized version — they MUST surface an error to the user.

This NIP defines a protocol for securely transferring secrets between two devices over standard Nostr relays using QR-code-initiated, end-to-end encrypted channels with visual confirmation.

## Motivation

Users need their Nostr identity on multiple devices. Today the options are:

- Paste a raw `nsec` — insecure, no authentication, no encryption in transit
- Use [NIP-46](46.md) remote signing — requires the signer device to be online for every operation
- Enter a [NIP-06](06.md) mnemonic — manual, error-prone, not all clients support it

NIP-46 solves *ongoing delegation*: the key stays on one device and signs remotely. This NIP solves *one-time transfer*: the key moves to the new device, which then operates independently. They are complementary — this NIP can even bootstrap a NIP-46 session as one of its payload types.

This NIP provides a secure, authenticated channel between two devices that can carry any secret payload — a private key, a [NIP-46](46.md) session bootstrap, or application-specific data — without trusting the relay.

## Terminology

- **source**: The device that holds the secret and initiates pairing (e.g., a desktop app).
- **target**: The device that wants to receive the secret (e.g., a mobile phone).
- **pairing relay**: Any [NIP-01](01.md) compliant relay used to route pairing events. The relay learns nothing about the payload.
- **session secret**: A 32-byte random value shared via QR code, used to derive encryption keys.
- **SAS (Short Authentication String)**: A short code displayed on both devices for the user to visually confirm, preventing man-in-the-middle attacks.

## Overview

1. _source_ generates an ephemeral keypair and a session secret, encodes them in a QR code.
2. _target_ scans the QR code, generates its own ephemeral keypair.
3. Both devices connect to the pairing relay and exchange ephemeral public keys via `kind:24134` events.
4. Both devices derive a shared secret via ECDH and display a SAS code for the user to confirm.
5. After confirmation, _source_ sends the encrypted payload via a `kind:24134` event.
6. _target_ decrypts and imports the payload.

All events use ephemeral keypairs that are discarded after the session. The relay sees only opaque ciphertext addressed to throwaway public keys.

## Limitations

This NIP provides a secure one-time transfer channel. It does not provide:

- **No ongoing security**: once the payload is transferred, this NIP's security guarantees end. The transferred key's security depends entirely on the receiving device's storage and the user's operational security.
- **No key revocation**: there is no mechanism to invalidate a completed pairing. If the _target_ device is later compromised, the transferred key is compromised.
- **No multi-device coordination**: this NIP transfers a key to one device at a time. Managing keys across N devices requires N separate pairing sessions.
- **No relay confidentiality**: the pairing relay learns the timing and approximate frequency of pairing events, even though it cannot read the payload. For high-risk users, a private relay is recommended.
- **No post-quantum security**: the ECDH key exchange is vulnerable to a sufficiently powerful quantum computer. The NIP-44 encryption layer inherits the same limitation.
- **Physical presence assumption**: SAS verification requires the user to visually compare codes on two physical screens. An attacker with physical access to both devices simultaneously can bypass this check.
- **QR code window**: the session secret is exposed in the QR code for up to 120 seconds. Screen capture, shoulder surfing, or a compromised camera can expose it.
- **Single-use only**: this protocol is not designed for repeated or automated transfers. Each transfer requires a new QR scan and user confirmation.

For ongoing remote signing without key transfer, use [NIP-46](46.md) instead.

## QR Code Format

The _source_ generates:

- An ephemeral secp256k1 keypair (`source_ephemeral_privkey`, `source_ephemeral_pubkey`)
- A 32-byte cryptographically random `session_secret`

The QR code encodes a URI:

```
nostrpair://<source_ephemeral_pubkey_hex>?secret=<session_secret_hex>&relay=<wss://relay.example.com>&v=1
```

- `source_ephemeral_pubkey_hex`: 64-character lowercase hex-encoded 32-byte x-only public key (as used throughout Nostr per [BIP-340](https://github.com/bitcoin/bips/blob/master/bip-0340.mediawiki))
- `session_secret_hex`: 64-character lowercase hex-encoded 32 random bytes
- `relay`: percent-encoded WebSocket URL of the pairing relay. MUST appear at least once. MAY appear multiple times (see §Multi-Relay Considerations).
- `v`: protocol version integer (see §Versions). Defaults to `1` if absent.

The total URI length MUST NOT exceed 2048 characters. Reject any URI that exceeds this limit (prevents DoS via QR scanning).

Implementations MUST validate the QR URI before processing:
- `source_ephemeral_pubkey_hex` MUST be exactly 64 lowercase hex characters (32 bytes). Reject if not.
- `session_secret_hex` MUST be exactly 64 lowercase hex characters (32 bytes). Reject if not.
- `relay` MUST be a valid WebSocket URL beginning with `wss://` or `ws://`. Reject if not.
- Implementations MUST NOT process a `nostrpair://` URI that fails any of the above checks.

Both _source_ and _target_ connect to the relay specified in the QR URI. If the relay is unreachable, the session MUST be aborted. There is no relay discovery mechanism; the QR code is the authoritative relay list.

The QR code MUST NOT contain any private key material. If intercepted, an attacker obtains only an ephemeral public key and a session secret, which are useless without completing the handshake within the session timeout.

Clients MAY support additional query parameters for forward compatibility. Unknown parameters MUST be ignored.

## Event Kind

All pairing messages use a single event kind:

```
kind: 24134
```

This kind is in the ephemeral event range. Relays SHOULD treat these events as ephemeral and MAY delete them after delivery or after a short TTL (e.g., 5 minutes). Relays do not need any special handling for this kind — standard NIP-01 event routing is sufficient.

## Event Structure

All `kind:24134` events follow this structure:

```jsonc
{
  "id": "<sha256 hash per NIP-01>",
  "pubkey": "<sender's ephemeral pubkey>",
  "kind": 24134,
  "content": "<NIP-44 encrypted JSON>",
  "tags": [["p", "<recipient's ephemeral pubkey>"]],
  "created_at": <unix timestamp>,
  "sig": "<schnorr signature per NIP-01>"
}
```

The `content` field is always encrypted using **NIP-44 version 2** (the `0x02` algorithm: secp256k1 ECDH, HKDF, padding, ChaCha20, HMAC-SHA256), as specified in [NIP-44](44.md). The conversation key is derived from the sender's ephemeral private key and the recipient's ephemeral public key. Implementations MUST use NIP-44 v2 and MUST reject events whose NIP-44 version byte is not `0x02`.

NIP-AB does not negotiate encryption versions. If a future NIP-44 version is required, this NIP will be updated with a new version indicator. Implementations MUST NOT silently fall back to an older NIP-44 version.

The encrypted plaintext is always a JSON object containing a `type` field that identifies the message:

```jsonc
{
  "type": "<message_type>",
  // ... type-specific fields
}
```

Message types are: `offer`, `sas-confirm`, `payload`, `complete`, `abort`.

There are no unencrypted type indicators in tags or other visible fields. The relay sees only the `p` tag (an ephemeral pubkey with no link to any real identity) and opaque ciphertext.

## Event Validation

Before processing any `kind:24134` event, implementations MUST:

1. Validate the event `id` and `sig` per [NIP-01](01.md).
2. Validate that `pubkey` is a valid, non-zero secp256k1 curve point per [BIP-340](https://github.com/bitcoin/bips/blob/master/bip-0340.mediawiki).
3. Validate that the event contains a `p` tag whose value matches the local device's ephemeral public key. This guards against misdelivery by a malicious or buggy relay.
4. Validate that `pubkey` matches the expected peer for the current session state:
   - _source_ expects events from `target_ephemeral_pubkey` (learned from the first valid `offer`).
   - _target_ expects events from `source_ephemeral_pubkey` (learned from the QR code).
   - Before the first valid `offer`, _source_ accepts events from any `pubkey` (since `target_ephemeral_pubkey` is not yet known), but MUST lock to that pubkey after accepting.
5. Decrypt `content` per [NIP-44](44.md). The `content` field MUST be a valid NIP-44 v2 payload (base64, 132–87472 characters per NIP-44). Events with `content` outside this range MUST be silently discarded.
6. Parse the decrypted JSON and validate the `type` field against the expected message for the current state.
7. **Out-of-order messages**: A message whose `type` does not match the expected message for the current protocol state is considered out-of-order. Out-of-order messages MUST be silently discarded; the session state MUST NOT advance. Implementations MUST NOT send an `abort` in response to an out-of-order message, as doing so would allow a relay to probe session state.

   The valid `type` for each state is:

   | State | Role | Expected `type` |
   |-------|------|-----------------|
   | `Waiting` | Source | `offer` |
   | `Confirming` | Source | *(awaiting user; no inbound expected)* |
   | `Confirming` | Target | `sas-confirm` |
   | `AwaitingConfirmation` | Target | `payload` *(buffer until user confirms SAS; do not process until state advances to `Transferring`)* |
   | `Transferring` | Target | `payload` |
   | `PayloadExchanged` | Source | `complete` |

   `abort` is valid in any non-terminal state from a known peer (see §Abort). All other combinations are out-of-order and MUST be discarded.

Events that fail any validation step MUST be silently discarded. Implementations MUST NOT reveal validation failure details to the relay or to the sender.

### Duplicate Event Handling

Relays MAY deliver the same event more than once (e.g., on reconnect or when multiple relay connections are active). Implementations MUST handle duplicate delivery idempotently.

An event is a duplicate if its `id` matches an event already successfully processed in the current session. Implementations MUST track the `id` of each successfully processed event and MUST silently discard any event whose `id` has already been processed.

Implementations SHOULD maintain a per-session set of processed event IDs. This set need not persist beyond the session lifetime (120 seconds maximum).

A duplicate `offer` event (same `id`) received after the source has already accepted an offer MUST be discarded, not treated as a new session attempt. A duplicate `payload` event received after the target has already imported the payload MUST be discarded; the target MUST NOT re-import or re-send `complete`.

## Pairing Protocol

### Step 1: Source Subscribes

After displaying the QR code, _source_ subscribes to the pairing relay for events tagged to its ephemeral public key:

```json
["REQ", "<sub_id>", {"kinds": [24134], "#p": ["<source_ephemeral_pubkey>"]}]
```

### Step 2: Target Sends Offer

_target_ scans the QR code, generates its own ephemeral secp256k1 keypair (`target_ephemeral_privkey`, `target_ephemeral_pubkey`), and publishes an `offer` event:

```jsonc
{
  "kind": 24134,
  "pubkey": "<target_ephemeral_pubkey>",
  "content": "<NIP-44 encrypted>",
  "tags": [["p", "<source_ephemeral_pubkey>"]],
  "created_at": <unix_timestamp>,
  // id, sig per NIP-01
}
```

Encrypted plaintext:

```jsonc
{
  "type": "offer",
  "version": 1,
  "session_id": "<hex, 32 bytes>"
}
```

Where `session_id` is derived as:

```
session_id = HKDF-SHA256(
    IKM  = session_secret,   // 32 bytes from QR code
    salt = "",               // empty
    info = "nostr-pair-session-id",
    L    = 32
)
```

The `session_id` proves the _target_ possesses the QR code's `session_secret` without revealing the secret on the wire.

_source_ MUST verify the `session_id` matches its own derivation. _source_ MUST accept at most one valid `offer` per session. After accepting an offer, _source_ MUST ignore all subsequent `offer` events and MUST record `target_ephemeral_pubkey` as the only valid peer for the remainder of the session.

### Step 3: SAS Verification

Both devices now have each other's ephemeral public keys. Both compute:

```
ecdh_shared = ECDH(own_ephemeral_privkey, other_ephemeral_pubkey)
```

Where `ecdh_shared` is the 32-byte x-coordinate of the shared point (unhashed), as produced by standard secp256k1 scalar multiplication.

Then:

```
sas_input = HKDF-SHA256(
    IKM  = ecdh_shared,       // 32 bytes
    salt = session_secret,    // 32 bytes from QR code
    info = "nostr-pair-sas-v1",
    L    = 32
)

sas_code = be_u32(sas_input[0..4]) mod 1000000
```

Where `be_u32(bytes)` interprets the first 4 bytes of `sas_input` as a big-endian unsigned 32-bit integer.

Both devices display the `sas_code` as a zero-padded 6-digit decimal string (e.g., `"047291"`). The user MUST visually confirm the codes match on both screens before proceeding.

**UX requirement**: The confirmation prompt MUST clearly state what is being authorized. Example: *"You are about to transfer your Nostr identity to another device. Does your other device show: **047291**?"* with prominent Confirm and Deny buttons. If the user denies the SAS on either device, that device MUST immediately send `abort` with reason `"user_denied"`, discard all session state, and terminate the session. SAS denial is the primary MITM defense — implementations MUST NOT allow the protocol to continue after a denial.

After the user confirms on the _source_ device, _source_ publishes a `sas-confirm` event:

```jsonc
{
  "kind": 24134,
  "pubkey": "<source_ephemeral_pubkey>",
  "content": "<NIP-44 encrypted>",
  "tags": [["p", "<target_ephemeral_pubkey>"]],
  // ...
}
```

Encrypted plaintext:

```jsonc
{
  "type": "sas-confirm",
  "transcript_hash": "<hex, 32 bytes>"
}
```

Where `transcript_hash` binds the confirmation to the full session transcript:

```
transcript = session_id
           || source_ephemeral_pubkey   // 32 bytes, x-coordinate
           || target_ephemeral_pubkey   // 32 bytes, x-coordinate
           || sas_input                 // 32 bytes

transcript_hash = HKDF-SHA256(
    IKM  = transcript,                  // 128 bytes
    salt = session_secret,
    info = "nostr-pair-transcript-v1",
    L    = 32
)
```

_target_ MUST compute the same `transcript_hash` and verify it matches before proceeding. Implementations MUST use constant-time comparison when checking `transcript_hash` to prevent timing side-channels. A mismatch indicates session inconsistency or parameter tampering; _target_ MUST send `abort` with reason `"sas_mismatch"`, discard any payload received in this session, and terminate. Note: because _source_ sends the payload immediately after `sas-confirm` (without waiting for an acknowledgment), the payload may already be in transit or delivered when the mismatch is detected. The transcript hash is a **detection** mechanism, not a prevention gate — MITM prevention relies on the user's visual SAS comparison on the _source_ device *before* the source confirms and sends the payload.

After verifying the transcript hash, _target_ enters the `AwaitingConfirmation` state. _target_ transitions to `Transferring` when the user confirms the SAS on the target device. _target_ MUST NOT import, process, or act on the secret material within any received `payload` event until **both** the transcript hash has been verified **and** the user has confirmed the SAS on the target device. (Implementations may NIP-44-decrypt the event content to validate the message `type` for state-machine routing. However, implementations MUST NOT deserialize, extract, log, persist, or act on the `payload` field within a `payload`-type message until both conditions are met. If early decryption is used, the decrypted content MUST be treated as opaque for all purposes other than `type` classification, and MUST be zeroized if the session is aborted before dual consent. The safest implementation strategy — and the one closest to the formal proof — is to buffer the raw NIP-44 ciphertext and defer all decryption until after dual consent.)

### Step 4: Payload Transfer

After the user confirms the SAS on the _source_ device, _source_ publishes the `sas-confirm` event (Step 3) followed immediately by a `payload` event:

Encrypted plaintext:

```jsonc
{
  "type": "payload",
  "payload_type": "<string>",
  "payload": "<string>"
}
```

Defined payload types:

| `payload_type` | Description | `payload` format |
|----------------|-------------|------------------|
| `nsec` | Private key transfer | [NIP-49](49.md) `ncryptsec1...` string (recommended) or `nsec1...` bech32 |
| `bunker` | NIP-46 signer-initiated session | `bunker://...` URI as defined in [NIP-46](46.md) |
| `connect` | NIP-46 client-initiated session | `nostrconnect://...` URI as defined in [NIP-46](46.md) |
| `custom` | Application-specific data | String (see §Custom Payloads) |

**Payload size limits**: The total serialized JSON plaintext of a `kind:24134` event's decrypted content MUST NOT exceed 65,535 bytes (the NIP-44 v2 plaintext limit). For `payload` messages, this means the `payload` field plus JSON envelope overhead (typically 50–80 bytes depending on `payload_type` and JSON escaping) must fit within this limit. In practice, `payload` values up to 65,400 bytes are safe. Implementations MUST reject (silently discard) `payload` events where the decrypted plaintext JSON exceeds 65,535 bytes.

For the defined payload types (`nsec`, `bunker`, `connect`), payloads are expected to be well under 1,024 bytes. Implementations MAY enforce a stricter limit of 4,096 bytes for these types and SHOULD document any custom limit for `custom` payloads.

_Source_ implementations MUST NOT construct a `payload` event whose plaintext JSON exceeds 65,535 bytes; doing so will cause NIP-44 encryption to fail.

### Custom Payloads

The `custom` payload type carries application-defined data. The `payload` field MUST be a string. Applications that need to transfer structured data SHOULD encode it as JSON and then serialize the JSON object to a string (i.e., JSON-in-string, consistent with Nostr convention).

To prevent cross-application misinterpretation, applications using `custom` payloads SHOULD include an application identifier in the payload. The RECOMMENDED format is:

```jsonc
{
  "type": "payload",
  "payload_type": "custom",
  "payload": "{\"app\":\"com.example.myapp\",\"version\":1,\"data\":\"...\"}"
}
```

The `app` field SHOULD use reverse-DNS notation to namespace the payload. Implementations that receive a `custom` payload with an unrecognized `app` value SHOULD surface this to the user rather than silently discarding it.

`custom` payloads are subject to the general 65,535-byte plaintext limit (65,400 bytes is a safe practical bound for the `payload` field). Applications SHOULD document their expected payload size. Applications with payloads larger than 4,096 bytes SHOULD consider whether NIP-AB is the appropriate transport — NIP-AB is designed for short secrets, not bulk data transfer.

NIP-AB does not provide a mechanism for _target_ to reject a `custom` payload based on its content. If _target_ does not understand the payload, it SHOULD send `complete` with `success: false` and inform the user.

For `nsec` payloads using [NIP-49](49.md) `ncryptsec` format, clients SHOULD set `KEY_SECURITY_BYTE = 0x02` (client does not track provenance) unless the client can positively assert the key has never been handled insecurely, in which case `0x01` MAY be used.

### Step 5: Completion

_target_ decrypts the payload, imports the secret into secure storage, and SHOULD publish a `complete` event:

```jsonc
{ "type": "complete", "success": true }
```

**`complete` is advisory, not required for security.** The payload transfer is complete when _target_ successfully decrypts and stores the payload. `complete` is a best-effort acknowledgment that allows _source_ to display a success confirmation to the user.

**If _target_ crashes or disconnects after importing but before sending `complete`**: The import has succeeded. _target_ MUST NOT re-request the payload. On next launch, _target_ SHOULD display a success state (the key is present in storage). _source_ will time out waiting for `complete` and MAY display an ambiguous state ("Transfer may have succeeded — check your other device").

**`success: false`**: _target_ SHOULD send `complete` with `success: false` if it successfully received and decrypted the payload but failed to import it into secure storage (e.g., keychain write failed). This allows _source_ to inform the user of a partial failure. _source_ MUST NOT retry sending the payload in response to `success: false` — the session is over. The user must initiate a new pairing.

**Source timeout for `complete`**: _source_ SHOULD wait up to 30 seconds for `complete` after sending `payload`. If `complete` is not received within this window, _source_ SHOULD display an ambiguous confirmation ("Transfer sent — verify on your other device") rather than an error. _source_ MUST NOT re-send `payload`.

_source_ MUST process at most one `complete` event per session. Subsequent `complete` events MUST be silently discarded.

Both devices MUST close their subscriptions and discard their ephemeral keypairs after either (a) receiving `complete`, (b) the per-step timeout expires, or (c) the session timeout (120 seconds) expires. Implementations MUST zero the ephemeral private keys, session secret, and any decrypted payload plaintext from memory before freeing. On the _target_ side, the decrypted payload MUST be zeroed from working memory once it has been committed to platform-secure storage.

### Implementation Pseudocode

The following Python-like pseudocode is normative. Implementations MUST produce identical outputs for identical inputs.

```python
# --- Key Derivation ---

def derive_session_id(session_secret: bytes) -> bytes:
    # session_secret: 32 bytes from QR code
    assert len(session_secret) == 32
    return hkdf_sha256(IKM=session_secret, salt=b"", info=b"nostr-pair-session-id", L=32)

def derive_sas_input(ecdh_shared: bytes, session_secret: bytes) -> bytes:
    # ecdh_shared: 32-byte x-coordinate of secp256k1 shared point (unhashed)
    assert len(ecdh_shared) == 32
    assert len(session_secret) == 32
    return hkdf_sha256(IKM=ecdh_shared, salt=session_secret, info=b"nostr-pair-sas-v1", L=32)

def derive_sas_code(sas_input: bytes) -> str:
    # Returns zero-padded 6-digit decimal string
    n = int.from_bytes(sas_input[0:4], byteorder='big')
    return str(n % 1_000_000).zfill(6)

def derive_transcript_hash(
    session_id: bytes,
    source_pubkey: bytes,   # 32-byte x-coordinate
    target_pubkey: bytes,   # 32-byte x-coordinate
    sas_input: bytes,
    session_secret: bytes
) -> bytes:
    assert all(len(x) == 32 for x in [session_id, source_pubkey, target_pubkey, sas_input, session_secret])
    transcript = session_id + source_pubkey + target_pubkey + sas_input  # 128 bytes
    return hkdf_sha256(IKM=transcript, salt=session_secret, info=b"nostr-pair-transcript-v1", L=32)

# --- Message Encryption (wraps NIP-44) ---

def encrypt_message(msg: dict, sender_privkey: bytes, recipient_pubkey: bytes) -> str:
    # msg: dict with "type" field and type-specific fields
    plaintext = json_encode(msg)  # UTF-8 JSON, no trailing whitespace
    conversation_key = nip44_get_conversation_key(sender_privkey, recipient_pubkey)
    nonce = secure_random_bytes(32)
    return nip44_encrypt(plaintext, conversation_key, nonce)

def decrypt_message(ciphertext: str, recipient_privkey: bytes, sender_pubkey: bytes) -> dict:
    conversation_key = nip44_get_conversation_key(recipient_privkey, sender_pubkey)
    plaintext = nip44_decrypt(ciphertext, conversation_key)
    return json_decode(plaintext)

# --- Usage example ---
# session_secret = secure_random_bytes(32)
# session_id = derive_session_id(session_secret)
# ecdh_shared = secp256k1_ecdh(own_privkey, peer_pubkey)  # x-coordinate, unhashed
# sas_input = derive_sas_input(ecdh_shared, session_secret)
# sas_code = derive_sas_code(sas_input)  # display to user, e.g. "047291"
# transcript_hash = derive_transcript_hash(session_id, source_pub, target_pub, sas_input, session_secret)

# --- Transcript Verification (target side) ---
# After receiving sas-confirm:
# expected = derive_transcript_hash(session_id, source_pub, target_pub, sas_input, session_secret)
# if not constant_time_equal(received_hash, expected):
#     discard_buffered_payload()  # payload may have arrived early
#     send_abort(reason="sas_mismatch")
#     raise TranscriptMismatchError
```

### Abort

Either device MAY send an `abort` message at any point during the protocol:

Encrypted plaintext:

```jsonc
{
  "type": "abort",
  "reason": "<string>"
}
```

Defined reason strings:

| `reason` | Meaning |
|----------|---------|
| `"sas_mismatch"` | SAS codes did not match, or transcript hash verification failed |
| `"user_denied"` | User explicitly denied the pairing |
| `"timeout"` | Session timed out |
| `"protocol_error"` | Local fatal condition (e.g., internal state corruption, unrecoverable implementation error). MUST NOT be sent in response to a peer's out-of-order or validation-failing event — those MUST be silently discarded per §Event Validation. |

Upon receiving an `abort`, the other device MUST terminate the session, discard ephemeral keys, and inform the user. Implementations MAY define additional reason strings; unknown reasons SHOULD be treated as `"protocol_error"`.

## Protocol Diagram

```
  Source (Desktop)                    Relay                     Target (Phone)
  ────────────────                    ─────                     ───────────────
  Generate ephemeral keypair
  Generate session_secret
  Display QR code
  Subscribe: kind:24134
  #p: source_ephemeral_pubkey ──────►
                                                               Scan QR code
                                                               Generate ephemeral keypair
                                      ◄─────────────────────── Publish offer
                                                               {type:"offer", session_id}
  ◄──────────────────────────────────
  Validate sig, pubkey, session_id
  Accept offer, lock to this peer
  Compute SAS code ◄─────────────────────────────────────────► Compute SAS code
  Display: "047291"                                            Display: "047291"

  [User confirms SAS on source]

  Publish sas-confirm ──────────────►
  {type:"sas-confirm",                ──────────────────────►  Verify transcript_hash
   transcript_hash}
  Publish payload ──────────────────►  (sent immediately;
  {type:"payload",                     source does not wait
   payload_type:"nsec",                for target)
   payload:"ncryptsec1..."}           ──────────────────────►  Buffer payload

                                                               [User confirms SAS on target]

                                                               Decrypt payload
                                                               Import to secure storage
                                      ◄─────────────────────── Publish complete
  ◄──────────────────────────────────                          {type:"complete"}

  Discard ephemeral keys                                       Discard ephemeral keys
  Zero session_secret                                          Zero session_secret
```

## Security Considerations

### Man-in-the-Middle Attacks

An attacker who intercepts the QR code (e.g., by photographing the screen or creating a fake QR code) could attempt to race the legitimate _target_ and establish their own session. The SAS verification step prevents this: the attacker's ECDH shared secret will differ from the legitimate pair, producing a different SAS code. The user will observe mismatched codes and abort.

This is the same defense used by Matrix (emoji verification), Bluetooth Secure Simple Pairing, and ZRTP. Signal's device linking omitted SAS verification and was subsequently exploited by state-level attackers who created fake QR codes to silently link unauthorized devices.

Clients MUST display an unambiguous confirmation prompt. The prompt MUST explicitly state what is being authorized and display the SAS code prominently with a clear option to deny.

### Relay Compromise

A compromised relay can:
- **Drop events** (denial of service) — mitigated by session timeout and retry with alternate relays
- **Delay events** — mitigated by session timeout
- **Attempt MITM** — defeated by SAS verification (relay does not possess ephemeral private keys)

A compromised relay **cannot**:
- Read the payload (NIP-44 encrypted with ECDH keys the relay does not possess)
- Forge events (events are signed by ephemeral keys; signatures are validated before processing)
- Correlate pairing sessions with real user identities (ephemeral keys are unlinked to real identities)

### QR Code Exposure

The QR code contains only an ephemeral public key and a session secret. If an attacker captures the QR code and races the legitimate _target_ to send the first `offer`, the _source_ will accept the attacker's offer and compute a SAS using the attacker's ephemeral key. However:

1. The _source_ displays a SAS code derived from the ECDH shared secret with the attacker.
2. The user's physical phone (the legitimate _target_) either (a) failed to connect (if the attacker's offer was accepted first) and shows an error, or (b) is not displaying any SAS code at all.
3. The user observes that their phone does not show the expected SAS code and denies the pairing on the _source_.

The defense is **user verification against their physical device**, not cryptographic impossibility. This is the same security model as Bluetooth Secure Simple Pairing and ZRTP: the SAS step converts a network-level MITM into a physical-presence requirement.

The _source_ MUST reject additional `offer` events after accepting one. If the legitimate _target_'s offer arrives after an attacker's, the _target_ will receive no response and SHOULD time out.

### Session Timeout

Implementations MUST enforce a session timeout (recommended: 120 seconds from QR display). After timeout, the _source_ MUST discard the ephemeral keypair and session secret. A new QR code MUST be generated for a new attempt.

### Key Material on Two Devices

After an `nsec` transfer, the private key exists on both devices. This is an inherent tradeoff of key transfer versus remote signing ([NIP-46](46.md)). Clients MUST store imported keys in platform-secure storage (iOS Keychain, Android Keystore, OS-level credential managers).

### Replay Protection

Session secrets are random and single-use. Ephemeral keypairs are generated per session. Two independent mechanisms prevent cross-session replay:

**1. `p` tag binding**: Every event carries a `p` tag containing the recipient's ephemeral public key. The recipient validates that this tag matches their own ephemeral public key (§Event Validation, step 3). A replayed event from session A has `p` = `source_A_ephemeral_pubkey`; session B's source has a different ephemeral key and will reject it at the `p` tag check, before any decryption is attempted.

**2. NIP-44 key binding**: Even if the `p` tag check were bypassed, NIP-44 decryption would fail. The conversation key is derived from `ECDH(own_ephemeral_privkey, sender_pubkey)`. A replayed event encrypted for session A's keypair cannot be decrypted by session B's keypair.

These two mechanisms are independent; either alone is sufficient to prevent cross-session replay. Together they provide defense in depth.

**Within-session replay**: The state machine provides within-session replay protection. Once a message type has been processed and the state has advanced, a replayed copy of the same message is out-of-order and MUST be discarded (§Event Validation, item 7). The duplicate event ID check (§Duplicate Event Handling) provides an additional layer.

### Metadata Privacy

All pairing events use ephemeral pubkeys that are unlinked to the user's real Nostr identity. The relay cannot determine which real user is pairing devices.

Implementations SHOULD set `created_at` to the current time minus a random value between 0 and 30 seconds. This provides metadata privacy (obscuring the exact time of each protocol step) while remaining within the timestamp acceptance window of all known relay implementations.

Implementations MUST NOT set `created_at` to a future time. Implementations MUST NOT set `created_at` more than 60 seconds in the past, as some relays enforce a `created_at_lower_limit` (per NIP-11) and may reject events with timestamps too far in the past.

If a relay rejects an event with an `invalid: event creation date` error (NIP-01 `OK` message), the implementation SHOULD retry with `created_at` set to the current time (no jitter). The privacy benefit of jitter is secondary to successful delivery.

## Design Rationale

### Why HKDF for `session_id` instead of a direct hash?

`session_id = HKDF(session_secret, ...)` rather than `SHA256(session_secret)` provides domain separation. Using HKDF with a labeled `info` string ensures that the `session_id` output is cryptographically independent from any other value derived from `session_secret` (e.g., `sas_input`). This prevents cross-protocol attacks where an attacker tricks one derivation path into producing a value valid for another.

### Why 6-digit decimal SAS?

6 decimal digits provide ~20 bits of entropy (10^6 = ~2^20). An attacker who can race the legitimate target has a 1-in-1,000,000 chance of a matching SAS per attempt. The session timeout (120 seconds) and single-offer acceptance limit make brute force infeasible. Decimal was chosen over emoji (Matrix) for cross-client compatibility — emoji sets vary by platform and font, causing display inconsistencies. Decimal was chosen over 4-digit (Bluetooth) because 4 digits (1-in-10,000) is considered insufficient against targeted attacks.

### Why `session_secret` in the QR code instead of deriving it from the ephemeral keypair?

The `session_secret` is independent of the ephemeral keypair. This means that even if an attacker somehow learns the ephemeral private key (e.g., via a side-channel), they cannot compute the `session_id` or `sas_input` without also knowing `session_secret`. The QR code is a separate out-of-band channel; requiring knowledge of both the QR code AND the ECDH handshake provides defense-in-depth for session establishment (offer authentication and SAS derivation). Note: the payload encryption key is derived purely from ECDH and does not depend on `session_secret`, so this defense-in-depth applies to the pairing handshake, not to payload confidentiality directly.

### Why transcript binding (`transcript_hash`)?

The `transcript_hash` in `sas-confirm` commits the source to the exact session parameters: the `session_id`, both ephemeral public keys, and the `sas_input`. This gives the _target_ a cryptographic consistency check that detects session inconsistency or parameter tampering. (Cross-session replay is already prevented independently by `p`-tag binding and NIP-44 key binding — see §Replay Protection.) The transcript hash is **not** the MITM prevention mechanism — that role belongs to the user's visual SAS comparison on the _source_ device, which gates whether `sas-confirm` and the payload are sent at all.

### Why NIP-44 for event encryption instead of a custom scheme?

NIP-44 is the Nostr standard for authenticated encryption. Using it here means NIP-AB inherits NIP-44's security audit, test vectors, and broad implementation support. A custom scheme would require separate review and implementation work in every client.

### Audit

An independent security audit of this protocol is planned. Until an audit is completed, implementations in high-security contexts should treat this NIP as `draft` and conduct their own review.

## Formal Verification

A Tamarin model of the protocol lives at [NIP-AB.spthy](NIP-AB.spthy). The model focuses on the security-critical core of the protocol:

- QR distribution of `session_secret` and `source_ephemeral_pubkey`
- `offer` authentication via possession of the QR secret
- SAS comparison as an explicit user-mediated gate
- `sas-confirm` transcript binding
- encrypted `payload` delivery
- advisory `complete` acknowledgment

The model treats the relay and network as a full **Dolev-Yao attacker**: the adversary can intercept, reorder, replay, drop, and fabricate messages. It also includes explicit compromise rules for:

- QR-code exposure (`session_secret` leaks out-of-band)
- source-session compromise
- target-session compromise

Under those assumptions, the proved lemmas are:

**Core security invariants:**

- **`executable_core_flow`** *(executability)*: the happy-path protocol completes — both sides reach `complete` with the same session and payload.
- **`payload_requires_successful_sas_match`** *(SAS gate)*: an honest source can only send `payload` after a successful SAS match.
- **`payload_secrecy_without_endpoint_compromise`** *(payload secrecy)*: the payload remains unknown to the attacker unless one endpoint session is compromised. QR-code exposure alone does not break secrecy, because the SAS gate pins delivery to an honest target-role execution in the model. (This assumes correct SAS verification — the model treats SAS comparison as perfect; the ~20-bit collision bound is a separate computational argument, see §Design Rationale.)
- **`target_completion_agrees_on_source_payload`** *(target agreement)*: under no-compromise assumptions, if the target completes, then the source previously sent that exact payload in the same session.
- **`source_completion_implies_prior_target_completion_without_compromise`** *(source completion soundness)*: under the same no-compromise assumptions, if the source accepts `complete`, the target previously sent `complete` for the same session. (The model abstracts away `success:true/false` semantics — this proves the `complete` event is authentic, not that import succeeded.)

- **`injective_target_source_agreement`** *(injective agreement, target → source)*: each target completion corresponds to a unique prior source payload send with the same `(sid, pkS, pkT, payload)`, and that send is itself unique. This is a one-directional injective mapping; the reverse (every send leads to a completion) is a liveness property not provable under Dolev-Yao scheduling.

**MITM resistance:**

- **`sas_match_implies_genuine_target`**: every SAS match is bound to a `pkT` that an honest target-role instance in the model actually generated (i.e., from `Target_Scan_QR_And_Send_Offer` with a fresh ephemeral). A network adversary that substitutes the offer's ephemeral key with an attacker-chosen value cannot cause the SAS-match rule to fire. This proves resistance to network key-substitution, not physical-device authenticity — the latter relies on the user's physical verification of the SAS code and is outside the symbolic model's scope.
- **`payload_delivery_requires_genuine_target`** *(composition)*: no payload is ever sent under a `pkT` that lacks a prior honest target-role execution. Follows from the SAS gate combined with the genuine-target lemma.

**Dual consent and payload buffering:**

- **`target_decrypts_payload_only_after_dual_consent`**: the target never decrypts the payload without **both** transcript verification **and** an explicit user-approval step. The model proves a stronger abstraction than the spec requires: payload plaintext is not made available to protocol logic before both conditions are met. (The spec permits implementations to NIP-44-decrypt the event content early for message-type classification, but the model conservatively defers all decryption — this is strictly stronger. Early type-field decryption on the target is a local operation that does not emit network-observable messages or alter protocol flow; since the Dolev-Yao attacker already possesses the ciphertext, local decryption reveals nothing new to the adversary, and all proved properties (secrecy, agreement, MITM resistance) hold a fortiori for the spec's more permissive buffering model.)
- **`decryption_requires_prior_buffering`**: every decryption is preceded by buffering — the intended two-phase flow (buffer ciphertext, then decrypt after approval) is explicit in the proof surface.
- **`executable_payload_buffered_before_approval`** *(sanity)*: the payload **can** arrive and be buffered before the target user approves, proving the buffering path is reachable and the dual-consent gate is not vacuously enforced by message ordering alone.

**Reachability and anti-vacuousness:**

- **`executable_with_qr_leak`**, **`executable_with_source_compromise`**, **`executable_with_target_compromise`**: each compromise rule is reachable from protocol state (i.e., the compromise rules are not dead code), so the no-compromise guards in the secrecy and agreement lemmas are non-trivial.
- **`source_compromise_can_leak_payload`**, **`target_compromise_can_leak_payload`**: there exist traces where endpoint compromise (leakage of session-ephemeral private keys) leads to attacker knowledge of the payload, confirming that the no-compromise guards in the secrecy lemma are load-bearing.

The Tamarin model intentionally abstracts away details that are orthogonal to the cryptographic proof:

- exact NIP-01 event IDs / Schnorr signatures — relay anti-forgery relies on these but is not proved symbolically
- exact NIP-44 ciphertext framing, padding, version bytes, and nonce handling — modeled as ideal authenticated encryption (`senc`) over a DH-derived key
- HKDF-SHA256 — collapsed to tagged hashes (`h(< label, inputs >)`) preserving domain separation but not RFC 5869 internals
- ECDH — modeled as symbolic Diffie-Hellman, not exact secp256k1 x-coordinate extraction
- SAS comparison — modeled as perfect (requiring actual key agreement); the ~20-bit collision bound (1/10^6) is a separate computational argument (see §Design Rationale)
- timeout and abort branches
- duplicate-event bookkeeping
- `p`-tag validation and within-session replay protection — these are state-machine / implementation requirements, not Tamarin results
- version negotiation (`version` field in `offer`)
- `complete` success/failure semantics
- payload typing (`nsec` / `bunker` / `connect` / `custom`)

Those behaviors remain normative in this document and in the Rust implementation; they are simply not the focus of the symbolic proof.

Run the proof with:

```bash
tamarin-prover --prove crates/sprout-core/src/pairing/NIP-AB.spthy
```

## Cryptographic Primitives

### ECDH

`secp256k1_ecdh(priv, pub)` is scalar multiplication of point `pub` by scalar `priv`, as defined in [BIP-340](https://github.com/bitcoin/bips/blob/master/bip-0340.mediawiki). The result is the shared point `P`; this function returns the 32-byte x-coordinate of `P` using BIP-340's `bytes(P)` encoding. The result is **not hashed**.

⚠️ **Implementation warning**: many secp256k1 libraries (including some bindings to libsecp256k1) hash the ECDH output with SHA-256 by default. This NIP requires the **unhashed** x-coordinate. Verify your library's behavior before shipping.

Private keys MUST be validated as scalars in range `[1, secp256k1_order - 1]`. Public keys MUST be validated as valid, non-zero curve points per BIP-340.

### HKDF-SHA256

[RFC 5869](https://datatracker.ietf.org/doc/html/rfc5869) with SHA-256.

- **Extract**: `PRK = HMAC-SHA256(salt, IKM)`. When `salt` is specified as `""` (empty string), use a zero-length byte array (not the string literal).
- **Expand**: `OKM = HKDF-Expand(PRK, info, L)` where `info` is the UTF-8 encoding of the specified string and `L` is the output length in bytes.

### Operators and Notation

- `||` denotes byte array concatenation with no length prefixes or delimiters.
- `x[i:j]` where `x` is a byte array returns bytes `i` (inclusive) through `j` (exclusive).
- `be_u32(x)` interprets the first 4 bytes of `x` as a big-endian unsigned 32-bit integer.

### Constants

| Name | Value | Description |
|------|-------|-------------|
| `SESSION_TIMEOUT` | 120 seconds | Maximum time from QR display to session completion |
| `STEP_TIMEOUT` | 30 seconds | Maximum time to wait for each protocol step |
| `SAS_DIGITS` | 6 | Number of decimal digits in SAS code |
| `SAS_MODULUS` | 1,000,000 | `10^SAS_DIGITS` |
| `SESSION_SECRET_LEN` | 32 bytes | Length of session secret |
| `MAX_URI_LEN` | 2048 characters | Maximum total length of the `nostrpair://` URI |
| `MAX_PAYLOAD_LEN` | 65,400 bytes | Safe practical maximum for the `payload` field (65,535-byte NIP-44 limit minus JSON envelope overhead) |

## Test Vectors

```
session_secret (hex):
  a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5b6c7d8e9f0a1b2

source_ephemeral_privkey (hex):
  7f4c11a9c9d1e3b5a7f2e4d6c8b0a2f4e6d8c0b2a4f6e8d0c2b4a6f8e0d2c4b5

source_ephemeral_pubkey (hex):
  199e64ca60662cb2d6e91d16cb065be51ad74a6ee5f8c5b0fdc53d246611ed9a

target_ephemeral_privkey (hex):
  3a5b7c9d1e3f5a7b9c1d3e5f7a9b1c3d5e7f9a1b3c5d7e9f1a3b5c7d9e1f3a5b

target_ephemeral_pubkey (hex):
  89a9fa762105d0aee2b19678246fe7b823aabbc4f4bf691a1ce8a70fcd36d6e4

session_id = HKDF-SHA256(IKM=session_secret, salt="", info="nostr-pair-session-id", L=32):
  fb357d0f8e8d5a5ba3b2a91cb18c119e1567b07ffa38cdebb73e68df78f5a380

ecdh_shared = ECDH(source_priv, target_pub) x-coordinate:
  9b4b6d6990713d89d6d9982e506ee1bbcde6f05c54d9d2978696e8a7274d4408

sas_input = HKDF-SHA256(IKM=ecdh_shared, salt=session_secret, info="nostr-pair-sas-v1", L=32):
  e8b03a329f3a0ac37fe7fbe929171e14b72812be67e33c5d6e193543c41798d3

sas_code = be_u32(sas_input[0..4]) mod 1000000:
  863346

transcript = session_id || source_pubkey || target_pubkey || sas_input  (128 bytes)

transcript_hash = HKDF-SHA256(IKM=transcript, salt=session_secret, info="nostr-pair-transcript-v1", L=32):
  d662818ff8911fc60a2d025f8b8b4756107104e85888dd202d28db5ca2cf28d3
```

Implementations MUST validate against these vectors. They can be reproduced with `sprout-pair test-vectors`.

A future external vector file (`nip-ab.vectors.json`) with a sha256 checksum committed in this document is planned. When published, it will include categorized intermediate-value vectors for each derivation step and negative/invalid test cases. The sha256 checksum will be the canonical commitment; implementations MUST verify against the checksum before using the file.

Implementations MUST also test rejection of invalid inputs. Examples of what to test:

- `session_secret` with wrong length (< 32 or > 32 bytes) → MUST be rejected
- `session_secret` that is all zeros → MUST be rejected
- `offer` with `session_id` that does not match the derived value → MUST be silently discarded
- `sas-confirm` with a mismatched `transcript_hash` → MUST trigger `abort` with reason `"sas_mismatch"`
- NIP-44 ciphertext with version byte ≠ `0x02` → MUST be silently discarded
- `content` field outside the 132–87472 character range → MUST be silently discarded
- decrypted plaintext JSON exceeding 65,535 bytes → MUST be silently discarded
- Duplicate event `id` within a session → MUST be silently discarded

## Implementation Notes

### Choosing a Pairing Relay

The _source_ encodes the relay URL in the QR code. Implementations MAY:
- Use the user's preferred relay from [NIP-65](65.md)
- Use a hardcoded default relay
- Allow the user to choose

The protocol is secure regardless of relay trustworthiness. For additional metadata privacy, a relay that supports [NIP-42](42.md) AUTH is preferred but not required.

### SAS Display

Implementations MUST display the SAS code as a zero-padded 6-digit decimal number (e.g., `047291`). Implementations MAY additionally display an emoji representation for improved usability, but the 6-digit decimal MUST always be shown as the canonical representation to ensure cross-client compatibility.

### Secure Storage

After importing a key, clients MUST store it in platform-secure storage:
- **iOS**: Keychain Services with `kSecAttrAccessibleWhenUnlockedThisDeviceOnly`
- **Android**: Android Keystore or EncryptedSharedPreferences
- **Desktop**: OS credential manager or encrypted keyring

### Error Handling

If _source_ receives an `offer` with an invalid `session_id`, it MUST silently ignore it and continue waiting for a valid offer (up to the session timeout).

If either device receives an event with an unexpected `type` for the current state, it MUST silently discard it (see §Event Validation, item 7 — out-of-order messages). Implementations MUST NOT send `abort` in response to an out-of-order message.

If either device does not receive the expected next message within a reasonable time (recommended: 30 seconds per step), it SHOULD send an `abort` with reason `"timeout"` and terminate the session.

### Concurrent Sessions

**Source**: A _source_ implementation MAY run multiple pairing sessions simultaneously. Each session MUST use a distinct ephemeral keypair and session secret, and therefore a distinct QR code. Sessions are fully independent — an event addressed to one session's ephemeral pubkey cannot affect another session. Implementations SHOULD limit the number of concurrent active sessions to a small number (recommended: 3) to prevent resource exhaustion.

**Target**: A _target_ implementation MAY scan multiple QR codes and run multiple pairing sessions simultaneously. Each session is independent. However, importing the same payload type (e.g., `nsec`) from two concurrent sessions is application-defined behavior; implementations SHOULD prompt the user to confirm each import individually.

**Session isolation**: Because each session uses independent ephemeral keypairs, there is no cryptographic interaction between concurrent sessions. A compromised or malicious session cannot affect the security of other sessions.

**UX recommendation**: Implementations SHOULD display each active session distinctly (e.g., by SAS code) so the user can match the correct QR code to the correct device.

## Multi-Relay Considerations

The QR URI format supports multiple `relay` parameters for redundancy. Multi-relay support is OPTIONAL — implementations that use a single relay are fully conformant. The guidance below is for implementations that choose to support multiple relays.

**Recommended relay count**: 1–3 relay URLs. More than 3 increases QR code size and connection overhead without proportional benefit.

**Source behavior**: _source_ SHOULD subscribe to **all** listed relays simultaneously. This ensures _target_ can reach _source_ regardless of which relay _target_ connects to first. Subscribing to all relays has no privacy cost since all events use ephemeral pubkeys.

**Target behavior**: _target_ SHOULD attempt to connect to listed relays in parallel and use the first relay that both (a) accepts the WebSocket connection and (b) successfully delivers the subscription (confirmed by receiving an `EOSE` or the first event). If a relay connection fails after the session is underway, _target_ MAY attempt the next relay in the list; however, _target_ MUST NOT construct a new `offer` event. If _target_ needs to reach _source_ via a different relay, _target_ SHOULD re-publish the **same signed `offer` event** (identical bytes, same event ID) to the new relay. This is safe because the event is already signed and addressed to `source_ephemeral_pubkey`; _source_ will deduplicate by event ID if it receives the offer on multiple relays.

**Cross-relay delivery**: Because _source_ subscribes to all listed relays, events published by _target_ to any listed relay will be received by _source_. The protocol is relay-agnostic: _source_ and _target_ do not need to be connected to the same relay simultaneously.

**Fallback**: If all listed relays fail, the session MUST be aborted. There is no relay discovery mechanism; the QR code is the authoritative relay list.

## Relation to Other NIPs

- [NIP-01](01.md): All pairing events are valid NIP-01 events.
- [NIP-44](44.md): Used for all encryption within pairing events.
- [NIP-46](46.md): This NIP can bootstrap a NIP-46 session via the `bunker` or `connect` payload types. NIP-46 provides ongoing remote signing; this NIP provides one-time secure transfer. They are complementary.
- [NIP-49](49.md): Recommended format for `nsec` payloads.
- [NIP-59](59.md): Gift Wrap uses ephemeral keys for metadata privacy; this NIP uses ephemeral keys for session isolation. Both demonstrate the pattern of throwaway Nostr identities for protocol-level operations.
