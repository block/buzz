import assert from "node:assert/strict";
import test from "node:test";

import { pendingMcpAppPostInvalidationReason } from "./useChannelMcpAppExperience.tsx";
import { runInitialMcpAppTool } from "../ui/McpAppFrame.tsx";

function bridgeCalls() {
  const calls = [];
  return {
    calls,
    bridge: {
      async sendToolCancelled(value) {
        calls.push(["cancelled", value]);
      },
      async sendToolInput(value) {
        calls.push(["input", value]);
      },
      async sendToolResult(value) {
        calls.push(["result", value]);
      },
    },
  };
}

test("rejects a pending app post when channel access is revoked", () => {
  assert.equal(
    pendingMcpAppPostInvalidationReason("channel-1", {
      archivedAt: null,
      id: "channel-1",
      isMember: false,
    }),
    "The channel became read-only before the app post was approved.",
  );
  assert.equal(
    pendingMcpAppPostInvalidationReason("channel-1", {
      archivedAt: 123,
      id: "channel-1",
      isMember: true,
    }),
    "The channel became read-only before the app post was approved.",
  );
});

test("sends one terminal result when the initial tool succeeds", async () => {
  const { bridge, calls } = bridgeCalls();
  const lifecycle = { started: false, terminalSent: false };
  await runInitialMcpAppTool(
    bridge,
    "server-1",
    { name: "board.open", arguments: { project: "launch" } },
    lifecycle,
    async (serverId, name) => ({
      content: [{ type: "text", text: `${serverId}:${name}` }],
    }),
  );
  assert.equal(lifecycle.started, true);
  assert.equal(lifecycle.terminalSent, true);
  assert.deepEqual(
    calls.map(([kind]) => kind),
    ["input", "result"],
  );
});

test("sends one terminal cancellation when the initial tool fails", async () => {
  const { bridge, calls } = bridgeCalls();
  const lifecycle = { started: false, terminalSent: false };
  await assert.rejects(
    runInitialMcpAppTool(
      bridge,
      "server-1",
      { name: "board.open", arguments: {} },
      lifecycle,
      async () => {
        throw new Error("remote failed");
      },
    ),
    /remote failed/,
  );
  assert.equal(lifecycle.terminalSent, true);
  assert.deepEqual(
    calls.map(([kind]) => kind),
    ["input", "cancelled"],
  );
});
