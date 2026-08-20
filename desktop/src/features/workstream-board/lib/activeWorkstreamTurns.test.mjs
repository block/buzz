import assert from "node:assert/strict";
import test from "node:test";

import { getActiveWorkstreamTurns } from "./activeWorkstreamTurns.ts";

const boardChannel = { id: "board", name: "loganj-ws-board" };
const otherBoardChannel = { id: "other-board", name: "loganj-ws-other" };
const turns = [
  {
    channelId: "board",
    anchorAt: 100,
    agentCount: 2,
    agentPubkeys: ["agent-a", "agent-b"],
  },
  {
    channelId: "general",
    anchorAt: 200,
    agentCount: 1,
    agentPubkeys: ["agent-c"],
  },
];

test("keeps the store's aggregate for board channels", () => {
  assert.deepEqual(getActiveWorkstreamTurns([boardChannel], turns), [turns[0]]);
});

test("does not project unrelated channel turns onto the board", () => {
  assert.deepEqual(
    getActiveWorkstreamTurns([boardChannel, otherBoardChannel], turns),
    [turns[0]],
  );
});
