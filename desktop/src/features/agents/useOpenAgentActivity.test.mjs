import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  isChannelOpenable,
  resolveOpenableActivityChannelId,
} from "./useOpenAgentActivity.ts";

describe("isChannelOpenable", () => {
  it("allows joined channels regardless of visibility", () => {
    assert.equal(
      isChannelOpenable({ isMember: true, visibility: "private" }),
      true,
    );
    assert.equal(
      isChannelOpenable({ isMember: true, visibility: "open" }),
      true,
    );
  });

  it("allows open channels the viewer hasn't joined (read-only)", () => {
    assert.equal(
      isChannelOpenable({ isMember: false, visibility: "open" }),
      true,
    );
  });

  it("rejects private channels the viewer hasn't joined", () => {
    assert.equal(
      isChannelOpenable({ isMember: false, visibility: "private" }),
      false,
    );
  });

  it("rejects channels missing from the viewer's channel list", () => {
    assert.equal(isChannelOpenable(undefined), false);
  });
});

describe("resolveOpenableActivityChannelId", () => {
  it("prefers the first openable working channel", () => {
    assert.equal(
      resolveOpenableActivityChannelId({
        agentChannelIds: ["member-1"],
        openableChannelIds: new Set(["working-2", "member-1"]),
        workingChannelIds: ["working-1", "working-2"],
      }),
      "working-2",
    );
  });

  it("falls back to the agent's first openable member channel", () => {
    assert.equal(
      resolveOpenableActivityChannelId({
        agentChannelIds: ["hidden-2", "member-1"],
        openableChannelIds: new Set(["member-1"]),
        workingChannelIds: ["hidden-1"],
      }),
      "member-1",
    );
  });

  it("returns null when the agent is only active in inaccessible rooms", () => {
    assert.equal(
      resolveOpenableActivityChannelId({
        agentChannelIds: ["hidden-2"],
        openableChannelIds: new Set(["unrelated"]),
        workingChannelIds: ["hidden-1"],
      }),
      null,
    );
  });

  it("returns null with no candidate channels at all", () => {
    assert.equal(
      resolveOpenableActivityChannelId({
        agentChannelIds: [],
        openableChannelIds: new Set(),
        workingChannelIds: [],
      }),
      null,
    );
  });
});
