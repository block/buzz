import assert from "node:assert/strict";
import test from "node:test";

import { parseChannelMcpAppStore } from "./channelMcpAppStorage.ts";

test("parses only complete MCP App installations", () => {
  assert.deepEqual(
    parseChannelMcpAppStore({
      version: 1,
      channels: {
        channel: [
          {
            id: "board",
            endpoint: "https://runtime.example/mcp",
            serverName: "Runtime",
            toolName: "board.open",
            title: "Board",
            resourceUri: "ui://runtime/board",
            arguments: { space: "alpha" },
          },
          {
            id: "bad",
            endpoint: "https://runtime.example/mcp",
            toolName: "bad.open",
            resourceUri: "https://runtime.example/app",
          },
        ],
      },
    }),
    {
      version: 1,
      channels: {
        channel: [
          {
            id: "board",
            endpoint: "https://runtime.example/mcp",
            serverName: "Runtime",
            toolName: "board.open",
            title: "Board",
            resourceUri: "ui://runtime/board",
            arguments: { space: "alpha" },
          },
        ],
      },
    },
  );
});

test("rejects unknown store versions", () => {
  assert.equal(parseChannelMcpAppStore({ version: 2, channels: {} }), null);
});
