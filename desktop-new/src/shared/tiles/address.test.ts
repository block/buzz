import { describe, expect, it } from "vitest";

import {
  formatTileAddress,
  parseTileAddress,
  sameTileAddress,
  tileAddressKey,
} from "./address";

describe("tile address", () => {
  it("round-trips every kind through its canonical form", () => {
    for (const address of [
      { kind: "person" as const, id: "pk-morgan" },
      { kind: "agent" as const, id: "pk-vogue" },
      { kind: "channel" as const, id: "8f14e45f-ea11-4d3a-9f2c-1b7e5d830612" },
    ]) {
      expect(parseTileAddress(formatTileAddress(address))).toEqual(address);
    }
  });

  it("refuses text that is not an address rather than guessing", () => {
    for (const value of [
      "@Morgan",
      "buzz://person/",
      "buzz://unknown/pk-morgan",
      "https://example.com/person/pk-morgan",
      "",
    ]) {
      expect(parseTileAddress(value)).toBeNull();
    }
  });

  /**
   * The property the whole model exists for. Two identities that share a
   * display name are different addresses, and nothing about the name enters
   * the comparison — so there is no same-name collision to disambiguate and no
   * need to write an identity into visible text.
   */
  it("distinguishes two identities that share a display name", () => {
    const first = { kind: "person" as const, id: "pk-morgan-1" };
    const second = { kind: "person" as const, id: "pk-morgan-2" };

    expect(sameTileAddress(first, second)).toBe(false);
    expect(tileAddressKey(first)).not.toBe(tileAddressKey(second));
  });

  it("keeps kinds apart even when an id repeats across them", () => {
    const person = { kind: "person" as const, id: "shared-id" };
    const agent = { kind: "agent" as const, id: "shared-id" };

    expect(sameTileAddress(person, agent)).toBe(false);
    expect(formatTileAddress(person)).not.toBe(formatTileAddress(agent));
  });
});
