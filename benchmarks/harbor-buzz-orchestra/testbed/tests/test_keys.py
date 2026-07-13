"""Keygen and NIP-OA attestation unit tests."""

from __future__ import annotations

import hashlib
import json

import coincurve
from harbor_buzz_testbed.keys import (
    compute_auth_tag,
    encode_nsec,
    generate_keypair,
)

# Produced by the Rust reference implementation
# (crates/buzz-sdk/examples/compute_auth_tag.rs) for owner secret 0x...03 and
# agent pubkey "a" * 64. Pins the preimage format across implementations.
RUST_OWNER_SECRET = "0" * 63 + "3"
RUST_AGENT_PUBKEY = "a" * 64
RUST_TAG = [
    "auth",
    "f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9",
    "",
    "20105c618d6e5d8f559cffb6f0d7a7b4f44f3a567e1be94c96378d45ac3625da"
    "34c2e7357ea1d3ce980978334546b3e740c155e81b833ebe140d519d39ed8867",
]


def preimage_digest(agent_pubkey: str, conditions: str) -> bytes:
    return hashlib.sha256(
        f"nostr:agent-auth:{agent_pubkey}:{conditions}".encode()
    ).digest()


def test_generate_keypair_is_fresh_and_hex():
    first, second = generate_keypair(), generate_keypair()
    assert first.secret_key != second.secret_key
    assert first.pubkey != second.pubkey
    assert len(first.secret_key) == 64
    assert len(first.pubkey) == 64
    int(first.secret_key, 16)
    int(first.pubkey, 16)


def test_auth_tag_shape_and_owner_pubkey():
    tag = json.loads(compute_auth_tag(RUST_OWNER_SECRET, RUST_AGENT_PUBKEY))
    assert tag[0] == "auth"
    assert tag[1] == RUST_TAG[1]  # same owner pubkey as the Rust implementation
    assert tag[2] == ""


def test_auth_tag_signature_verifies_over_nip_oa_preimage():
    agent = generate_keypair()
    tag = json.loads(compute_auth_tag(RUST_OWNER_SECRET, agent.pubkey))
    owner_pubkey = coincurve.PublicKeyXOnly(bytes.fromhex(tag[1]))
    assert owner_pubkey.verify(bytes.fromhex(tag[3]), preimage_digest(agent.pubkey, ""))


def test_rust_reference_tag_verifies_under_python_preimage():
    """The Rust-signed vector must verify against our preimage construction."""
    owner_pubkey = coincurve.PublicKeyXOnly(bytes.fromhex(RUST_TAG[1]))
    assert owner_pubkey.verify(
        bytes.fromhex(RUST_TAG[3]), preimage_digest(RUST_AGENT_PUBKEY, "")
    )


def test_encode_nsec_matches_nip19_vector():
    # NIP-19 reference vector from the spec.
    assert (
        encode_nsec("67dea2ed018072d675f5415ecfaed7d2597555e202d85b3d65ea4e58d2d92ffa")
        == "nsec1vl029mgpspedva04g90vltkh6fvh240zqtv9k0t9af8935ke9laqsnlfe5"
    )
