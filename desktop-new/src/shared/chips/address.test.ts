import { describe, expect, it } from "vitest";

import {
  AUTHORABLE_CHIP_KINDS,
  CHIP_KINDS,
  formatChipAddress,
  parseChipAddress,
  sameChipAddress,
  chipAddressKey,
} from "./address";

describe("chip address", () => {
  it("round-trips every kind through its canonical form", () => {
    for (const address of [
      { kind: "person" as const, id: "pk-morgan" },
      { kind: "agent" as const, id: "pk-vogue" },
      { kind: "channel" as const, id: "8f14e45f-ea11-4d3a-9f2c-1b7e5d830612" },
      { kind: "message" as const, id: "ev-9f2c4a1b7e5d8306" },
      // A link's id is a URL, punctuation and all.
      { kind: "link" as const, id: "https://github.com/block/buzz/pull/7366" },
    ]) {
      expect(parseChipAddress(formatChipAddress(address))).toEqual(address);
    }
  });

  /**
   * Every kind the component can draw must be a real address, or a chip could
   * render something that cannot be referenced.
   */
  it("accepts an address for every kind the system knows", () => {
    for (const kind of CHIP_KINDS) {
      expect(parseChipAddress(`buzz://${kind}/some-id`)).toEqual({
        kind,
        id: "some-id",
      });
    }
  });

  it("keeps the authorable kinds a subset of all kinds", () => {
    for (const kind of AUTHORABLE_CHIP_KINDS) {
      expect(CHIP_KINDS).toContain(kind);
    }
    // Message and link arrive by another route than typing a trigger.
    expect(AUTHORABLE_CHIP_KINDS).not.toContain("message");
    expect(AUTHORABLE_CHIP_KINDS).not.toContain("link");
  });

  it("refuses text that is not an address rather than guessing", () => {
    for (const value of [
      "@Morgan",
      "buzz://person/",
      "buzz://unknown/pk-morgan",
      "https://example.com/person/pk-morgan",
      "",
    ]) {
      expect(parseChipAddress(value)).toBeNull();
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

    expect(sameChipAddress(first, second)).toBe(false);
    expect(chipAddressKey(first)).not.toBe(chipAddressKey(second));
  });

  it("keeps kinds apart even when an id repeats across them", () => {
    const person = { kind: "person" as const, id: "shared-id" };
    const agent = { kind: "agent" as const, id: "shared-id" };

    expect(sameChipAddress(person, agent)).toBe(false);
    expect(formatChipAddress(person)).not.toBe(formatChipAddress(agent));
  });
});
