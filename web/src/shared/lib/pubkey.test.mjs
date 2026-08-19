import assert from "node:assert/strict";
import test from "node:test";
import { nip19 } from "nostr-tools";

import { formatNpub, pubkeyAvatarLabel, truncatePubkey } from "./pubkey.ts";

const HEX = "ea9b4d7a7a78a3e3729e5568b14d764d4962be0e1f20f749bcf8d9dbbf9a9328";
const NPUB = "npub1a2d567n60z37xu57245tzntkf4yk90swrus0wjdulrvah0u6jv5qusyp60";

test("public-key displays use npub for protocol hex", () => {
  assert.equal(formatNpub(HEX), NPUB);
  assert.equal(formatNpub(NPUB), NPUB);
  assert.equal(truncatePubkey(HEX), "npub1a2d…yp60");
  assert.equal(pubkeyAvatarLabel(HEX), "a2");
});

test("malformed values fail closed instead of echoing raw input", () => {
  assert.equal(formatNpub("invalid-pubkey"), "Invalid public key");
  assert.equal(formatNpub("a".repeat(63)), "Invalid public key");
});

test("off-curve public-key bytes fail closed in hex and npub form", () => {
  const invalidPoint = "ff".repeat(32);
  assert.equal(formatNpub(invalidPoint), "Invalid public key");
  assert.equal(
    formatNpub(nip19.npubEncode(invalidPoint)),
    "Invalid public key",
  );
});
