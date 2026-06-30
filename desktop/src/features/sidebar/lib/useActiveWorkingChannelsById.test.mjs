import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { resolveActiveWorkingChannelNames } from "./useActiveWorkingChannelsById.ts";

describe("resolveActiveWorkingChannelNames", () => {
  it("resolves active agent pubkeys to managed agent names", () => {
    const resolved = resolveActiveWorkingChannelNames(
      {
        channelId: "chan-1",
        anchorAt: 0,
        agentCount: 2,
        agentPubkeys: ["AAAA", "bbbb"],
      },
      [
        { pubkey: "aaaa", name: "Ned" },
        { pubkey: "BBBB", name: "Bart" },
      ],
    );

    assert.deepEqual(resolved.agentNames, ["Ned", "Bart"]);
  });
});
