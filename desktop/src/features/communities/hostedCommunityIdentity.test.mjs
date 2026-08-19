import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { npubEncode } from "nostr-tools/nip19";

import { compareHostedCommunityIdentity } from "./hostedCommunityIdentity.ts";

const LOCAL_HEX =
  "ea9b4d7a7a78a3e3729e5568b14d764d4962be0e1f20f749bcf8d9dbbf9a9328";
const LOCAL_NPUB =
  "npub1a2d567n60z37xu57245tzntkf4yk90swrus0wjdulrvah0u6jv5qusyp60";
const OTHER_HEX =
  "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";
const OTHER_NPUB = npubEncode(OTHER_HEX);

describe("compareHostedCommunityIdentity", () => {
  it("matches an npub-only Builderlab identity to the local signing key", () => {
    assert.deepEqual(
      compareHostedCommunityIdentity({ npub: LOCAL_NPUB }, LOCAL_HEX),
      {
        boundNpub: LOCAL_NPUB,
        localNpub: LOCAL_NPUB,
        identityMismatch: false,
      },
    );
  });

  it("detects an npub-only Builderlab identity bound to another account", () => {
    assert.deepEqual(
      compareHostedCommunityIdentity({ npub: OTHER_NPUB }, LOCAL_HEX),
      {
        boundNpub: OTHER_NPUB,
        localNpub: LOCAL_NPUB,
        identityMismatch: true,
      },
    );
  });

  it("fails closed when a present bound identity cannot be canonicalized", () => {
    for (const identity of [
      { npub: "not-an-npub" },
      { pubkey_hex: "ff".repeat(32) },
      { npub: "not-an-npub", pubkey_hex: LOCAL_HEX },
      {},
    ]) {
      const comparison = compareHostedCommunityIdentity(identity, LOCAL_HEX);
      assert.equal(comparison.boundNpub, null);
      assert.equal(comparison.identityMismatch, true);
    }
  });

  it("falls back to a legacy pubkey_hex identity response", () => {
    assert.deepEqual(
      compareHostedCommunityIdentity({ pubkey_hex: LOCAL_HEX }, LOCAL_NPUB),
      {
        boundNpub: LOCAL_NPUB,
        localNpub: LOCAL_NPUB,
        identityMismatch: false,
      },
    );
  });

  it("does not report a mismatch before Builderlab returns an identity", () => {
    assert.deepEqual(compareHostedCommunityIdentity(null, LOCAL_HEX), {
      boundNpub: null,
      localNpub: LOCAL_NPUB,
      identityMismatch: false,
    });
  });
});
