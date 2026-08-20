"""Nostr keygen and NIP-OA owner attestation for trial agents."""

from __future__ import annotations

import hashlib
import json
from dataclasses import dataclass

import coincurve


@dataclass(frozen=True, slots=True)
class NostrKeypair:
    secret_key: str  # hex
    pubkey: str  # x-only hex


def generate_keypair() -> NostrKeypair:
    """Generate a fresh secp256k1 keypair in Nostr hex form."""
    key = coincurve.PrivateKey()
    return NostrKeypair(
        secret_key=key.to_hex(),
        pubkey=key.public_key_xonly.format().hex(),
    )


def keypair_from_secret(secret_key: str) -> NostrKeypair:
    """Rebuild the keypair for an existing hex secret key."""
    key = coincurve.PrivateKey(bytes.fromhex(secret_key))
    return NostrKeypair(
        secret_key=secret_key,
        pubkey=key.public_key_xonly.format().hex(),
    )


_BECH32_CHARSET = "qpzry9x8gf2tvdw0s3jn54khce6mua7l"


def _bech32_polymod(values: list[int]) -> int:
    generator = (0x3B6A57B2, 0x26508E6D, 0x1EA119FA, 0x3D4233DD, 0x2A1462B3)
    checksum = 1
    for value in values:
        top = checksum >> 25
        checksum = (checksum & 0x1FFFFFF) << 5 ^ value
        for i in range(5):
            checksum ^= generator[i] if (top >> i) & 1 else 0
    return checksum


def encode_nsec(secret_key: str) -> str:
    """Encode a hex secret key as a NIP-19 bech32 ``nsec1…`` string —
    the form the desktop GUI's key-import onboarding accepts."""
    hrp = "nsec"
    data: list[int] = []
    accumulator = bits = 0
    for byte in bytes.fromhex(secret_key):
        accumulator = accumulator << 8 | byte
        bits += 8
        while bits >= 5:
            bits -= 5
            data.append(accumulator >> bits & 31)
    if bits:
        data.append(accumulator << (5 - bits) & 31)
    expanded = [ord(c) >> 5 for c in hrp] + [0] + [ord(c) & 31 for c in hrp]
    polymod = _bech32_polymod(expanded + data + [0] * 6) ^ 1
    checksum = [polymod >> 5 * (5 - i) & 31 for i in range(6)]
    return hrp + "1" + "".join(_BECH32_CHARSET[d] for d in data + checksum)


def compute_auth_tag(
    owner_secret_key: str, agent_pubkey: str, conditions: str = ""
) -> str:
    """Compute the NIP-OA ``["auth", ...]`` tag authorising an agent key.

    Mirrors crates/buzz-sdk/src/nip_oa.rs:
    sig = schnorr(SHA256("nostr:agent-auth:" || agent_pubkey || ":" || conditions),
    owner_secret_key). Returns the tag as a JSON string.
    """
    owner = coincurve.PrivateKey(bytes.fromhex(owner_secret_key))
    preimage = f"nostr:agent-auth:{agent_pubkey}:{conditions}".encode()
    signature = owner.sign_schnorr(hashlib.sha256(preimage).digest())
    return json.dumps(
        [
            "auth",
            owner.public_key_xonly.format().hex(),
            conditions,
            signature.hex(),
        ],
        separators=(",", ":"),
    )
