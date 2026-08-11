import { describe, expect, it } from "vitest";
import { nip19 } from "nostr-tools";
import { formatNpub, truncatePubkey } from "./pubkey";

const HEX = "ea9b4d7a7a78a3e3729e5568b14d764d4962be0e1f20f749bcf8d9dbbf9a9328";
const NPUB = "npub1a2d567n60z37xu57245tzntkf4yk90swrus0wjdulrvah0u6jv5qusyp60";

describe("public-key display", () => {
  it("uses npub for full and compact displays", () => {
    expect(formatNpub(HEX)).toBe(NPUB);
    expect(formatNpub(NPUB)).toBe(NPUB);
    expect(truncatePubkey(HEX)).toBe("npub1a2d56…5qusyp60");
  });

  it("fails closed instead of echoing malformed API values", () => {
    expect(formatNpub("invalid-pubkey")).toBe("Invalid public key");
    expect(formatNpub("a".repeat(63))).toBe("Invalid public key");
  });

  it("rejects public-key bytes that cannot lift to secp256k1", () => {
    const invalidPoint = "ff".repeat(32);
    expect(formatNpub(invalidPoint)).toBe("Invalid public key");
    expect(formatNpub(nip19.npubEncode(invalidPoint))).toBe(
      "Invalid public key",
    );
  });
});
